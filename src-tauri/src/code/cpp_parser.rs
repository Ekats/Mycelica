//! C++ parser using tree-sitter.
//!
//! Extracts:
//! - Functions and methods with a body: free functions, in-class definitions,
//!   out-of-line definitions (`void ns::Foo::bar() {}`), constructors,
//!   destructors, operators. The item *name* is the scope-qualified,
//!   parameter-typed form `ns::Foo::bar(int,const T&) const`, so overloads and
//!   same-named methods in different classes or namespaces hash to distinct
//!   node IDs; the title is the signature, which is what call resolution reads
//!   the bare name back out of. Prototypes (no body) are not indexed, as in C.
//! - Classes (→ `class`), structs (→ `struct`), enums, namespaces (→ `module`),
//!   typedefs and `using` aliases (→ `type`), macros.
//! - A template is the item it declares; its line range includes the
//!   `template <...>` header.
//!
//! Methods carry `impl_for` = the bare class name: the enclosing class for an
//! in-class definition, the qualifier right before the name for an
//! out-of-line one (`Foo` in `ns::Foo::bar`), unless that qualifier is a
//! namespace declared in the same file or `std`.
//!
//! Call receivers follow the Rust parser: `this->f()` → `self`, `obj.f()` →
//! `obj`, `ptr->f()` → `ptr`, `ns::Foo::f()` → `Foo`. `new Foo(...)` is a call
//! to `Foo` (its constructor).
//!
//! Every slice of source text below is taken from a tree-sitter node's byte
//! range (always on a character boundary) or an already-split line.

use std::collections::HashSet;
use std::path::Path;
use tree_sitter::{Node, Parser};

use super::types::{CallRef, CodeItem};

/// Longest signature kept verbatim before it is truncated.
const MAX_SIGNATURE_BYTES: usize = 160;

/// Whether a `.h` file is parsed as C++ rather than C.
///
/// A header is C unless it shows C++: after comments and string/char literals
/// are blanked out, any of the words `class`, `namespace`, `template`, the
/// labels `public:` / `private:`, or a `::` followed by a name (`std::x`,
/// `::f`, `Foo::~Foo`) makes it C++. A `::` not followed by a name, or inside
/// `:::`, is GCC inline asm's operand separator (`asm("" ::: "memory")`) and
/// does not count. A `.cpp`, `.cc` or `.cxx` file with the same stem in the
/// same directory also makes it C++. An unreadable header stays C.
pub fn header_is_cpp(path: &Path) -> bool {
    if ["cpp", "cc", "cxx"].iter().any(|ext| path.with_extension(ext).is_file()) {
        return true;
    }
    match std::fs::read_to_string(path) {
        Ok(src) => source_looks_cpp(&src),
        Err(_) => false,
    }
}

/// The token half of `header_is_cpp`.
fn source_looks_cpp(src: &str) -> bool {
    static MARKER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let marker = MARKER.get_or_init(|| {
        regex::Regex::new(r"\b(?:class|namespace|template)\b|\b(?:public|private)\s*:|(?:^|[^:])::\s*[A-Za-z_~]").unwrap()
    });
    marker.is_match(&strip_comments_and_literals(src))
}

/// `src` with comments and string/char literals replaced by spaces. Literals
/// end at an unescaped newline at the latest, so one stray quote cannot blank
/// the rest of the file.
fn strip_comments_and_literals(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                if b[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i += 2;
            out.push(' ');
        } else if c == b'"' || c == b'\'' {
            i += 1;
            while i < b.len() && b[i] != c && b[i] != b'\n' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            out.push(' ');
        } else {
            out.push(if c.is_ascii() { c as char } else { ' ' });
            i += 1;
        }
    }
    out
}

