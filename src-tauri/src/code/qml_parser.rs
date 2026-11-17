//! QML parser using tree-sitter (`tree-sitter-qmljs`, QML plus the JavaScript
//! inside it).
//!
//! Extracts:
//! - The component the file defines (→ `class`), named after the file stem
//!   as QML names it, and inline `component Name: Type {}` (→ `class`)
//! - Objects with an `id:` (→ `var`), signature `Type { id: name }`
//! - Custom `property` declarations (→ `var`)
//! - `signal`s (→ `function`: emitting one is a call)
//! - JavaScript `function`s declared on an object (→ `function`)
//! - Signal handlers, bindings named `on<Upper>…` such as `onClicked` or
//!   `Component.onCompleted` (→ `function`): they carry outgoing calls only,
//!   and their title has no `name(` for call resolution to read
//! - `enum`s (→ `enum`)
//!
//! Every member's `impl_for` is its owner's label: the enclosing object's
//! `id`, else (for the root object) the component name, else its type name.
//! A call `root.reset()` or `txt.update()` records that id as its receiver, so
//! the shared resolver ties it to the member of that object. Function titles
//! are `owner.name(params)` rather than `function name(params)`, because the
//! shared name reader leaves `function f(` titles (TS/JS) unread.
//!
//! Item names are dotted owner paths (`root.txt.update`); an object with no
//! id below the root contributes `Type@line`, so two anonymous `Item`s each
//! declaring `function f()` stay distinct node IDs.
//!
//! Out of scope: edges from QML into C++ (`Q_INVOKABLE` methods, context
//! properties, registered types) and into imported `.js` files, which are a
//! different source (`code-javascript`).

use std::path::Path;
use tree_sitter::{Node, Parser};

use super::types::{CallRef, CodeItem};

/// Longest signature kept verbatim before it is truncated.
const MAX_SIGNATURE_BYTES: usize = 160;

/// Parse a QML file and extract code items.
pub fn parse_qml_file(path: &Path) -> Result<Vec<CodeItem>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_qmljs::LANGUAGE.into())
        .map_err(|e| format!("Failed to set language: {:?}", e))?;

    let tree = parser
        .parse(&content, None)
        .ok_or_else(|| "Failed to parse QML".to_string())?;

    let lines: Vec<&str> = content.lines().collect();
    let ctx = Ctx { file_path: path.to_string_lossy().to_string(), content: &content, lines: &lines };
    let component = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Component").to_string();

    let mut items = Vec::new();
    let root = tree.root_node();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        let def = match child.kind() {
            "ui_object_definition" => Some(child),
            "ui_annotated_object" => child.child_by_field_name("definition"),
            _ => None,
        };
        if let Some(def) = def {
            extract_component(def, child, &component, &[], &ctx, &mut items);
        }
    }
    Ok(items)
}

struct Ctx<'a> {
    file_path: String,
    content: &'a str,
    lines: &'a [&'a str],
}

/// A component: the file's root object, or an inline `component Name: Type`.
/// `span` is the node whose lines and leading comment the item takes.
fn extract_component(def: Node, span: Node, name: &str, path: &[String], ctx: &Ctx, items: &mut Vec<CodeItem>) {
    let ty = type_name(def, ctx.content);
    let mut qualified = path.to_vec();
    qualified.push(name.to_string());
    let sig = format!("component {name} : {ty}");
    items.push(base_item(span, ctx, qualified.join("."), "class", Some(sig), path.last().cloned()));

    // Members of the root object belong to its id, or to the component.
    let label = object_id(def, ctx.content).unwrap_or_else(|| name.to_string());
    let mut own = qualified;
    if label != name {
        own.push(label.clone());
    }
    if let Some(id) = object_id(def, ctx.content) {
        let sig = format!("{ty} {{ id: {id} }}");
        items.push(base_item(def, ctx, own.join("."), "var", Some(sig), Some(name.to_string())));
    }
    extract_members(def, &label, &own, ctx, items);
}

/// A nested object: an item of its own when it has an id; its members either way.
fn extract_object(def: Node, parent_label: &str, path: &[String], ctx: &Ctx, items: &mut Vec<CodeItem>) {
    let ty = type_name(def, ctx.content);
    let id = object_id(def, ctx.content);
    let mut own = path.to_vec();
    let label = match &id {
        Some(id) => {
            own.push(id.clone());
            let sig = format!("{ty} {{ id: {id} }}");
            items.push(base_item(def, ctx, own.join("."), "var", Some(sig), Some(parent_label.to_string())));
            id.clone()
        }
        None => {
            own.push(format!("{ty}@{}", def.start_position().row + 1));
            ty.clone()
        }
    };
    extract_members(def, &label, &own, ctx, items);
}

