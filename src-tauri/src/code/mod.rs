//! Code import module for indexing source code as graph nodes.
//!
//! Supports:
//! - Rust (.rs) - parsed with `syn` crate
//! - TypeScript/TSX (.ts, .tsx) - parsed with tree-sitter
//! - JavaScript/JSX (.js, .jsx) - parsed with tree-sitter
//! - Markdown (.md) - documentation files

pub mod rust_parser;
pub mod ts_parser;
pub mod types;

use std::collections::HashSet;
use std::path::Path;

use ignore::WalkBuilder;

use crate::db::{Edge, EdgeType, Node, NodeType, Position};
use crate::db::Database;

pub use types::{CodeImportResult, CodeItem};

/// Supported languages for code import
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Markdown,
}

impl Language {
    /// Detect language from file extension
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "rs" => Some(Language::Rust),
            "ts" | "tsx" => Some(Language::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
            "md" => Some(Language::Markdown),
            _ => None,
        }
    }

    /// Get file extensions for this language
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            Language::Rust => &["rs"],
            Language::TypeScript => &["ts", "tsx"],
            Language::JavaScript => &["js", "jsx", "mjs", "cjs"],
            Language::Markdown => &["md"],
        }
    }

    /// Parse language from CLI argument
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "rust" | "rs" => Some(Language::Rust),
            "typescript" | "ts" | "tsx" => Some(Language::TypeScript),
            "javascript" | "js" | "jsx" => Some(Language::JavaScript),
            "markdown" | "md" => Some(Language::Markdown),
            _ => None,
        }
    }
}

/// Import source code from a path into the database.
///
/// # Arguments
/// * `db` - Database connection
/// * `path` - Path to file or directory
/// * `language_filter` - Optional language filter (None = all supported languages)
///
/// # Returns
/// Import result with counts and any errors
///
/// # Note
/// Respects .gitignore files automatically via the `ignore` crate.
pub fn import_code(
    db: &Database,
    path: &str,
    language_filter: Option<&str>,
) -> Result<CodeImportResult, String> {
    let path = Path::new(path);
    let mut result = CodeImportResult::default();

    // Parse language filter
    let lang_filter = language_filter
        .filter(|s| !s.is_empty() && *s != "auto")
        .and_then(Language::from_str);

    // Collect files to process (respects .gitignore)
    let files = collect_files(path, lang_filter)?;

    if files.is_empty() {
        return Ok(result);
    }

    // Get existing code node IDs for deduplication
    let existing_ids = get_existing_code_ids(db);

    // Process each file
    for (file_path, language) in files {
        match process_file(db, &file_path, language, &existing_ids, &mut result) {
            Ok(_) => result.files_processed += 1,
            Err(e) => {
                result.errors.push(format!("{}: {}", file_path.display(), e));
                result.files_skipped += 1;
            }
        }
    }

    // Second pass: extract doc→code edges from backtick references
    result.doc_edges = extract_doc_code_edges(db);
    result.edges_created += result.doc_edges;

    Ok(result)
}

/// Collect all files to process from a path.
/// Uses the `ignore` crate to respect .gitignore files automatically.
/// Skips nested git repositories (directories with their own .git folder).
fn collect_files(
    path: &Path,
    lang_filter: Option<Language>,
) -> Result<Vec<(std::path::PathBuf, Language)>, String> {
    let mut files = Vec::new();

    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()));
    }

    // Get the root path to identify nested repos
    let root_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // Build walker that respects .gitignore
    let walker = WalkBuilder::new(path)
        .hidden(false)         // Don't skip hidden files (let .gitignore handle it)
        .git_ignore(true)      // Respect .gitignore
        .git_global(true)      // Respect global gitignore
        .git_exclude(true)     // Respect .git/info/exclude
        .require_git(false)    // Work even if not a git repo
        .filter_entry(move |entry| {
            // Skip nested git repositories (directories with their own .git)
            let entry_path = entry.path();
            if entry_path.is_dir() {
                let canonical = entry_path.canonicalize().unwrap_or_else(|_| entry_path.to_path_buf());
                // If this directory has .git and is NOT the root, skip it
                if canonical != root_path && entry_path.join(".git").exists() {
                    return false;
                }
            }
            true
        })
        .build();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[Code] Skipping entry: {}", e);
                continue;
            }
        };

        let path = entry.path();

        // Skip directories
        if path.is_dir() {
            continue;
        }

        // Check file extension
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if let Some(lang) = Language::from_extension(ext) {
                if lang_filter.is_none() || lang_filter == Some(lang) {
                    files.push((path.to_path_buf(), lang));
                }
            }
        }
    }

    Ok(files)
}

