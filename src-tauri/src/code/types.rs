//! Data types for code import and parsing.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRef {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<String>,
    pub line: usize,
}

/// A parsed code item (function, struct, enum, trait, impl, etc.)
#[derive(Debug, Clone)]
pub struct CodeItem {
    /// Item name (function name, struct name, etc.)
    pub name: String,
    /// Type of item: "function", "struct", "enum", "trait", "impl", "module", "macro"
    pub item_type: String,
    /// Full path to source file
    pub file_path: String,
    /// 1-indexed line number where item starts
    pub line_start: usize,
    /// 1-indexed line number where item ends
    pub line_end: usize,
    /// Full source code of the item
    pub content: String,
    /// Function/method signature (without body) for functions
    pub signature: Option<String>,
    /// Visibility: "pub", "pub(crate)", "pub(super)", "" (private)
    pub visibility: String,
    /// Doc comment (/// or //!) if present
    pub doc_comment: Option<String>,
    /// Whether function is async
    pub is_async: bool,
    /// Whether function is unsafe
    pub is_unsafe: bool,
    /// For impl blocks: the type being implemented
    pub impl_for: Option<String>,
    /// For impl blocks: the trait being implemented (if any)
    pub impl_trait: Option<String>,
    /// Call sites found in function body (populated by AST walkers)
    pub calls: Vec<CallRef>,
}

impl CodeItem {
    /// Generate a stable ID for this code item from its file path, name, and type.
    /// `file_path` must already be the root-relative form stored in the node's
    /// tags (`code::rel_path`): it is hashed as-is, so two files differ in id
    /// exactly when their stored paths differ.
    pub fn generate_id(&self) -> String {
        code_id(&self.file_path, &self.id_parts())
    }

    /// ID this item had before ids were hashed from the root-relative path:
    /// `file_path` went through `legacy_normalize_path` first. Only for
    /// `mca code remap-ids`, which maps old ids to new.
    pub fn legacy_id(&self) -> String {
        code_id(&legacy_normalize_path(&self.file_path), &self.id_parts())
    }

    fn id_parts(&self) -> Vec<&str> {
        let mut parts = vec![self.name.as_str(), self.item_type.as_str()];
        // Include impl_for for impl blocks to differentiate impls for different types
        if let Some(ref impl_for) = self.impl_for {
            parts.push(impl_for);
        }
        parts
    }

    /// Get the content_type value for database storage
    pub fn content_type(&self) -> String {
        format!("code_{}", self.item_type)
    }

    /// Create metadata JSON for storage in tags field
    pub fn metadata_json(&self) -> String {
        let metadata = CodeItemMetadata {
            file_path: self.file_path.clone(),
            line_start: self.line_start,
            line_end: self.line_end,
            visibility: self.visibility.clone(),
            signature: self.signature.clone(),
            is_async: self.is_async,
            is_unsafe: self.is_unsafe,
            impl_for: self.impl_for.clone(),
            impl_trait: self.impl_trait.clone(),
            calls: self.calls.clone(),
        };
        serde_json::to_string(&metadata).unwrap_or_default()
    }
}

/// Metadata stored in node.tags field as JSON
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeItemMetadata {
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub visibility: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default)]
    pub is_async: bool,
    #[serde(default)]
    pub is_unsafe: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impl_for: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impl_trait: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<CallRef>,
}

/// Result of a code import operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeImportResult {
    // Rust-specific
    pub functions: usize,
    pub structs: usize,
    pub enums: usize,
    pub traits: usize,
    pub impls: usize,
    pub modules: usize,
    pub macros: usize,
    pub docs: usize,
    // TypeScript/JavaScript
    pub classes: usize,
    pub interfaces: usize,
    pub types: usize,
    pub consts: usize,
    pub vars: usize,
    // Common
    pub files_processed: usize,
    pub files_skipped: usize,
    pub edges_created: usize,
    pub call_edges: usize,
    pub doc_edges: usize,
    /// References deliberately left unlinked because the name was ambiguous.
    /// Reported so a narrowed graph is visible rather than merely absent.
    #[serde(default)]
    pub call_edges_skipped: usize,
    #[serde(default)]
    pub doc_edges_skipped: usize,
    /// Files left out because they carry a generated-code marker.
    #[serde(default)]
    pub generated_skipped: usize,
    /// Ids of the nodes this import wrote (the ones to embed). Not serialized:
    /// a full import would put every id in the JSON output.
    #[serde(skip)]
    pub node_ids: Vec<String>,
    pub errors: Vec<String>,
}

impl Default for CodeImportResult {
    fn default() -> Self {
        Self {
            functions: 0,
            structs: 0,
            enums: 0,
            traits: 0,
            impls: 0,
            modules: 0,
            macros: 0,
            docs: 0,
            classes: 0,
            interfaces: 0,
            types: 0,
            consts: 0,
            vars: 0,
            files_processed: 0,
            files_skipped: 0,
            edges_created: 0,
            call_edges: 0,
            doc_edges: 0,
            call_edges_skipped: 0,
            doc_edges_skipped: 0,
            generated_skipped: 0,
            node_ids: Vec::new(),
            errors: Vec::new(),
        }
    }
}

impl CodeImportResult {
    pub fn total_items(&self) -> usize {
        self.functions + self.structs + self.enums + self.traits + self.impls
            + self.modules + self.macros + self.docs
            + self.classes + self.interfaces + self.types + self.consts + self.vars
    }