/// Walk an object's initializer. `label` is what members carry as `impl_for`,
/// `path` the owner chain their names are qualified with.
fn extract_members(def: Node, label: &str, path: &[String], ctx: &Ctx, items: &mut Vec<CodeItem>) {
    let Some(init) = def.child_by_field_name("initializer") else { return };
    let mut cursor = init.walk();
    for member in init.named_children(&mut cursor) {
        let (member, span) = match member.kind() {
            "ui_annotated_object_member" => match member.child_by_field_name("definition") {
                Some(d) => (d, member),
                None => continue,
            },
            _ => (member, member),
        };
        let qualify = |n: &str| {
            let mut p = path.to_vec();
            p.push(n.to_string());
            p.join(".")
        };
        match member.kind() {
            "ui_object_definition" => extract_object(member, label, path, ctx, items),
            "ui_object_definition_binding" => extract_object(member, label, path, ctx, items),
            "ui_inline_component" => {
                let name = member.child_by_field_name("name").map(|n| node_slice(n, ctx.content).to_string());
                if let (Some(name), Some(comp)) = (name, member.child_by_field_name("component")) {
                    extract_component(comp, span, &name, path, ctx, items);
                }
            }
            "ui_binding" => {
                let Some(name) = member.child_by_field_name("name").map(|n| collapse_ws(node_slice(n, ctx.content))) else {
                    continue;
                };
                let value = member.child_by_field_name("value");
                // `delegate: Item {}` / `contentItem: Text { id: x }` hold objects.
                if let Some(v) = value {
                    match v.kind() {
                        "ui_object_definition" => {
                            extract_object(v, label, path, ctx, items);
                            continue;
                        }
                        "ui_object_array" => {
                            let mut c = v.walk();
                            for obj in v.named_children(&mut c).filter(|o| o.kind() == "ui_object_definition") {
                                extract_object(obj, label, path, ctx, items);
                            }
                            continue;
                        }
                        _ => {}
                    }
                }
                if !is_handler(&name) {
                    continue;
                }
                let sig = format!("{label}.{name}: handler");
                let mut item = base_item(span, ctx, qualify(&name), "function", Some(sig), Some(label.to_string()));
                if let Some(v) = value {
                    collect_calls(v, ctx.content, &mut item.calls);
                }
                items.push(item);
            }
            "ui_property" => {
                let Some(name) = member.child_by_field_name("name").map(|n| node_slice(n, ctx.content).to_string()) else {
                    continue;
                };
                let ty = member.child_by_field_name("type").map(|n| collapse_ws(node_slice(n, ctx.content))).unwrap_or_default();
                let mut words = Vec::new();
                let mut c = member.walk();
                for m in member.children(&mut c).filter(|m| m.kind() == "ui_property_modifier") {
                    words.push(node_slice(m, ctx.content).to_string());
                }
                words.push(format!("property {ty} {name}"));
                let mut item = base_item(span, ctx, qualify(&name), "var", Some(words.join(" ")), Some(label.to_string()));
                if let Some(v) = member.child_by_field_name("value") {
                    match v.kind() {
                        "ui_object_definition" => extract_object(v, label, path, ctx, items),
                        _ => collect_calls(v, ctx.content, &mut item.calls),
                    }
                }
                items.push(item);
            }
            "ui_signal" => {
                let Some(name) = member.child_by_field_name("name").map(|n| node_slice(n, ctx.content).to_string()) else {
                    continue;
                };
                let params = member
                    .child_by_field_name("parameters")
                    .map(|p| collapse_ws(node_slice(p, ctx.content)))
                    .unwrap_or_else(|| "()".to_string());
                let sig = truncated_signature(&format!("signal {name}{params}"));
                items.push(base_item(span, ctx, qualify(&name), "function", Some(sig), Some(label.to_string())));
            }
            "function_declaration" | "generator_function_declaration" => {
                let Some(name) = member.child_by_field_name("name").map(|n| node_slice(n, ctx.content).to_string()) else {
                    continue;
                };
                let params = member
                    .child_by_field_name("parameters")
                    .map(|p| collapse_ws(node_slice(p, ctx.content)))
                    .unwrap_or_else(|| "()".to_string());
                let sig = truncated_signature(&format!("{label}.{name}{params}"));
                let mut item = base_item(span, ctx, qualify(&name), "function", Some(sig), Some(label.to_string()));
                if let Some(body) = member.child_by_field_name("body") {
                    collect_calls(body, ctx.content, &mut item.calls);
                }
                items.push(item);
            }
            "enum_declaration" => {
                if let Some(name) = member.child_by_field_name("name").map(|n| node_slice(n, ctx.content).to_string()) {
                    let sig = format!("enum {name}");
                    items.push(base_item(span, ctx, qualify(&name), "enum", Some(sig), Some(label.to_string())));
                }
            }
            _ => {}
        }
    }
}