/// Parse a C++ file and extract code items.
pub fn parse_cpp_file(path: &Path) -> Result<Vec<CodeItem>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_cpp::LANGUAGE.into())
        .map_err(|e| format!("Failed to set language: {:?}", e))?;

    let tree = parser
        .parse(&content, None)
        .ok_or_else(|| "Failed to parse C++".to_string())?;

    let lines: Vec<&str> = content.lines().collect();
    let mut namespaces = HashSet::from(["std".to_string()]);
    collect_namespaces(tree.root_node(), &content, &mut namespaces);
    let ctx = Ctx {
        file_path: path.to_string_lossy().to_string(),
        content: &content,
        lines: &lines,
        namespaces,
    };

    let mut items = Vec::new();
    let mut scope = Vec::new();
    walk(tree.root_node(), &ctx, &mut scope, &mut items);
    Ok(items)
}

struct Ctx<'a> {
    file_path: String,
    content: &'a str,
    lines: &'a [&'a str],
    /// Namespace names declared in this file, plus `std`: a qualifier that is
    /// one of these is not a class.
    namespaces: HashSet<String>,
}

/// One enclosing namespace or class.
struct Scope {
    /// Qualifies member names (`Box<int*>`, `ns`, `a::b`).
    seg: String,
    /// Bare class name, `None` for a namespace.
    class: Option<String>,
    /// Current access level inside a class body.
    access: String,
}

fn collect_namespaces(node: Node, content: &str, out: &mut HashSet<String>) {
    if node.kind() == "namespace_definition" {
        if let Some(name) = node.child_by_field_name("name") {
            for seg in node_slice(name, content).split("::") {
                out.insert(seg.trim().to_string());
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_namespaces(child, content, out);
    }
}

/// Recursively extract code items. Function bodies and lambdas are never
/// entered: what is inside them belongs to the enclosing function.
fn walk(node: Node, ctx: &Ctx, scope: &mut Vec<Scope>, items: &mut Vec<CodeItem>) {
    match node.kind() {
        "template_declaration" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "template_parameter_list" {
                    declare(child, node, ctx, scope, items);
                }
            }
        }
        "compound_statement" | "lambda_expression" => {}
        "access_specifier" => {
            if let Some(s) = scope.last_mut() {
                s.access = collapse_ws(node_slice(node, ctx.content));
            }
        }
        _ => declare(node, node, ctx, scope, items),
    }
}

/// Handle one declaration. `span` is the node whose lines and leading comment
/// the item takes: the `template_declaration` around it, or the node itself.
fn declare(node: Node, span: Node, ctx: &Ctx, scope: &mut Vec<Scope>, items: &mut Vec<CodeItem>) {
    match node.kind() {
        "function_definition" => {
            if let Some(item) = extract_function(node, span, ctx, scope) {
                items.push(item);
            }
        }
        "class_specifier" | "struct_specifier" => extract_class(node, span, ctx, scope, items),
        "enum_specifier" => {
            if let Some(item) = extract_enum(node, span, ctx, scope) {
                items.push(item);
            }
        }
        "namespace_definition" => {
            let name = node.child_by_field_name("name").map(|n| collapse_ws(node_slice(n, ctx.content)));
            if let Some(ref name) = name {
                let sig = format!("namespace {name}");
                items.push(base_item(span, ctx, qualify(scope, name), "module", Some(sig), "pub".into(), None));
                scope.push(Scope { seg: name.clone(), class: None, access: String::new() });
            }
            if let Some(body) = node.child_by_field_name("body") {
                walk(body, ctx, scope, items);
            }
            if name.is_some() {
                scope.pop();
            }
        }
        "type_definition" => {
            if let Some(item) = extract_typedef(node, span, ctx, scope) {
                items.push(item);
            }
            // `typedef struct Foo { ... } Foo;` defines the struct too.
            if let Some(ty) = node.child_by_field_name("type") {
                walk(ty, ctx, scope, items);
            }
        }
        "alias_declaration" => {
            let name = node.child_by_field_name("name").map(|n| node_slice(n, ctx.content).to_string());
            let ty = node.child_by_field_name("type").map(|n| collapse_ws(node_slice(n, ctx.content)));
            if let (Some(name), Some(ty)) = (name, ty) {
                let sig = truncated_signature(&format!("using {name} = {ty}"));
                items.push(base_item(span, ctx, qualify(scope, &name), "type", Some(sig), member_visibility(scope), enclosing_class(scope)));
            }
        }
        "preproc_def" | "preproc_function_def" => {
            if let Some(item) = extract_macro(node, ctx) {
                items.push(item);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk(child, ctx, scope, items);
            }
        }
    }
}

