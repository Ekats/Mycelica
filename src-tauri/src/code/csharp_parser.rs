//! C# parser using tree-sitter.
//!
//! Extracts:
//! - Classes, records (`record`, `record class`, `record struct` → `class`),
//!   structs, interfaces, enums, delegates (→ `type`), nested at any depth and
//!   inside block or file-scoped namespaces
//! - `const` fields (→ `const`)
//! - Methods, constructors, destructors, operators and conversion operators
//!   (→ `function`). The item *name* is the qualified, parameter-typed form
//!   `Outer.Inner.Method(int,string)`, so overloads hash to distinct node IDs;
//!   the title is the signature, which is what call resolution reads the bare
//!   name back out of.
//! - Properties, indexers and events whose accessors have a body, or that have
//!   an expression body (→ `function`): they carry outgoing calls only.
//!   Auto-properties and plain fields are not indexed.
//! - Call sites: `Foo()`, `recv.Foo()`, `Foo<T>()`, `recv?.Foo()` and
//!   `new Foo(...)`, collected from bodies, expression bodies, constructor
//!   initializer arguments, lambdas and local functions. A local function is
//!   not an item of its own; its calls belong to the enclosing member.
//!
//! Every slice of source text below is taken either from a tree-sitter node's
//! byte range (always on a character boundary) or from an already-split line,
//! never from a hand-computed offset — a byte index into the middle of a
//! multi-byte character panics the whole import (MCA-PANIC-01).

use std::path::Path;
use tree_sitter::{Node, Parser};

use super::types::{CallRef, CodeItem};

/// Longest signature kept verbatim before it is truncated.
const MAX_SIGNATURE_BYTES: usize = 160;

/// Parse a C# file and extract code items.
pub fn parse_csharp_file(path: &Path) -> Result<Vec<CodeItem>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .map_err(|e| format!("Failed to set language: {:?}", e))?;

    let tree = parser
        .parse(&content, None)
        .ok_or_else(|| "Failed to parse C#".to_string())?;

    let file_path = path.to_string_lossy().to_string();
    let lines: Vec<&str> = content.lines().collect();

    let mut items = Vec::new();
    let mut scope = Vec::new();
    extract_items(tree.root_node(), &file_path, &content, &lines, &mut items, &mut scope);

    Ok(items)
}

/// Recursively extract code items from the AST.
///
/// `scope` is the chain of types we are inside, outermost first. The last
/// one's bare name becomes `impl_for` on every member — what `pick_callee`
/// narrows a `Type.Method()` call against — and the chain of generic-qualified
/// names qualifies member names.
fn extract_items(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    items: &mut Vec<CodeItem>,
    scope: &mut Vec<Scope>,
) {
    let kind = node.kind();
    let item_type = match kind {
        "class_declaration" | "record_declaration" => Some("class"),
        "struct_declaration" => Some("struct"),
        "interface_declaration" => Some("interface"),
        "enum_declaration" => Some("enum"),
        "delegate_declaration" => Some("type"),
        _ => None,
    };

    if let Some(item_type) = item_type {
        let Some(bare) = node.child_by_field_name("name").and_then(|n| node_text(n, content)) else {
            return;
        };
        // `Wrapper<T>` and `Wrapper` in one file are two types; so are two
        // nested `Ping`s in different outer classes. The type parameters and
        // the enclosing type keep their IDs apart.
        let qualified = format!("{bare}{}", type_params(node, content));
        let enclosing = scope.last().map(|s| s.bare.clone());
        items.push(base_item(node, file_path, content, lines, qualified.clone(), item_type, None, enclosing));
        if let Some(body) = node.child_by_field_name("body") {
            scope.push(Scope { qualified, bare });
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                extract_items(child, file_path, content, lines, items, scope);
            }
            scope.pop();
        }
        return;
    }

    match kind {
        "method_declaration"
        | "constructor_declaration"
        | "destructor_declaration"
        | "operator_declaration"
        | "conversion_operator_declaration" => {
            if let Some(item) = extract_method(node, file_path, content, lines, scope) {
                items.push(item);
            }
        }
        "property_declaration" | "indexer_declaration" | "event_declaration" => {
            if let Some(item) = extract_property(node, file_path, content, lines, scope) {
                items.push(item);
            }
        }
        "field_declaration" => {
            if has_modifier(node, content, "const") {
                extract_consts(node, file_path, content, lines, scope, items);
            }
        }
        // Namespaces, `compilation_unit`, declaration lists, `#if` blocks:
        // containers whose children may be declarations.
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_items(child, file_path, content, lines, items, scope);
            }
        }
    }
}

