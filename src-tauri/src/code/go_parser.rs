//! Go parser using tree-sitter.
//!
//! Extracts:
//! - Functions (func)
//! - Methods (func with receiver)
//! - Structs (type X struct)
//! - Interfaces (type X interface)
//! - Type definitions (type X underlying)
//! - Constants (const)
//! - Variables (var) — package-level only
//!
//! Handles _test.go files by tagging items with is_test metadata.

use std::path::Path;
use tree_sitter::{Parser, Node};

use super::types::{CodeItem, CallRef};

/// Parse a Go file and extract code items.
pub fn parse_go_file(path: &Path) -> Result<Vec<CodeItem>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_go::LANGUAGE.into())
        .map_err(|e| format!("Failed to set language: {:?}", e))?;

    let tree = parser.parse(&content, None)
        .ok_or_else(|| "Failed to parse Go".to_string())?;

    let file_path = path.to_string_lossy().to_string();
    let lines: Vec<&str> = content.lines().collect();
    let test_file = is_test_file(path);

    let mut items = Vec::new();
    extract_items(tree.root_node(), &file_path, &content, &lines, &mut items, test_file);

    Ok(items)
}

/// Check if a file is a Go test file (_test.go).
fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.ends_with("_test.go"))
        .unwrap_or(false)
}

/// Recursively extract code items from the AST.
fn extract_items(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    items: &mut Vec<CodeItem>,
    test_file: bool,
) {
    let kind = node.kind();

    match kind {
        "function_declaration" => {
            if let Some(mut item) = extract_function(node, file_path, content, lines) {
                if test_file {
                    item.doc_comment = tag_test_doc(item.doc_comment);
                }
                items.push(item);
            }
        }

        "method_declaration" => {
            if let Some(mut item) = extract_method(node, file_path, content, lines) {
                if test_file {
                    item.doc_comment = tag_test_doc(item.doc_comment);
                }
                items.push(item);
            }
        }

        "type_declaration" => {
            // type_declaration contains one or more type_spec children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_spec" {
                    if let Some(mut item) = extract_type_spec(child, file_path, content, lines, node) {
                        if test_file {
                            item.doc_comment = tag_test_doc(item.doc_comment);
                        }
                        items.push(item);
                    }
                }
            }
        }

        "const_declaration" => {
            extract_const_declaration(node, file_path, content, lines, items, test_file);
        }

        "var_declaration" => {
            extract_var_declaration(node, file_path, content, lines, items, test_file);
        }

        _ => {}
    }

    // Recurse for non-handled nodes (source_file, etc.)
    if !matches!(kind, "function_declaration" | "method_declaration"
                      | "type_declaration" | "const_declaration" | "var_declaration") {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_items(child, file_path, content, lines, items, test_file);
        }
    }
}

/// Extract a function declaration.
fn extract_function(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    // Build signature: func Name(params) result
    let params = node.child_by_field_name("parameters")
        .and_then(|n| get_node_text(n, content))
        .unwrap_or("()".to_string());

    let result = node.child_by_field_name("result")
        .and_then(|n| get_node_text(n, content))
        .map(|r| format!(" {}", r))
        .unwrap_or_default();

    let signature = format!("func {}{}{}", name, params, result);

    let visibility = if is_exported(&name) { "pub".to_string() } else { String::new() };
    let doc_comment = extract_leading_comment(node, content, lines);
    let calls = node.child_by_field_name("body")
        .map(|body| extract_calls(body, content))
        .unwrap_or_default();

    Some(CodeItem {
        name,
        item_type: "function".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: Some(signature),
        visibility,
        doc_comment,
        is_async: false,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
        calls,
    })
}

/// Extract a method declaration (function with receiver).
fn extract_method(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    // Extract receiver text and type
    let receiver_node = node.child_by_field_name("receiver")?;
    let receiver_text = get_node_text(receiver_node, content).unwrap_or("()".to_string());
    let receiver_type = extract_receiver_type(receiver_node, content);

    let params = node.child_by_field_name("parameters")
        .and_then(|n| get_node_text(n, content))
        .unwrap_or("()".to_string());

    let result = node.child_by_field_name("result")
        .and_then(|n| get_node_text(n, content))
        .map(|r| format!(" {}", r))
        .unwrap_or_default();

    let signature = format!("func {} {}{}{}", receiver_text, name, params, result);

    let visibility = if is_exported(&name) { "pub".to_string() } else { String::new() };
    let doc_comment = extract_leading_comment(node, content, lines);
    let calls = node.child_by_field_name("body")
        .map(|body| extract_calls(body, content))
        .unwrap_or_default();

    Some(CodeItem {
        name,
        item_type: "function".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: Some(signature),
        visibility,
        doc_comment,
        is_async: false,
        is_unsafe: false,
        impl_for: receiver_type,
        impl_trait: None,
        calls,
    })
}

