//! Kotlin parser using tree-sitter.
//!
//! Extracts:
//! - Classes, interfaces, enums, and the `data` / `sealed` / `value` /
//!   `annotation` variants of a class (`class_declaration`)
//! - Objects (`object X`) and companion objects — Kotlin singletons, mapped to
//!   `class`
//! - Functions and methods, including extension functions (`fun Foo.bar()`),
//!   whose receiver becomes `impl_for`
//! - Top-level and class-level `val` / `var` (→ `const` / `var`)
//! - Type aliases (`typealias X = Y`)
//! - Call sites, with the navigation receiver when there is one
//!
//! Unlike the Swift grammar, `class_declaration` carries no `declaration_kind`
//! field: class / interface / enum are told apart by the keyword token among
//! the node's direct children (`interface`), by an `enum_class_body` child, or
//! by an `enum` entry in `modifiers`.
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

/// Parse a Kotlin file and extract code items.
pub fn parse_kotlin_file(path: &Path) -> Result<Vec<CodeItem>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
        .map_err(|e| format!("Failed to set language: {:?}", e))?;

    let tree = parser
        .parse(&content, None)
        .ok_or_else(|| "Failed to parse Kotlin".to_string())?;

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
/// type plays, which is what `pick_callee` narrows a qualified call against.
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
            if let Some((item, type_name)) = extract_class(node, file_path, content, lines) {
                items.push(item);
                owned_name = type_name;
                child_enclosing = Some(&owned_name);
            }
        }

        "object_declaration" => {
            if let Some((item, type_name)) = extract_object(node, file_path, content, lines) {
                items.push(item);
                owned_name = type_name;
                child_enclosing = Some(&owned_name);
            }
        }

        "companion_object" => {
            let (item, member_scope) = extract_companion(node, file_path, content, lines, enclosing);
            items.push(item);
            owned_name = member_scope;
            child_enclosing = Some(&owned_name);
        }

        "function_declaration" => {
            if let Some(item) = extract_function(node, file_path, content, lines, enclosing) {
                items.push(item);
            }
        }

        "type_alias" => {
            if let Some(item) = extract_typealias(node, file_path, content, lines) {
                items.push(item);
            }
        }

        "property_declaration" => {
            // Top level and class level only. The same node kind represents
            // `val x = ...` inside a function body, and those are locals, not
            // graph nodes. Function bodies are not descended into either, so
            // this is belt and braces.
            let parent_kind = node.parent().map(|p| p.kind()).unwrap_or("");
            if matches!(parent_kind, "source_file" | "class_body" | "enum_class_body") {
                if let Some(item) = extract_property(node, file_path, content, lines, enclosing) {
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
            | "property_declaration"
            | "type_alias"
            | "secondary_constructor"
            | "anonymous_initializer"
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_items(child, file_path, content, lines, items, child_enclosing);
        }
    }
}

/// Extract a `class_declaration` — class, interface, or enum, in any of their
/// `data` / `sealed` / `value` / `annotation` / `fun interface` spellings.
///
/// Returns the item plus the type name to use as `impl_for` for its members.
fn extract_class(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<(CodeItem, String)> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, content)?;
    let (line_start, line_end) = node_lines(node);

    Some((
        CodeItem {
            name: name.clone(),
            item_type: class_item_type(node, content).to_string(),
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
        },
        name,
    ))
}

/// `class` / `interface` / `enum` for a `class_declaration`.
///
/// `interface` is an anonymous keyword token among the direct children (`fun
/// interface X` carries both `fun` and `interface`). `enum` is read from the
/// `enum` `class_modifier`, which `enum class X` always carries, and never
/// from the presence of an `enum_class_body`: under error recovery the grammar
/// labels an ordinary one-line class body `enum_class_body`, so that node is
/// not evidence of an enum.
fn class_item_type(node: Node, content: &str) -> &'static str {
    if has_child_kind(node, "interface") {
        return "trait";
    }
    if has_class_modifier(node, content, "enum") {
        return "enum";
    }
    "class"
}

