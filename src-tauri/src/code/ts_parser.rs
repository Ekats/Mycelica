//! TypeScript/JavaScript parser using tree-sitter.
//!
//! Extracts:
//! - Functions (regular and arrow)
//! - Classes
//! - Interfaces
//! - Type aliases
//! - Enums
//! - Exported constants

use std::path::Path;
use tree_sitter::{Parser, Node};

use super::types::CodeItem;

/// Parse a TypeScript or JavaScript file and extract code items.
pub fn parse_ts_file(path: &Path) -> Result<Vec<CodeItem>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let is_tsx = path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e == "tsx" || e == "jsx")
        .unwrap_or(false);

    let language = if is_tsx {
        tree_sitter_typescript::LANGUAGE_TSX
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    };

    let mut parser = Parser::new();
    parser.set_language(&language.into())
        .map_err(|e| format!("Failed to set language: {:?}", e))?;

    let tree = parser.parse(&content, None)
        .ok_or_else(|| "Failed to parse TypeScript".to_string())?;

    let file_path = path.to_string_lossy().to_string();
    let lines: Vec<&str> = content.lines().collect();

    let mut items = Vec::new();
    extract_items(tree.root_node(), &file_path, &content, &lines, &mut items, false);

    Ok(items)
}

/// Recursively extract code items from the AST.
fn extract_items(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    items: &mut Vec<CodeItem>,
    is_exported: bool,
) {
    let kind = node.kind();

    match kind {
        // Export statement wraps other declarations
        "export_statement" => {
            // Process children with is_exported = true
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_items(child, file_path, content, lines, items, true);
            }
            return; // Don't recurse again below
        }

        // Function declarations
        "function_declaration" => {
            if let Some(item) = extract_function(node, file_path, content, lines, is_exported) {
                items.push(item);
            }
        }

        // Arrow functions assigned to const/let
        "lexical_declaration" | "variable_declaration" => {
            extract_variable_declarations(node, file_path, content, lines, items, is_exported);
        }

        // Classes
        "class_declaration" => {
            if let Some(item) = extract_class(node, file_path, content, lines, is_exported) {
                items.push(item);
            }
        }

        // Interfaces (TypeScript only)
        "interface_declaration" => {
            if let Some(item) = extract_interface(node, file_path, content, lines, is_exported) {
                items.push(item);
            }
        }

        // Type aliases (TypeScript only)
        "type_alias_declaration" => {
            if let Some(item) = extract_type_alias(node, file_path, content, lines, is_exported) {
                items.push(item);
            }
        }

        // Enums (TypeScript only)
        "enum_declaration" => {
            if let Some(item) = extract_enum(node, file_path, content, lines, is_exported) {
                items.push(item);
            }
        }

        // Module/namespace declarations
        "module" | "internal_module" => {
            if let Some(item) = extract_module(node, file_path, content, lines, is_exported) {
                items.push(item);
            }
        }

        _ => {}
    }

    // Recurse into children (but not for export_statement, handled above)
    if kind != "export_statement" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            // Don't inherit is_exported for nested items
            extract_items(child, file_path, content, lines, items, false);
        }
    }
}

/// Extract a function declaration.
fn extract_function(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    is_exported: bool,
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    // Check for async
    let is_async = has_child_of_kind(node, "async");

    // Build signature
    let params = node.child_by_field_name("parameters")
        .and_then(|n| get_node_text(n, content))
        .unwrap_or("()".to_string());

    let return_type = node.child_by_field_name("return_type")
        .and_then(|n| get_node_text(n, content))
        .unwrap_or_default();

    let type_params = node.child_by_field_name("type_parameters")
        .and_then(|n| get_node_text(n, content))
        .unwrap_or_default();

    let async_prefix = if is_async { "async " } else { "" };
    let signature = format!("{}function {}{}{}{}",
        async_prefix, name, type_params, params, return_type);

    let visibility = if is_exported { "export".to_string() } else { String::new() };
    let doc_comment = extract_leading_comment(node, content);

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
        is_async,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
    })
}

/// Extract variable declarations that might be arrow functions or important constants.
fn extract_variable_declarations(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    items: &mut Vec<CodeItem>,
    is_exported: bool,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(item) = extract_variable_declarator(child, node, file_path, content, lines, is_exported) {
                items.push(item);
            }
        }
    }
}