/// Public wrapper to get list of files that would be imported.
/// Returns paths only (no language info) for use by CLI --update mode.
pub fn collect_code_files(path: &Path, lang_filter: Option<&str>) -> Result<Vec<std::path::PathBuf>, String> {
    let lang = lang_filter.and_then(Language::from_str);
    let files = collect_files(path, lang)?;
    Ok(files.into_iter().map(|(p, _)| p).collect())
}

/// Get existing code node IDs from the database.
fn get_existing_code_ids(db: &Database) -> HashSet<String> {
    db.get_items()
        .unwrap_or_default()
        .into_iter()
        .filter(|n| n.source.as_deref().map(|s| s.starts_with("code-")).unwrap_or(false))
        .map(|n| n.id)
        .collect()
}

/// Process a single source file.
fn process_file(
    db: &Database,
    path: &Path,
    language: Language,
    existing_ids: &HashSet<String>,
    result: &mut CodeImportResult,
) -> Result<(), String> {
    // Handle markdown separately (single doc node, not code items)
    if language == Language::Markdown {
        return process_markdown_file(db, path, existing_ids, result);
    }

    // Parse code items based on language
    let items = match language {
        Language::Rust => rust_parser::parse_rust_file(path)?,
        Language::TypeScript | Language::JavaScript => ts_parser::parse_ts_file(path)?,
        Language::Markdown => unreachable!(), // Handled above
    };

    // Create module node for the file itself
    let file_module_id = create_file_module_node(db, path, &language)?;
    result.modules += 1;

    // Track IDs we've seen in this import to catch duplicates within same run
    let mut seen_ids: HashSet<String> = HashSet::new();

    // Create nodes for each item
    for item in items {
        let node_id = item.generate_id();

        // Skip if already exists in DB or seen in this import
        if existing_ids.contains(&node_id) || seen_ids.contains(&node_id) {
            continue;
        }

        // Create node (handle UNIQUE constraint gracefully)
        match create_code_node(db, &item, &node_id) {
            Ok(()) => {
                seen_ids.insert(node_id.clone());
                result.increment(&item.item_type);

                // Create DefinedIn edge: item -> file module
                if let Err(e) = create_defined_in_edge(db, &node_id, &file_module_id) {
                    // Edge might exist already, not fatal
                    eprintln!("[Code] Warning: {}", e);
                } else {
                    result.edges_created += 1;
                }
            }
            Err(e) if e.contains("UNIQUE constraint") => {
                // Duplicate ID - skip silently (item already exists)
                continue;
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

/// Process a markdown file as a documentation node.
fn process_markdown_file(
    db: &Database,
    path: &Path,
    existing_ids: &HashSet<String>,
    result: &mut CodeImportResult,
) -> Result<(), String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let file_path_str = path.to_string_lossy().to_string();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    // Generate stable ID based on normalized file path
    let normalized_path = types::normalize_path(&file_path_str);
    let mut hasher = DefaultHasher::new();
    normalized_path.hash(&mut hasher);
    "doc".hash(&mut hasher);
    let node_id = format!("code-{:016x}", hasher.finish());

    // Skip if already exists
    if existing_ids.contains(&node_id) {
        return Ok(());
    }

    // Read file content
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    // Extract title: first H1 heading or filename
    let title = extract_markdown_title(&content)
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(file_name)
                .to_string()
        });

    let now = chrono::Utc::now().timestamp_millis();

    // Create metadata JSON (same format as code nodes)
    let metadata = serde_json::json!({
        "file_path": file_path_str,
        "language": "code-markdown"
    });

    let node = Node {
        id: node_id.clone(),
        node_type: NodeType::Thought,
        title,
        url: None,
        content: Some(content),
        position: Position { x: 0.0, y: 0.0 },
        created_at: now,
        updated_at: now,
        cluster_id: None,
        cluster_label: None,
        depth: 3,
        is_item: true, // Doc is a leaf item for clustering
        is_universe: false,
        parent_id: None, // No parent container - clusters naturally
        child_count: 0,
        ai_title: None,
        summary: None,
        tags: Some(metadata.to_string()),
        emoji: Some("📖".to_string()),
        is_processed: false, // Needs embedding generation
        conversation_id: None,
        sequence_index: None,
        is_pinned: false,
        last_accessed_at: None,
        latest_child_date: None,
        is_private: None,
        privacy_reason: None,
        source: Some("code-markdown".to_string()),
        pdf_available: None,
        content_type: Some("code_doc".to_string()),
        associated_idea_id: None,
        privacy: None,
    };

    db.insert_node(&node)
        .map_err(|e| format!("Failed to insert doc node: {}", e))?;

    result.docs += 1;
    Ok(())
}

/// Extract title from markdown: first # heading
fn extract_markdown_title(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("# ") {
            return Some(trimmed[2..].trim().to_string());
        }
    }
    None
}

