//! Rust source code parser using the `syn` crate.

use std::path::Path;
use syn::{
    parse_file, Attribute, File, Item, ItemEnum, ItemFn, ItemImpl, ItemMacro, ItemMod,
    ItemStruct, ItemTrait, Visibility,
};

use super::types::CodeItem;

/// Parse a Rust source file and extract all code items.
pub fn parse_rust_file(path: &Path) -> Result<Vec<CodeItem>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let syntax = parse_file(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;

    let file_path = path.to_string_lossy().to_string();
    let lines: Vec<&str> = content.lines().collect();

    let mut items = Vec::new();
    extract_items(&syntax, &file_path, &content, &lines, &mut items);

    Ok(items)
}

/// Extract all code items from a parsed file.
fn extract_items(
    file: &File,
    file_path: &str,
    content: &str,
    lines: &[&str],
    items: &mut Vec<CodeItem>,
) {
    for item in &file.items {
        match item {
            Item::Fn(f) => {
                if let Some(code_item) = extract_function(f, file_path, content, lines) {
                    items.push(code_item);
                }
            }
            Item::Struct(s) => {
                if let Some(code_item) = extract_struct(s, file_path, content, lines) {
                    items.push(code_item);
                }
            }
            Item::Enum(e) => {
                if let Some(code_item) = extract_enum(e, file_path, content, lines) {
                    items.push(code_item);
                }
            }
            Item::Trait(t) => {
                if let Some(code_item) = extract_trait(t, file_path, content, lines) {
                    items.push(code_item);
                }
            }
            Item::Impl(i) => {
                if let Some(code_item) = extract_impl(i, file_path, content, lines) {
                    items.push(code_item);
                }
            }
            Item::Mod(m) => {
                // Only extract inline modules (with content), not `mod foo;` declarations
                if m.content.is_some() {
                    if let Some(code_item) = extract_module(m, file_path, content, lines) {
                        items.push(code_item);
                    }
                }
            }
            Item::Macro(m) => {
                if let Some(code_item) = extract_macro(m, file_path, content, lines) {
                    items.push(code_item);
                }
            }
            _ => {}
        }
    }
}

/// Extract a function item.
fn extract_function(f: &ItemFn, file_path: &str, content: &str, lines: &[&str]) -> Option<CodeItem> {
    let name = f.sig.ident.to_string();
    let span = f.sig.ident.span();

    let (line_start, line_end) = get_item_lines(span, content, lines)?;
    let item_content = extract_source_range(lines, line_start, line_end);

    // Build signature (without body)
    let signature = build_function_signature(f);
    let doc_comment = extract_doc_comment(&f.attrs);
    let visibility = visibility_to_string(&f.vis);

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
        is_async: f.sig.asyncness.is_some(),
        is_unsafe: f.sig.unsafety.is_some(),
        impl_for: None,
        impl_trait: None,
    })
}

/// Extract a struct item.
fn extract_struct(s: &ItemStruct, file_path: &str, content: &str, lines: &[&str]) -> Option<CodeItem> {
    let name = s.ident.to_string();
    let span = s.ident.span();

    let (line_start, line_end) = get_item_lines(span, content, lines)?;
    let item_content = extract_source_range(lines, line_start, line_end);

    let doc_comment = extract_doc_comment(&s.attrs);
    let visibility = visibility_to_string(&s.vis);

    Some(CodeItem {
        name,
        item_type: "struct".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: None,
        visibility,
        doc_comment,
        is_async: false,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
    })
}

/// Extract an enum item.
fn extract_enum(e: &ItemEnum, file_path: &str, content: &str, lines: &[&str]) -> Option<CodeItem> {
    let name = e.ident.to_string();
    let span = e.ident.span();

    let (line_start, line_end) = get_item_lines(span, content, lines)?;
    let item_content = extract_source_range(lines, line_start, line_end);

    let doc_comment = extract_doc_comment(&e.attrs);
    let visibility = visibility_to_string(&e.vis);

    Some(CodeItem {
        name,
        item_type: "enum".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: None,
        visibility,
        doc_comment,
        is_async: false,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
    })
}