/// Extract the receiver type name from a parameter_list node.
/// For `(s *Server)` returns `Some("Server")`.
/// For `(s Server)` returns `Some("Server")`.
fn extract_receiver_type(receiver_node: Node, content: &str) -> Option<String> {
    // receiver is a parameter_list: (name type) or (type)
    let mut cursor = receiver_node.walk();
    for child in receiver_node.children(&mut cursor) {
        if child.kind() == "parameter_declaration" {
            // Get the type field
            if let Some(type_node) = child.child_by_field_name("type") {
                return extract_type_name(type_node, content);
            }
            // Fallback: if no named "type" field, look at last child
            let child_count = child.child_count();
            if child_count > 0 {
                let last = child.child(child_count - 1)?;
                return extract_type_name(last, content);
            }
        }
    }
    None
}

/// Extract the base type name, stripping pointer (*) and package qualifiers.
fn extract_type_name(node: Node, content: &str) -> Option<String> {
    match node.kind() {
        "pointer_type" => {
            // *Type -> recurse into the pointee
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() != "*" {
                    return extract_type_name(child, content);
                }
            }
            None
        }
        "type_identifier" => get_node_text(node, content),
        "qualified_type" => {
            // package.Type -> just use Type
            node.child_by_field_name("name")
                .and_then(|n| get_node_text(n, content))
        }
        _ => get_node_text(node, content),
    }
}

/// Extract a type_spec node (inside type_declaration).
/// Determines struct vs interface vs type alias from the type field.
fn extract_type_spec(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    parent_decl: Node,
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    let type_node = node.child_by_field_name("type")?;
    let type_kind = type_node.kind();

    // Use the parent type_declaration span for full content (includes `type` keyword)
    let (line_start, line_end) = get_node_lines(parent_decl);
    let item_content = extract_source_lines(lines, line_start, line_end);

    let (item_type, signature) = match type_kind {
        "struct_type" => ("struct", format!("type {} struct", name)),
        "interface_type" => ("interface", format!("type {} interface", name)),
        _ => {
            let type_text = get_node_text(type_node, content).unwrap_or_default();
            ("type", format!("type {} {}", name, type_text))
        }
    };

    let visibility = if is_exported(&name) { "pub".to_string() } else { String::new() };
    let doc_comment = extract_leading_comment(parent_decl, content, lines);

    Some(CodeItem {
        name,
        item_type: item_type.to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: Some(signature),
        visibility,
        doc_comment,
        is_async: false,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
        calls: Vec::new(),
    })
}

/// Extract const declarations. Handles both single consts and const blocks.
fn extract_const_declaration(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    items: &mut Vec<CodeItem>,
    test_file: bool,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "const_spec" {
            if let Some(mut item) = extract_const_or_var_spec(child, file_path, content, lines, "const", node) {
                if test_file {
                    item.doc_comment = tag_test_doc(item.doc_comment);
                }
                items.push(item);
            }
        }
    }
}

/// Extract var declarations. Handles both single vars and var blocks.
fn extract_var_declaration(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    items: &mut Vec<CodeItem>,
    test_file: bool,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "var_spec" {
            if let Some(mut item) = extract_const_or_var_spec(child, file_path, content, lines, "var", node) {
                if test_file {
                    item.doc_comment = tag_test_doc(item.doc_comment);
                }
                items.push(item);
            }
        }
    }
}

/// Extract a single const_spec or var_spec.
fn extract_const_or_var_spec(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    kind: &str, // "const" or "var"
    parent_decl: Node,
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    // Get optional type annotation
    let type_text = node.child_by_field_name("type")
        .and_then(|n| get_node_text(n, content));

    // Get optional value
    let value = node.child_by_field_name("value")
        .and_then(|n| get_node_text(n, content))
        .map(|v| {
            let trimmed = v.trim();
            if trimmed.len() > 40 {
                // Byte length, byte slice: `&trimmed[..37]` panicked whenever a
                // Go string literal held multibyte text.
                format!("{}...", crate::utils::safe_truncate(trimmed, 37))
            } else {
                trimmed.to_string()
            }
        });

    let signature = match (&type_text, &value) {
        (Some(t), Some(v)) => format!("{} {} {} = {}", kind, name, t, v),
        (Some(t), None) => format!("{} {} {}", kind, name, t),
        (None, Some(v)) => format!("{} {} = {}", kind, name, v),
        (None, None) => format!("{} {}", kind, name),
    };

    let visibility = if is_exported(&name) { "pub".to_string() } else { String::new() };
    let doc_comment = extract_leading_comment(parent_decl, content, lines);

    Some(CodeItem {
        name,
        item_type: kind.to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: Some(signature),
        visibility,
        doc_comment,
        is_async: false,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
        calls: Vec::new(),
    })
}