/// Create a module node for a source file.
fn create_file_module_node(db: &Database, path: &Path, language: &Language) -> Result<String, String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let file_path = path.to_string_lossy().to_string();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    // Generate stable ID using normalized path
    let normalized_path = types::normalize_path(&file_path);
    let mut hasher = DefaultHasher::new();
    normalized_path.hash(&mut hasher);
    "module".hash(&mut hasher);
    let node_id = format!("code-{:016x}", hasher.finish());

    // Check if already exists
    if db.get_node(&node_id).ok().flatten().is_some() {
        return Ok(node_id);
    }

    let now = chrono::Utc::now().timestamp_millis();
    let source = match language {
        Language::Rust => "code-rust",
        Language::TypeScript => "code-typescript",
        Language::JavaScript => "code-javascript",
        Language::Markdown => "code-markdown",
    };

    let node = Node {
        id: node_id.clone(),
        node_type: NodeType::Thought,
        title: file_name.to_string(),
        url: None,
        content: Some(format!("Source file: {}", file_path)),
        position: Position { x: 0.0, y: 0.0 },
        created_at: now,
        updated_at: now,
        cluster_id: None,
        cluster_label: None,
        depth: 2,
        is_item: false, // File modules are containers, not leaf items
        is_universe: false,
        parent_id: None,
        child_count: 0,
        ai_title: None,
        summary: None,
        tags: Some(format!(r#"{{"file_path":"{}","language":"{}"}}"#, file_path, source)),
        emoji: Some("📄".to_string()),
        is_processed: true,
        conversation_id: None,
        sequence_index: None,
        is_pinned: false,
        last_accessed_at: None,
        latest_child_date: None,
        is_private: None,
        privacy_reason: None,
        source: Some(source.to_string()),
        pdf_available: None,
        content_type: Some("code_module".to_string()),
        associated_idea_id: None,
        privacy: None,
    };

    db.insert_node(&node)
        .map_err(|e| format!("Failed to insert module node: {}", e))?;

    Ok(node_id)
}

/// Create a node for a code item.
fn create_code_node(db: &Database, item: &CodeItem, node_id: &str) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();

    // Build title: include signature for functions
    let title = if let Some(ref sig) = item.signature {
        sig.clone()
    } else if item.item_type == "impl" {
        item.name.clone()
    } else {
        format!("{} {}", item.item_type, item.name)
    };

    // Truncate title if too long
    let title = if title.len() > 100 {
        format!("{}...", &title[..97])
    } else {
        title
    };

    let source = format!("code-{}", detect_language_from_path(&item.file_path));

    let node = Node {
        id: node_id.to_string(),
        node_type: NodeType::Thought,
        title,
        url: None,
        content: Some(item.content.clone()),
        position: Position { x: 0.0, y: 0.0 },
        created_at: now,
        updated_at: now,
        cluster_id: None,
        cluster_label: None,
        depth: 3, // Below file module
        is_item: true, // Code items are leaf nodes
        is_universe: false,
        parent_id: None, // Will be set by hierarchy builder
        child_count: 0,
        ai_title: None,
        summary: item.doc_comment.clone(),
        tags: Some(item.metadata_json()),
        emoji: Some(emoji_for_item_type(&item.item_type)),
        is_processed: false, // Needs embedding generation
        conversation_id: None,
        sequence_index: None,
        is_pinned: false,
        last_accessed_at: None,
        latest_child_date: None,
        is_private: None,
        privacy_reason: None,
        source: Some(source),
        pdf_available: None,
        content_type: Some(item.content_type()),
        associated_idea_id: None,
        privacy: None,
    };

    db.insert_node(&node)
        .map_err(|e| format!("Failed to insert node: {}", e))?;

    Ok(())
}

