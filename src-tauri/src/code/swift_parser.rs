//! Swift parser using tree-sitter.
//!
//! Extracts:
//! - Structs (`struct X`)
//! - Classes and actors (`class X`, `actor X`) — both map to `class`
//! - Enums (`enum X`)
//! - Extensions (`extension X: P`) — mapped to `impl`, like a Rust impl block
//! - Protocols (`protocol X`) — mapped to `trait`
//! - Functions and methods, including `init` / `deinit`
//! - Top-level `let` / `var` (→ `const` / `var`)
//! - Type aliases (`typealias X = Y`)
//! - Call sites, with the navigation receiver when there is one
//!
//! The grammar folds struct/class/actor/enum/extension into a single
//! `class_declaration` node and tells them apart with an explicit
//! `declaration_kind` field, so nothing here pattern-matches on keywords.
//!
//! Every slice of source text below is taken either from a tree-sitter node's
//! byte range (always on a character boundary) or from an already-split line,
//! never from a hand-computed offset — Swift sources carry emoji and CJK, and a
//! byte index into the middle of one panics the whole import.

use std::path::Path;
use tree_sitter::{Node, Parser};

use super::types::{CallRef, CodeItem};

/// Longest signature kept verbatim before it is truncated.
const MAX_SIGNATURE_BYTES: usize = 160;

/// Parse a Swift file and extract code items.
pub fn parse_swift_file(path: &Path) -> Result<Vec<CodeItem>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_swift::LANGUAGE.into())
        .map_err(|e| format!("Failed to set language: {:?}", e))?;

    let tree = parser
        .parse(&content, None)
        .ok_or_else(|| "Failed to parse Swift".to_string())?;

    let file_path = path.to_string_lossy().to_string();
    let lines: Vec<&str> = content.lines().collect();

    let mut items = Vec::new();
    extract_items(tree.root_node(), &file_path, &content, &lines, &mut items, None);

    Ok(items)
}

/// Recursively extract code items from the AST.
///
/// `enclosing` is the name of the type whose body we are inside, and becomes
/// `impl_for` on every function declared there — the same role a Go receiver
/// type plays, which is what `pick_callee` narrows `self.foo()` against.
fn extract_items(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    items: &mut Vec<CodeItem>,
    enclosing: Option<&str>,
) {
    let kind = node.kind();
    let mut child_enclosing = enclosing;
    let owned_name;

    match kind {
        "class_declaration" => {
            // struct / class / actor / enum / extension
            if let Some((item, type_name)) = extract_type_declaration(node, file_path, content, lines) {
                items.push(item);
                owned_name = type_name;
                child_enclosing = Some(&owned_name);
            }
        }

        "protocol_declaration" => {
            if let Some(item) = extract_protocol(node, file_path, content, lines) {
                items.push(item);
            }
        }

        "function_declaration" => {
            if let Some(item) = extract_function(node, file_path, content, lines, enclosing) {
                items.push(item);
            }
        }

        "init_declaration" => {
            items.push(extract_initializer(node, file_path, content, lines, enclosing, "init"));
        }

        "deinit_declaration" => {
            items.push(extract_initializer(node, file_path, content, lines, enclosing, "deinit"));
        }

        "typealias_declaration" => {
            if let Some(item) = extract_typealias(node, file_path, content, lines) {
                items.push(item);
            }
        }

        "property_declaration" => {
            // Only file-scope bindings. `property_declaration` is also how the
            // grammar represents `let x = ...` inside a function body, and
            // those are locals, not graph nodes.
            if node.parent().map(|p| p.kind()) == Some("source_file") {
                if let Some(item) = extract_property(node, file_path, content, lines) {
                    items.push(item);
                }
            }
        }

        _ => {}
    }

    // Declarations that own their whole subtree are not descended into; the
    // rest (source_file, class_body, enum_class_body, …) are.
    if !matches!(
        kind,
        "function_declaration"
            | "init_declaration"
            | "deinit_declaration"
            | "typealias_declaration"
            | "property_declaration"
            | "protocol_declaration"
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_items(child, file_path, content, lines, items, child_enclosing);
        }
    }
}