/// One enclosing type: `qualified` (`Wrapper<T>`) builds member names, `bare`
/// (`Wrapper`) is what a call receiver can match as `impl_for`.
struct Scope {
    qualified: String,
    bare: String,
}

/// `<T,U>` from a declaration's type parameter list, or empty.
fn type_params(node: Node, content: &str) -> String {
    let Some(list) = child_of_kind(node, "type_parameter_list") else {
        return String::new();
    };
    let mut cursor = list.walk();
    let names: Vec<String> = list
        .children(&mut cursor)
        .filter(|c| c.kind() == "type_parameter")
        .filter_map(|c| c.child_by_field_name("name").and_then(|n| node_text(n, content)))
        .collect();
    format!("<{}>", names.join(","))
}

/// A `CodeItem` with the fields every declaration shares filled in.
#[allow(clippy::too_many_arguments)]
fn base_item(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    name: String,
    item_type: &str,
    signature: Option<String>,
    impl_for: Option<String>,
) -> CodeItem {
    let (line_start, line_end) = node_lines(node);
    CodeItem {
        name,
        item_type: item_type.to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: source_lines(lines, line_start, line_end),
        signature,
        visibility: visibility_of(node, content),
        doc_comment: extract_doc_comment(node, lines),
        is_async: has_modifier(node, content, "async"),
        is_unsafe: has_modifier(node, content, "unsafe"),
        impl_for,
        impl_trait: None,
        calls: Vec::new(),
    }
}

/// Method, constructor, destructor, operator, conversion operator.
fn extract_method(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    scope: &[Scope],
) -> Option<CodeItem> {
    let member = match node.kind() {
        "operator_declaration" => {
            let op = node.child_by_field_name("operator").and_then(|n| node_text(n, content))?;
            format!("operator {op}")
        }
        "conversion_operator_declaration" => {
            let ty = node.child_by_field_name("type").map(|n| collapse_ws(node_slice(n, content)))?;
            let dir = if has_child_kind(node, "explicit") { "explicit" } else { "implicit" };
            format!("{dir} operator {ty}")
        }
        "destructor_declaration" => {
            format!("~{}", node.child_by_field_name("name").and_then(|n| node_text(n, content))?)
        }
        // `Add<T>(X)` and `Add<T,U>(X)` are overloads with the same parameters.
        _ => format!(
            "{}{}",
            node.child_by_field_name("name").and_then(|n| node_text(n, content))?,
            type_params(node, content)
        ),
    };
    let params = node
        .child_by_field_name("parameters")
        .map(|p| param_types(p, content))
        .unwrap_or_default();
    let name = qualify(scope, &format!("{member}({params})"));

    let end = ["constructor_initializer", "block", "arrow_expression_clause"]
        .iter()
        .find_map(|k| child_of_kind(node, k))
        .map(|b| b.start_byte());
    let signature = header_signature(node, content, end);

    let mut item = base_item(node, file_path, content, lines, name, "function", Some(signature), scope.last().map(|s| s.bare.clone()));
    collect_calls(node, content, &mut item.calls);
    Some(item)
}