/// A `CodeItem` with the fields every declaration shares filled in.
fn base_item(
    span: Node,
    ctx: &Ctx,
    name: String,
    item_type: &str,
    signature: Option<String>,
    visibility: String,
    impl_for: Option<String>,
) -> CodeItem {
    let (line_start, line_end) = node_lines(span);
    CodeItem {
        name,
        item_type: item_type.to_string(),
        file_path: ctx.file_path.clone(),
        line_start,
        line_end,
        content: source_lines(ctx.lines, line_start, line_end),
        signature,
        visibility,
        doc_comment: extract_leading_comment(span, ctx.lines),
        is_async: false,
        is_unsafe: false,
        impl_for,
        impl_trait: None,
        calls: Vec::new(),
    }
}

/// `ns::Foo::member`, or just `member` at file scope.
fn qualify(scope: &[Scope], member: &str) -> String {
    let mut parts: Vec<&str> = scope.iter().map(|s| s.seg.as_str()).collect();
    parts.push(member);
    parts.join("::")
}

/// The bare name of the class we are directly inside, if any.
fn enclosing_class(scope: &[Scope]) -> Option<String> {
    scope.last().and_then(|s| s.class.clone())
}

/// Access level inside a class, `pub` outside one.
fn member_visibility(scope: &[Scope]) -> String {
    match scope.last() {
        Some(s) if s.class.is_some() => s.access.clone(),
        _ => "pub".to_string(),
    }
}

/// Class or struct with a body. Its members are walked with the class pushed
/// onto the scope.
fn extract_class(node: Node, span: Node, ctx: &Ctx, scope: &mut Vec<Scope>, items: &mut Vec<CodeItem>) {
    let (Some(name_node), Some(body)) = (node.child_by_field_name("name"), node.child_by_field_name("body")) else {
        // Anonymous or forward-declared: nothing to name, but an anonymous
        // struct's members are still there.
        if let Some(body) = node.child_by_field_name("body") {
            walk(body, ctx, scope, items);
        }
        return;
    };
    let is_class = node.kind() == "class_specifier";
    let keyword = if is_class { "class" } else { "struct" };
    let seg = collapse_ws(node_slice(name_node, ctx.content));
    let bare = bare_segment(name_node, ctx.content);
    let sig = format!("{keyword} {seg}");
    items.push(base_item(span, ctx, qualify(scope, &seg), keyword, Some(sig), member_visibility(scope), enclosing_class(scope)));

    let access = if is_class { "private" } else { "public" };
    scope.push(Scope { seg, class: Some(bare), access: access.to_string() });
    walk(body, ctx, scope, items);
    scope.pop();
}

fn extract_enum(node: Node, span: Node, ctx: &Ctx, scope: &[Scope]) -> Option<CodeItem> {
    let name = node.child_by_field_name("name").map(|n| collapse_ws(node_slice(n, ctx.content)))?;
    node.child_by_field_name("body")?;
    let scoped = has_child_kind(node, "class") || has_child_kind(node, "struct");
    let sig = if scoped { format!("enum class {name}") } else { format!("enum {name}") };
    Some(base_item(span, ctx, qualify(scope, &name), "enum", Some(sig), member_visibility(scope), enclosing_class(scope)))
}