/// Extract a `class_declaration` — struct, class, actor, enum, or extension.
///
/// Returns the item plus the type name to use as `impl_for` for its members.
fn extract_type_declaration(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<(CodeItem, String)> {
    let kind_node = node.child_by_field_name("declaration_kind")?;
    let decl_kind = node_text(kind_node, content)?;

    let name_node = node.child_by_field_name("name")?;
    let name = type_name_of(name_node, content)?;

    let item_type = match decl_kind.as_str() {
        "struct" => "struct",
        "class" | "actor" => "class",
        "enum" => "enum",
        "extension" => "impl",
        _ => return None,
    };

    // An extension is Swift's impl block: it attaches members to a named type,
    // optionally declaring a protocol conformance.
    let (impl_for, impl_trait) = if item_type == "impl" {
        (Some(name.clone()), first_inherited_type(node, content))
    } else {
        (None, None)
    };

    let (line_start, line_end) = node_lines(node);

    Some((
        CodeItem {
            name: name.clone(),
            item_type: item_type.to_string(),
            file_path: file_path.to_string(),
            line_start,
            line_end,
            content: source_lines(lines, line_start, line_end),
            signature: Some(header_signature(node, content)),
            visibility: visibility_of(node, content),
            doc_comment: extract_doc_comment(node, lines),
            is_async: false,
            is_unsafe: false,
            impl_for,
            impl_trait,
            calls: Vec::new(),
        },
        name,
    ))
}

/// Extract a `protocol_declaration` as a trait.
fn extract_protocol(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = type_name_of(name_node, content)?;
    let (line_start, line_end) = node_lines(node);

    Some(CodeItem {
        name,
        item_type: "trait".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: source_lines(lines, line_start, line_end),
        signature: Some(header_signature(node, content)),
        visibility: visibility_of(node, content),
        doc_comment: extract_doc_comment(node, lines),
        is_async: false,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
        calls: Vec::new(),
    })
}

/// Extract a `func` declaration.
fn extract_function(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    enclosing: Option<&str>,
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, content)?;
    let (line_start, line_end) = node_lines(node);

    Some(CodeItem {
        name,
        item_type: "function".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: source_lines(lines, line_start, line_end),
        signature: Some(header_signature(node, content)),
        visibility: visibility_of(node, content),
        doc_comment: extract_doc_comment(node, lines),
        is_async: has_child_kind(node, "async"),
        is_unsafe: false,
        impl_for: enclosing.map(str::to_string),
        impl_trait: None,
        calls: body_calls(node, content),
    })
}

/// Extract an `init` or `deinit` declaration, which carry no name of their own.
fn extract_initializer(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    enclosing: Option<&str>,
    name: &str,
) -> CodeItem {
    let (line_start, line_end) = node_lines(node);

    CodeItem {
        name: name.to_string(),
        item_type: "function".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: source_lines(lines, line_start, line_end),
        signature: Some(header_signature(node, content)),
        visibility: visibility_of(node, content),
        doc_comment: extract_doc_comment(node, lines),
        is_async: has_child_kind(node, "async"),
        is_unsafe: false,
        impl_for: enclosing.map(str::to_string),
        impl_trait: None,
        calls: body_calls(node, content),
    }
}

/// Extract a `typealias`.
fn extract_typealias(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = type_name_of(name_node, content)?;
    let (line_start, line_end) = node_lines(node);

    Some(CodeItem {
        name,
        item_type: "type".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: source_lines(lines, line_start, line_end),
        signature: Some(truncated_signature(&collapse_ws(node_slice(node, content)))),
        visibility: visibility_of(node, content),
        doc_comment: extract_doc_comment(node, lines),
        is_async: false,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
        calls: Vec::new(),
    })
}