    pub fn merge(&mut self, other: &CodeImportResult) {
        self.functions += other.functions;
        self.structs += other.structs;
        self.enums += other.enums;
        self.traits += other.traits;
        self.impls += other.impls;
        self.modules += other.modules;
        self.macros += other.macros;
        self.docs += other.docs;
        self.classes += other.classes;
        self.interfaces += other.interfaces;
        self.types += other.types;
        self.consts += other.consts;
        self.vars += other.vars;
        self.files_processed += other.files_processed;
        self.files_skipped += other.files_skipped;
        self.edges_created += other.edges_created;
        self.call_edges += other.call_edges;
        self.doc_edges += other.doc_edges;
        self.call_edges_skipped += other.call_edges_skipped;
        self.doc_edges_skipped += other.doc_edges_skipped;
        self.generated_skipped += other.generated_skipped;
        self.node_ids.extend(other.node_ids.iter().cloned());
        self.errors.extend(other.errors.iter().cloned());
    }

    /// Increment counter based on item type
    pub fn increment(&mut self, item_type: &str) {
        match item_type {
            "function" => self.functions += 1,
            "struct" => self.structs += 1,
            "enum" => self.enums += 1,
            "trait" => self.traits += 1,
            "impl" => self.impls += 1,
            "module" => self.modules += 1,
            "macro" => self.macros += 1,
            "doc" => self.docs += 1,
            "class" => self.classes += 1,
            "interface" => self.interfaces += 1,
            "type" => self.types += 1,
            "const" => self.consts += 1,
            "var" => self.vars += 1,
            _ => {}
        }
    }
}

/// Hash a code node id from its stored `file_path` and the parts that tell
/// nodes in one file apart (`"module"`, `"doc"`, or an item's name/type).
pub fn code_id(path: &str, parts: &[&str]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    for p in parts {
        p.hash(&mut hasher);
    }
    format!("code-{:016x}", hasher.finish())
}

/// The old id path form, kept only so `mca code remap-ids` can find the ids
/// existing graphs hold. Do not use for new ids: it cuts everything before the
/// first `src/`, so same-tail files in different repos collapsed to one id,
/// and it canonicalizes against the CWD, so a file's id depended on where mca
/// ran.
/// - `/abs/path/to/repo/src/foo.rs` → `src/foo.rs`
/// - `./src/foo.rs` → `src/foo.rs`
/// - `src/foo.rs` → `src/foo.rs`
pub fn legacy_normalize_path(path: &str) -> String {
    use std::path::Path;

    let path = Path::new(path);

    // Try to get canonical path, fall back to original
    let path_str = if let Ok(canonical) = path.canonicalize() {
        canonical.to_string_lossy().to_string()
    } else {
        path.to_string_lossy().to_string()
    };

    // Find common repo markers and extract relative path
    // Look for patterns like: /path/to/repo/src-tauri/... or /path/to/repo/src/...
    let markers = ["src-tauri/", "/src/", "src/"];
    for marker in markers {
        if let Some(idx) = path_str.find(marker) {
            // Include the marker in the result for src-tauri, exclude leading slash for /src/
            let start = if marker == "/src/" { idx + 1 } else { idx };
            return path_str[start..].to_string();
        }
    }

    // If no marker found, strip leading ./ and any absolute path prefix
    let stripped = path_str
        .trim_start_matches("./")
        .trim_start_matches('/');

    // If it looks like an absolute path with home dir, try to extract relative part
    if let Some(idx) = stripped.find("/Repos/") {
        if let Some(end_idx) = stripped[idx + 7..].find('/') {
            // Skip past /Repos/RepoName/
            return stripped[idx + 7 + end_idx + 1..].to_string();
        }
    }

    stripped.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::legacy_normalize_path as normalize_path;

    #[test]
    fn test_merge_sums_call_edges() {
        let mut a = CodeImportResult { call_edges: 3, ..Default::default() };
        let b = CodeImportResult { call_edges: 4, ..Default::default() };
        a.merge(&b);
        assert_eq!(a.call_edges, 7);
    }

    #[test]
    fn test_merge_sums_skipped_edges() {
        let mut a = CodeImportResult { call_edges_skipped: 1, doc_edges_skipped: 2, ..Default::default() };
        let b = CodeImportResult { call_edges_skipped: 10, doc_edges_skipped: 20, ..Default::default() };
        a.merge(&b);
        assert_eq!(a.call_edges_skipped, 11);
        assert_eq!(a.doc_edges_skipped, 22);
    }

    #[test]
    fn test_normalize_path_relative() {
        // Relative paths with ./
        assert_eq!(normalize_path("./src-tauri/src/main.rs"), "src-tauri/src/main.rs");
        assert_eq!(normalize_path("./src/App.tsx"), "src/App.tsx");
    }

    #[test]
    fn test_normalize_path_no_prefix() {
        // Relative paths without ./
        assert_eq!(normalize_path("src-tauri/src/main.rs"), "src-tauri/src/main.rs");
        assert_eq!(normalize_path("src/App.tsx"), "src/App.tsx");
    }

    #[test]
    fn test_normalize_path_absolute() {
        // Absolute paths should extract relative portion
        let abs = "/home/user/Repos/Mycelinet/src-tauri/src/main.rs";
        assert_eq!(normalize_path(abs), "src-tauri/src/main.rs");

        let abs2 = "/home/user/Repos/Mycelinet/src/App.tsx";
        assert_eq!(normalize_path(abs2), "src/App.tsx");
    }

    #[test]
    fn test_normalize_path_consistency() {
        // All these should normalize to the same value
        let paths = [
            "./src-tauri/src/main.rs",
            "src-tauri/src/main.rs",
            "/home/user/Repos/Mycelinet/src-tauri/src/main.rs",
        ];

        let normalized: Vec<_> = paths.iter().map(|p| normalize_path(p)).collect();
        assert!(normalized.iter().all(|p| p == &normalized[0]),
            "All paths should normalize to same value: {:?}", normalized);
    }
}
