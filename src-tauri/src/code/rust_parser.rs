//! Rust source code parser using the `syn` crate.

use std::path::Path;
use syn::{
    parse_file, Attribute, ImplItem, Item, ItemEnum, ItemFn, ItemImpl, ItemMacro, ItemMod,
    ItemStruct, ItemTrait, TraitItem, Visibility,
};

use quote::ToTokens;
use super::types::{CodeItem, CallRef};

/// Parse a Rust source file and extract all code items.
pub fn parse_rust_file(path: &Path) -> Result<Vec<CodeItem>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let syntax = parse_file(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;

    let file_path = path.to_string_lossy().to_string();
    let lines: Vec<&str> = content.lines().collect();

    let mut items = Vec::new();
    extract_items(&syntax.items, "", &file_path, &content, &lines, &mut items);

    Ok(items)
}

/// Extract all code items from a list of items (a file, or an inline module's body).
/// `mod_path` is the enclosing inline-module path (`""` at file level, `outer::inner`).
fn extract_items(
    ast_items: &[Item],
    mod_path: &str,
    file_path: &str,
    content: &str,
    lines: &[&str],
    items: &mut Vec<CodeItem>,
) {
    for item in ast_items {
        match item {
            Item::Fn(f) => {
                if let Some(code_item) = extract_function(f, mod_path, file_path, lines) {
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
                extract_trait_default_methods(t, mod_path, file_path, lines, items);
            }
            Item::Impl(i) => {
                if let Some(code_item) = extract_impl(i, file_path, content, lines) {
                    items.push(code_item);
                }
                extract_impl_methods(i, mod_path, file_path, lines, items);
            }
            Item::Mod(m) => {
                // Only extract inline modules (with content), not `mod foo;` declarations
                if let Some((_, mod_items)) = &m.content {
                    if let Some(code_item) = extract_module(m, file_path, content, lines) {
                        items.push(code_item);
                    }
                    let inner = if mod_path.is_empty() {
                        m.ident.to_string()
                    } else {
                        format!("{}::{}", mod_path, m.ident)
                    };
                    extract_items(mod_items, &inner, file_path, content, lines, items);
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

struct CallCollector {
    calls: Vec<CallRef>,
}

impl<'ast> syn::visit::Visit<'ast> for CallCollector {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(ref path) = *node.func {
            let segs = &path.path.segments;
            if let Some(seg) = segs.last() {
                // `Database::new()` → receiver `Database`; `parser::parse()` → `parser`.
                // `Self::f()` is a call on the caller's own type, like `self.f()`.
                let receiver = segs.iter().rev().nth(1).map(|s| {
                    if s.ident == "Self" { "self".to_string() } else { s.ident.to_string() }
                });
                self.calls.push(CallRef {
                    name: seg.ident.to_string(),
                    receiver,
                    line: seg.ident.span().start().line,
                });
            }
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let receiver = match *node.receiver {
            syn::Expr::Path(ref p) => p.path.segments.last().map(|s| s.ident.to_string()),
            syn::Expr::Field(ref f) => Some(f.member.to_token_stream().to_string()),
            _ => None,
        };
        self.calls.push(CallRef {
            name: node.method.to_string(),
            receiver,
            line: node.method.span().start().line,
        });
        syn::visit::visit_expr_method_call(self, node);
    }
}

fn collect_calls_from_block(block: &syn::Block) -> Vec<CallRef> {
    use syn::visit::Visit;
    let mut collector = CallCollector { calls: Vec::new() };
    collector.visit_block(block);
    collector.calls
}

/// Extract a function item.
fn extract_function(f: &ItemFn, mod_path: &str, file_path: &str, lines: &[&str]) -> Option<CodeItem> {
    let name = f.sig.ident.to_string();
    build_function_item(name, &f.vis, &f.sig, &f.attrs, &f.block, None, None, mod_path, file_path, lines)
}

/// Build a `function` item from the parts a free function, an impl method and a
/// trait default method have in common.
#[allow(clippy::too_many_arguments)]
fn build_function_item(
    name: String,
    vis: &Visibility,
    sig: &syn::Signature,
    attrs: &[Attribute],
    block: &syn::Block,
    impl_for: Option<String>,
    impl_trait: Option<String>,
    mod_path: &str,
    file_path: &str,
    lines: &[&str],
) -> Option<CodeItem> {
    // Inside an inline module the name is module-qualified, so `tests::helper`
    // cannot share an id with a top-level `helper` in the same file.
    let name = if mod_path.is_empty() { name } else { format!("{}::{}", mod_path, name) };

    // The range ends at the body's closing brace as syn sees it. Brace matching
    // on text stops at a `;` in the signature (`-> [&str; 2] {`) or a brace in a string.
    let line_start = sig.ident.span().start().line;
    let line_end = block.brace_token.span.close().end().line;
    let item_content = extract_source_range(lines, line_start, line_end);

    // Build signature (without body)
    let signature = build_function_signature(vis, sig);
    let doc_comment = extract_doc_comment(attrs);
    let visibility = visibility_to_string(vis);

    let calls = collect_calls_from_block(block);

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
        is_async: sig.asyncness.is_some(),
        is_unsafe: sig.unsafety.is_some(),
        impl_for,
        impl_trait,
        calls,
    })
}

/// Emit one `function` item per method of an impl block.
fn extract_impl_methods(
    i: &ItemImpl,
    mod_path: &str,
    file_path: &str,
    lines: &[&str],
    items: &mut Vec<CodeItem>,
) {
    let self_ty = type_to_string(&i.self_ty);
    // Bare type name: call resolution compares it to a receiver identifier.
    let impl_for = bare_type_name(&i.self_ty);
    let impl_trait = i.trait_.as_ref().map(|(_, path, _)| trait_idents(path));
    let trait_name = i.trait_.as_ref().map(|(_, path, _)| trait_name_with_args(path));

    for impl_item in &i.items {
        if let ImplItem::Fn(m) = impl_item {
            // Qualified so that `from` in `impl From<A> for X` and in
            // `impl From<B> for X` get distinct ids. The title comes from the signature.
            let name = match trait_name {
                Some(ref t) => format!("<{} as {}>::{}", self_ty, t, m.sig.ident),
                None => format!("{}::{}", self_ty, m.sig.ident),
            };
            if let Some(code_item) = build_function_item(
                name, &m.vis, &m.sig, &m.attrs, &m.block,
                Some(impl_for.clone()), impl_trait.clone(),
                mod_path, file_path, lines,
            ) {
                items.push(code_item);
            }
        }
    }
}

/// Emit one `function` item per trait method that has a default body.
fn extract_trait_default_methods(
    t: &ItemTrait,
    mod_path: &str,
    file_path: &str,
    lines: &[&str],
    items: &mut Vec<CodeItem>,
) {
    for trait_item in &t.items {
        if let TraitItem::Fn(m) = trait_item {
            if let Some(ref block) = m.default {
                if let Some(code_item) = build_function_item(
                    m.sig.ident.to_string(), &Visibility::Inherited, &m.sig, &m.attrs, block,
                    Some(t.ident.to_string()), None,
                    mod_path, file_path, lines,
                ) {
                    items.push(code_item);
                }
            }
        }
    }
}

/// Last path segment of a type, without generics (`Foo<T>` → `Foo`, `&a::Foo` → `Foo`).
/// Falls back to the full token string for types that are not a path.
fn bare_type_name(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(p) => match p.path.segments.last() {
            Some(seg) => seg.ident.to_string(),
            None => type_to_string(ty),
        },
        syn::Type::Reference(r) => bare_type_name(&r.elem),
        _ => type_to_string(ty),
    }
}

/// Trait path as its idents joined with `::`, no generic arguments.
fn trait_idents(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

/// `trait_idents` plus the trait's generic arguments when it has any
/// (`From < A >`), so two impls of one generic trait for one type differ.
fn trait_name_with_args(path: &syn::Path) -> String {
    let idents = trait_idents(path);
    match path.segments.last().map(|s| &s.arguments) {
        Some(args) if !args.is_none() => format!("{} {}", idents, args.to_token_stream()),
        _ => idents,
    }
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
        calls: Vec::new(),
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
        calls: Vec::new(),
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
        calls: Vec::new(),
    })
}

/// Extract an impl block.
fn extract_impl(i: &ItemImpl, file_path: &str, content: &str, lines: &[&str]) -> Option<CodeItem> {
    // Get the type being implemented
    let impl_for = type_to_string(&i.self_ty);

    // Get the trait being implemented (if any)
    let impl_trait = i.trait_.as_ref().map(|(_, path, _)| trait_idents(path));

    // Name is "impl Type" or "impl Trait for Type". The trait keeps its generic
    // arguments here so `impl From<A> for X` and `impl From<B> for X` get distinct ids.
    let name = if let Some((_, path, _)) = i.trait_.as_ref() {
        let trait_name = trait_name_with_args(path);
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
        calls: Vec::new(),
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
        calls: Vec::new(),
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
        calls: Vec::new(),
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
                        let line_end = idx + 1; // 1-indexed
                        return Some((line_start, line_end));
                    }
                }
                ';' if !found_start => {
                    // Item without body (e.g., struct Foo;)
                    let line_end = idx + 1;
                    return Some((line_start, line_end));
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
fn build_function_signature(vis: &Visibility, sig: &syn::Signature) -> String {
    let vis = visibility_to_string(vis);
    let asyncness = if sig.asyncness.is_some() { "async " } else { "" };
    let unsafety = if sig.unsafety.is_some() { "unsafe " } else { "" };
    let constness = if sig.constness.is_some() { "const " } else { "" };

    let generics = if sig.generics.params.is_empty() {
        String::new()
    } else {
        format!("<{}>",
            sig.generics.params.iter()
                .map(|p| quote::quote!(#p).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    let inputs = sig.inputs.iter()
        .map(|arg| quote::quote!(#arg).to_string())
        .collect::<Vec<_>>()
        .join(", ");

    let output = match &sig.output {
        syn::ReturnType::Default => String::new(),
        syn::ReturnType::Type(_, ty) => format!(" -> {}", quote::quote!(#ty)),
    };

    let where_clause = sig.generics.where_clause.as_ref()
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
        sig.ident,
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

    #[test]
    fn test_extract_calls() {
        let code = r#"
fn helper() -> i32 { 42 }

fn main() {
    let x = helper();
    let s = String::from("hi");
    let v = vec![1, 2, 3];
    v.push(4);
    s.len();
}
"#;
        let mut file = NamedTempFile::with_suffix(".rs").unwrap();
        file.write_all(code.as_bytes()).unwrap();
        let items = parse_rust_file(file.path()).unwrap();
        let main_fn = items.iter().find(|i| i.name == "main").unwrap();
        assert!(main_fn.calls.iter().any(|c| c.name == "helper" && c.receiver.is_none()));
        assert!(main_fn.calls.iter().any(|c| c.name == "from" && c.receiver.as_deref() == Some("String")));
        assert!(main_fn.calls.iter().any(|c| c.name == "push" && c.receiver.as_deref() == Some("v")));
        assert!(main_fn.calls.iter().any(|c| c.name == "len" && c.receiver.as_deref() == Some("s")));
    }

    fn parse_snippet(code: &str) -> Vec<CodeItem> {
        let mut file = NamedTempFile::with_suffix(".rs").unwrap();
        file.write_all(code.as_bytes()).unwrap();
        parse_rust_file(file.path()).unwrap()
    }

    fn function_items(items: &[CodeItem]) -> Vec<&CodeItem> {
        items.iter().filter(|i| i.item_type == "function").collect()
    }

    #[test]
    fn test_inherent_method_emitted() {
        let items = parse_snippet(r#"
struct Store<T> { v: T }

impl<T> Store<T> {
    /// Opens it.
    pub fn open(&self) -> i32 {
        self.check();
        1
    }

    fn check(&self) {}
}
"#);
        // The impl node itself stays.
        assert!(items.iter().any(|i| i.item_type == "impl" && i.name == "impl Store < T >"));

        let fns = function_items(&items);
        assert_eq!(fns.len(), 2);
        let open = fns.iter().find(|i| i.name == "Store < T >::open").unwrap();
        assert_eq!(open.impl_for.as_deref(), Some("Store"));
        assert_eq!(open.impl_trait, None);
        assert_eq!(open.visibility, "pub");
        assert_eq!((open.line_start, open.line_end), (6, 9));
        assert!(open.signature.as_ref().unwrap().starts_with("pub fn open("));
        assert!(open.doc_comment.as_ref().unwrap().contains("Opens it."));
        assert!(open.calls.iter().any(|c| c.name == "check" && c.receiver.as_deref() == Some("self")));
    }

    #[test]
    fn test_generic_trait_impls_get_distinct_ids() {
        let items = parse_snippet(r#"
struct X;
struct A;
struct B;

impl From<A> for X {
    fn from(_: A) -> Self { X }
}

impl From<B> for X {
    fn from(_: B) -> Self { X }
}

impl std::fmt::Display for X {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) }
}
"#);
        let froms: Vec<_> = function_items(&items).into_iter()
            .filter(|i| i.signature.as_ref().unwrap().starts_with("fn from("))
            .collect();
        assert_eq!(froms.len(), 2);
        assert_ne!(froms[0].generate_id(), froms[1].generate_id());
        assert_eq!(froms[0].name, "<X as From < A >>::from");
        assert_eq!(froms[0].impl_for.as_deref(), Some("X"));
        assert_eq!(froms[0].impl_trait.as_deref(), Some("From"));

        let impls: Vec<_> = items.iter().filter(|i| i.item_type == "impl").collect();
        assert_eq!(impls.len(), 3);
        assert_eq!(impls[0].name, "impl From < A > for X");
        assert_eq!(impls[1].name, "impl From < B > for X");
        assert_ne!(impls[0].generate_id(), impls[1].generate_id());
        // The metadata field keeps the bare trait path.
        assert_eq!(impls[0].impl_trait.as_deref(), Some("From"));
        // A trait without generic arguments keeps its ident-joined name, and so its id.
        assert_eq!(impls[2].name, "impl std::fmt::Display for X");
    }

    #[test]
    fn test_path_call_receivers() {
        let items = parse_snippet(r#"
struct Db;

impl Db {
    fn new() -> Self { Db }

    fn build() -> Self {
        let a = Self::new();
        let b = Db::new();
        let c = parser::parse();
        let d = crate::code::parser::parse();
        a
    }
}
"#);
        let build = items.iter().find(|i| i.name == "Db::build").unwrap();
        let recv = |line: usize| {
            build.calls.iter().find(|c| c.line == line).unwrap().receiver.clone()
        };
        assert_eq!(recv(8).as_deref(), Some("self"));
        assert_eq!(recv(9).as_deref(), Some("Db"));
        assert_eq!(recv(10).as_deref(), Some("parser"));
        assert_eq!(recv(11).as_deref(), Some("parser"));
    }

    #[test]
    fn test_function_inside_inline_module() {
        let items = parse_snippet(r#"
mod outer {
    pub fn helper() -> i32 { 1 }

    mod inner {
        struct Deep;
        impl Deep {
            fn go(&self) { super::helper(); }
        }
    }
}
"#);
        assert_eq!(items.iter().filter(|i| i.item_type == "module").count(), 2);
        let helper = items.iter().find(|i| i.name == "outer::helper").unwrap();
        assert_eq!(helper.item_type, "function");
        assert_eq!(helper.impl_for, None);
        let go = items.iter().find(|i| i.name == "outer::inner::Deep::go").unwrap();
        assert_eq!(go.impl_for.as_deref(), Some("Deep"));
        assert!(go.calls.iter().any(|c| c.name == "helper" && c.receiver.as_deref() == Some("super")));
    }

    #[test]
    fn test_module_function_does_not_share_id_with_top_level() {
        let items = parse_snippet(r#"
fn helper() -> i32 { 1 }

mod tests {
    fn helper() -> i32 { 2 }
}
"#);
        let fns = function_items(&items);
        assert_eq!(fns.len(), 2);
        // The top-level name, and so its id, is what it always was.
        assert_eq!(fns[0].name, "helper");
        assert_eq!(fns[1].name, "tests::helper");
        assert_ne!(fns[0].generate_id(), fns[1].generate_id());
        // The title still carries the bare name.
        assert_eq!(fns[1].signature.as_deref(), Some("fn helper() -> i32"));
    }

    #[test]
    fn test_function_range_ends_at_block_end() {
        let items = parse_snippet(r#"
fn pair(a: &str) -> [&str; 2] {
    let open = "{";
    [a, open]
}

fn after() {}
"#);
        let pair = items.iter().find(|i| i.name == "pair").unwrap();
        // Not line 2 (the `;` in the return type), and not past line 5 (the `{` in the string).
        assert_eq!((pair.line_start, pair.line_end), (2, 5));
        assert!(pair.content.trim_end().ends_with('}'));
        let after = items.iter().find(|i| i.name == "after").unwrap();
        assert_eq!((after.line_start, after.line_end), (7, 7));
    }

    #[test]
    fn test_trait_default_method() {
        let items = parse_snippet(r#"
trait Greeter {
    fn name(&self) -> String;

    fn greet(&self) -> String {
        format_greeting(self.name())
    }
}
"#);
        assert!(items.iter().any(|i| i.item_type == "trait" && i.name == "Greeter"));
        let fns = function_items(&items);
        // The bodiless declaration is not emitted.
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "greet");
        assert_eq!(fns[0].impl_for.as_deref(), Some("Greeter"));
        assert_eq!(fns[0].impl_trait, None);
        assert_eq!((fns[0].line_start, fns[0].line_end), (5, 7));
        assert!(fns[0].calls.iter().any(|c| c.name == "format_greeting"));
        assert!(fns[0].calls.iter().any(|c| c.name == "name" && c.receiver.as_deref() == Some("self")));
    }
}