/// Extract a file-scope `let` / `var` binding.
fn extract_property(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<CodeItem> {
    let pattern = node.child_by_field_name("name")?;
    // `pattern` wraps the bound identifier.
    let name = first_descendant(pattern, "simple_identifier")
        .and_then(|n| node_text(n, content))
        .or_else(|| node_text(pattern, content))?;

    // `value_binding_pattern` holds the `let` / `var` keyword.
    let mutability = node
        .children(&mut node.walk())
        .find(|c| c.kind() == "value_binding_pattern")
        .and_then(|b| node_text(b, content))
        .unwrap_or_default();

    let item_type = if mutability.trim_start().starts_with("let") { "const" } else { "var" };
    let (line_start, line_end) = node_lines(node);

    Some(CodeItem {
        name,
        item_type: item_type.to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: source_lines(lines, line_start, line_end),
        signature: Some(truncated_signature(&collapse_ws(node_slice(node, content)))),
        visibility: visibility_of(node, content),
        doc_comment: extract_doc_comment(node, lines),
        is_async: false,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
        calls: Vec::new(),
    })
}

// ============== Calls ==============

/// Collect call sites from a declaration's body.
fn body_calls(node: Node, content: &str) -> Vec<CallRef> {
    let mut calls = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        collect_calls(body, content, &mut calls);
    }
    calls
}