/// Create a DefinedIn edge from a code item to its file module.
fn create_defined_in_edge(db: &Database, item_id: &str, module_id: &str) -> Result<(), String> {
    let edge = Edge {
        id: format!("edge-{}-{}", item_id, module_id),
        source: item_id.to_string(),
        target: module_id.to_string(),
        edge_type: EdgeType::DefinedIn,
        label: None,
        weight: Some(1.0),
        edge_source: Some("code".to_string()),
        evidence_id: None,
        confidence: Some(1.0),
        created_at: chrono::Utc::now().timestamp_millis(),
    };

    db.insert_edge(&edge)
        .map_err(|e| format!("Failed to insert edge: {}", e))?;

    Ok(())
}

/// Detect language from file path extension.
fn detect_language_from_path(path: &str) -> &'static str {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| match ext {
            "rs" => "rust",
            "ts" | "tsx" => "typescript",
            "js" | "jsx" | "mjs" | "cjs" => "javascript",
            "md" => "markdown",
            _ => "unknown",
        })
        .unwrap_or("unknown")
}

/// Get an emoji for a code item type.
fn emoji_for_item_type(item_type: &str) -> String {
    match item_type {
        // Rust
        "function" => "🔧",
        "struct" => "📦",
        "enum" => "🔢",
        "trait" => "🎭",
        "impl" => "⚙️",
        "module" => "📁",
        "macro" => "🔮",
        "doc" => "📖",
        // TypeScript/JavaScript
        "class" => "🏛️",
        "interface" => "📋",
        "type" => "🏷️",
        "const" => "📌",
        _ => "📝",
    }
    .to_string()
}