/// Extract a single variable declarator (might be arrow function or const).
fn extract_variable_declarator(
    declarator: Node,
    declaration: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    is_exported: bool,
) -> Option<CodeItem> {
    let name_node = declarator.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    let value_node = declarator.child_by_field_name("value")?;
    let value_kind = value_node.kind();

    // Check if it's an arrow function or function expression
    let is_function = matches!(value_kind, "arrow_function" | "function" | "function_expression");

    // Skip non-exported non-functions (we only want exported consts or functions)
    if !is_function && !is_exported {
        return None;
    }

    let (line_start, line_end) = get_node_lines(declaration);
    let item_content = extract_source_lines(lines, line_start, line_end);

    let visibility = if is_exported { "export".to_string() } else { String::new() };
    let doc_comment = extract_leading_comment(declaration, content);

    if is_function {
        // It's an arrow function or function expression
        let is_async = has_child_of_kind(value_node, "async")
            || get_node_text(value_node, content).map(|t| t.starts_with("async")).unwrap_or(false);

        // Build signature from the value
        let params = value_node.child_by_field_name("parameters")
            .or_else(|| value_node.child_by_field_name("parameter"))
            .and_then(|n| get_node_text(n, content))
            .unwrap_or("()".to_string());

        let return_type = value_node.child_by_field_name("return_type")
            .and_then(|n| get_node_text(n, content))
            .unwrap_or_default();

        // Get type annotation from the declarator
        let type_annotation = declarator.child_by_field_name("type")
            .and_then(|n| get_node_text(n, content))
            .unwrap_or_default();

        let async_prefix = if is_async { "async " } else { "" };
        let signature = if !type_annotation.is_empty() {
            format!("const {}{} = {}{}{}",  name, type_annotation, async_prefix, params, return_type)
        } else {
            format!("const {} = {}{} => ...", name, async_prefix, params)
        };

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
            is_async,
            is_unsafe: false,
            impl_for: None,
            impl_trait: None,
        })
    } else {
        // It's an exported constant
        let type_annotation = declarator.child_by_field_name("type")
            .and_then(|n| get_node_text(n, content))
            .unwrap_or_default();

        let signature = format!("const {}{}", name, type_annotation);

        Some(CodeItem {
            name,
            item_type: "const".to_string(),
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
        })
    }
}

/// Extract a class declaration.
fn extract_class(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    is_exported: bool,
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    // Get heritage (extends/implements)
    let heritage = node.children(&mut node.walk())
        .find(|c| c.kind() == "class_heritage")
        .and_then(|h| get_node_text(h, content))
        .unwrap_or_default();

    let type_params = node.child_by_field_name("type_parameters")
        .and_then(|n| get_node_text(n, content))
        .unwrap_or_default();

    let signature = if heritage.is_empty() {
        format!("class {}{}", name, type_params)
    } else {
        format!("class {}{} {}", name, type_params, heritage)
    };

    let visibility = if is_exported { "export".to_string() } else { String::new() };
    let doc_comment = extract_leading_comment(node, content);

    Some(CodeItem {
        name,
        item_type: "class".to_string(),
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
    })
}

/// Extract an interface declaration.
fn extract_interface(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    is_exported: bool,
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    let type_params = node.child_by_field_name("type_parameters")
        .and_then(|n| get_node_text(n, content))
        .unwrap_or_default();

    // Check for extends
    let extends = node.children(&mut node.walk())
        .find(|c| c.kind() == "extends_type_clause")
        .and_then(|h| get_node_text(h, content))
        .unwrap_or_default();

    let signature = if extends.is_empty() {
        format!("interface {}{}", name, type_params)
    } else {
        format!("interface {}{} {}", name, type_params, extends)
    };

    let visibility = if is_exported { "export".to_string() } else { String::new() };
    let doc_comment = extract_leading_comment(node, content);

    Some(CodeItem {
        name,
        item_type: "interface".to_string(),
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
    })
}

/// Extract a type alias declaration.
fn extract_type_alias(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    is_exported: bool,
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    let type_params = node.child_by_field_name("type_parameters")
        .and_then(|n| get_node_text(n, content))
        .unwrap_or_default();

    // Get the type value (abbreviated if long)
    let type_value = node.child_by_field_name("value")
        .and_then(|n| get_node_text(n, content))
        .map(|v| if v.len() > 50 { format!("{}...", &v[..47]) } else { v })
        .unwrap_or_default();

    let signature = format!("type {}{} = {}", name, type_params, type_value);

    let visibility = if is_exported { "export".to_string() } else { String::new() };
    let doc_comment = extract_leading_comment(node, content);

    Some(CodeItem {
        name,
        item_type: "type".to_string(),
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
    })
}

/// Extract an enum declaration.
fn extract_enum(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    is_exported: bool,
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    // Check if const enum
    let is_const = has_child_of_kind(node, "const");
    let prefix = if is_const { "const " } else { "" };
    let signature = format!("{}enum {}", prefix, name);

    let visibility = if is_exported { "export".to_string() } else { String::new() };
    let doc_comment = extract_leading_comment(node, content);

    Some(CodeItem {
        name,
        item_type: "enum".to_string(),
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
    })
}