/// Property, indexer or event — only when it has code: an accessor with a
/// body, or an expression body. `int X { get; set; }` is data, not code.
fn extract_property(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    scope: &[Scope],
) -> Option<CodeItem> {
    let accessors = node.child_by_field_name("accessors");
    let value = node.child_by_field_name("value").filter(|v| v.kind() == "arrow_expression_clause");

    let accessor_names: Vec<String> = match (accessors, value) {
        (_, Some(_)) => vec!["get".to_string()],
        (Some(list), None) => {
            let mut cursor = list.walk();
            let decls: Vec<Node> = list
                .children(&mut cursor)
                .filter(|c| c.kind() == "accessor_declaration")
                .collect();
            if !decls.iter().any(|d| d.child_by_field_name("body").is_some()) {
                return None;
            }
            decls
                .iter()
                .filter_map(|d| d.child_by_field_name("name").and_then(|n| node_text(n, content)))
                .collect()
        }
        (None, None) => return None,
    };

    let member = if node.kind() == "indexer_declaration" {
        let params = node
            .child_by_field_name("parameters")
            .map(|p| param_types(p, content))
            .unwrap_or_default();
        format!("this[{params}]")
    } else {
        node.child_by_field_name("name").and_then(|n| node_text(n, content))?
    };
    let name = qualify(scope, &member);

    let end = accessors.or(value).map(|n| n.start_byte());
    let signature = truncated_signature(&format!(
        "{} {{ {} }}",
        header_signature(node, content, end),
        accessor_names.join("; ")
    ));

    let mut item = base_item(node, file_path, content, lines, name, "function", Some(signature), scope.last().map(|s| s.bare.clone()));
    collect_calls(node, content, &mut item.calls);
    Some(item)
}

/// Every declarator of a `const` field (`const int A = 1, B = 2;`).
///
/// `impl_for` records the owning type: two classes in one file each declaring
/// `const int Max` would otherwise hash to one node ID.
fn extract_consts(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    scope: &[Scope],
    items: &mut Vec<CodeItem>,
) {
    let Some(decl) = child_of_kind(node, "variable_declaration") else {
        return;
    };
    let mut cursor = decl.walk();
    for declarator in decl.children(&mut cursor) {
        if declarator.kind() != "variable_declarator" {
            continue;
        }
        if let Some(name) = declarator.child_by_field_name("name").and_then(|n| node_text(n, content)) {
            items.push(base_item(node, file_path, content, lines, name, "const", None, scope.last().map(|s| s.bare.clone())));
        }
    }
}

/// `Outer.Inner<T>.member`, or just `member` outside any type.
fn qualify(scope: &[Scope], member: &str) -> String {
    let mut parts: Vec<&str> = scope.iter().map(|s| s.qualified.as_str()).collect();
    parts.push(member);
    parts.join(".")
}

/// Parameter types, comma-joined with no spaces: `ref int,string,params T[]`.
/// Modifiers stay, because `F(ref int)` and `F(int)` are distinct overloads.
///
/// A `params` array is not a `parameter` node in this grammar: its type and
/// name sit directly on the list as the `type` / `name` fields.
fn param_types(list: Node, content: &str) -> String {
    let mut cursor = list.walk();
    let mut parts: Vec<String> = list
        .children(&mut cursor)
        .filter(|c| c.kind() == "parameter")
        .map(|p| {
            let mut words = Vec::new();
            let mut pc = p.walk();
            for child in p.children(&mut pc) {
                if child.kind() == "modifier" {
                    words.push(collapse_ws(node_slice(child, content)));
                }
            }
            if let Some(ty) = p.child_by_field_name("type") {
                words.push(collapse_ws(node_slice(ty, content)).replace(", ", ","));
            }
            words.join(" ")
        })
        .collect();
    if let Some(ty) = list.child_by_field_name("type") {
        parts.push(format!("params {}", collapse_ws(node_slice(ty, content)).replace(", ", ",")));
    }
    parts.join(",")
}

// ============== Calls ==============