/// Extract doc→code edges from backtick references and function call patterns in documentation.
/// Scans all doc nodes and creates Documents edges.
/// Handles patterns like:
/// - Backticks: `function`, `function()`, `Type::method`, `module::func()`
/// - Plain text: function_name(), method_call(args)
fn extract_doc_code_edges(db: &Database) -> usize {
    use std::collections::{HashMap, HashSet};
    use regex::Regex;

    // Common keywords that precede parens but aren't function calls
    const SKIP_KEYWORDS: &[&str] = &[
        "if", "for", "while", "match", "loop", "return", "break", "continue",
        "let", "const", "static", "type", "where", "impl", "trait", "struct",
        "enum", "fn", "pub", "mod", "use", "as", "in", "ref", "mut", "self",
        "super", "crate", "async", "await", "move", "dyn", "box", "yield",
        // Common non-function words
        "e", "g", "i", "s", "t", "a", "eg", "ie", "etc", "vs", "or", "and",
    ];

    // Get all items from database
    let all_items = match db.get_items() {
        Ok(items) => items,
        Err(_) => return 0,
    };

    // Build name→id map for functions and structs
    let mut name_to_id: HashMap<String, String> = HashMap::new();
    for item in &all_items {
        let content_type = item.content_type.as_deref().unwrap_or("");
        if content_type == "code_function" || content_type == "code_struct"
            || content_type == "code_enum" || content_type == "code_trait" {
            // Extract name from title
            if let Some(name) = extract_code_name(&item.title, content_type) {
                name_to_id.insert(name, item.id.clone());
            }
        }
    }

    if name_to_id.is_empty() {
        eprintln!("[DocEdges] No code names found to index");
        return 0;
    }

    // Get doc nodes
    let doc_nodes: Vec<_> = all_items.iter()
        .filter(|n| n.content_type.as_deref() == Some("code_doc"))
        .collect();

    eprintln!("[DocEdges] {} code names indexed, {} docs found", name_to_id.len(), doc_nodes.len());

    if doc_nodes.is_empty() {
        return 0;
    }

    // Get existing Documents edges to avoid duplicates
    let existing_edges: HashSet<(String, String)> = db
        .get_all_edges()
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.edge_type == EdgeType::Documents)
        .map(|e| (e.source, e.target))
        .collect();

    // Match any backtick content
    let backtick_re = Regex::new(r"`([^`]+)`").unwrap();
    // Match snake_case function calls in plain text: word_name() or word_name(args)
    let fn_call_re = Regex::new(r"\b([a-z_][a-z0-9_]*)\s*\(").unwrap();
    // Match CamelCase type names: Node, Edge, Database, EdgeType
    let type_re = Regex::new(r"\b([A-Z][a-zA-Z0-9]+)\b").unwrap();

    let skip_set: HashSet<&str> = SKIP_KEYWORDS.iter().copied().collect();
    let mut edges_created = 0;

    for doc in doc_nodes {
        let content = match &doc.content {
            Some(c) => c,
            None => continue,
        };

        let mut seen_in_doc: HashSet<String> = HashSet::new();

        // Helper closure to try creating an edge for a name
        let mut try_create_edge = |name: String| {
            // Skip if already linked from this doc
            if seen_in_doc.contains(&name) {
                return;
            }

            // Check if this name maps to a code item
            if let Some(code_id) = name_to_id.get(&name) {
                // Skip if edge already exists
                if existing_edges.contains(&(doc.id.clone(), code_id.clone())) {
                    seen_in_doc.insert(name);
                    return;
                }

                // Create Documents edge: doc → code
                let edge = Edge {
                    id: format!("edge-doc-{}-{}", &doc.id[..8.min(doc.id.len())], &code_id[..8.min(code_id.len())]),
                    source: doc.id.clone(),
                    target: code_id.clone(),
                    edge_type: EdgeType::Documents,
                    label: None,
                    weight: Some(1.0),
                    edge_source: Some("code-import".to_string()),
                    evidence_id: None,
                    confidence: Some(0.9),
                    created_at: chrono::Utc::now().timestamp_millis(),
                };

                if db.insert_edge(&edge).is_ok() {
                    edges_created += 1;
                }
                seen_in_doc.insert(name);
            }
        };

        // Pattern 1: Backtick references (existing)
        for cap in backtick_re.captures_iter(content) {
            let backtick_content = &cap[1];
            let identifiers = extract_backtick_identifiers(backtick_content);
            for name in identifiers {
                try_create_edge(name);
            }
        }

        // Pattern 2: Plain text function calls
        // Match snake_case identifiers followed by (
        for cap in fn_call_re.captures_iter(content) {
            let name = cap[1].to_string();

            // Skip common keywords
            if skip_set.contains(name.as_str()) {
                continue;
            }

            // Only match if it looks like a function name (has underscore or >3 chars)
            if name.len() <= 3 && !name.contains('_') {
                continue;
            }

            try_create_edge(name);
        }

        // Pattern 3: CamelCase type names in plain text
        // Match: Node, Edge, Database, EdgeType, etc.
        for cap in type_re.captures_iter(content) {
            let name = cap[1].to_string();

            // Skip common non-type words (articles, questions, acronyms)
            if matches!(name.as_str(),
                "The" | "This" | "That" | "These" | "Those" |
                "When" | "What" | "How" | "Why" | "Where" | "Which" |
                "See" | "For" | "From" | "With" | "Into" | "After" | "Before" |
                "JSON" | "API" | "SQL" | "FTS" | "PDF" | "URL" | "CSS" | "SVG" |
                "CLI" | "TUI" | "GUI" | "IPC" | "BFS" | "DFS" | "CRUD" | "BLOB" |
                "HTML" | "HTTP" | "HTTPS" | "UTF" | "ASCII" | "UUID" |
                "README" | "TODO" | "FIXME" | "NOTE" | "IMPORTANT" |
                "OK" | "NULL" | "TRUE" | "FALSE" | "NONE"
            ) {
                continue;
            }

            try_create_edge(name);
        }
    }

    eprintln!("[DocEdges] Created {} doc→code edges", edges_created);
    edges_created
}

