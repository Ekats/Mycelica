//! Data types for code import and parsing.

use serde::{Deserialize, Serialize};

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
}

impl CodeItem {
    /// Generate a stable ID for this code item based on file path, name, and type.
    /// Path is normalized to prevent duplicates from different path formats.
    pub fn generate_id(&self) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        // Normalize path before hashing to prevent duplicates from:
        // ./src/foo.rs, src/foo.rs, /abs/path/src/foo.rs
        let normalized_path = normalize_path(&self.file_path);
        normalized_path.hash(&mut hasher);
        self.name.hash(&mut hasher);
        self.item_type.hash(&mut hasher);
        // Include impl_for for impl blocks to differentiate impls for different types
        if let Some(ref impl_for) = self.impl_for {
            impl_for.hash(&mut hasher);
        }
        format!("code-{:016x}", hasher.finish())
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
    // Common
    pub files_processed: usize,
    pub files_skipped: usize,
    pub edges_created: usize,
    pub doc_edges: usize,  // doc → code edges from backtick references
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
            files_processed: 0,
            files_skipped: 0,
            edges_created: 0,
            doc_edges: 0,
            errors: Vec::new(),
        }
    }
}

impl CodeImportResult {
    pub fn total_items(&self) -> usize {
        self.functions + self.structs + self.enums + self.traits + self.impls
            + self.modules + self.macros + self.docs
            + self.classes + self.interfaces + self.types + self.consts
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
            _ => {}
        }
    }
}

/// Normalize a file path for consistent ID generation.
/// Converts any path format to a canonical relative path:
/// - `/abs/path/to/repo/src/foo.rs` → `src/foo.rs`
/// - `./src/foo.rs` → `src/foo.rs`
/// - `src/foo.rs` → `src/foo.rs`
///
/// This ensures the same file always generates the same ID regardless
/// of how the import path was specified.
pub fn normalize_path(path: &str) -> String {
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
        let abs = "/home/user/Repos/Mycelica/src-tauri/src/main.rs";
        assert_eq!(normalize_path(abs), "src-tauri/src/main.rs");

        let abs2 = "/home/user/Repos/Mycelica/src/App.tsx";
        assert_eq!(normalize_path(abs2), "src/App.tsx");
    }

    #[test]
    fn test_normalize_path_consistency() {
        // All these should normalize to the same value
        let paths = [
            "./src-tauri/src/main.rs",
            "src-tauri/src/main.rs",
            "/home/user/Repos/Mycelica/src-tauri/src/main.rs",
        ];

        let normalized: Vec<_> = paths.iter().map(|p| normalize_path(p)).collect();
        assert!(normalized.iter().all(|p| p == &normalized[0]),
            "All paths should normalize to same value: {:?}", normalized);
    }
}
