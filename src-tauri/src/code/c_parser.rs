//! C parser using tree-sitter.
//!
//! Extracts:
//! - Functions
//! - Structs
//! - Enums
//! - Typedefs
//! - Macros (#define)
//!
//! Handles both .c and .h files.

use std::path::Path;
use tree_sitter::{Parser, Node};

use super::types::CodeItem;

/// Parse a C file and extract code items.
pub fn parse_c_file(path: &Path) -> Result<Vec<CodeItem>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_c::LANGUAGE.into())
        .map_err(|e| format!("Failed to set language: {:?}", e))?;

    let tree = parser.parse(&content, None)
        .ok_or_else(|| "Failed to parse C".to_string())?;

    let file_path = path.to_string_lossy().to_string();
    let lines: Vec<&str> = content.lines().collect();

    let mut items = Vec::new();
    extract_items(tree.root_node(), &file_path, &content, &lines, &mut items);

    Ok(items)
}

/// Recursively extract code items from the AST.
fn extract_items(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    items: &mut Vec<CodeItem>,
) {
    let kind = node.kind();

    match kind {
        // Function definitions
        "function_definition" => {
            if let Some(item) = extract_function(node, file_path, content, lines) {
                items.push(item);
            }
        }

        // Struct specifiers (can be standalone or part of declaration)
        "struct_specifier" => {
            if let Some(item) = extract_struct(node, file_path, content, lines) {
                items.push(item);
            }
        }

        // Enum specifiers
        "enum_specifier" => {
            if let Some(item) = extract_enum(node, file_path, content, lines) {
                items.push(item);
            }
        }

        // Type definitions (typedef)
        "type_definition" => {
            if let Some(item) = extract_typedef(node, file_path, content, lines) {
                items.push(item);
            }
        }

        // Preprocessor definitions (#define NAME value)
        "preproc_def" => {
            if let Some(item) = extract_macro(node, file_path, content, lines) {
                items.push(item);
            }
        }

        // Preprocessor function definitions (#define NAME(args) body)
        "preproc_function_def" => {
            if let Some(item) = extract_macro_function(node, file_path, content, lines) {
                items.push(item);
            }
        }

        // Declaration - may contain struct/enum definitions
        "declaration" => {
            // Look for struct/enum specifiers within declarations
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let child_kind = child.kind();
                if child_kind == "struct_specifier" || child_kind == "enum_specifier" {
                    extract_items(child, file_path, content, lines, items);
                }
            }
        }

        _ => {}
    }

    // Recurse into children for top-level constructs
    // Skip recursion for items we've already fully handled
    if !matches!(kind, "function_definition" | "struct_specifier" | "enum_specifier"
                      | "type_definition" | "preproc_def" | "preproc_function_def") {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_items(child, file_path, content, lines, items);
        }
    }
}

/// Extract a function definition.
fn extract_function(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<CodeItem> {
    // Get the declarator which contains the function name
    let declarator = node.child_by_field_name("declarator")?;
    let (name, params) = extract_function_declarator(declarator, content)?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    // Get return type from type specifier
    let return_type = node.child_by_field_name("type")
        .and_then(|n| get_node_text(n, content))
        .unwrap_or_else(|| "void".to_string());

    // Check for storage class specifiers (static = private)
    let is_static = has_storage_class(node, content, "static");

    let signature = format!("{} {}({})", return_type, name, params);
    let visibility = if is_static { "".to_string() } else { "pub".to_string() };

    // Extract leading comment as doc
    let doc_comment = extract_leading_comment(node, content, lines);

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
        is_unsafe: false, // C functions are inherently "unsafe" by Rust standards
        impl_for: None,
        impl_trait: None,
    })
}

/// Extract function name and parameters from a declarator.
fn extract_function_declarator(node: Node, content: &str) -> Option<(String, String)> {
    let kind = node.kind();

    match kind {
        "function_declarator" => {
            // Get the declarator (name or pointer declarator)
            let name_node = node.child_by_field_name("declarator")?;
            let name = extract_identifier_from_declarator(name_node, content)?;

            // Get parameters
            let params = node.child_by_field_name("parameters")
                .and_then(|n| get_node_text(n, content))
                .map(|s| {
                    // Strip outer parentheses
                    s.trim_start_matches('(').trim_end_matches(')').to_string()
                })
                .unwrap_or_default();

            Some((name, params))
        }
        "pointer_declarator" => {
            // Recurse through pointer declarators
            let inner = node.child_by_field_name("declarator")?;
            extract_function_declarator(inner, content)
        }
        _ => None,
    }
}