/// Walk a tree-sitter node for call_expression nodes and collect CallRefs.
fn extract_calls(node: Node, content: &str) -> Vec<CallRef> {
    let mut calls = Vec::new();
    collect_calls(node, content, &mut calls);
    calls
}

fn collect_calls(node: Node, content: &str, calls: &mut Vec<CallRef>) {
    if node.kind() == "call_expression" {
        if let Some(func_node) = node.child_by_field_name("function") {
            match func_node.kind() {
                "identifier" => {
                    if let Some(name) = get_node_text(func_node, content) {
                        calls.push(CallRef {
                            name,
                            receiver: None,
                            line: func_node.start_position().row + 1,
                        });
                    }
                }
                "selector_expression" => {
                    if let Some(field) = func_node.child_by_field_name("field") {
                        if let Some(name) = get_node_text(field, content) {
                            let receiver = func_node.child_by_field_name("operand")
                                .and_then(|n| get_node_text(n, content));
                            calls.push(CallRef {
                                name,
                                receiver,
                                line: field.start_position().row + 1,
                            });
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

/// Get text content of a node.
fn get_node_text(node: Node, content: &str) -> Option<String> {
    node.utf8_text(content.as_bytes())
        .ok()
        .map(|s| s.to_string())
}

/// Get 1-indexed line numbers for a node.
fn get_node_lines(node: Node) -> (usize, usize) {
    let start = node.start_position().row + 1;
    let end = node.end_position().row + 1;
    (start, end)
}

/// Extract source lines from line numbers.
fn extract_source_lines(lines: &[&str], start: usize, end: usize) -> String {
    lines
        .get(start.saturating_sub(1)..end.min(lines.len()))
        .map(|slice| slice.join("\n"))
        .unwrap_or_default()
}

/// Check if a Go identifier is exported (starts with uppercase).
fn is_exported(name: &str) -> bool {
    name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
}

/// Tag a doc comment with is_test marker for _test.go files.
fn tag_test_doc(doc: Option<String>) -> Option<String> {
    match doc {
        Some(d) => Some(format!("[test] {}", d)),
        None => Some("[test]".to_string()),
    }
}

/// Extract leading Go-style comments (// lines) preceding a declaration.
fn extract_leading_comment(node: Node, content: &str, lines: &[&str]) -> Option<String> {
    let start_line = node.start_position().row;
    if start_line == 0 {
        return None;
    }

    let mut comments = Vec::new();
    let mut line_idx = start_line.saturating_sub(1);

    // Look backwards for // comment lines
    loop {
        let line = lines.get(line_idx).map(|s| s.trim()).unwrap_or("");

        if line.starts_with("//") {
            comments.insert(0, line.to_string());
        } else if line.is_empty() && !comments.is_empty() {
            // Empty line after finding comments — stop
            break;
        } else if !line.is_empty() {
            // Non-comment, non-empty line — stop
            break;
        }

        if line_idx == 0 {
            break;
        }
        line_idx -= 1;
    }

    // Also check prev_sibling for comment nodes in the AST
    if comments.is_empty() {
        let mut prev = node.prev_sibling();
        while let Some(p) = prev {
            if p.kind() == "comment" {
                if let Some(text) = get_node_text(p, content) {
                    comments.insert(0, text);
                    break;
                }
            } else {
                break;
            }
            prev = p.prev_sibling();
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
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn parse_code(code: &str) -> Vec<CodeItem> {
        let mut file = NamedTempFile::with_suffix(".go").unwrap();
        file.write_all(code.as_bytes()).unwrap();
        parse_go_file(file.path()).unwrap()
    }

    fn parse_test_code(code: &str) -> Vec<CodeItem> {
        let mut file = NamedTempFile::with_suffix("_test.go").unwrap();
        file.write_all(code.as_bytes()).unwrap();
        parse_go_file(file.path()).unwrap()
    }

    #[test]
    fn test_parse_exported_function() {
        let items = parse_code(r#"
package main

func Add(a int, b int) int {
    return a + b
}
"#);
        let funcs: Vec<_> = items.iter().filter(|i| i.item_type == "function").collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "Add");
        assert_eq!(funcs[0].visibility, "pub");
        assert!(funcs[0].signature.as_ref().unwrap().contains("func Add"));
        assert!(funcs[0].signature.as_ref().unwrap().contains("int"));
    }

    #[test]
    fn test_parse_unexported_function() {
        let items = parse_code(r#"
package main

func helper() {
    // internal
}
"#);
        let funcs: Vec<_> = items.iter().filter(|i| i.item_type == "function").collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "helper");
        assert_eq!(funcs[0].visibility, "");
    }

    #[test]
    fn test_parse_method_pointer_receiver() {
        let items = parse_code(r#"
package main

type Server struct{}

func (s *Server) Listen() error {
    return nil
}
"#);
        let methods: Vec<_> = items.iter()
            .filter(|i| i.item_type == "function" && i.impl_for.is_some())
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "Listen");
        assert_eq!(methods[0].impl_for, Some("Server".to_string()));
        assert_eq!(methods[0].visibility, "pub");
    }

    #[test]
    fn test_parse_method_value_receiver() {
        let items = parse_code(r#"
package main

type Point struct{ X, Y int }

func (p Point) String() string {
    return "point"
}
"#);
        let methods: Vec<_> = items.iter()
            .filter(|i| i.impl_for.is_some())
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "String");
        assert_eq!(methods[0].impl_for, Some("Point".to_string()));
    }

    #[test]
    fn test_parse_struct() {
        let items = parse_code(r#"
package main

type Config struct {
    Host    string
    Port    int
    Timeout int
}
"#);
        let structs: Vec<_> = items.iter().filter(|i| i.item_type == "struct").collect();
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "Config");
        assert_eq!(structs[0].visibility, "pub");
        assert!(structs[0].signature.as_ref().unwrap().contains("type Config struct"));
    }

    #[test]
    fn test_parse_interface() {
        let items = parse_code(r#"
package io

type Reader interface {
    Read(p []byte) (n int, err error)
}
"#);
        let interfaces: Vec<_> = items.iter().filter(|i| i.item_type == "interface").collect();
        assert_eq!(interfaces.len(), 1);
        assert_eq!(interfaces[0].name, "Reader");
        assert_eq!(interfaces[0].visibility, "pub");
        assert!(interfaces[0].signature.as_ref().unwrap().contains("type Reader interface"));
    }

    #[test]
    fn test_parse_type_alias() {
        let items = parse_code(r#"
package main

type Milliseconds int64
"#);
        let types: Vec<_> = items.iter().filter(|i| i.item_type == "type").collect();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].name, "Milliseconds");
        assert!(types[0].signature.as_ref().unwrap().contains("type Milliseconds int64"));
    }

    #[test]
    fn test_parse_single_const() {
        let items = parse_code(r#"
package main

const MaxRetries = 3
"#);
        let consts: Vec<_> = items.iter().filter(|i| i.item_type == "const").collect();
        assert_eq!(consts.len(), 1);
        assert_eq!(consts[0].name, "MaxRetries");
        assert_eq!(consts[0].visibility, "pub");
    }

    #[test]
    fn test_parse_const_block() {
        let items = parse_code(r#"
package main

const (
    Red   = iota
    Green
    Blue
)
"#);
        let consts: Vec<_> = items.iter().filter(|i| i.item_type == "const").collect();
        assert_eq!(consts.len(), 3);
        assert_eq!(consts[0].name, "Red");
        assert_eq!(consts[1].name, "Green");
        assert_eq!(consts[2].name, "Blue");
    }

    #[test]
    fn test_parse_var_declaration() {
        let items = parse_code(r#"
package main

import "errors"

var ErrNotFound = errors.New("not found")
"#);
        let vars: Vec<_> = items.iter().filter(|i| i.item_type == "var").collect();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "ErrNotFound");
        assert_eq!(vars[0].visibility, "pub");
    }

    #[test]
    fn test_parse_doc_comment() {
        let items = parse_code(r#"
package main

// Add adds two integers and returns the result.
// It handles both positive and negative numbers.
func Add(a, b int) int {
    return a + b
}
"#);
        let funcs: Vec<_> = items.iter().filter(|i| i.item_type == "function").collect();
        assert_eq!(funcs.len(), 1);
        assert!(funcs[0].doc_comment.is_some());
        let doc = funcs[0].doc_comment.as_ref().unwrap();
        assert!(doc.contains("Add adds two integers"));
    }

    #[test]
    fn test_parse_init_functions() {
        let items = parse_code(r#"
package main

func init() {
    // first init
}

func init() {
    // second init
}
"#);
        let inits: Vec<_> = items.iter().filter(|i| i.name == "init").collect();
        assert_eq!(inits.len(), 2);
        assert_eq!(inits[0].visibility, "");
        assert_eq!(inits[1].visibility, "");
    }

    #[test]
    fn test_test_file_tagging() {
        let items = parse_test_code(r#"
package main

func TestAdd(t *testing.T) {
    // test
}
"#);
        let funcs: Vec<_> = items.iter().filter(|i| i.item_type == "function").collect();
        assert_eq!(funcs.len(), 1);
        assert!(funcs[0].doc_comment.as_ref().unwrap().contains("[test]"));
    }

    #[test]
    fn test_extract_calls() {
        let items = parse_code(r#"
package main

import "fmt"

func greet(name string) string {
    return fmt.Sprintf("hello %s", name)
}

func main() {
    msg := greet("world")
    fmt.Println(msg)
}
"#);
        let main_fn = items.iter().find(|i| i.name == "main").unwrap();
        assert!(main_fn.calls.iter().any(|c| c.name == "greet" && c.receiver.is_none()));
        assert!(main_fn.calls.iter().any(|c| c.name == "Println" && c.receiver.as_deref() == Some("fmt")));
        let greet_fn = items.iter().find(|i| i.name == "greet").unwrap();
        assert!(greet_fn.calls.iter().any(|c| c.name == "Sprintf" && c.receiver.as_deref() == Some("fmt")));
    }

    #[test]
    fn test_parse_full_file() {
        let items = parse_code(r#"
package session

import (
    "context"
    "fmt"
)

// Manager handles session lifecycle.
type Manager struct {
    sessions map[string]*Session
}

// Session represents a single session.
type Session struct {
    Name   string
    Status string
}

// NewManager creates a new Manager.
func NewManager() *Manager {
    return &Manager{sessions: make(map[string]*Session)}
}

// Start starts a session by name.
func (m *Manager) Start(ctx context.Context, name string) error {
    return nil
}

func (m *Manager) stop(name string) {
    // unexported method
}

const DefaultTimeout = 30

var ErrTimeout = fmt.Errorf("timeout")

type SessionID string
"#);
        // Count by type
        let structs: Vec<_> = items.iter().filter(|i| i.item_type == "struct").collect();
        let functions: Vec<_> = items.iter().filter(|i| i.item_type == "function" && i.impl_for.is_none()).collect();
        let methods: Vec<_> = items.iter().filter(|i| i.item_type == "function" && i.impl_for.is_some()).collect();
        let consts: Vec<_> = items.iter().filter(|i| i.item_type == "const").collect();
        let vars: Vec<_> = items.iter().filter(|i| i.item_type == "var").collect();
        let types: Vec<_> = items.iter().filter(|i| i.item_type == "type").collect();

        assert_eq!(structs.len(), 2, "Expected 2 structs (Manager, Session)");
        assert_eq!(functions.len(), 1, "Expected 1 function (NewManager)");
        assert_eq!(methods.len(), 2, "Expected 2 methods (Start, stop)");
        assert_eq!(consts.len(), 1, "Expected 1 const (DefaultTimeout)");
        assert_eq!(vars.len(), 1, "Expected 1 var (ErrTimeout)");
        assert_eq!(types.len(), 1, "Expected 1 type (SessionID)");

        // Check visibility
        assert_eq!(functions[0].visibility, "pub"); // NewManager
        assert_eq!(methods[0].visibility, "pub");   // Start
        assert_eq!(methods[1].visibility, "");       // stop
    }

    /// A Go string literal holding multibyte text must not abort the parse.
    ///
    /// The const value was truncated with `&trimmed[..37]` under a `len() > 40`
    /// byte check, so a CJK or emoji literal whose 37th byte fell inside a
    /// character panicked — and because the panic unwound out of the importer,
    /// one such file stopped the whole graph import.
    ///
    /// Padded three ways so one of them lands mid-character whatever the cut
    /// offset is; a single fixture can sit on a boundary by luck and prove
    /// nothing.
    #[test]
    fn test_parse_const_with_multibyte_value_does_not_panic() {
        for pad in 0..3 {
            let value = format!("{}配置文件已更新测试全部通过继续下一步 ✔ 🚀", "x".repeat(pad));
            let items = parse_code(&format!("package main\n\nconst Notice = \"{value}\"\n"));
            let c = items.iter().find(|i| i.name == "Notice").expect("const parsed");
            let sig = c.signature.as_ref().unwrap();
            assert!(sig.ends_with("..."), "pad {pad}: long value should still be truncated: {sig}");
            assert!(sig.chars().count() < sig.len(), "pad {pad}: fixture must be multibyte");
        }
    }
}