fn extract_typedef(node: Node, span: Node, ctx: &Ctx, scope: &[Scope]) -> Option<CodeItem> {
    let name = typedef_name(node.child_by_field_name("declarator")?, ctx.content)?;
    let ty = collapse_ws(node_slice(node.child_by_field_name("type")?, ctx.content));
    let sig = truncated_signature(&format!("typedef {ty} {name}"));
    Some(base_item(span, ctx, qualify(scope, &name), "type", Some(sig), member_visibility(scope), enclosing_class(scope)))
}

/// The declared name inside a typedef declarator (`*Ptr`, `Arr[4]`, `(*Fn)(int)`).
fn typedef_name(node: Node, content: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" | "identifier" | "primitive_type" => Some(node_slice(node, content).to_string()),
        _ => {
            if let Some(inner) = node.child_by_field_name("declarator") {
                return typedef_name(inner, content);
            }
            let mut cursor = node.walk();
            let found = node.named_children(&mut cursor).find_map(|c| typedef_name(c, content));
            found
        }
    }
}

fn extract_macro(node: Node, ctx: &Ctx) -> Option<CodeItem> {
    let name = node_text(node.child_by_field_name("name")?, ctx.content)?;
    let sig = if node.kind() == "preproc_function_def" {
        let params = node
            .child_by_field_name("parameters")
            .map(|n| node_slice(n, ctx.content).to_string())
            .unwrap_or_else(|| "()".to_string());
        format!("#define {name}{params}")
    } else {
        let value = node
            .child_by_field_name("value")
            .map(|n| node_slice(n, ctx.content).trim().to_string())
            .unwrap_or_default();
        if value.is_empty() {
            format!("#define {name}")
        } else if value.len() > 30 {
            format!("#define {name} {}...", crate::utils::safe_truncate(&value, 27))
        } else {
            format!("#define {name} {value}")
        }
    };
    let mut item = base_item(node, ctx, name, "macro", Some(sig), "pub".into(), None);
    item.doc_comment = None;
    Some(item)
}

// ============== Functions ==============

fn extract_function(node: Node, span: Node, ctx: &Ctx, scope: &[Scope]) -> Option<CodeItem> {
    let fn_decl = function_declarator(node.child_by_field_name("declarator")?)?;
    let name_node = fn_decl.child_by_field_name("declarator")?;
    let (quals, last) = split_qualified(name_node);

    let member = match last.kind() {
        "identifier" | "field_identifier" | "destructor_name" | "operator_name" | "template_function" | "operator_cast" => {
            collapse_ws(node_slice(last, ctx.content))
        }
        _ => return None,
    };
    let params = fn_decl
        .child_by_field_name("parameters")
        .map(|p| param_types(p, ctx.content))
        .unwrap_or_default();
    // `f()` and `f() const` are distinct overloads.
    let mut suffix = String::new();
    let mut cursor = fn_decl.walk();
    for child in fn_decl.children(&mut cursor) {
        if child.kind() == "type_qualifier" || child.kind() == "ref_qualifier" {
            suffix.push(' ');
            suffix.push_str(node_slice(child, ctx.content));
        }
    }

    let mut path: Vec<String> = quals.iter().map(|q| collapse_ws(node_slice(*q, ctx.content))).collect();
    path.push(format!("{member}({params}){suffix}"));
    let name = qualify(scope, &path.join("::"));

    let impl_for = match quals.last() {
        Some(q) => {
            let bare = bare_segment(*q, ctx.content);
            if ctx.namespaces.contains(&bare) { None } else { Some(bare) }
        }
        None => enclosing_class(scope),
    };

    let visibility = if scope.last().is_some_and(|s| s.class.is_some()) {
        member_visibility(scope)
    } else if has_storage_class(node, ctx.content, "static") {
        String::new()
    } else {
        "pub".to_string()
    };

    let end = ["field_initializer_list", "body", "default_method_clause", "delete_method_clause", "pure_virtual_clause"]
        .iter()
        .find_map(|k| node.child_by_field_name(k).or_else(|| child_of_kind(node, k)))
        .map(|n| n.start_byte())
        .unwrap_or_else(|| node.end_byte());
    let signature = truncated_signature(&collapse_ws(
        ctx.content.get(node.start_byte()..end).unwrap_or("").trim_end_matches(';'),
    ));

    let mut item = base_item(span, ctx, name, "function", Some(signature), visibility, impl_for);
    let mut cursor = node.walk();
    for (i, child) in node.children(&mut cursor).enumerate() {
        if !matches!(node.field_name_for_child(i as u32), Some("declarator") | Some("type")) {
            collect_calls(child, ctx.content, &mut item.calls);
        }
    }
    Some(item)
}