/// `onClicked`, `Component.onCompleted`, `Keys.onPressed`: the last segment
/// is `on` followed by an uppercase letter.
fn is_handler(binding: &str) -> bool {
    let last = binding.rsplit('.').next().unwrap_or(binding);
    last.strip_prefix("on").and_then(|r| r.chars().next()).is_some_and(|c| c.is_ascii_uppercase())
}

/// The value of an object's `id:` binding.
fn object_id(def: Node, content: &str) -> Option<String> {
    let init = def.child_by_field_name("initializer")?;
    let mut cursor = init.walk();
    let found = init.named_children(&mut cursor).find_map(|m| {
        if m.kind() != "ui_binding" || m.child_by_field_name("name").map(|n| node_slice(n, content)) != Some("id") {
            return None;
        }
        let v = m.child_by_field_name("value")?;
        let text = node_slice(v, content).trim().trim_end_matches(';').trim();
        (!text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| text.to_string())
    });
    found
}

fn type_name(def: Node, content: &str) -> String {
    def.child_by_field_name("type_name")
        .map(|n| collapse_ws(node_slice(n, content)))
        .unwrap_or_default()
}

/// A `CodeItem` with the fields every declaration shares filled in.
fn base_item(span: Node, ctx: &Ctx, name: String, item_type: &str, signature: Option<String>, impl_for: Option<String>) -> CodeItem {
    let (line_start, line_end) = (span.start_position().row + 1, span.end_position().row + 1);
    CodeItem {
        name,
        item_type: item_type.to_string(),
        file_path: ctx.file_path.clone(),
        line_start,
        line_end,
        content: source_lines(ctx.lines, line_start, line_end),
        signature,
        visibility: "pub".to_string(),
        doc_comment: extract_leading_comment(span, ctx.lines),
        is_async: false,
        is_unsafe: false,
        impl_for,
        impl_trait: None,
        calls: Vec::new(),
    }
}

// ============== Calls ==============