fn collect_calls(node: Node, content: &str, calls: &mut Vec<CallRef>) {
    if node.kind() == "call_expression" {
        // The callee is the first named child; `call_suffix` is the second.
        if let Some(callee) = node.named_child(0) {
            match callee.kind() {
                "simple_identifier" => {
                    if let Some(name) = node_text(callee, content) {
                        calls.push(CallRef {
                            name,
                            receiver: None,
                            line: callee.start_position().row + 1,
                        });
                    }
                }
                "navigation_expression" => {
                    // `receiver.method` — the method name lives under the
                    // navigation suffix, the receiver under `target`.
                    let member = callee
                        .child_by_field_name("suffix")
                        .and_then(|s| s.child_by_field_name("suffix"));
                    if let Some(member) = member {
                        if member.kind() == "simple_identifier" {
                            if let Some(name) = node_text(member, content) {
                                let receiver = callee
                                    .child_by_field_name("target")
                                    .map(|t| collapse_ws(node_slice(t, content)))
                                    .filter(|r| !r.is_empty())
                                    .map(|r| crate::utils::safe_truncate(&r, 64).to_string());
                                calls.push(CallRef {
                                    name,
                                    receiver,
                                    line: member.start_position().row + 1,
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls(child, content, calls);
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

/// The declaration's header: everything up to its body (or the whole node when
/// it has none, as for a protocol requirement).
fn header_signature(node: Node, content: &str) -> String {
    let end = node
        .child_by_field_name("body")
        .map(|b| b.start_byte())
        .unwrap_or_else(|| node.end_byte());
    // Both offsets come from node boundaries.
    let raw = content.get(node.start_byte()..end).unwrap_or("");
    truncated_signature(&collapse_ws(raw))
}

/// Visibility as written: `public`, `private`, `private(set)`, `fileprivate`,
/// `internal`, `open`. Empty when the declaration has no visibility modifier.
fn visibility_of(node: Node, content: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mut inner = child.walk();
            for m in child.children(&mut inner) {
                if m.kind() == "visibility_modifier" {
                    return node_text(m, content).unwrap_or_default();
                }
            }
        }
    }
    String::new()
}

/// Whether the declaration has a direct child of this kind — used for the
/// `async` keyword, which the grammar exposes as an anonymous child.
fn has_child_kind(node: Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).any(|c| c.kind() == kind);
    found
}

/// The first `inheritance_specifier` on a declaration, as a type name.
fn first_inherited_type(node: Node, content: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "inheritance_specifier" {
            let target = child.child_by_field_name("inherits_from").unwrap_or(child);
            return type_name_of(target, content);
        }
    }
    None
}

/// Reduce a type node to a bare name: `user_type` and the various wrappers all
/// carry a `type_identifier` somewhere inside.
fn type_name_of(node: Node, content: &str) -> Option<String> {
    if matches!(node.kind(), "type_identifier" | "simple_identifier") {
        return node_text(node, content);
    }
    if let Some(id) = first_descendant(node, "type_identifier") {
        return node_text(id, content);
    }
    let text = collapse_ws(node_slice(node, content));
    if text.is_empty() { None } else { Some(text) }
}

/// First descendant of the given kind, breadth-first from `node` itself.
fn first_descendant<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = first_descendant(child, kind) {
            return Some(found);
        }
    }
    None
}

/// Leading `///` documentation comment lines preceding a declaration.
fn extract_doc_comment(node: Node, lines: &[&str]) -> Option<String> {
    let start_line = node.start_position().row;
    if start_line == 0 {
        return None;
    }

    let mut comments = Vec::new();
    let mut idx = start_line - 1;
    loop {
        let line = lines.get(idx).map(|s| s.trim()).unwrap_or("");
        if line.starts_with("///") {
            comments.insert(0, line.to_string());
        } else {
            break;
        }
        if idx == 0 {
            break;
        }
        idx -= 1;
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
        let mut file = NamedTempFile::with_suffix(".swift").unwrap();
        file.write_all(code.as_bytes()).unwrap();
        parse_swift_file(file.path()).unwrap()
    }

    fn named<'a>(items: &'a [CodeItem], name: &str) -> &'a CodeItem {
        items.iter().find(|i| i.name == name).unwrap_or_else(|| {
            panic!(
                "no item named {name}; got {:?}",
                items.iter().map(|i| (&i.name, &i.item_type)).collect::<Vec<_>>()
            )
        })
    }

    /// The grammar folds five different declarations into `class_declaration`
    /// and separates them only by the `declaration_kind` field.
    #[test]
    fn test_declaration_kind_disambiguation() {
        let items = parse_code(
            r#"
struct Memo {}
class Store {}
actor Loader {}
enum Kind {}
extension Memo {}
"#,
        );
        assert_eq!(named(&items, "Memo").item_type, "struct");
        assert_eq!(named(&items, "Store").item_type, "class");
        assert_eq!(named(&items, "Loader").item_type, "class");
        assert_eq!(named(&items, "Kind").item_type, "enum");

        // The extension and the struct share the name, so pick by type.
        let ext = items.iter().find(|i| i.item_type == "impl").expect("extension parsed");
        assert_eq!(ext.name, "Memo");
        assert_eq!(ext.impl_for, Some("Memo".to_string()));
    }

    #[test]
    fn test_extension_records_conformance_as_impl_trait() {
        let items = parse_code(
            r#"
extension Memo: CustomStringConvertible {
    func describe() -> String { "x" }
}
"#,
        );
        let ext = items.iter().find(|i| i.item_type == "impl").unwrap();
        assert_eq!(ext.impl_for, Some("Memo".to_string()));
        assert_eq!(ext.impl_trait, Some("CustomStringConvertible".to_string()));
        assert_eq!(named(&items, "describe").impl_for, Some("Memo".to_string()));
    }

    #[test]
    fn test_protocol_is_a_trait() {
        let items = parse_code(
            r#"
public protocol MemoService: Sendable {
    func list() async throws -> [String]
}
"#,
        );
        let p = named(&items, "MemoService");
        assert_eq!(p.item_type, "trait");
        assert_eq!(p.visibility, "public");
    }

    #[test]
    fn test_method_carries_enclosing_type() {
        let items = parse_code(
            r#"
final class ViewModel {
    fileprivate func refresh() {}
    public func reload() async {}
}
"#,
        );
        let refresh = named(&items, "refresh");
        assert_eq!(refresh.item_type, "function");
        assert_eq!(refresh.impl_for, Some("ViewModel".to_string()));
        assert_eq!(refresh.visibility, "fileprivate");
        assert!(!refresh.is_async);

        let reload = named(&items, "reload");
        assert_eq!(reload.impl_for, Some("ViewModel".to_string()));
        assert_eq!(reload.visibility, "public");
        assert!(reload.is_async, "async keyword should set is_async");
    }

    #[test]
    fn test_free_function_has_no_impl_for() {
        let items = parse_code("func helper() -> Int { 1 }\n");
        let f = named(&items, "helper");
        assert_eq!(f.item_type, "function");
        assert_eq!(f.impl_for, None);
        assert_eq!(f.visibility, "");
        assert!(f.signature.as_ref().unwrap().contains("func helper"));
    }

    #[test]
    fn test_init_and_deinit() {
        let items = parse_code(
            r#"
class Store {
    init(id: String) {}
    deinit {}
}
"#,
        );
        let init = named(&items, "init");
        assert_eq!(init.item_type, "function");
        assert_eq!(init.impl_for, Some("Store".to_string()));
        assert!(init.signature.as_ref().unwrap().contains("id: String"));

        let deinit = named(&items, "deinit");
        assert_eq!(deinit.item_type, "function");
        assert_eq!(deinit.impl_for, Some("Store".to_string()));
    }

    #[test]
    fn test_top_level_let_and_var() {
        let items = parse_code(
            r#"
let maxRetries = 3
public var current: Int = 0
"#,
        );
        assert_eq!(named(&items, "maxRetries").item_type, "const");
        let current = named(&items, "current");
        assert_eq!(current.item_type, "var");
        assert_eq!(current.visibility, "public");
    }

    /// A `let` inside a function body is a local, not a graph node.
    #[test]
    fn test_local_bindings_are_not_items() {
        let items = parse_code(
            r#"
func run() {
    let local = 1
    var other = 2
}
"#,
        );
        assert!(items.iter().all(|i| i.name != "local"), "locals must not be items");
        assert!(items.iter().all(|i| i.name != "other"), "locals must not be items");
    }

    #[test]
    fn test_typealias() {
        let items = parse_code("typealias MemoList = [Memo]\n");
        let t = named(&items, "MemoList");
        assert_eq!(t.item_type, "type");
        assert!(t.signature.as_ref().unwrap().contains("typealias MemoList"));
    }

    #[test]
    fn test_calls_with_and_without_receiver() {
        let items = parse_code(
            r#"
class ViewModel {
    func refresh() {
        print("hi")
        self.reload()
        service.list()
    }
    func reload() {}
}
"#,
        );
        let refresh = named(&items, "refresh");
        assert!(
            refresh.calls.iter().any(|c| c.name == "print" && c.receiver.is_none()),
            "bare call should have no receiver: {:?}",
            refresh.calls
        );
        assert!(
            refresh.calls.iter().any(|c| c.name == "reload" && c.receiver.as_deref() == Some("self")),
            "self call should carry receiver `self`: {:?}",
            refresh.calls
        );
        assert!(
            refresh.calls.iter().any(|c| c.name == "list" && c.receiver.as_deref() == Some("service")),
            "member call should carry its receiver: {:?}",
            refresh.calls
        );
    }

    /// Calls inside a trailing closure belong to the enclosing function.
    #[test]
    fn test_calls_inside_closures() {
        let items = parse_code(
            r#"
class ViewModel {
    func refresh() {
        Task { await self.reload() }
    }
    func reload() async {}
}
"#,
        );
        let refresh = named(&items, "refresh");
        assert!(refresh.calls.iter().any(|c| c.name == "Task"));
        assert!(refresh.calls.iter().any(|c| c.name == "reload" && c.receiver.as_deref() == Some("self")));
    }

    #[test]
    fn test_doc_comment() {
        let items = parse_code(
            r#"
/// Loads a memo from disk.
/// Returns nil when absent.
func load() {}
"#,
        );
        let doc = named(&items, "load").doc_comment.as_ref().unwrap();
        assert!(doc.contains("Loads a memo from disk"));
        assert!(doc.contains("Returns nil when absent"));
    }

    /// Multibyte text anywhere in the source must not panic and must not shift
    /// line numbers. MoeMemos is a memo app: its sources and fixtures carry CJK
    /// and emoji, and byte-slicing one on a non-character boundary has already
    /// taken down this importer once (MCA-PANIC-01, in the Go parser).
    #[test]
    fn test_multibyte_source_does_not_panic_and_keeps_line_numbers() {
        // Padded so that whatever fixed offset a truncation lands on, one of
        // the three variants cuts mid-character.
        for pad in 0..3 {
            let filler = "x".repeat(pad);
            let code = format!(
                "// 注释 🚀 comment\n\
                 /// 说明：这个函数返回一个备忘录 ✔\n\
                 struct 备忘录 {{\n\
                 \x20   let 标题 = \"{filler}配置文件已更新测试全部通过继续下一步 ✔ 🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀\"\n\
                 }}\n\
                 \n\
                 /// 载入 🚀\n\
                 func 载入(名字: String) -> String {{\n\
                 \x20   print(\"你好 🌏\")\n\
                 \x20   return \"{filler}结果 ✔\"\n\
                 }}\n"
            );
            let items = parse_code(&code);

            let f = named(&items, "载入");
            assert_eq!(f.item_type, "function", "pad {pad}");
            assert_eq!(f.line_start, 8, "pad {pad}: line numbers count lines, not bytes");
            assert_eq!(f.line_end, 11, "pad {pad}");
            assert!(f.doc_comment.as_ref().unwrap().contains("载入"), "pad {pad}");
            assert!(f.calls.iter().any(|c| c.name == "print"), "pad {pad}");

            let s = named(&items, "备忘录");
            assert_eq!(s.item_type, "struct", "pad {pad}");
            assert_eq!(s.line_start, 3, "pad {pad}");
            assert!(s.content.contains("标题"), "pad {pad}: content must keep the source text");
        }
    }

    /// A long multibyte signature is truncated on a character boundary.
    #[test]
    fn test_long_multibyte_signature_truncates_safely() {
        for pad in 0..3 {
            let code = format!(
                "let 通知 = \"{}配置文件已更新测试全部通过{} ✔ 🚀\"\n",
                "x".repeat(pad),
                "继续".repeat(40)
            );
            let items = parse_code(&code);
            let c = named(&items, "通知");
            let sig = c.signature.as_ref().unwrap();
            assert!(sig.ends_with("..."), "pad {pad}: long signature should truncate: {sig}");
            assert!(sig.chars().count() < sig.len(), "pad {pad}: fixture must be multibyte");
            assert!(
                sig.len() <= MAX_SIGNATURE_BYTES,
                "pad {pad}: truncated signature should respect the cap: {} bytes",
                sig.len()
            );
        }
    }

    #[test]
    fn test_parse_full_file() {
        let items = parse_code(
            r#"
import Foundation

/// A stored memo.
public struct Memo: Identifiable, Codable {
    public let id: String

    public init(id: String) {
        self.id = id
    }

    public func render() -> String {
        return format(id)
    }
}

protocol MemoService {
    func list() async throws -> [Memo]
}

extension Memo: CustomStringConvertible {
    func describe() -> String { render() }
}

actor Loader {
    private func go() {}
}

enum Kind: String {
    case a
}

typealias MemoList = [Memo]

let defaultLimit = 20
var cursor: Int = 0
"#,
        );

        let by = |t: &str| items.iter().filter(|i| i.item_type == t).count();
        assert_eq!(by("struct"), 1, "Memo");
        assert_eq!(by("class"), 1, "Loader (actor)");
        assert_eq!(by("enum"), 1, "Kind");
        assert_eq!(by("trait"), 1, "MemoService");
        assert_eq!(by("impl"), 1, "extension Memo");
        assert_eq!(by("type"), 1, "MemoList");
        assert_eq!(by("const"), 1, "defaultLimit");
        assert_eq!(by("var"), 1, "cursor");
        // init, render, describe, go — the protocol requirement is not a def.
        assert_eq!(by("function"), 4, "{:?}", items.iter().map(|i| &i.name).collect::<Vec<_>>());

        let memo = named(&items, "Memo");
        assert_eq!(memo.visibility, "public");
        assert!(memo.doc_comment.as_ref().unwrap().contains("A stored memo"));

        assert_eq!(named(&items, "describe").impl_for, Some("Memo".to_string()));
        assert_eq!(named(&items, "go").impl_for, Some("Loader".to_string()));
        assert_eq!(named(&items, "go").visibility, "private");
    }
}