/// The `function_declarator` under pointer / reference / parenthesized
/// declarators (`int *f()`, `const T& f()`).
fn function_declarator(node: Node) -> Option<Node> {
    match node.kind() {
        "function_declarator" => Some(node),
        "pointer_declarator" | "reference_declarator" | "parenthesized_declarator" | "attributed_declarator" => {
            let inner = node.child_by_field_name("declarator").or_else(|| node.named_child(0))?;
            function_declarator(inner)
        }
        _ => None,
    }
}

/// `a::b::c` → (`[a, b]`, `c`). A non-qualified node is (`[]`, itself).
fn split_qualified(node: Node) -> (Vec<Node>, Node) {
    let mut quals = Vec::new();
    let mut cur = node;
    while cur.kind() == "qualified_identifier" {
        if let Some(s) = cur.child_by_field_name("scope") {
            quals.push(s);
        }
        match cur.child_by_field_name("name") {
            Some(n) => cur = n,
            None => break,
        }
    }
    (quals, cur)
}

/// `Box` from `Box<T>`, the text of anything else.
fn bare_segment(node: Node, content: &str) -> String {
    match node.kind() {
        "template_type" | "template_function" | "template_method" => node
            .child_by_field_name("name")
            .map(|n| node_slice(n, content).to_string())
            .unwrap_or_else(|| collapse_ws(node_slice(node, content))),
        _ => collapse_ws(node_slice(node, content)),
    }
}

/// Parameter types, comma-joined: `const Foo&,int*,T`. Each is the parameter's
/// text with its name and default value cut out.
fn param_types(list: Node, content: &str) -> String {
    let mut cursor = list.walk();
    let parts: Vec<String> = list
        .named_children(&mut cursor)
        .filter(|p| p.kind() != "comment")
        .map(|p| {
            let start = p.start_byte();
            let mut end = p.end_byte();
            if let Some(dv) = p.child_by_field_name("default_value") {
                end = dv.start_byte();
            }
            let text = match declared_identifier(p) {
                Some(id) if id.end_byte() <= end => {
                    format!("{}{}", content.get(start..id.start_byte()).unwrap_or(""), content.get(id.end_byte()..end).unwrap_or(""))
                }
                _ => content.get(start..end).unwrap_or("").to_string(),
            };
            let text = collapse_ws(text.trim_end().trim_end_matches('='));
            text.replace(" *", "*").replace(" &", "&")
        })
        .collect();
    parts.join(",")
}

/// The identifier a parameter declares, under any pointer/reference/array
/// declarators.
fn declared_identifier(param: Node) -> Option<Node> {
    let mut cur = param.child_by_field_name("declarator")?;
    loop {
        match cur.kind() {
            "identifier" => return Some(cur),
            _ => {
                cur = cur.child_by_field_name("declarator").or_else(|| {
                    let mut cursor = cur.walk();
                    let found = cur.named_children(&mut cursor).find(|c| c.kind().ends_with("declarator") || c.kind() == "identifier");
                    found
                })?;
            }
        }
    }
}

// ============== Calls ==============