/// Extract a trait item.
fn extract_trait(t: &ItemTrait, file_path: &str, content: &str, lines: &[&str]) -> Option<CodeItem> {
    let name = t.ident.to_string();
    let span = t.ident.span();

    let (line_start, line_end) = get_item_lines(span, content, lines)?;
    let item_content = extract_source_range(lines, line_start, line_end);

    let doc_comment = extract_doc_comment(&t.attrs);
    let visibility = visibility_to_string(&t.vis);

    Some(CodeItem {
        name,
        item_type: "trait".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: None,
        visibility,
        doc_comment,
        is_async: false,
        is_unsafe: t.unsafety.is_some(),
        impl_for: None,
        impl_trait: None,
    })
}

/// Extract an impl block.
fn extract_impl(i: &ItemImpl, file_path: &str, content: &str, lines: &[&str]) -> Option<CodeItem> {
    // Get the type being implemented
    let impl_for = type_to_string(&i.self_ty);

    // Get the trait being implemented (if any)
    let impl_trait = i.trait_.as_ref().map(|(_, path, _)| {
        path.segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::")
    });

    // Name is "impl Type" or "impl Trait for Type"
    let name = if let Some(ref trait_name) = impl_trait {
        format!("impl {} for {}", trait_name, impl_for)
    } else {
        format!("impl {}", impl_for)
    };

    let span = i.impl_token.span;
    let (line_start, line_end) = get_item_lines(span, content, lines)?;
    let item_content = extract_source_range(lines, line_start, line_end);

    let doc_comment = extract_doc_comment(&i.attrs);

    Some(CodeItem {
        name,
        item_type: "impl".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: None,
        visibility: String::new(), // impl blocks don't have visibility
        doc_comment,
        is_async: false,
        is_unsafe: i.unsafety.is_some(),
        impl_for: Some(impl_for),
        impl_trait,
    })
}

/// Extract an inline module.
fn extract_module(m: &ItemMod, file_path: &str, content: &str, lines: &[&str]) -> Option<CodeItem> {
    let name = m.ident.to_string();
    let span = m.ident.span();

    let (line_start, line_end) = get_item_lines(span, content, lines)?;
    let item_content = extract_source_range(lines, line_start, line_end);

    let doc_comment = extract_doc_comment(&m.attrs);
    let visibility = visibility_to_string(&m.vis);

    Some(CodeItem {
        name,
        item_type: "module".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: None,
        visibility,
        doc_comment,
        is_async: false,
        is_unsafe: m.unsafety.is_some(),
        impl_for: None,
        impl_trait: None,
    })
}