/// Extract identifier from various declarator types.
fn extract_identifier_from_declarator(node: Node, content: &str) -> Option<String> {
    let kind = node.kind();
    match kind {
        "identifier" => get_node_text(node, content),
        "pointer_declarator" => {
            let inner = node.child_by_field_name("declarator")?;
            extract_identifier_from_declarator(inner, content)
        }
        "parenthesized_declarator" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(name) = extract_identifier_from_declarator(child, content) {
                    return Some(name);
                }
            }
            None
        }
        _ => None,
    }
}

/// Extract a struct definition.
fn extract_struct(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<CodeItem> {
    // Only extract named structs with bodies
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    // Must have a body (field declaration list) to be a definition
    let _body = node.child_by_field_name("body")?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    let signature = format!("struct {}", name);
    let doc_comment = extract_leading_comment(node, content, lines);

    Some(CodeItem {
        name,
        item_type: "struct".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: Some(signature),
        visibility: "pub".to_string(), // C structs are public by default
        doc_comment,
        is_async: false,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
    })
}

/// Extract an enum definition.
fn extract_enum(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<CodeItem> {
    // Get enum name (may be anonymous)
    let name = node.child_by_field_name("name")
        .and_then(|n| get_node_text(n, content))?;

    // Must have a body to be a definition
    let _body = node.child_by_field_name("body")?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    let signature = format!("enum {}", name);
    let doc_comment = extract_leading_comment(node, content, lines);

    Some(CodeItem {
        name,
        item_type: "enum".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: Some(signature),
        visibility: "pub".to_string(),
        doc_comment,
        is_async: false,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
    })
}

/// Extract a typedef.
fn extract_typedef(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<CodeItem> {
    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    // Get the typedef name from the declarator
    let declarator = node.child_by_field_name("declarator")?;
    let name = extract_typedef_name(declarator, content)?;

    // Get the type being aliased
    let type_node = node.child_by_field_name("type")?;
    let type_text = get_node_text(type_node, content)?;

    let signature = format!("typedef {} {}", type_text, name);
    let doc_comment = extract_leading_comment(node, content, lines);

    Some(CodeItem {
        name,
        item_type: "type".to_string(), // Using "type" like TypeScript type aliases
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: Some(signature),
        visibility: "pub".to_string(),
        doc_comment,
        is_async: false,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
    })
}

/// Extract the name from a typedef declarator.
fn extract_typedef_name(node: Node, content: &str) -> Option<String> {
    let kind = node.kind();
    match kind {
        "type_identifier" | "identifier" => get_node_text(node, content),
        "pointer_declarator" => {
            let inner = node.child_by_field_name("declarator")?;
            extract_typedef_name(inner, content)
        }
        "array_declarator" => {
            let inner = node.child_by_field_name("declarator")?;
            extract_typedef_name(inner, content)
        }
        "function_declarator" => {
            let inner = node.child_by_field_name("declarator")?;
            extract_typedef_name(inner, content)
        }
        _ => {
            // Try to find type_identifier in children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier" || child.kind() == "identifier" {
                    return get_node_text(child, content);
                }
            }
            None
        }
    }
}

/// Extract a simple macro (#define NAME value).
fn extract_macro(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    // Get the value if present
    let value = node.child_by_field_name("value")
        .and_then(|n| get_node_text(n, content))
        .map(|v| {
            let trimmed = v.trim();
            if trimmed.len() > 30 {
                format!("{}...", &trimmed[..27])
            } else {
                trimmed.to_string()
            }
        })
        .unwrap_or_default();

    let signature = if value.is_empty() {
        format!("#define {}", name)
    } else {
        format!("#define {} {}", name, value)
    };

    Some(CodeItem {
        name,
        item_type: "macro".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: Some(signature),
        visibility: "pub".to_string(), // Macros are always visible where included
        doc_comment: None,
        is_async: false,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
    })
}

/// Extract a function-like macro (#define NAME(args) body).
fn extract_macro_function(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    // Get parameters
    let params = node.child_by_field_name("parameters")
        .and_then(|n| get_node_text(n, content))
        .unwrap_or_else(|| "()".to_string());

    let signature = format!("#define {}{}", name, params);

    Some(CodeItem {
        name,
        item_type: "macro".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: Some(signature),
        visibility: "pub".to_string(),
        doc_comment: None,
        is_async: false,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
    })
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

/// Check if a declaration has a specific storage class specifier.
fn has_storage_class(node: Node, content: &str, class: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "storage_class_specifier" {
            if let Some(text) = get_node_text(child, content) {
                if text == class {
                    return true;
                }
            }
        }
    }
    false
}

/// Extract source lines from line numbers.
fn extract_source_lines(lines: &[&str], start: usize, end: usize) -> String {
    lines
        .get(start.saturating_sub(1)..end.min(lines.len()))
        .map(|slice| slice.join("\n"))
        .unwrap_or_default()
}

/// Extract leading C-style comments (/* */ or //).
fn extract_leading_comment(node: Node, content: &str, lines: &[&str]) -> Option<String> {
    let start_line = node.start_position().row;
    if start_line == 0 {
        return None;
    }

    let mut comments = Vec::new();
    let mut line_idx = start_line.saturating_sub(1);

    // Look backwards for comments
    while line_idx > 0 || (line_idx == 0 && !comments.is_empty()) {
        let line = lines.get(line_idx).map(|s| s.trim()).unwrap_or("");

        if line.starts_with("//") {
            comments.insert(0, line.to_string());
        } else if line.starts_with("/*") || line.ends_with("*/") || line.starts_with("*") {
            comments.insert(0, line.to_string());
        } else if line.is_empty() && !comments.is_empty() {
            // Empty line after finding comments - stop
            break;
        } else if !line.is_empty() {
            // Non-comment, non-empty line - stop
            break;
        }

        if line_idx == 0 {
            break;
        }
        line_idx -= 1;
    }

    // Also check for block comments in the AST
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

    fn parse_code(code: &str, ext: &str) -> Vec<CodeItem> {
        let mut file = NamedTempFile::with_suffix(ext).unwrap();
        file.write_all(code.as_bytes()).unwrap();
        parse_c_file(file.path()).unwrap()
    }

    #[test]
    fn test_parse_function() {
        let items = parse_code(r#"
int add(int a, int b) {
    return a + b;
}
"#, ".c");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "add");
        assert_eq!(items[0].item_type, "function");
        assert!(items[0].signature.as_ref().unwrap().contains("int add"));
        assert_eq!(items[0].visibility, "pub");
    }

    #[test]
    fn test_parse_static_function() {
        let items = parse_code(r#"
static void helper(void) {
    // internal helper
}
"#, ".c");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "helper");
        assert_eq!(items[0].visibility, ""); // static = private
    }

    #[test]
    fn test_parse_struct() {
        let items = parse_code(r#"
struct Point {
    int x;
    int y;
};
"#, ".c");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Point");
        assert_eq!(items[0].item_type, "struct");
        assert!(items[0].signature.as_ref().unwrap().contains("struct Point"));
    }

    #[test]
    fn test_parse_enum() {
        let items = parse_code(r#"
enum Color {
    RED,
    GREEN,
    BLUE
};
"#, ".c");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Color");
        assert_eq!(items[0].item_type, "enum");
    }

    #[test]
    fn test_parse_typedef() {
        let items = parse_code(r#"
typedef unsigned int uint;
"#, ".c");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "uint");
        assert_eq!(items[0].item_type, "type");
        assert!(items[0].signature.as_ref().unwrap().contains("typedef"));
    }

    #[test]
    fn test_parse_typedef_struct() {
        let items = parse_code(r#"
typedef struct {
    int x;
    int y;
} Point;
"#, ".c");
        // Should get the typedef (struct is anonymous)
        assert!(items.iter().any(|i| i.name == "Point" && i.item_type == "type"));
    }

    #[test]
    fn test_parse_macro() {
        let items = parse_code(r#"
#define MAX_SIZE 100
"#, ".c");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "MAX_SIZE");
        assert_eq!(items[0].item_type, "macro");
        assert!(items[0].signature.as_ref().unwrap().contains("#define MAX_SIZE"));
    }

    #[test]
    fn test_parse_macro_function() {
        let items = parse_code(r#"
#define MAX(a, b) ((a) > (b) ? (a) : (b))
"#, ".c");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "MAX");
        assert_eq!(items[0].item_type, "macro");
        assert!(items[0].signature.as_ref().unwrap().contains("#define MAX("));
    }

    #[test]
    fn test_parse_header_file() {
        let items = parse_code(r#"
#ifndef MYHEADER_H
#define MYHEADER_H

struct Config {
    int timeout;
    char* host;
};

int initialize(struct Config* cfg);

#endif
"#, ".h");
        // Should have: 2 macros (MYHEADER_H guard), struct, function declaration won't be captured
        // Actually function declarations (without body) won't be captured
        assert!(items.iter().any(|i| i.name == "MYHEADER_H"));
        assert!(items.iter().any(|i| i.name == "Config"));
    }

    #[test]
    fn test_parse_pointer_function() {
        let items = parse_code(r#"
char* strdup(const char* s) {
    return NULL;
}
"#, ".c");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "strdup");
        assert_eq!(items[0].item_type, "function");
    }

    #[test]
    fn test_parse_with_comment() {
        let items = parse_code(r#"
/* Initialize the system */
int init(void) {
    return 0;
}
"#, ".c");
        assert_eq!(items.len(), 1);
        assert!(items[0].doc_comment.is_some());
        assert!(items[0].doc_comment.as_ref().unwrap().contains("Initialize"));
    }
}