/// Extract an `object X { … }` declaration — a Kotlin singleton, which has no
/// counterpart in `CodeItem` beyond `class`.
fn extract_object(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<(CodeItem, String)> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, content)?;
    let (line_start, line_end) = node_lines(node);

    Some((
        CodeItem {
            name: name.clone(),
            item_type: "class".to_string(),
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
        },
        name,
    ))
}

/// Extract a `companion object`.
///
/// Returns the item plus the name its members should carry as `impl_for`.
/// Those differ: the item is named `Companion` when the source gives it no
/// name, but its members are *called* through the enclosing class
/// (`Store.create()`), so that is what has to match a call receiver.
///
/// `impl_for` on the companion item itself records the owning class, which
/// also keeps two unnamed companions in one file from hashing to the same node
/// ID — `generate_id` folds `impl_for` in.
fn extract_companion(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    enclosing: Option<&str>,
) -> (CodeItem, String) {
    let declared = node
        .child_by_field_name("name")
        .and_then(|n| node_text(n, content));
    let name = declared.clone().unwrap_or_else(|| "Companion".to_string());
    let member_scope = declared
        .or_else(|| enclosing.map(str::to_string))
        .unwrap_or_else(|| "Companion".to_string());
    let (line_start, line_end) = node_lines(node);

    (
        CodeItem {
            name,
            item_type: "class".to_string(),
            file_path: file_path.to_string(),
            line_start,
            line_end,
            content: source_lines(lines, line_start, line_end),
            signature: Some(header_signature(node, content)),
            visibility: visibility_of(node, content),
            doc_comment: extract_doc_comment(node, lines),
            is_async: false,
            is_unsafe: false,
            impl_for: enclosing.map(str::to_string),
            impl_trait: None,
            calls: Vec::new(),
        },
        member_scope,
    )
}

/// Extract a `fun` declaration.
///
/// An extension function puts its receiver type as a `user_type` /
/// `nullable_type` child *before* the `name` field, with no field of its own;
/// that receiver wins over the enclosing type, because `fun Foo.bar()` is
/// reached as `Foo.bar()` no matter where it is declared.
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

    let impl_for = extension_receiver(node, content, name_node).or_else(|| enclosing.map(str::to_string));

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
        // Kotlin has no `async` keyword. `suspend` is the marker for a function
        // that can pause and resume, so it is what `is_async` records.
        is_async: has_function_modifier(node, content, "suspend"),
        is_unsafe: false,
        impl_for,
        impl_trait: None,
        calls: body_calls(node, content),
    })
}

/// Extract a `typealias`.
///
/// The field called `type` is the alias *name*, not the aliased type — the
/// aliased type is a separate, unnamed `user_type` child.
fn extract_typealias(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("type")?;
    let name = node_text(name_node, content)?;
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

/// Extract a `val` / `var` binding. `property_declaration` carries no fields:
/// the name lives inside its `variable_declaration` child and the mutability
/// is an anonymous `val` / `var` keyword token.
///
/// `impl_for` records the owning type. Kotlin puts properties inside classes,
/// so without it two same-named properties in one file — `val id` on two
/// classes — hash to the same node ID and the second is silently dropped.
fn extract_property(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    enclosing: Option<&str>,
) -> Option<CodeItem> {
    let decl = child_of_kind(node, "variable_declaration")?;
    let name = first_descendant(decl, "identifier").and_then(|n| node_text(n, content))?;

    let item_type = if has_child_kind(node, "val") { "const" } else { "var" };
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
        impl_for: enclosing.map(str::to_string),
        impl_trait: None,
        calls: Vec::new(),
    })
}

// ============== Calls ==============

/// Collect call sites from a declaration's body. `function_body` covers both
/// `fun f() { … }` and the expression form `fun f() = …`; an abstract or
/// interface declaration has none.
fn body_calls(node: Node, content: &str) -> Vec<CallRef> {
    let mut calls = Vec::new();
    if let Some(body) = child_of_kind(node, "function_body") {
        collect_calls(body, content, &mut calls);
    }
    calls
}

