//! Python parser using tree-sitter.
//!
//! Extracts:
//! - Functions (def)
//! - Async functions (async def)
//! - Classes
//! - Methods (with impl_for set to class name)
//! - Decorators (stored in doc_comment)

use std::path::Path;
use tree_sitter::{Parser, Node};

use super::types::CodeItem;

/// Parse a Python file and extract code items.
pub fn parse_py_file(path: &Path) -> Result<Vec<CodeItem>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|e| format!("Failed to set language: {:?}", e))?;

    let tree = parser.parse(&content, None)
        .ok_or_else(|| "Failed to parse Python".to_string())?;

    let file_path = path.to_string_lossy().to_string();
    let lines: Vec<&str> = content.lines().collect();

    let mut items = Vec::new();
    extract_items(tree.root_node(), &file_path, &content, &lines, &mut items, None);

    Ok(items)
}

/// Recursively extract code items from the AST.
/// `class_context` is Some(class_name) when inside a class body.
fn extract_items(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    items: &mut Vec<CodeItem>,
    class_context: Option<&str>,
) {
    let kind = node.kind();

    match kind {
        // Decorated definitions (decorators + function/class)
        "decorated_definition" => {
            // Extract decorators
            let decorators = extract_decorators(node, content);

            // Find the actual definition (function or class)
            if let Some(definition) = node.child_by_field_name("definition") {
                let def_kind = definition.kind();
                match def_kind {
                    "function_definition" => {
                        if let Some(mut item) = extract_function(definition, file_path, content, lines, class_context) {
                            // Include decorators in doc_comment
                            item.doc_comment = merge_doc_comment(decorators, item.doc_comment);
                            // Use the decorated_definition span for full content
                            let (line_start, line_end) = get_node_lines(node);
                            item.line_start = line_start;
                            item.line_end = line_end;
                            item.content = extract_source_lines(lines, line_start, line_end);
                            items.push(item);
                        }
                    }
                    "class_definition" => {
                        if let Some(mut item) = extract_class(definition, file_path, content, lines) {
                            item.doc_comment = merge_doc_comment(decorators, item.doc_comment);
                            let (line_start, line_end) = get_node_lines(node);
                            item.line_start = line_start;
                            item.line_end = line_end;
                            item.content = extract_source_lines(lines, line_start, line_end);

                            // Extract class name before pushing
                            let class_name = item.name.clone();
                            items.push(item);

                            // Recurse into class body for methods
                            if let Some(body) = definition.child_by_field_name("body") {
                                extract_items(body, file_path, content, lines, items, Some(&class_name));
                            }
                        }
                    }
                    _ => {}
                }
            }
            return; // Don't recurse further for decorated definitions
        }

        // Regular function definition
        "function_definition" => {
            if let Some(item) = extract_function(node, file_path, content, lines, class_context) {
                items.push(item);
            }
        }

        // Class definition
        "class_definition" => {
            if let Some(item) = extract_class(node, file_path, content, lines) {
                // Extract class name before pushing
                let class_name = item.name.clone();
                items.push(item);

                // Recurse into class body for methods
                if let Some(body) = node.child_by_field_name("body") {
                    extract_items(body, file_path, content, lines, items, Some(&class_name));
                }
            }
            return; // Already recursed into class body
        }

        _ => {}
    }

    // Recurse into children (but not for items we've already handled)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_items(child, file_path, content, lines, items, class_context);
    }
}

/// Extract a function definition.
fn extract_function(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
    class_context: Option<&str>,
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

    // Return type annotation
    let return_type = node.child_by_field_name("return_type")
        .and_then(|n| get_node_text(n, content))
        .map(|t| format!(" -> {}", t))
        .unwrap_or_default();

    let async_prefix = if is_async { "async " } else { "" };
    let signature = format!("{}def {}{}{}", async_prefix, name, params, return_type);

    // Extract docstring as doc_comment
    let doc_comment = extract_docstring(node, content);

    // Determine visibility (Python doesn't have explicit visibility, use _ prefix convention)
    // Dunder methods (__init__, __str__, etc.) are public - they start AND end with __
    let visibility = if name.starts_with("__") && name.ends_with("__") {
        "pub".to_string() // dunder methods are public
    } else if name.starts_with("__") {
        "private".to_string() // name-mangled private
    } else if name.starts_with('_') {
        "protected".to_string() // conventional private
    } else {
        "pub".to_string() // public by convention
    };

    // Determine item_type based on context
    let (item_type, impl_for) = if let Some(class_name) = class_context {
        ("function".to_string(), Some(class_name.to_string())) // method
    } else {
        ("function".to_string(), None) // standalone function
    };

    Some(CodeItem {
        name,
        item_type,
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: Some(signature),
        visibility,
        doc_comment,
        is_async,
        is_unsafe: false,
        impl_for,
        impl_trait: None,
    })
}

/// Extract a class definition.
fn extract_class(
    node: Node,
    file_path: &str,
    content: &str,
    lines: &[&str],
) -> Option<CodeItem> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(name_node, content)?;

    let (line_start, line_end) = get_node_lines(node);
    let item_content = extract_source_lines(lines, line_start, line_end);

    // Get base classes (inheritance)
    let bases = node.child_by_field_name("superclasses")
        .and_then(|n| get_node_text(n, content))
        .unwrap_or_default();

    let signature = if bases.is_empty() {
        format!("class {}", name)
    } else {
        format!("class {}{}", name, bases)
    };

    // Extract docstring
    let doc_comment = extract_docstring(node, content);

    let visibility = if name.starts_with('_') {
        "private".to_string()
    } else {
        "pub".to_string()
    };

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