fn collect_calls(node: Node, content: &str, calls: &mut Vec<CallRef>) {
    match node.kind() {
        "call_expression" => {
            if let Some(callee) = node.child_by_field_name("function") {
                push_callee(callee, content, calls);
            }
        }
        "new_expression" => {
            // `new Foo(...)` / `new ns::Foo<T>(...)` calls Foo's constructor.
            if let Some(ty) = node.child_by_field_name("type") {
                let (_, last) = split_qualified(ty);
                if matches!(last.kind(), "type_identifier" | "template_type") {
                    calls.push(CallRef { name: bare_segment(last, content), receiver: None, line: ty.start_position().row + 1 });
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
        "identifier" | "template_function" => {
            calls.push(CallRef { name: bare_segment(callee, content), receiver: None, line });
        }
        "qualified_identifier" => {
            let (quals, last) = split_qualified(callee);
            if matches!(last.kind(), "identifier" | "template_function") {
                let receiver = quals.last().map(|q| bare_segment(*q, content));
                calls.push(CallRef { name: bare_segment(last, content), receiver, line });
            }
        }
        "field_expression" => {
            let Some(field) = callee.child_by_field_name("field") else { return };
            let name = match field.kind() {
                "field_identifier" | "template_method" => bare_segment(field, content),
                _ => return,
            };
            let receiver = callee.child_by_field_name("argument").map(|a| {
                if a.kind() == "this" {
                    "self".to_string()
                } else {
                    crate::utils::safe_truncate(&collapse_ws(node_slice(a, content)), 64).to_string()
                }
            });
            calls.push(CallRef { name, receiver, line: field.start_position().row + 1 });
        }
        _ => {}
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

/// Whether the node has a direct child of this kind — used for anonymous
/// keyword tokens (`class` in `enum class`).
fn has_child_kind(node: Node, kind: &str) -> bool {
    child_of_kind(node, kind).is_some()
}

/// The first direct child of the given kind.
fn child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| c.kind() == kind);
    found
}

/// Whether a declaration has this storage class specifier (`static`).
fn has_storage_class(node: Node, content: &str, class: &str) -> bool {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|c| c.kind() == "storage_class_specifier" && node_slice(c, content) == class);
    found
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
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn parse_code(code: &str) -> Vec<CodeItem> {
        let mut file = NamedTempFile::with_suffix(".cpp").unwrap();
        file.write_all(code.as_bytes()).unwrap();
        parse_cpp_file(file.path()).unwrap()
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
namespace app {
class Store {};
struct Point { int x; };
enum Color { Red };
enum class Mode { A, B };
typedef unsigned int uint;
using Ids = std::vector<int>;
}
#define MAX_SIZE 100
#define MAX(a, b) ((a) > (b) ? (a) : (b))
class Forward;
"#,
        );
        let ns = named(&items, "app");
        assert_eq!(ns.item_type, "module");
        assert_eq!(ns.signature.as_deref(), Some("namespace app"));
        assert_eq!(named(&items, "app::Store").item_type, "class");
        assert_eq!(named(&items, "app::Store").signature.as_deref(), Some("class Store"));
        assert_eq!(named(&items, "app::Point").item_type, "struct");
        assert_eq!(named(&items, "app::Color").item_type, "enum");
        assert_eq!(named(&items, "app::Mode").signature.as_deref(), Some("enum class Mode"));
        assert_eq!(named(&items, "app::uint").item_type, "type");
        let ids = named(&items, "app::Ids");
        assert_eq!(ids.item_type, "type");
        assert_eq!(ids.signature.as_deref(), Some("using Ids = std::vector<int>"));
        assert_eq!(named(&items, "MAX_SIZE").item_type, "macro");
        assert_eq!(named(&items, "MAX").signature.as_deref(), Some("#define MAX(a, b)"));
        // A forward declaration is not a definition.
        assert!(!items.iter().any(|i| i.name.contains("Forward")));
    }

    #[test]
    fn test_free_functions() {
        let items = parse_code(
            r#"
int add(int a, int b) { return a + b; }
static void helper() {}
int *ptr_fn() { return nullptr; }
const std::string& ref_fn(const Foo& f, int n = 3) { return s; }
void proto(int);
"#,
        );
        let add = named(&items, "add(int,int)");
        assert_eq!(add.item_type, "function");
        assert_eq!(add.signature.as_deref(), Some("int add(int a, int b)"));
        assert_eq!(add.visibility, "pub");
        assert_eq!(add.impl_for, None);
        assert_eq!(named(&items, "helper()").visibility, "");
        named(&items, "ptr_fn()");
        named(&items, "ref_fn(const Foo&,int)");
        // A prototype is not a definition.
        assert!(!items.iter().any(|i| i.name.starts_with("proto")));
    }

    #[test]
    fn test_in_class_methods_carry_class_and_access() {
        let items = parse_code(
            r#"
class Foo {
    void hidden() {}
public:
    Foo(int x) : base(x) { init(); }
    ~Foo() {}
    int size() const { return 0; }
    bool operator==(const Foo& o) const { return true; }
};
struct Bar { void open() {} };
"#,
        );
        let hidden = named(&items, "Foo::hidden()");
        assert_eq!(hidden.impl_for.as_deref(), Some("Foo"));
        assert_eq!(hidden.visibility, "private");
        let ctor = named(&items, "Foo::Foo(int)");
        assert_eq!(ctor.visibility, "public");
        assert_eq!(ctor.signature.as_deref(), Some("Foo(int x)"));
        assert_eq!(calls_of(ctor), vec![call("init", None)]);
        named(&items, "Foo::~Foo()");
        assert_eq!(named(&items, "Foo::size() const").impl_for.as_deref(), Some("Foo"));
        named(&items, "Foo::operator==(const Foo&) const");
        let open = named(&items, "Bar::open()");
        assert_eq!(open.impl_for.as_deref(), Some("Bar"));
        assert_eq!(open.visibility, "public", "struct members default to public");
    }

    #[test]
    fn test_out_of_line_methods() {
        let items = parse_code(
            r#"
namespace ns {
void Foo::bar(int a) {}
void free_fn() {}
}
void ns::Foo::baz() const {}
QString Foo::name() const { return QString(); }
Foo::Foo() {}
Foo::~Foo() {}
template <typename T> T Box<T>::get() { return v; }
void Outer::Inner::deep() {}
"#,
        );
        let bar = named(&items, "ns::Foo::bar(int)");
        assert_eq!(bar.impl_for.as_deref(), Some("Foo"));
        assert_eq!(bar.signature.as_deref(), Some("void Foo::bar(int a)"));
        // `ns` is a namespace declared in this file, so not a class.
        assert_eq!(named(&items, "ns::free_fn()").impl_for, None);
        assert_eq!(named(&items, "ns::Foo::baz() const").impl_for.as_deref(), Some("Foo"));
        assert_eq!(named(&items, "Foo::name() const").impl_for.as_deref(), Some("Foo"));
        assert_eq!(named(&items, "Foo::Foo()").impl_for.as_deref(), Some("Foo"));
        named(&items, "Foo::~Foo()");
        let get = named(&items, "Box<T>::get()");
        assert_eq!(get.impl_for.as_deref(), Some("Box"));
        assert_eq!(get.line_start, 10, "template header is part of the item");
        assert_eq!(named(&items, "Outer::Inner::deep()").impl_for.as_deref(), Some("Inner"));
    }

    #[test]
    fn test_overloads_and_same_names_get_distinct_ids() {
        let items = parse_code(
            r#"
namespace a { void run() {} }
namespace b { void run() {} }
class X { void f() {} void f() const {} void f(int) {} void f(double) {} };
class Y { void f() {} };
"#,
        );
        let ids: HashSet<String> = items.iter().map(|i| i.generate_id()).collect();
        assert_eq!(ids.len(), items.len(), "{:?}", items.iter().map(|i| &i.name).collect::<Vec<_>>());
        assert_eq!(items.iter().filter(|i| i.item_type == "function").count(), 7);
    }

    #[test]
    fn test_templates_count_as_what_they_declare() {
        let items = parse_code(
            r#"
// Larger of two.
template <typename T>
T max(T a, T b) { return a > b ? a : b; }

template <typename T> class Box { T get() { return v; } };
template <typename T> using Vec = std::vector<T>;
"#,
        );
        let max = named(&items, "max(T,T)");
        assert_eq!(max.item_type, "function");
        assert_eq!((max.line_start, max.line_end), (3, 4));
        assert_eq!(max.doc_comment.as_deref(), Some("// Larger of two."));
        assert_eq!(max.signature.as_deref(), Some("T max(T a, T b)"));
        assert_eq!(named(&items, "Box").item_type, "class");
        assert_eq!(named(&items, "Box::get()").impl_for.as_deref(), Some("Box"));
        assert_eq!(named(&items, "Vec").item_type, "type");
    }

    #[test]
    fn test_call_receivers() {
        let items = parse_code(
            r#"
void Foo::bar() {
    this->baz();
    other.qux();
    ptr->quux();
    ns::Foo::stat();
    helper<int>(1);
    obj.tmpl<int>();
    plain();
    auto f = [&]() { inLambda(); };
    Foo* p = new Foo(1);
}
"#,
        );
        assert_eq!(
            calls_of(named(&items, "Foo::bar()")),
            vec![
                call("baz", Some("self")),
                call("qux", Some("other")),
                call("quux", Some("ptr")),
                call("stat", Some("Foo")),
                call("helper", None),
                call("tmpl", Some("obj")),
                call("plain", None),
                call("inLambda", None),
                call("Foo", None),
            ]
        );
    }

    #[test]
    fn test_header_language_rule() {
        let c = "#ifndef X_H\n#define X_H\n#ifdef __cplusplus\nextern \"C\" {\n#endif\n\
                 /* not a class, no namespace here */\nstruct s { int a; };\n\
                 const char *msg = \"std::string template\";\nint f(int x);\n#endif\n";
        assert!(!source_looks_cpp(c), "comments and strings do not count");
        // GCC inline asm separates operand lists with colons (curl's
        // lib/easy_lock.h); that is not a scope operator.
        assert!(!source_looks_cpp("__asm__ volatile(\"yield\" ::: \"memory\");\nasm(\"\" :: \"r\"(x));\n"));
        for cpp in [
            "class Foo {};",
            "namespace ns {}",
            "template <typename T> T f(T);",
            "struct S {\npublic:\n int x; };",
            "struct S { private : int x; };",
            "int x = ns::y;",
        ] {
            assert!(source_looks_cpp(cpp), "{cpp}");
        }

        let dir = tempfile::tempdir().unwrap();
        let h = dir.path().join("util.h");
        std::fs::write(&h, c).unwrap();
        assert!(!header_is_cpp(&h));
        // Same pure-C text, but a C++ source with the same stem sits next to it.
        std::fs::write(dir.path().join("util.cpp"), "").unwrap();
        assert!(header_is_cpp(&h));
        let h2 = dir.path().join("other.h");
        std::fs::write(&h2, "class Other {};\n").unwrap();
        assert!(header_is_cpp(&h2));
        // A `.c` sibling does not make it C++.
        let h3 = dir.path().join("plain.h");
        std::fs::write(&h3, c).unwrap();
        std::fs::write(dir.path().join("plain.c"), "").unwrap();
        assert!(!header_is_cpp(&h3));
    }

    /// Byte-slice truncation on multi-byte `#define` values, as in the C parser.
    #[test]
    fn test_parse_define_with_multibyte_value_does_not_panic() {
        for pad in 0..3 {
            let value = format!("{}配置文件已更新测试全部通过继续下一步", "x".repeat(pad));
            let items = parse_code(&format!("#define NOTICE \"{value}\"\n"));
            let d = items.iter().find(|i| i.name == "NOTICE").expect("define parsed");
            assert!(d.signature.as_ref().unwrap().ends_with("..."), "pad {pad}");
        }
    }
}