fn collect_calls(node: Node, content: &str, calls: &mut Vec<CallRef>) {
    if node.kind() == "call_expression" {
        // The callee is the first named child; `value_arguments` and any
        // trailing lambda follow it.
        if let Some(callee) = node.named_child(0) {
            match callee.kind() {
                "identifier" => {
                    if let Some(name) = node_text(callee, content) {
                        calls.push(CallRef {
                            name,
                            receiver: None,
                            line: callee.start_position().row + 1,
                        });
                    }
                }
                "navigation_expression" => {
                    // `receiver.method` — the grammar exposes no fields here,
                    // so the member is the last named child and the receiver
                    // the first. `this.foo()` keeps `this` verbatim: the
                    // shared resolver only recognises `self` as the own-type
                    // sentinel, and inventing one would put a token in the
                    // stored metadata that is not in the source.
                    let count = callee.named_child_count();
                    if count >= 2 {
                        if let (Some(member), Some(target)) =
                            (callee.named_child(count - 1), callee.named_child(0))
                        {
                            if member.kind() == "identifier" {
                                if let Some(name) = node_text(member, content) {
                                    let receiver = Some(collapse_ws(node_slice(target, content)))
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
/// it has none, as for `data class Memo(val id: String)`).
fn header_signature(node: Node, content: &str) -> String {
    let end = ["function_body", "class_body", "enum_class_body"]
        .iter()
        .find_map(|k| child_of_kind(node, k))
        .map(|b| b.start_byte())
        .unwrap_or_else(|| node.end_byte());
    // Both offsets come from node boundaries.
    let raw = content.get(node.start_byte()..end).unwrap_or("");
    truncated_signature(&collapse_ws(raw))
}

/// Visibility as written: `public`, `private`, `internal`, `protected`. Empty
/// when the declaration has no visibility modifier.
fn visibility_of(node: Node, content: &str) -> String {
    modifier_text(node, content, "visibility_modifier").unwrap_or_default()
}

/// Whether `modifiers` carries a `class_modifier` with this exact text
/// (`enum`, `data`, `sealed`, `value`, `annotation`).
fn has_class_modifier(node: Node, content: &str, want: &str) -> bool {
    let Some(modifiers) = child_of_kind(node, "modifiers") else {
        return false;
    };
    let mut cursor = modifiers.walk();
    let found = modifiers.children(&mut cursor).any(|m| {
        m.kind() == "class_modifier" && node_text(m, content).as_deref() == Some(want)
    });
    found
}

/// Whether `modifiers` carries a `function_modifier` with this exact text
/// (`suspend`, `operator`, `inline`, …).
fn has_function_modifier(node: Node, content: &str, want: &str) -> bool {
    let Some(modifiers) = child_of_kind(node, "modifiers") else {
        return false;
    };
    let mut cursor = modifiers.walk();
    let found = modifiers.children(&mut cursor).any(|m| {
        m.kind() == "function_modifier" && node_text(m, content).as_deref() == Some(want)
    });
    found
}

/// Text of the first modifier of the given kind inside the `modifiers` child.
fn modifier_text(node: Node, content: &str, kind: &str) -> Option<String> {
    let modifiers = child_of_kind(node, "modifiers")?;
    let mut cursor = modifiers.walk();
    let text = modifiers
        .children(&mut cursor)
        .find(|m| m.kind() == kind)
        .and_then(|m| node_text(m, content));
    text
}

/// The receiver type of an extension function, or `None` for a plain function.
///
/// The receiver is positional: a type child that starts before the `name`
/// field. Anything after it is the return type, and `type_parameters`
/// (`fun <T> f()`) is a different node kind.
fn extension_receiver(node: Node, content: &str, name_node: Node) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.start_byte() >= name_node.start_byte() {
            break;
        }
        if matches!(
            child.kind(),
            "user_type" | "nullable_type" | "function_type" | "parenthesized_type"
        ) {
            return type_name_of(child, content);
        }
    }
    None
}

/// Reduce a type node to a bare name: `List<String>` → `List`, `Foo?` → `Foo`.
fn type_name_of(node: Node, content: &str) -> Option<String> {
    if let Some(id) = first_descendant(node, "identifier") {
        return node_text(id, content);
    }
    let text = collapse_ws(node_slice(node, content));
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Whether the node has a direct child of this kind — used for the anonymous
/// keyword tokens (`interface`, `val`, `var`) the grammar exposes unnamed.
fn has_child_kind(node: Node, kind: &str) -> bool {
    child_of_kind(node, kind).is_some()
}

/// The first direct child of the given kind.
fn child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| c.kind() == kind);
    found
}

/// First descendant of the given kind, depth-first from `node` itself.
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

/// The KDoc block (`/** … */`) immediately preceding a declaration.
///
/// Line-based rather than node-based: the comment is a sibling of the
/// declaration, and annotations sit inside the declaration's own `modifiers`,
/// so the KDoc is always on the lines directly above `node.start_position()`.
fn extract_doc_comment(node: Node, lines: &[&str]) -> Option<String> {
    let start_line = node.start_position().row;
    if start_line == 0 {
        return None;
    }

    let mut idx = start_line - 1;
    if !lines.get(idx).map(|s| s.trim_end().ends_with("*/")).unwrap_or(false) {
        return None;
    }

    let mut comments = Vec::new();
    loop {
        let line = lines.get(idx).map(|s| s.trim()).unwrap_or("");
        comments.insert(0, line.to_string());
        if line.starts_with("/**") {
            return Some(comments.join("\n"));
        }
        // A plain `/* … */` block is not KDoc.
        if line.starts_with("/*") || idx == 0 {
            return None;
        }
        idx -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn parse_code(code: &str) -> Vec<CodeItem> {
        let mut file = NamedTempFile::with_suffix(".kt").unwrap();
        file.write_all(code.as_bytes()).unwrap();
        parse_kotlin_file(file.path()).unwrap()
    }

    fn named<'a>(items: &'a [CodeItem], name: &str) -> &'a CodeItem {
        items.iter().find(|i| i.name == name).unwrap_or_else(|| {
            panic!(
                "no item named {name}; got {:?}",
                items.iter().map(|i| (&i.name, &i.item_type)).collect::<Vec<_>>()
            )
        })
    }

    /// The grammar folds class / interface / enum into `class_declaration` with
    /// no `declaration_kind` field, so the keyword token and `enum_class_body`
    /// are the only signals.
    #[test]
    fn test_class_interface_enum_disambiguation() {
        let items = parse_code(
            r#"
class Store {}
interface MemoService {}
fun interface Runner { fun run() }
enum class Kind { A, B }
enum class Empty
data class Memo(val id: String)
sealed class Result2 {}
annotation class MyAnno
value class Meters(val m: Double)
"#,
        );
        assert_eq!(named(&items, "Store").item_type, "class");
        assert_eq!(named(&items, "MemoService").item_type, "trait");
        assert_eq!(named(&items, "Runner").item_type, "trait", "fun interface is still an interface");
        assert_eq!(named(&items, "Kind").item_type, "enum");
        assert_eq!(named(&items, "Empty").item_type, "enum", "enum without a body");
        assert_eq!(named(&items, "Memo").item_type, "class");
        assert_eq!(named(&items, "Result2").item_type, "class");
        assert_eq!(named(&items, "MyAnno").item_type, "class");
        assert_eq!(named(&items, "Meters").item_type, "class");
    }

    #[test]
    fn test_object_and_companion() {
        let items = parse_code(
            r#"
object Singleton {
    fun go() {}
}

class Store {
    companion object {
        fun create(): Store = Store()
    }
}

class Other {
    companion object Named {
        fun make(): Other = Other()
    }
}
"#,
        );
        assert_eq!(named(&items, "Singleton").item_type, "class");
        assert_eq!(named(&items, "go").impl_for, Some("Singleton".to_string()));

        // Unnamed companion: item is `Companion`, owned by the enclosing class.
        let comp = named(&items, "Companion");
        assert_eq!(comp.item_type, "class");
        assert_eq!(comp.impl_for, Some("Store".to_string()));
        // ...but its members answer to `Store.create()`.
        assert_eq!(named(&items, "create").impl_for, Some("Store".to_string()));

        let named_comp = named(&items, "Named");
        assert_eq!(named_comp.item_type, "class");
        assert_eq!(named(&items, "make").impl_for, Some("Named".to_string()));
    }

    /// Two unnamed companions in one file must not collide into one node.
    #[test]
    fn test_two_unnamed_companions_get_distinct_ids() {
        let items = parse_code(
            r#"
class A {
    companion object {
        fun a() {}
    }
}
class B {
    companion object {
        fun b() {}
    }
}
"#,
        );
        let comps: Vec<&CodeItem> = items.iter().filter(|i| i.name == "Companion").collect();
        assert_eq!(comps.len(), 2);
        assert_ne!(comps[0].generate_id(), comps[1].generate_id());
    }

    #[test]
    fn test_method_carries_enclosing_type() {
        let items = parse_code(
            r#"
class ViewModel {
    private fun refresh() {}
    internal fun reload() {}
    suspend fun load() {}
}
"#,
        );
        let refresh = named(&items, "refresh");
        assert_eq!(refresh.item_type, "function");
        assert_eq!(refresh.impl_for, Some("ViewModel".to_string()));
        assert_eq!(refresh.visibility, "private");
        assert!(!refresh.is_async);

        assert_eq!(named(&items, "reload").visibility, "internal");

        let load = named(&items, "load");
        assert!(load.is_async, "suspend is Kotlin's async marker");
        assert_eq!(load.visibility, "", "no visibility modifier written");
    }

    /// Android code is full of extension functions; the receiver is positional,
    /// with no field to read it from.
    #[test]
    fn test_extension_function_receiver_becomes_impl_for() {
        let items = parse_code(
            r#"
fun Memo.describe(): String = "x"
fun Foo?.onNullable() {}
fun List<String>.joinAll(): String = ""
suspend fun CoroutineScope.ext() {}
fun <T> generic(x: T): T = x
fun plain(): Int = 1

class Holder {
    fun Memo.scoped() {}
}
"#,
        );
        assert_eq!(named(&items, "describe").impl_for, Some("Memo".to_string()));
        assert_eq!(named(&items, "onNullable").impl_for, Some("Foo".to_string()));
        assert_eq!(named(&items, "joinAll").impl_for, Some("List".to_string()));
        assert_eq!(named(&items, "ext").impl_for, Some("CoroutineScope".to_string()));
        assert!(named(&items, "ext").is_async);

        // `fun <T> generic(x: T): T` — the type parameter and the *return*
        // type must not be mistaken for a receiver.
        assert_eq!(named(&items, "generic").impl_for, None);
        assert_eq!(named(&items, "plain").impl_for, None);

        // A receiver beats the enclosing type.
        assert_eq!(named(&items, "scoped").impl_for, Some("Memo".to_string()));
    }

    #[test]
    fn test_top_level_val_and_var() {
        let items = parse_code(
            r#"
const val MAX = 3
var cursor: Int = 0
internal val name: String = "x"

class Store {
    private val cache = mutableListOf<String>()
    var dirty = false
}
"#,
        );
        assert_eq!(named(&items, "MAX").item_type, "const");
        assert_eq!(named(&items, "cursor").item_type, "var");
        assert_eq!(named(&items, "name").item_type, "const");
        assert_eq!(named(&items, "name").visibility, "internal");
        assert_eq!(named(&items, "cache").item_type, "const");
        assert_eq!(named(&items, "cache").visibility, "private");
        assert_eq!(named(&items, "cache").impl_for, Some("Store".to_string()));
        assert_eq!(named(&items, "dirty").item_type, "var");
        assert_eq!(named(&items, "MAX").impl_for, None, "top level has no owner");
    }

    /// Two classes in one file with a same-named property must stay two nodes.
    #[test]
    fn test_same_named_properties_in_one_file_get_distinct_ids() {
        let items = parse_code(
            r#"
class A {
    val id = 1
}
class B {
    val id = 2
}
"#,
        );
        let ids: Vec<&CodeItem> = items.iter().filter(|i| i.name == "id").collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0].generate_id(), ids[1].generate_id());
    }

    /// A `val` inside a function body is a local, not a graph node.
    #[test]
    fn test_local_bindings_are_not_items() {
        let items = parse_code(
            r#"
fun run() {
    val local = 1
    var other = 2
}

class A {
    constructor(x: Int) {
        val inCtor = 3
    }
    init {
        val inInit = 4
    }
}
"#,
        );
        for local in ["local", "other", "inCtor", "inInit"] {
            assert!(
                items.iter().all(|i| i.name != local),
                "{local} is a local, must not be an item: {:?}",
                items.iter().map(|i| &i.name).collect::<Vec<_>>()
            );
        }
    }

    /// The `type` field on `type_alias` is the alias *name*, not the aliased
    /// type — the trap this test exists to pin down.
    #[test]
    fn test_typealias_takes_the_alias_name() {
        let items = parse_code("typealias MemoList = List<Memo>\n");
        let t = named(&items, "MemoList");
        assert_eq!(t.item_type, "type");
        assert!(
            items.iter().all(|i| i.name != "List<Memo>" && i.name != "List"),
            "the aliased type must not become the item name"
        );
        assert!(t.signature.as_ref().unwrap().contains("typealias MemoList"));
    }

    #[test]
    fn test_calls_with_and_without_receiver() {
        let items = parse_code(
            r#"
class ViewModel {
    private val cache = mutableListOf<String>()
    fun refresh(id: String) {
        println("hi")
        this.reload()
        cache.add(id)
        Store.create()
    }
    fun reload() {}
}
"#,
        );
        let refresh = named(&items, "refresh");
        assert!(
            refresh.calls.iter().any(|c| c.name == "println" && c.receiver.is_none()),
            "bare call should have no receiver: {:?}",
            refresh.calls
        );
        assert!(
            refresh.calls.iter().any(|c| c.name == "reload" && c.receiver.as_deref() == Some("this")),
            "`this.` call keeps `this` verbatim: {:?}",
            refresh.calls
        );
        assert!(
            refresh.calls.iter().any(|c| c.name == "add" && c.receiver.as_deref() == Some("cache")),
            "member call should carry its receiver: {:?}",
            refresh.calls
        );
        assert!(
            refresh.calls.iter().any(|c| c.name == "create" && c.receiver.as_deref() == Some("Store")),
            "static-ish call should carry the type as receiver: {:?}",
            refresh.calls
        );

        // A property initializer is not a call site of the enclosing type.
        assert!(named(&items, "cache").calls.is_empty());
    }

    /// Calls inside a trailing lambda belong to the enclosing function, and the
    /// expression-body form (`fun f() = …`) has call sites too.
    #[test]
    fn test_calls_inside_lambdas_and_expression_bodies() {
        let items = parse_code(
            r#"
class ViewModel {
    fun refresh() {
        viewModelScope.launch {
            val memos = repository.listMemos()
            memos.forEach { it.render() }
        }
    }
    fun short() = helper(compute())
}
"#,
        );
        let refresh = named(&items, "refresh");
        for (name, recv) in [
            ("launch", Some("viewModelScope")),
            ("listMemos", Some("repository")),
            ("forEach", Some("memos")),
            ("render", Some("it")),
        ] {
            assert!(
                refresh.calls.iter().any(|c| c.name == name && c.receiver.as_deref() == recv),
                "expected {name} on {recv:?}: {:?}",
                refresh.calls
            );
        }

        let short = named(&items, "short");
        assert!(short.calls.iter().any(|c| c.name == "helper" && c.receiver.is_none()));
        assert!(short.calls.iter().any(|c| c.name == "compute" && c.receiver.is_none()));
    }

    #[test]
    fn test_kdoc_comment() {
        let items = parse_code(
            r#"
/**
 * Loads a memo from disk.
 * Returns null when absent.
 */
@Composable
fun load() {}

/** One-liner. */
class Store {}

// Not KDoc.
fun plain() {}

/* Plain block. */
fun alsoPlain() {}
"#,
        );
        let doc = named(&items, "load").doc_comment.as_ref().unwrap();
        assert!(doc.contains("Loads a memo from disk"));
        assert!(doc.contains("Returns null when absent"));
        assert!(
            doc.starts_with("/**"),
            "KDoc must survive an annotation between it and the declaration: {doc}"
        );

        assert_eq!(
            named(&items, "Store").doc_comment.as_deref(),
            Some("/** One-liner. */")
        );
        assert_eq!(named(&items, "plain").doc_comment, None);
        assert_eq!(named(&items, "alsoPlain").doc_comment, None);
    }

    /// Multibyte text anywhere in the source must not panic and must not shift
    /// line numbers. Byte-slicing one on a non-character boundary has already
    /// taken down this importer once (MCA-PANIC-01, in the Go parser).
    ///
    /// Identifiers here are Cyrillic rather than Han: the grammar rejects most
    /// CJK identifiers outright — see
    /// `test_cjk_identifiers_are_a_grammar_limitation` — so Han text lives in
    /// the strings, comments and KDoc, which is where the byte-slicing paths
    /// (`content`, `signature`, `doc_comment`, receiver truncation) run.
    #[test]
    fn test_multibyte_source_does_not_panic_and_keeps_line_numbers() {
        // Padded so that whatever fixed offset a truncation lands on, one of
        // the three variants cuts mid-character.
        for pad in 0..3 {
            let filler = "x".repeat(pad);
            let code = format!(
                "// 注释 🚀 comment\n\
                 /**\n\
                 \x20* 说明：这个函数返回一个备忘录 ✔\n\
                 \x20*/\n\
                 class Заметка {{\n\
                 \x20   val заголовок = \"{filler}配置文件已更新测试全部通过继续下一步 ✔ 🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀\"\n\
                 }}\n\
                 \n\
                 /** 载入 🚀 */\n\
                 fun загрузить(имя: String): String {{\n\
                 \x20   println(\"你好 🌏\")\n\
                 \x20   return \"{filler}结果 ✔\"\n\
                 }}\n"
            );
            let items = parse_code(&code);

            let f = named(&items, "загрузить");
            assert_eq!(f.item_type, "function", "pad {pad}");
            assert_eq!(f.line_start, 10, "pad {pad}: line numbers count lines, not bytes");
            assert_eq!(f.line_end, 13, "pad {pad}");
            assert!(f.doc_comment.as_ref().unwrap().contains("载入"), "pad {pad}");
            assert!(f.calls.iter().any(|c| c.name == "println"), "pad {pad}");

            let c = named(&items, "Заметка");
            assert_eq!(c.item_type, "class", "pad {pad}");
            assert_eq!(c.line_start, 5, "pad {pad}");
            assert!(
                c.content.contains("配置文件已更新"),
                "pad {pad}: content must keep the source text"
            );
            assert!(named(&items, "заголовок").signature.is_some(), "pad {pad}");
        }
    }

    /// A long multibyte signature is truncated on a character boundary.
    #[test]
    fn test_long_multibyte_signature_truncates_safely() {
        for pad in 0..3 {
            let code = format!(
                "val уведомление = \"{}配置文件已更新测试全部通过{} ✔ 🚀\"\n",
                "x".repeat(pad),
                "继续".repeat(40)
            );
            let items = parse_code(&code);
            let c = named(&items, "уведомление");
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

    /// `tree-sitter-kotlin-ng` 1.1.0's compiled `identifier` character set has
    /// holes in the Han block: `一` (U+4E00) and `鿿` (U+9FFF) are accepted but
    /// `备` (U+5907) is not, and a rejected identifier becomes a zero-width
    /// MISSING node that takes its declaration down with it. Kotlin itself
    /// allows the full `\p{L}`. Pinned so the day the grammar is fixed, this
    /// test fails and the limitation can be retired (MCA-KOTLIN-CJK-01).
    #[test]
    fn test_cjk_identifiers_are_a_grammar_limitation() {
        // Cyrillic, Greek, Kana and Hangul all round-trip.
        let ok = parse_code("val Ω = 1\nval заметка = 2\nval あ = 3\nval 가 = 4\n");
        for name in ["Ω", "заметка", "あ", "가"] {
            assert_eq!(named(&ok, name).item_type, "const");
        }

        // Han does not: the name comes back empty rather than wrong.
        let bad = parse_code("class 备忘录 {\n    val 标题 = 1\n}\n");
        assert!(
            bad.iter().all(|i| i.name != "备忘录"),
            "grammar unexpectedly handles Han identifiers now: {:?}",
            bad.iter().map(|i| &i.name).collect::<Vec<_>>()
        );
        // Whatever survives, nothing panics and no partial byte slice escapes.
        for item in &bad {
            assert!(item.signature.is_some());
        }
    }

    #[test]
    fn test_parse_full_file() {
        let items = parse_code(
            r#"
package me.mudkip.moememos.viewmodel

import androidx.lifecycle.ViewModel

/** A stored memo. */
data class Memo(val id: String, var title: String)

interface MemoService {
    suspend fun list(): List<Memo>
}

enum class Kind { A, B }

object Registry {
    fun register() {}
}

typealias MemoList = List<Memo>

const val DEFAULT_LIMIT = 20
var cursor: Int = 0

@HiltViewModel
class MemosViewModel @Inject constructor(
    private val repository: MemoRepository
) : ViewModel() {
    companion object {
        const val TAG = "MemosViewModel"
    }

    val memos = mutableStateListOf<Memo>()

    suspend fun refresh() {
        memos.addAll(repository.listMemos())
    }
}

fun Memo.render(): String = title
"#,
        );

        let by = |t: &str| items.iter().filter(|i| i.item_type == t).count();
        // Memo, Registry, Companion, MemosViewModel
        assert_eq!(by("class"), 4, "{:?}", items.iter().map(|i| (&i.name, &i.item_type)).collect::<Vec<_>>());
        assert_eq!(by("trait"), 1, "MemoService");
        assert_eq!(by("enum"), 1, "Kind");
        assert_eq!(by("type"), 1, "MemoList");
        // DEFAULT_LIMIT, TAG, memos
        assert_eq!(by("const"), 3);
        assert_eq!(by("var"), 1, "cursor");
        // list, register, refresh, render
        assert_eq!(by("function"), 4, "{:?}", items.iter().map(|i| &i.name).collect::<Vec<_>>());

        let memo = named(&items, "Memo");
        assert!(memo.doc_comment.as_ref().unwrap().contains("A stored memo"));

        let refresh = named(&items, "refresh");
        assert!(refresh.is_async);
        assert_eq!(refresh.impl_for, Some("MemosViewModel".to_string()));
        assert!(refresh
            .calls
            .iter()
            .any(|c| c.name == "listMemos" && c.receiver.as_deref() == Some("repository")));

        // `TAG` lives in MemosViewModel's companion, so it answers to
        // `MemosViewModel.TAG`.
        assert_eq!(named(&items, "TAG").impl_for, Some("MemosViewModel".to_string()));
        assert_eq!(named(&items, "DEFAULT_LIMIT").impl_for, None);
        assert_eq!(named(&items, "render").impl_for, Some("Memo".to_string()));
    }
}