/// Collect call sites anywhere under `node`. Attributes are skipped: their
/// arguments are compile-time constants, not calls made by this member.
fn collect_calls(node: Node, content: &str, calls: &mut Vec<CallRef>) {
    match node.kind() {
        "attribute_list" => return,
        "invocation_expression" => {
            if let Some(callee) = node.child_by_field_name("function") {
                push_callee(callee, content, calls);
            }
        }
        "object_creation_expression" => {
            // `new Foo(...)` / `new Ns.Foo<T>(...)` calls Foo's constructor.
            if let Some(ty) = node.child_by_field_name("type") {
                if let Some(name) = type_simple_name(ty, content) {
                    calls.push(CallRef { name, receiver: None, line: ty.start_position().row + 1 });
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls(child, content, calls);
    }
}

fn push_callee(callee: Node, content: &str, calls: &mut Vec<CallRef>) {
    let line = callee.start_position().row + 1;
    match callee.kind() {
        "identifier" | "generic_name" => {
            if let Some(name) = simple_name(callee, content) {
                calls.push(CallRef { name, receiver: None, line });
            }
        }
        "member_access_expression" => {
            // `recv.Foo()`. `this.Foo()` / `base.Foo()` keep the keyword
            // verbatim, as Kotlin keeps `this`: the shared resolver only knows
            // `self` as an own-type sentinel, so it falls through to the
            // same-file narrowing a bare `Foo()` gets.
            let name = callee.child_by_field_name("name").and_then(|n| simple_name(n, content));
            let receiver = callee
                .child_by_field_name("expression")
                .map(|r| collapse_ws(node_slice(r, content)))
                .filter(|r| !r.is_empty())
                .map(|r| crate::utils::safe_truncate(&r, 64).to_string());
            if let Some(name) = name {
                calls.push(CallRef { name, receiver, line });
            }
        }
        "conditional_access_expression" => {
            // `recv?.Foo()`: the grammar puts the whole `recv?.Foo` in the
            // function position, the member as a `member_binding_expression`.
            let Some(member) = child_of_kind(callee, "member_binding_expression") else {
                return;
            };
            let name = member.child_by_field_name("name").and_then(|n| simple_name(n, content));
            let receiver = callee
                .child_by_field_name("condition")
                .map(|r| collapse_ws(node_slice(r, content)))
                .filter(|r| !r.is_empty())
                .map(|r| crate::utils::safe_truncate(&r, 64).to_string());
            if let Some(name) = name {
                calls.push(CallRef { name, receiver, line: member.start_position().row + 1 });
            }
        }
        _ => {}
    }
}

/// `Foo` from an `identifier` or `Foo<T>` (`generic_name`).
fn simple_name(node: Node, content: &str) -> Option<String> {
    match node.kind() {
        "identifier" => node_text(node, content),
        "generic_name" => child_of_kind(node, "identifier").and_then(|id| node_text(id, content)),
        _ => None,
    }
}

/// The bare name of a constructed type: `Foo`, `Foo<T>`, `Ns.Foo` → `Foo`.
fn type_simple_name(node: Node, content: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "generic_name" => simple_name(node, content),
        "qualified_name" => node.child_by_field_name("name").and_then(|n| simple_name(n, content)),
        _ => None,
    }
}

// ============== Helper Functions ==============

/// Source text of a node. Node byte ranges always land on character
/// boundaries, so this can never split a multi-byte character.
fn node_slice<'a>(node: Node, content: &'a str) -> &'a str {
    content.get(node.byte_range()).unwrap_or("")
}

fn node_text(node: Node, content: &str) -> Option<String> {
    node.utf8_text(content.as_bytes()).ok().map(|s| s.to_string())
}

/// 1-indexed start and end lines of a node.
fn node_lines(node: Node) -> (usize, usize) {
    (node.start_position().row + 1, node.end_position().row + 1)
}

/// Extract source lines from 1-indexed line numbers.
fn source_lines(lines: &[&str], start: usize, end: usize) -> String {
    lines
        .get(start.saturating_sub(1)..end.min(lines.len()))
        .map(|slice| slice.join("\n"))
        .unwrap_or_default()
}

/// Collapse all runs of whitespace to single spaces.
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cap a signature at a readable length, on a character boundary.
fn truncated_signature(sig: &str) -> String {
    if sig.len() <= MAX_SIGNATURE_BYTES {
        return sig.to_string();
    }
    format!("{}...", crate::utils::safe_truncate(sig, MAX_SIGNATURE_BYTES - 3))
}

/// The declaration's header without attributes: from the first child that is
/// not an `attribute_list` up to `end` (the body), or to the node's end minus
/// a trailing `;` when it has none (abstract and interface members).
fn header_signature(node: Node, content: &str, end: Option<usize>) -> String {
    let mut cursor = node.walk();
    let start = node
        .children(&mut cursor)
        .find(|c| c.kind() != "attribute_list")
        .map(|c| c.start_byte())
        .unwrap_or_else(|| node.start_byte());
    let end = end.unwrap_or_else(|| node.end_byte()).max(start);
    // Both offsets come from node boundaries.
    let raw = content.get(start..end).unwrap_or("");
    let sig = collapse_ws(raw);
    truncated_signature(sig.trim_end_matches(';').trim_end())
}