/// Extract a macro_rules! macro.
fn extract_macro(m: &ItemMacro, file_path: &str, content: &str, lines: &[&str]) -> Option<CodeItem> {
    let name = m.ident.as_ref()?.to_string();
    let span = m.ident.as_ref()?.span();

    let (line_start, line_end) = get_item_lines(span, content, lines)?;
    let item_content = extract_source_range(lines, line_start, line_end);

    let doc_comment = extract_doc_comment(&m.attrs);

    Some(CodeItem {
        name,
        item_type: "macro".to_string(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        content: item_content,
        signature: None,
        visibility: String::new(),
        doc_comment,
        is_async: false,
        is_unsafe: false,
        impl_for: None,
        impl_trait: None,
    })
}

/// Get start and end lines for an item based on its span.
/// Returns 1-indexed line numbers.
fn get_item_lines(span: proc_macro2::Span, _content: &str, lines: &[&str]) -> Option<(usize, usize)> {
    let start = span.start();
    let line_start = start.line; // Already 1-indexed

    // Find the end by searching for the closing brace or semicolon
    // Start from the span line and look for balanced braces
    let mut brace_depth = 0;
    let mut found_start = false;

    for (idx, line) in lines.iter().enumerate().skip(line_start.saturating_sub(1)) {
        for ch in line.chars() {
            match ch {
                '{' => {
                    found_start = true;
                    brace_depth += 1;
                }
                '}' => {
                    brace_depth -= 1;
                    if found_start && brace_depth == 0 {
                        return Some((line_start, idx + 1));
                    }
                }
                ';' if !found_start => {
                    // Item without body (e.g., struct Foo;)
                    return Some((line_start, idx + 1));
                }
                _ => {}
            }
        }
    }

    // Fallback: use just the starting line
    Some((line_start, line_start))
}

/// Extract source code for a range of lines.
fn extract_source_range(lines: &[&str], start: usize, end: usize) -> String {
    // Expand start to include doc comments and attributes
    let mut actual_start = start;
    if actual_start > 1 {
        // Look backwards for doc comments and attributes
        for i in (0..start.saturating_sub(1)).rev() {
            let line = lines.get(i).map(|s| s.trim()).unwrap_or("");
            if line.starts_with("///")
                || line.starts_with("//!")
                || line.starts_with("#[")
                || line.starts_with("#![")
            {
                actual_start = i + 1; // 1-indexed
            } else if line.is_empty() {
                // Continue looking past empty lines
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

/// Build a function signature string (without the body).
fn build_function_signature(f: &ItemFn) -> String {
    let vis = visibility_to_string(&f.vis);
    let asyncness = if f.sig.asyncness.is_some() { "async " } else { "" };
    let unsafety = if f.sig.unsafety.is_some() { "unsafe " } else { "" };
    let constness = if f.sig.constness.is_some() { "const " } else { "" };

    let generics = if f.sig.generics.params.is_empty() {
        String::new()
    } else {
        format!("<{}>",
            f.sig.generics.params.iter()
                .map(|p| quote::quote!(#p).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    let inputs = f.sig.inputs.iter()
        .map(|arg| quote::quote!(#arg).to_string())
        .collect::<Vec<_>>()
        .join(", ");

    let output = match &f.sig.output {
        syn::ReturnType::Default => String::new(),
        syn::ReturnType::Type(_, ty) => format!(" -> {}", quote::quote!(#ty)),
    };

    let where_clause = f.sig.generics.where_clause.as_ref()
        .map(|w| format!(" {}", quote::quote!(#w)))
        .unwrap_or_default();

    let vis_prefix = if vis.is_empty() {
        String::new()
    } else {
        format!("{} ", vis)
    };

    format!(
        "{}{}{}{}fn {}{}({}){}{}",
        vis_prefix,
        constness,
        asyncness,
        unsafety,
        f.sig.ident,
        generics,
        inputs,
        output,
        where_clause
    )
}

/// Extract doc comment from attributes.
fn extract_doc_comment(attrs: &[Attribute]) -> Option<String> {
    let docs: Vec<String> = attrs
        .iter()
        .filter_map(|attr| {
            if attr.path().is_ident("doc") {
                if let syn::Meta::NameValue(meta) = &attr.meta {
                    if let syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(s),
                        ..
                    }) = &meta.value
                    {
                        return Some(s.value());
                    }
                }
            }
            None
        })
        .collect();

    if docs.is_empty() {
        None
    } else {
        Some(docs.join("\n").trim().to_string())
    }
}

/// Convert visibility to a string representation.
fn visibility_to_string(vis: &Visibility) -> String {
    match vis {
        Visibility::Public(_) => "pub".to_string(),
        Visibility::Restricted(r) => {
            let path = &r.path;
            if path.is_ident("crate") {
                "pub(crate)".to_string()
            } else if path.is_ident("super") {
                "pub(super)".to_string()
            } else if path.is_ident("self") {
                "pub(self)".to_string()
            } else {
                format!("pub(in {})", quote::quote!(#path))
            }
        }
        Visibility::Inherited => String::new(),
    }
}

/// Convert a Type to a string representation.
fn type_to_string(ty: &syn::Type) -> String {
    quote::quote!(#ty).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_simple_function() {
        let code = r#"
/// A test function
pub fn hello_world() {
    println!("Hello, world!");
}
"#;
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(code.as_bytes()).unwrap();

        let items = parse_rust_file(file.path()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "hello_world");
        assert_eq!(items[0].item_type, "function");
        assert_eq!(items[0].visibility, "pub");
        assert!(items[0].doc_comment.as_ref().unwrap().contains("A test function"));
    }

    #[test]
    fn test_parse_struct() {
        let code = r#"
/// A test struct
pub struct MyStruct {
    field: i32,
}
"#;
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(code.as_bytes()).unwrap();

        let items = parse_rust_file(file.path()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "MyStruct");
        assert_eq!(items[0].item_type, "struct");
    }
}