/// Call sites anywhere under `node`, nested functions and arrow functions
/// included: `f()` → no receiver, `obj.f()` / `a.b.f()` → the object text.
fn collect_calls(node: Node, content: &str, calls: &mut Vec<CallRef>) {
    if node.kind() == "call_expression" {
        if let Some(callee) = node.child_by_field_name("function") {
            let line = callee.start_position().row + 1;
            match callee.kind() {
                "identifier" => calls.push(CallRef { name: node_slice(callee, content).to_string(), receiver: None, line }),
                "member_expression" => {
                    let name = callee.child_by_field_name("property").map(|p| node_slice(p, content).to_string());
                    let receiver = callee
                        .child_by_field_name("object")
                        .map(|o| crate::utils::safe_truncate(&collapse_ws(node_slice(o, content)), 64).to_string());
                    if let Some(name) = name {
                        calls.push(CallRef { name, receiver, line });
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

/// The `//` or `/* */` comment block directly above a declaration.
fn extract_leading_comment(node: Node, lines: &[&str]) -> Option<String> {
    let mut idx = node.start_position().row;
    let mut comments = Vec::new();
    while idx > 0 {
        idx -= 1;
        let line = lines.get(idx).map(|s| s.trim()).unwrap_or("");
        if line.starts_with("//") || line.starts_with("/*") || line.starts_with('*') || line.ends_with("*/") {
            comments.insert(0, line.to_string());
        } else {
            break;
        }
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
    use std::collections::HashSet;
    use std::io::Write;

    const SAMPLE: &str = r#"import QtQuick
import "logic.js" as Logic

// Main window.
Rectangle {
    id: root
    property int count: 0
    required property string name
    signal clicked(int x, string why)
    enum Mode { A, B }

    function reset(n) { count = n; helper(); root.helper(); Logic.go(); txt.update() }
    function helper() {}

    onClicked: (x) => reset(x)
    Component.onCompleted: { reset(1) }
    width: Math.max(1, 2)

    Text {
        id: txt
        onTextChanged: root.reset(0)
        function update() {}
    }
    component Badge: Item { id: badge; function show() {} }
    Item { function f() {} }
    Item { function f() {} }
}
"#;

    fn parse_sample() -> Vec<CodeItem> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Main.qml");
        std::fs::File::create(&path).unwrap().write_all(SAMPLE.as_bytes()).unwrap();
        parse_qml_file(&path).unwrap()
    }

    fn named<'a>(items: &'a [CodeItem], name: &str) -> &'a CodeItem {
        items.iter().find(|i| i.name == name).unwrap_or_else(|| {
            panic!("no item named {name}; got {:?}", items.iter().map(|i| (&i.name, &i.item_type)).collect::<Vec<_>>())
        })
    }

    fn calls_of(item: &CodeItem) -> Vec<(String, Option<String>)> {
        item.calls.iter().map(|c| (c.name.clone(), c.receiver.clone())).collect()
    }

    fn call(name: &str, receiver: Option<&str>) -> (String, Option<String>) {
        (name.to_string(), receiver.map(str::to_string))
    }

    #[test]
    fn test_component_and_ids() {
        let items = parse_sample();
        let main = named(&items, "Main");
        assert_eq!(main.item_type, "class");
        assert_eq!(main.signature.as_deref(), Some("component Main : Rectangle"));
        assert_eq!(main.doc_comment.as_deref(), Some("// Main window."));
        let root = named(&items, "Main.root");
        assert_eq!(root.item_type, "var");
        assert_eq!(root.signature.as_deref(), Some("Rectangle { id: root }"));
        let txt = named(&items, "Main.root.txt");
        assert_eq!(txt.signature.as_deref(), Some("Text { id: txt }"));
        assert_eq!(txt.impl_for.as_deref(), Some("root"));
        let badge = named(&items, "Main.root.Badge");
        assert_eq!(badge.item_type, "class");
        assert_eq!(badge.signature.as_deref(), Some("component Badge : Item"));
        named(&items, "Main.root.Badge.badge");
        assert_eq!(named(&items, "Main.root.Badge.badge.show").impl_for.as_deref(), Some("badge"));
    }

    #[test]
    fn test_members() {
        let items = parse_sample();
        let count = named(&items, "Main.root.count");
        assert_eq!(count.item_type, "var");
        assert_eq!(count.signature.as_deref(), Some("property int count"));
        assert_eq!(named(&items, "Main.root.name").signature.as_deref(), Some("required property string name"));
        let clicked = named(&items, "Main.root.clicked");
        assert_eq!(clicked.item_type, "function");
        assert_eq!(clicked.signature.as_deref(), Some("signal clicked(int x, string why)"));
        assert_eq!(named(&items, "Main.root.Mode").item_type, "enum");
        let reset = named(&items, "Main.root.reset");
        assert_eq!(reset.signature.as_deref(), Some("root.reset(n)"));
        assert_eq!(reset.impl_for.as_deref(), Some("root"));
        // A plain property binding is not an item.
        assert!(!items.iter().any(|i| i.name.ends_with(".width")));
    }

    #[test]
    fn test_calls_and_handlers() {
        let items = parse_sample();
        assert_eq!(
            calls_of(named(&items, "Main.root.reset")),
            vec![call("helper", None), call("helper", Some("root")), call("go", Some("Logic")), call("update", Some("txt"))]
        );
        let on_clicked = named(&items, "Main.root.onClicked");
        assert_eq!(on_clicked.item_type, "function");
        assert_eq!(on_clicked.signature.as_deref(), Some("root.onClicked: handler"));
        assert_eq!(calls_of(on_clicked), vec![call("reset", None)]);
        assert_eq!(calls_of(named(&items, "Main.root.Component.onCompleted")), vec![call("reset", None)]);
        assert_eq!(calls_of(named(&items, "Main.root.txt.onTextChanged")), vec![call("reset", Some("root"))]);
    }

    #[test]
    fn test_anonymous_objects_get_distinct_ids() {
        let items = parse_sample();
        let ids: HashSet<String> = items.iter().map(|i| i.generate_id()).collect();
        assert_eq!(ids.len(), items.len());
        assert_eq!(items.iter().filter(|i| i.name.ends_with(".f")).count(), 2);
    }

    #[test]
    fn test_handler_names() {
        assert!(is_handler("onClicked"));
        assert!(is_handler("Component.onCompleted"));
        assert!(!is_handler("one"));
        assert!(!is_handler("on"));
        assert!(!is_handler("width"));
    }
}