/// Extract decorators from a decorated_definition node.
fn extract_decorators(node: Node, content: &str) -> Option<String> {
    let mut decorators = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            if let Some(text) = get_node_text(child, content) {
                decorators.push(text);
            }
        }
    }

    if decorators.is_empty() {
        None
    } else {
        Some(decorators.join("\n"))
    }
}

/// Extract Python docstring from a function or class body.
fn extract_docstring(node: Node, content: &str) -> Option<String> {
    // Look for body and check first statement
    let body = node.child_by_field_name("body")?;
    let mut cursor = body.walk();

    // First child of body might be expression_statement containing a string
    for child in body.children(&mut cursor) {
        if child.kind() == "expression_statement" {
            // Check if it's a string (docstring)
            let mut child_cursor = child.walk();
            for grandchild in child.children(&mut child_cursor) {
                if grandchild.kind() == "string" {
                    return get_node_text(grandchild, content);
                }
            }
        }
        // Only check first statement
        break;
    }
    None
}

/// Merge decorators with existing doc_comment.
fn merge_doc_comment(decorators: Option<String>, doc: Option<String>) -> Option<String> {
    match (decorators, doc) {
        (Some(d), Some(doc)) => Some(format!("{}\n{}", d, doc)),
        (Some(d), None) => Some(d),
        (None, doc) => doc,
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

/// Extract source lines from line numbers.
fn extract_source_lines(lines: &[&str], start: usize, end: usize) -> String {
    lines
        .get(start.saturating_sub(1)..end.min(lines.len()))
        .map(|slice| slice.join("\n"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn parse_code(code: &str) -> Vec<CodeItem> {
        let mut file = NamedTempFile::with_suffix(".py").unwrap();
        file.write_all(code.as_bytes()).unwrap();
        parse_py_file(file.path()).unwrap()
    }

    #[test]
    fn test_parse_function() {
        let items = parse_code(r#"
def hello(name: str) -> str:
    """Say hello."""
    return f"Hello, {name}!"
"#);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "hello");
        assert_eq!(items[0].item_type, "function");
        assert!(items[0].signature.as_ref().unwrap().contains("def hello"));
        assert!(items[0].doc_comment.as_ref().unwrap().contains("Say hello"));
    }

    #[test]
    fn test_parse_async_function() {
        let items = parse_code(r#"
async def fetch_data(url: str) -> dict:
    """Fetch data asynchronously."""
    pass
"#);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "fetch_data");
        assert!(items[0].is_async);
        assert!(items[0].signature.as_ref().unwrap().starts_with("async def"));
    }

    #[test]
    fn test_parse_class() {
        let items = parse_code(r#"
class User:
    """A user class."""

    def __init__(self, name: str):
        self.name = name

    def greet(self) -> str:
        return f"Hello, {self.name}!"
"#);
        // Should have: class + __init__ method + greet method
        assert_eq!(items.len(), 3);

        // First item is the class
        assert_eq!(items[0].name, "User");
        assert_eq!(items[0].item_type, "class");

        // Methods should have impl_for set
        assert_eq!(items[1].name, "__init__");
        assert_eq!(items[1].impl_for, Some("User".to_string()));

        assert_eq!(items[2].name, "greet");
        assert_eq!(items[2].impl_for, Some("User".to_string()));
    }

    #[test]
    fn test_parse_class_inheritance() {
        let items = parse_code(r#"
class Admin(User):
    """An admin user."""
    pass
"#);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Admin");
        assert!(items[0].signature.as_ref().unwrap().contains("(User)"));
    }

    #[test]
    fn test_parse_decorated_function() {
        let items = parse_code(r#"
@staticmethod
@cache
def expensive_computation(x: int) -> int:
    """Cached computation."""
    return x * x
"#);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "expensive_computation");
        // Decorators should be in doc_comment
        let doc = items[0].doc_comment.as_ref().unwrap();
        assert!(doc.contains("@staticmethod"));
        assert!(doc.contains("@cache"));
    }

    #[test]
    fn test_parse_decorated_class() {
        let items = parse_code(r#"
@dataclass
class Point:
    x: int
    y: int
"#);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Point");
        assert!(items[0].doc_comment.as_ref().unwrap().contains("@dataclass"));
    }

    #[test]
    fn test_private_naming_convention() {
        let items = parse_code(r#"
def public_func():
    pass

def _protected_func():
    pass

def __private_func():
    pass
"#);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].visibility, "pub");
        assert_eq!(items[1].visibility, "protected");
        assert_eq!(items[2].visibility, "private");
    }

    #[test]
    fn test_dunder_methods() {
        let items = parse_code(r#"
class MyClass:
    def __init__(self):
        pass

    def __str__(self):
        return "MyClass"
"#);
        // __init__ and __str__ are dunder methods (not private)
        assert_eq!(items.len(), 3);
        // Dunder methods start with __ but end with __, so they're public
        assert_eq!(items[1].name, "__init__");
        assert_eq!(items[1].visibility, "pub");
        assert_eq!(items[2].name, "__str__");
        assert_eq!(items[2].visibility, "pub");
    }
}