/// Extract identifiers from backtick content.
/// Handles patterns like:
/// - `function` → ["function"]
/// - `function()` → ["function"]
/// - `function(args)` → ["function"]
/// - `Type::method` → ["Type", "method"]
/// - `Type::method()` → ["Type", "method"]
/// - `module::function` → ["module", "function"]
/// - `db.insert_edge()` → ["db", "insert_edge"]
fn extract_backtick_identifiers(content: &str) -> Vec<String> {
    // Strip trailing parentheses and their contents
    let content = if let Some(idx) = content.find('(') {
        &content[..idx]
    } else {
        content
    };

    // Split on :: or . to get segments
    let segments: Vec<&str> = content
        .split(|c| c == ':' || c == '.')
        .filter(|s| !s.is_empty())
        .collect();

    // Return all valid identifiers (start with letter or underscore)
    segments
        .iter()
        .filter(|s| {
            s.chars()
                .next()
                .map(|c| c.is_alphabetic() || c == '_')
                .unwrap_or(false)
        })
        .filter(|s| s.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .map(|s| s.to_string())
        .collect()
}

/// Extract code item name from title based on content_type.
fn extract_code_name(title: &str, content_type: &str) -> Option<String> {
    match content_type {
        "code_function" => {
            // Title format: "fn name(...)" or "pub fn name(...)" or "pub async fn name(...)"
            let re = regex::Regex::new(r"(?:pub\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?;
            re.captures(title).map(|c| c[1].to_string())
        }
        "code_struct" => {
            // Title format: "struct Name" or "pub struct Name"
            let re = regex::Regex::new(r"(?:pub\s+)?struct\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?;
            re.captures(title).map(|c| c[1].to_string())
        }
        "code_enum" => {
            let re = regex::Regex::new(r"(?:pub\s+)?enum\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?;
            re.captures(title).map(|c| c[1].to_string())
        }
        "code_trait" => {
            let re = regex::Regex::new(r"(?:pub\s+)?trait\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?;
            re.captures(title).map(|c| c[1].to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_from_extension() {
        assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("tsx"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("md"), Some(Language::Markdown));
        assert_eq!(Language::from_extension("py"), None);
    }

    #[test]
    fn test_extract_markdown_title() {
        assert_eq!(extract_markdown_title("# Hello World\nContent"), Some("Hello World".to_string()));
        assert_eq!(extract_markdown_title("No heading here"), None);
        assert_eq!(extract_markdown_title("## Not H1\n# This is H1"), Some("This is H1".to_string()));
    }

    #[test]
    fn test_extract_backtick_identifiers() {
        // Simple identifier
        assert_eq!(extract_backtick_identifiers("function"), vec!["function"]);

        // Function call
        assert_eq!(extract_backtick_identifiers("function()"), vec!["function"]);
        assert_eq!(extract_backtick_identifiers("function(args)"), vec!["function"]);

        // Type::method patterns
        assert_eq!(extract_backtick_identifiers("Type::method"), vec!["Type", "method"]);
        assert_eq!(extract_backtick_identifiers("Type::method()"), vec!["Type", "method"]);

        // Module paths
        assert_eq!(extract_backtick_identifiers("module::function"), vec!["module", "function"]);
        assert_eq!(extract_backtick_identifiers("a::b::c"), vec!["a", "b", "c"]);

        // Dot notation
        assert_eq!(extract_backtick_identifiers("db.insert_edge()"), vec!["db", "insert_edge"]);
        assert_eq!(extract_backtick_identifiers("self.field"), vec!["self", "field"]);

        // Mixed
        assert_eq!(extract_backtick_identifiers("Foo::bar.baz()"), vec!["Foo", "bar", "baz"]);

        // Edge cases
        assert_eq!(extract_backtick_identifiers("_private"), vec!["_private"]);
        assert_eq!(extract_backtick_identifiers("CamelCase"), vec!["CamelCase"]);
        assert!(extract_backtick_identifiers("123invalid").is_empty());
    }
}