/// Extract a module/namespace declaration.
fn extract_module(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    is_exported: bool,
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    let signature = format!("namespace {}", name);

    let visibility = if is_exported { "export".to_string() } else { String::new() };
    let doc_comment = extract_leading_comment(node, content);

    Some(CodeItem {
        name,
        item_type: "module".to_string(),
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

/// Check if node has a direct child of the given kind.
fn has_child_of_kind(node: Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return true;
        }
    }
    false
}

/// Extract source lines with leading comments/decorators.
fn extract_source_lines(lines: &[&str], start: usize, end: usize) -> String {
    // Look backwards for JSDoc comments and decorators
    let mut actual_start = start;
    if actual_start > 1 {
        for i in (0..start.saturating_sub(1)).rev() {
            let line = lines.get(i).map(|s| s.trim()).unwrap_or("");
            if line.starts_with("/**")
                || line.starts_with("*")
                || line.starts_with("//")
                || line.starts_with("@")
                || line.ends_with("*/")
            {
                actual_start = i + 1;
            } else if line.is_empty() {
                continue;
            } else {
                break;
            }
        }
    }

    lines
        .get(actual_start.saturating_sub(1)..end.min(lines.len()))
        .map(|slice| slice.join("\n"))
        .unwrap_or_default()
}

/// Extract leading JSDoc or line comments.
fn extract_leading_comment(node: Node, content: &str) -> Option<String> {
    let mut prev = node.prev_sibling();
    let mut comments = Vec::new();

    while let Some(p) = prev {
        match p.kind() {
            "comment" => {
                if let Some(text) = get_node_text(p, content) {
                    // JSDoc comment
                    if text.starts_with("/**") {
                        comments.insert(0, text);
                        break; // JSDoc is the primary doc
                    }
                    // Line comment
                    if text.starts_with("//") {
                        comments.insert(0, text);
                    }
                }
            }
            _ => break,
        }
        prev = p.prev_sibling();
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
        parse_ts_file(file.path()).unwrap()
    }

    #[test]
    fn test_parse_function() {
        let items = parse_code(r#"
function hello(name: string): string {
    return `Hello, ${name}!`;
}
"#, ".ts");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "hello");
        assert_eq!(items[0].item_type, "function");
        assert!(items[0].signature.as_ref().unwrap().contains("function hello"));
    }

    #[test]
    fn test_parse_exported_function() {
        let items = parse_code(r#"
export function greet(name: string): void {
    console.log(name);
}
"#, ".ts");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].visibility, "export");
    }

    #[test]
    fn test_parse_arrow_function() {
        let items = parse_code(r#"
export const add = (a: number, b: number): number => a + b;
"#, ".ts");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "add");
        assert_eq!(items[0].item_type, "function");
    }

    #[test]
    fn test_parse_async_function() {
        let items = parse_code(r#"
async function fetchData(): Promise<void> {
    await fetch('/api');
}
"#, ".ts");
        assert_eq!(items.len(), 1);
        assert!(items[0].is_async);
    }

    #[test]
    fn test_parse_interface() {
        let items = parse_code(r#"
export interface User {
    id: number;
    name: string;
}
"#, ".ts");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "User");
        assert_eq!(items[0].item_type, "interface");
    }

    #[test]
    fn test_parse_class() {
        let items = parse_code(r#"
export class UserService {
    constructor() {}
    getUser(id: number) {}
}
"#, ".ts");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "UserService");
        assert_eq!(items[0].item_type, "class");
    }

    #[test]
    fn test_parse_type_alias() {
        let items = parse_code(r#"
export type UserId = string | number;
"#, ".ts");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "UserId");
        assert_eq!(items[0].item_type, "type");
    }

    #[test]
    fn test_parse_enum() {
        let items = parse_code(r#"
export enum Status {
    Active,
    Inactive
}
"#, ".ts");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Status");
        assert_eq!(items[0].item_type, "enum");
    }

    #[test]
    fn test_parse_jsdoc() {
        let items = parse_code(r#"
/**
 * Adds two numbers together.
 * @param a First number
 * @param b Second number
 */
function add(a: number, b: number): number {
    return a + b;
}
"#, ".ts");
        assert_eq!(items.len(), 1);
        assert!(items[0].doc_comment.is_some());
        assert!(items[0].doc_comment.as_ref().unwrap().contains("Adds two numbers"));
    }

    #[test]
    fn test_parse_tsx() {
        let items = parse_code(r#"
export function Button({ label }: { label: string }) {
    return <button>{label}</button>;
}
"#, ".tsx");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Button");
    }
}