/// Visibility as written: `public`, `private`, `protected internal`, … Empty
/// when the declaration has no access modifier.
fn visibility_of(node: Node, content: &str) -> String {
    let mut cursor = node.walk();
    let words: Vec<String> = node
        .children(&mut cursor)
        .filter(|c| c.kind() == "modifier")
        .filter_map(|c| node_text(c, content))
        .filter(|w| matches!(w.as_str(), "public" | "private" | "protected" | "internal" | "file"))
        .collect();
    words.join(" ")
}

/// Whether the declaration carries this modifier (`async`, `const`, …).
fn has_modifier(node: Node, content: &str, want: &str) -> bool {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|c| c.kind() == "modifier" && node_text(c, content).as_deref() == Some(want));
    found
}

/// Whether the node has a direct child of this kind — used for anonymous
/// keyword tokens (`explicit`, `implicit`).
fn has_child_kind(node: Node, kind: &str) -> bool {
    child_of_kind(node, kind).is_some()
}

/// The first direct child of the given kind.
fn child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| c.kind() == kind);
    found
}

/// The `///` XML doc block immediately preceding a declaration.
///
/// Line-based: attributes are inside the declaration node, so the doc block is
/// always on the lines directly above `node.start_position()`.
fn extract_doc_comment(node: Node, lines: &[&str]) -> Option<String> {
    let mut idx = node.start_position().row;
    let mut comments = Vec::new();
    while idx > 0 {
        idx -= 1;
        let line = lines.get(idx).map(|s| s.trim()).unwrap_or("");
        if !line.starts_with("///") {
            break;
        }
        comments.insert(0, line.to_string());
    }
    if comments.is_empty() {
        None
    } else {
        Some(comments.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn parse_code(code: &str) -> Vec<CodeItem> {
        let mut file = NamedTempFile::with_suffix(".cs").unwrap();
        file.write_all(code.as_bytes()).unwrap();
        parse_csharp_file(file.path()).unwrap()
    }

    fn named<'a>(items: &'a [CodeItem], name: &str) -> &'a CodeItem {
        items.iter().find(|i| i.name == name).unwrap_or_else(|| {
            panic!(
                "no item named {name}; got {:?}",
                items.iter().map(|i| (&i.name, &i.item_type)).collect::<Vec<_>>()
            )
        })
    }

    fn calls_of(item: &CodeItem) -> Vec<(String, Option<String>)> {
        item.calls.iter().map(|c| (c.name.clone(), c.receiver.clone())).collect()
    }

    fn call(name: &str, receiver: Option<&str>) -> (String, Option<String>) {
        (name.to_string(), receiver.map(str::to_string))
    }

    #[test]
    fn test_type_declarations() {
        let items = parse_code(
            r#"
namespace App {
    public class Store {}
    public record Memo(string Id);
    public record class MemoClass(string Id);
    public record struct Point(int X, int Y);
    public struct Size {}
    public interface IStore {}
    public enum Kind { A, B }
    public delegate void Changed(object sender);
}
"#,
        );
        assert_eq!(named(&items, "Store").item_type, "class");
        assert_eq!(named(&items, "Memo").item_type, "class");
        assert_eq!(named(&items, "MemoClass").item_type, "class");
        assert_eq!(named(&items, "Point").item_type, "class", "record struct maps to class");
        assert_eq!(named(&items, "Size").item_type, "struct");
        assert_eq!(named(&items, "IStore").item_type, "interface");
        assert_eq!(named(&items, "Kind").item_type, "enum");
        assert_eq!(named(&items, "Changed").item_type, "type");
        // Types carry no signature, so the title is the existing `class Foo` form.
        assert!(items.iter().filter(|i| i.item_type != "function").all(|i| i.signature.is_none()));
    }

    /// `namespace X;` is a sibling of the declarations, not their parent.
    #[test]
    fn test_file_scoped_namespace() {
        let items = parse_code("namespace App.Core;\n\npublic class Store\n{\n    public void Save() { }\n}\n");
        assert_eq!(named(&items, "Store").item_type, "class");
        let save = named(&items, "Store.Save()");
        assert_eq!(save.impl_for.as_deref(), Some("Store"));
        assert_eq!(save.line_start, 5);
    }

    /// The walk is recursive: nested types and their members are found, the
    /// member name carries the whole type chain, `impl_for` the innermost type.
    #[test]
    fn test_nested_types() {
        let items = parse_code(
            r#"
namespace App {
    class Outer {
        void Top() {}
        class Inner {
            void Deep(int x) {}
            struct Leaf { void Grow() {} }
        }
    }
}
"#,
        );
        assert_eq!(named(&items, "Outer.Top()").impl_for.as_deref(), Some("Outer"));
        assert_eq!(named(&items, "Inner").item_type, "class");
        assert_eq!(named(&items, "Outer.Inner.Deep(int)").impl_for.as_deref(), Some("Inner"));
        assert_eq!(named(&items, "Leaf").item_type, "struct");
        assert_eq!(named(&items, "Outer.Inner.Leaf.Grow()").impl_for.as_deref(), Some("Leaf"));
    }

    /// Overloads differ only by parameters; their names (hence IDs) must too.
    #[test]
    fn test_overloads_get_distinct_ids() {
        let items = parse_code(
            r#"
class Sender {
    public void Send(int x) {}
    public void Send(string s) {}
    public void Send(ref int x) {}
    public void Send(Dictionary<string, int> d, params object[] rest) {}
    public void Send() {}
}
"#,
        );
        let sends: Vec<&CodeItem> = items.iter().filter(|i| i.name.starts_with("Sender.Send(")).collect();
        let names: Vec<&str> = sends.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Sender.Send(int)",
                "Sender.Send(string)",
                "Sender.Send(ref int)",
                "Sender.Send(Dictionary<string,int>,params object[])",
                "Sender.Send()"
            ]
        );
        let ids: std::collections::HashSet<String> = sends.iter().map(|i| i.generate_id()).collect();
        assert_eq!(ids.len(), 5);
    }

    /// Found on MediatR: overloads that differ only in generic arity, and
    /// same-named types that differ only in arity, sit in one file. Without
    /// the type parameters in the name, each pair hashed to one node ID.
    #[test]
    fn test_generic_arity_overloads_and_types_get_distinct_ids() {
        let items = parse_code(
            r#"
public interface IRequestHandler<in TRequest, TResponse> { Task<TResponse> Handle(TRequest request); }
public interface IRequestHandler<in TRequest> { Task Handle(TRequest request); }
public class Config {
    public Config AddBehavior<TService, TImpl>(ServiceLifetime lifetime) => this;
    public Config AddBehavior<TImpl>(ServiceLifetime lifetime) => this;
}
"#,
        );
        let two = named(&items, "IRequestHandler<TRequest,TResponse>");
        let one = named(&items, "IRequestHandler<TRequest>");
        assert_ne!(two.generate_id(), one.generate_id());
        let h2 = named(&items, "IRequestHandler<TRequest,TResponse>.Handle(TRequest)");
        let h1 = named(&items, "IRequestHandler<TRequest>.Handle(TRequest)");
        assert_ne!(h2.generate_id(), h1.generate_id());
        // impl_for stays the bare name a call receiver can match.
        assert_eq!(h2.impl_for.as_deref(), Some("IRequestHandler"));
        let a2 = named(&items, "Config.AddBehavior<TService,TImpl>(ServiceLifetime)");
        let a1 = named(&items, "Config.AddBehavior<TImpl>(ServiceLifetime)");
        assert_ne!(a2.generate_id(), a1.generate_id());
    }

    /// Test classes routinely nest same-named `Ping` / `Handler` types.
    #[test]
    fn test_same_named_nested_types_get_distinct_ids() {
        let items = parse_code(
            r#"
public class SendTests { public class Ping { } }
public class PublishTests { public class Ping { } }
"#,
        );
        let pings: Vec<&CodeItem> = items.iter().filter(|i| i.name == "Ping").collect();
        assert_eq!(pings.len(), 2);
        assert_eq!(pings[0].impl_for.as_deref(), Some("SendTests"));
        assert_ne!(pings[0].generate_id(), pings[1].generate_id());
    }

    #[test]
    fn test_constructors_destructors_operators() {
        let items = parse_code(
            r#"
class Unit {
    public Unit(int v) : this() { Init(v); }
    public Unit() : base(Make()) {}
    ~Unit() { Release(); }
    public static Unit operator +(Unit a, Unit b) => Add(a, b);
    public static implicit operator int(Unit u) => u.Value();
}
"#,
        );
        let ctor = named(&items, "Unit.Unit(int)");
        assert_eq!(ctor.item_type, "function");
        assert_eq!(ctor.signature.as_deref(), Some("public Unit(int v)"));
        assert_eq!(calls_of(ctor), [call("Init", None)]);
        // Calls in the constructor initializer's arguments belong to the ctor.
        assert_eq!(calls_of(named(&items, "Unit.Unit()")), [call("Make", None)]);
        assert_eq!(calls_of(named(&items, "Unit.~Unit()")), [call("Release", None)]);
        assert_eq!(calls_of(named(&items, "Unit.operator +(Unit,Unit)")), [call("Add", None)]);
        let conv = named(&items, "Unit.implicit operator int(Unit)");
        assert_eq!(calls_of(conv), [call("Value", Some("u"))]);
    }

    /// Only properties with code are indexed; they carry calls, and their
    /// signature has no `(`, so they never become call targets.
    #[test]
    fn test_properties_indexers_events() {
        let items = parse_code(
            r#"
class Box {
    public int Auto { get; set; }
    public int Plain;
    public int Computed => Compute();
    public int Full { get { return Load(); } set { Store(value); } }
    public int this[int i] { get => Fetch(i); }
    public event EventHandler Changed { add { Attach(value); } remove { } }
}
"#,
        );
        assert!(items.iter().all(|i| !i.name.contains("Auto") && !i.name.contains("Plain")));
        let computed = named(&items, "Box.Computed");
        assert_eq!(computed.item_type, "function");
        assert_eq!(computed.signature.as_deref(), Some("public int Computed { get }"));
        assert_eq!(calls_of(computed), [call("Compute", None)]);
        let full = named(&items, "Box.Full");
        assert_eq!(full.signature.as_deref(), Some("public int Full { get; set }"));
        assert_eq!(calls_of(full), [call("Load", None), call("Store", None)]);
        assert_eq!(calls_of(named(&items, "Box.this[int]")), [call("Fetch", None)]);
        assert_eq!(calls_of(named(&items, "Box.Changed")), [call("Attach", None)]);
    }

    #[test]
    fn test_const_fields() {
        let items = parse_code(
            r#"
class A { public const int Max = 1, Min = 0; private static readonly int NotConst = 2; }
class B { const int Max = 2; }
"#,
        );
        let maxes: Vec<&CodeItem> = items.iter().filter(|i| i.name == "Max").collect();
        assert_eq!(maxes.len(), 2);
        assert!(maxes.iter().all(|i| i.item_type == "const"));
        assert_ne!(maxes[0].generate_id(), maxes[1].generate_id());
        assert_eq!(named(&items, "Min").item_type, "const");
        assert!(items.iter().all(|i| i.name != "NotConst"));
    }

    #[test]
    fn test_calls() {
        let items = parse_code(
            r#"
class Mediator {
    [Obsolete(nameof(Old))]
    public async Task<T> Send<T>(IRequest<T> request) {
        Validate(request);
        this.Log("x");
        base.Dispose();
        var handler = _factory.Create<T>(request);
        handler?.Handle(request);
        Helpers.Run<int>();
        var w = new Wrapper<T>(request);
        var v = new App.Core.Validator();
        Point p = new(1, 2);
        items.ForEach(i => Process(i));
        Local();
        void Local() { Inner(); }
        return await handler.Handle(request).ConfigureAwait(false);
    }
}
"#,
        );
        let send = named(&items, "Mediator.Send<T>(IRequest<T>)");
        assert_eq!(
            calls_of(send),
            [
                call("Validate", None),
                call("Log", Some("this")),
                call("Dispose", Some("base")),
                call("Create", Some("_factory")),
                call("Handle", Some("handler")),
                call("Run", Some("Helpers")),
                call("Wrapper", None),
                call("Validator", None),
                call("ForEach", Some("items")),
                call("Process", None),
                call("Local", None),
                call("Inner", None),
                call("ConfigureAwait", Some("handler.Handle(request)")),
                call("Handle", Some("handler")),
            ]
        );
        // A local function is folded into its enclosing method.
        assert!(items.iter().all(|i| !i.name.contains("Local")));
        assert!(send.is_async);
        assert_eq!(send.visibility, "public");
    }

    /// The signature is the title, and the title is where call resolution
    /// reads the callee name back from: attributes must not be in it, and the
    /// body must not.
    #[test]
    fn test_signature_round_trips_through_extract_code_name() {
        let items = parse_code(
            r#"
public class Handler : IRequestHandler<Ping, Pong>
{
    [Fact]
    [Trait("a", "b")]
    public async Task<Pong> Handle(Ping request, CancellationToken ct)
    {
        return new Pong();
    }

    Task<Unit> IRequestHandler<Ping, Unit>.Handle(Ping r, CancellationToken ct) => Task.FromResult(Unit.Value);

    public Handler(IServiceProvider sp) { }

    public static IEnumerable<TResult> Map<TSource, TResult>(IEnumerable<TSource> src) where TResult : new() => null;

    public abstract void Abstract(int x);
}
"#,
        );
        let handle = items.iter().find(|i| i.name == "Handler.Handle(Ping,CancellationToken)").unwrap();
        assert_eq!(
            handle.signature.as_deref(),
            Some("public async Task<Pong> Handle(Ping request, CancellationToken ct)")
        );
        assert_eq!(named(&items, "Handler.Abstract(int)").signature.as_deref(), Some("public abstract void Abstract(int x)"));
        let names: Vec<Option<String>> = items
            .iter()
            .filter(|i| i.item_type == "function")
            .map(|i| crate::code::extract_code_name(i.signature.as_deref().unwrap(), "code_function"))
            .collect();
        assert_eq!(
            names,
            [
                Some("Handle".to_string()),
                Some("Handle".to_string()),
                Some("Handler".to_string()),
                Some("Map".to_string()),
                Some("Abstract".to_string()),
            ]
        );
    }

    #[test]
    fn test_xml_doc_comment() {
        let items = parse_code(
            r#"
class A {
    // not a doc comment
    /// <summary>Saves.</summary>
    /// <param name="x">x</param>
    [Pure]
    public void Save(int x) {}

    // plain
    public void Other() {}
}
"#,
        );
        assert_eq!(
            named(&items, "A.Save(int)").doc_comment.as_deref(),
            Some("/// <summary>Saves.</summary>\n/// <param name=\"x\">x</param>")
        );
        assert_eq!(named(&items, "A.Other()").doc_comment, None);
    }

    /// MCA-PANIC-01: no byte offset into the source is ever hand-computed.
    /// Multibyte identifiers, strings and comments parse, keep their line
    /// numbers, and a long multibyte signature truncates on a char boundary.
    #[test]
    fn test_multibyte_source_does_not_panic() {
        let long_param = "ä".repeat(200);
        let code = format!(
            // `void Lang(int ` is 14 bytes, so a byte cut at the 157-byte cap
            // would land inside a two-byte `ä`.
            "// Grüße — 日本語\nclass Café {{\n    public string Grüße(string s) {{ return Übersetze(\"日本語\"); }}\n    void Lang(int {long_param}) {{ }}\n}}\n"
        );
        let items = parse_code(&code);
        assert_eq!(named(&items, "Café").item_type, "class");
        let greet = named(&items, "Café.Grüße(string)");
        assert_eq!(greet.line_start, 3);
        assert_eq!(calls_of(greet), [call("Übersetze", None)]);
        let long = named(&items, "Café.Lang(int)");
        let sig = long.signature.as_deref().unwrap();
        assert!(sig.ends_with("..."), "long signature should truncate: {sig}");
        assert!(sig.len() <= MAX_SIGNATURE_BYTES);
    }
}
