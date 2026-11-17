//! Code import module for indexing source code as graph nodes.
//!
//! Supports:
//! - Rust (.rs) - parsed with `syn` crate
//! - TypeScript/TSX (.ts, .tsx) - parsed with tree-sitter
//! - JavaScript/JSX (.js, .jsx) - parsed with tree-sitter
//! - Python (.py) - parsed with tree-sitter
//! - C (.c, .h) - parsed with tree-sitter
//! - Go (.go) - parsed with tree-sitter
//! - Swift (.swift) - parsed with tree-sitter
//! - Kotlin (.kt) - parsed with tree-sitter
//! - C# (.cs) - parsed with tree-sitter
//! - C++ (.cpp, .cc, .cxx, .hpp, .hh, .hxx, and .h that is not pure C) - parsed with tree-sitter
//! - QML (.qml) - parsed with tree-sitter
//! - Markdown (.md) - documentation files
//! - reStructuredText (.rst) - documentation files

pub mod c_parser;
pub mod cpp_parser;
pub mod csharp_parser;
pub mod go_parser;
pub mod kotlin_parser;
pub mod python_parser;
pub mod qml_parser;
pub mod rust_parser;
pub mod swift_parser;
pub mod ts_parser;
pub mod types;

use std::collections::HashSet;
use std::path::Path;

use ignore::WalkBuilder;

use crate::db::{Edge, EdgeType, Node, NodeType, Position};
use crate::db::Database;

pub use types::{CodeImportResult, CodeItem, CallRef};

/// The edge types this importer regenerates on every run, and therefore the
/// only ones a reimport may destroy:
///
/// - `defined_in` — `process_file` writes one per item it creates
/// - `calls`      — `resolve_call_edges` drops and rebuilds the whole `ast-analysis` set
/// - `documents`  — `extract_doc_code_edges` re-derives doc→code links from doc content
///
/// Every other edge touching a code node was written from outside the importer
/// (reasoningbank's `derives_from` anchors a distilled strategy to the file node
/// it came from) and is never regenerated, so `run_code_update` snapshots those
/// before deleting the file's nodes and restores them after the reimport.
///
/// A deny-list, not an allow-list: an edge type nobody has invented yet is
/// preserved by default, and only a type added to the three passes above needs
/// to be listed here.
pub const IMPORTER_OWNED_EDGE_TYPES: &[&str] = &["calls", "defined_in", "documents"];

/// Supported languages for code import
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    C,
    Go,
    Swift,
    Kotlin,
    CSharp,
    Cpp,
    Qml,
    Markdown,
    Rst,
}

impl Language {
    /// Detect language from file extension
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "rs" => Some(Language::Rust),
            "ts" | "tsx" => Some(Language::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
            "py" | "pyi" => Some(Language::Python),
            "c" | "h" => Some(Language::C),
            "go" => Some(Language::Go),
            "swift" => Some(Language::Swift),
            "kt" => Some(Language::Kotlin),
            "cs" => Some(Language::CSharp),
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Some(Language::Cpp),
            "qml" => Some(Language::Qml),
            "md" => Some(Language::Markdown),
            "rst" => Some(Language::Rst),
            _ => None,
        }
    }

    /// Detect language from a file on disk: `from_extension`, except that a
    /// `.h` file is C++ when `cpp_parser::header_is_cpp` says so.
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension().and_then(|e| e.to_str())?;
        match Self::from_extension(ext)? {
            Language::C if ext.eq_ignore_ascii_case("h") && cpp_parser::header_is_cpp(path) => Some(Language::Cpp),
            lang => Some(lang),
        }
    }

    /// The `source` column value for this language's nodes; call resolution
    /// only links within one source.
    fn source(&self) -> &'static str {
        match self {
            Language::Rust => "code-rust",
            Language::TypeScript => "code-typescript",
            Language::JavaScript => "code-javascript",
            Language::Python => "code-python",
            Language::C => "code-c",
            Language::Go => "code-go",
            Language::Swift => "code-swift",
            Language::Kotlin => "code-kotlin",
            Language::CSharp => "code-csharp",
            Language::Cpp => "code-cpp",
            Language::Qml => "code-qml",
            Language::Markdown => "code-markdown",
            Language::Rst => "code-rst",
        }
    }

    /// Get file extensions for this language
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            Language::Rust => &["rs"],
            Language::TypeScript => &["ts", "tsx"],
            Language::JavaScript => &["js", "jsx", "mjs", "cjs"],
            Language::Python => &["py", "pyi"],
            Language::C => &["c", "h"],
            Language::Go => &["go"],
            Language::Swift => &["swift"],
            Language::Kotlin => &["kt"],
            Language::CSharp => &["cs"],
            // `.h` is listed under C; `from_path` moves a C++ header here.
            Language::Cpp => &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
            Language::Qml => &["qml"],
            Language::Markdown => &["md"],
            Language::Rst => &["rst"],
        }
    }

    /// Parse language from CLI argument
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "rust" | "rs" => Some(Language::Rust),
            "typescript" | "ts" | "tsx" => Some(Language::TypeScript),
            "javascript" | "js" | "jsx" => Some(Language::JavaScript),
            "python" | "py" => Some(Language::Python),
            "c" | "h" => Some(Language::C),
            "go" | "golang" => Some(Language::Go),
            "swift" => Some(Language::Swift),
            "kotlin" | "kt" => Some(Language::Kotlin),
            "csharp" | "cs" | "c#" => Some(Language::CSharp),
            "cpp" | "c++" | "cxx" => Some(Language::Cpp),
            "qml" => Some(Language::Qml),
            "markdown" | "md" => Some(Language::Markdown),
            "rst" | "restructuredtext" => Some(Language::Rst),
            _ => None,
        }
    }
}

/// Get or create the import container for a language.
/// Returns the container ID.
fn get_or_create_import_container(db: &Database, language: &Language) -> String {
    use crate::settings;

    let (container_id, title, emoji) = match language {
        Language::Rust => (settings::RUST_IMPORT_CONTAINER_ID, "Rust Code", "🦀"),
        Language::TypeScript => (settings::TYPESCRIPT_IMPORT_CONTAINER_ID, "TypeScript Code", "📘"),
        Language::JavaScript => (settings::JAVASCRIPT_IMPORT_CONTAINER_ID, "JavaScript Code", "📒"),
        Language::Python => (settings::PYTHON_IMPORT_CONTAINER_ID, "Python Code", "🐍"),
        Language::C => (settings::C_IMPORT_CONTAINER_ID, "C Code", "⚙️"),
        Language::Go => (settings::GO_IMPORT_CONTAINER_ID, "Go Code", "🐹"),
        Language::Swift => (settings::SWIFT_IMPORT_CONTAINER_ID, "Swift Code", "🐦"),
        Language::Kotlin => (settings::KOTLIN_IMPORT_CONTAINER_ID, "Kotlin Code", "🟣"),
        Language::CSharp => (settings::CSHARP_IMPORT_CONTAINER_ID, "C# Code", "🎼"),
        Language::Cpp => (settings::CPP_IMPORT_CONTAINER_ID, "C++ Code", "🔷"),
        Language::Qml => (settings::QML_IMPORT_CONTAINER_ID, "QML Code", "🟩"),
        Language::Markdown | Language::Rst => (settings::DOCS_IMPORT_CONTAINER_ID, "Documentation", "📄"),
    };

    // Check if container exists
    if db.get_node(container_id).ok().flatten().is_some() {
        return container_id.to_string();
    }

    // Get Universe as parent
    let universe_id = db.get_universe()
        .ok()
        .flatten()
        .map(|u| u.id)
        .unwrap_or_default();

    let now = chrono::Utc::now().timestamp_millis();
    let node = Node {
        id: container_id.to_string(),
        node_type: NodeType::Cluster,
        title: title.to_string(),
        url: None,
        content: Some(format!("Imported {} files", title.to_lowercase())),
        position: Position { x: 0.0, y: 0.0 },
        created_at: now,
        updated_at: now,
        cluster_id: None,
        cluster_label: Some(title.to_string()),
        ai_title: Some(title.to_string()),
        summary: Some(format!("Container for imported {} files", title.to_lowercase())),
        tags: None,
        emoji: Some(emoji.to_string()),
        is_processed: true,
        depth: 1,  // Direct child of Universe
        is_item: false,
        is_universe: false,
        parent_id: if universe_id.is_empty() { None } else { Some(universe_id) },
        child_count: 0,
        conversation_id: None,
        sequence_index: None,
        is_pinned: false,
        last_accessed_at: None,
        latest_child_date: None,
        is_private: None,
        privacy_reason: None,
        source: Some("code-import".to_string()),
        pdf_available: None,
        content_type: Some("import-container".to_string()),
        associated_idea_id: None,
        privacy: Some(1.0),  // Code is public
        human_edited: None,
        human_created: false,
        author: None,
        agent_id: None,
        node_class: None,
        meta_type: None,
    };

    if let Err(e) = db.insert_node(&node) {
        eprintln!("[Code] Failed to create import container {}: {}", container_id, e);
    } else {
        eprintln!("[Code] Created import container: {}", title);
    }

    container_id.to_string()
}

/// Import source code from a path into the database.
///
/// # Arguments
/// * `db` - Database connection
/// * `path` - Path to file or directory
/// * `language_filter` - Optional language filter (None = all supported languages)
/// * `include_nested` - Also walk nested git repos (see `collect_files`)
/// * `include_generated` - Also import files marked as generated (see `is_generated_file`)
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
    include_nested: bool,
    include_generated: bool,
) -> Result<CodeImportResult, String> {
    let path = Path::new(path);
    let mut result = CodeImportResult::default();

    // Parse language filter
    let lang_filter = language_filter
        .filter(|s| !s.is_empty() && *s != "auto")
        .and_then(Language::from_str);

    // Collect files to process (respects .gitignore)
    let (files, generated_skipped) = collect_files(path, lang_filter, include_nested, include_generated)?;
    result.generated_skipped = generated_skipped;

    if files.is_empty() {
        return Ok(result);
    }

    // Get existing code node IDs for deduplication
    let existing_ids = get_existing_code_ids(db);

    let root_prefix = code_root(db);

    for (file_path, language) in files {
        let outcome = catch_parser_panic(|| {
            process_file(db, &file_path, language, &existing_ids, &mut result, &root_prefix)
        });
        match outcome {
            Ok(_) => {
                result.files_processed += 1;
            }
            Err(e) => {
                result.errors.push(format!("{}: {}", file_path.display(), e));
                result.files_skipped += 1;
            }
        }
    }

    // Resolve call edges from stored metadata across ALL nodes
    let calls = resolve_call_edges(db);
    result.call_edges = calls.created;
    result.call_edges_skipped = calls.ambiguous_skipped;

    // Extract doc→code edges from backtick references
    let docs = extract_doc_code_edges(db);
    result.doc_edges = docs.created;
    result.doc_edges_skipped = docs.ambiguous_skipped;

    Ok(result)
}

/// Import a specific list of files (for incremental updates).
/// Runs doc edge extraction once at the end, not per file.
pub fn import_code_files(
    db: &Database,
    files: &[String],
    language_filter: Option<&str>,
) -> Result<CodeImportResult, String> {
    let mut result = CodeImportResult::default();
    let lang_filter = language_filter
        .filter(|s| !s.is_empty() && *s != "auto")
        .and_then(Language::from_str);
    let existing_ids = get_existing_code_ids(db);
    let root_prefix = code_root(db);

    for file_str in files {
        let path = Path::new(file_str);
        if !path.exists() {
            result.errors.push(format!("{}: file not found", file_str));
            result.files_skipped += 1;
            continue;
        }
        let lang = match Language::from_path(path) {
            Some(l) => l,
            None => { result.files_skipped += 1; continue; }
        };
        if lang_filter.is_some() && lang_filter != Some(lang) {
            result.files_skipped += 1;
            continue;
        }
        let outcome = catch_parser_panic(|| {
            process_file(db, path, lang, &existing_ids, &mut result, &root_prefix)
        });
        match outcome {
            Ok(_) => {
                result.files_processed += 1;
            }
            Err(e) => {
                result.errors.push(format!("{}: {}", file_str, e));
                result.files_skipped += 1;
            }
        }
    }

    let calls = resolve_call_edges(db);
    result.call_edges = calls.created;
    result.call_edges_skipped = calls.ambiguous_skipped;
    let docs = extract_doc_code_edges(db);
    result.doc_edges = docs.created;
    result.doc_edges_skipped = docs.ambiguous_skipped;
    Ok(result)
}

/// Resolve the prefix to strip from absolute file paths so stored `file_path`
/// tags come out repo-relative.
///
/// This must be the repository root, never the path being imported: importing
/// `spore/internal/session` used to strip that whole directory and store bare
/// basenames (`http.go`), while a full-tree import of the same file stored
/// `spore/internal/session/http.go`. The two forms hash to different node ids,
/// so a subtree update silently forked every node in it away from the ids the
/// rest of the system (reasoningbank anchors, doc→code edges) already held.
///
/// Falls back to the given path when it is not inside a git repo, which keeps
/// out-of-repo imports working exactly as before.
pub fn repo_root_prefix(path: &Path) -> String {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let start = if canon.is_dir() {
        canon.as_path()
    } else {
        canon.parent().unwrap_or(canon.as_path())
    };
    let mut cur = Some(start);
    while let Some(dir) = cur {
        if dir.join(".git").exists() {
            return with_trailing_slash(dir);
        }
        cur = dir.parent();
    }
    with_trailing_slash(&canon)
}

/// The root every code `file_path` tag and code node id is relative to: the git
/// repo containing the database, else the database's own directory. It is
/// fixed per database, so a file gets one id however it was imported (`.`, an
/// absolute path, a subdirectory, a nested repo, `--update`) and whatever the
/// CWD. The read side resolves tags against the same base (`code_path_base` in
/// the CLI).
pub fn code_root(db: &Database) -> String {
    let db_path = db.get_path();
    // A bare `--db graph.db` has an empty parent, which cannot be canonicalized.
    let db_dir = match Path::new(&db_path).parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => Path::new(".").to_path_buf(),
    };
    repo_root_prefix(&db_dir)
}

/// Stored `file_path` form of a file: canonical, relative to `root`
/// (`code_root`), or the canonical absolute path for a file outside it. This
/// string is hashed into the file's node ids.
pub fn rel_path(path: &Path, root: &str) -> String {
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().to_string();
    abs.strip_prefix(root).map(str::to_string).unwrap_or(abs)
}

fn with_trailing_slash(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    if s.ends_with('/') { s } else { format!("{}/", s) }
}

/// Collect all files to process from a path.
/// Uses the `ignore` crate to respect .gitignore and .mcaignore files automatically.
/// Skips nested git repositories (directories with their own .git folder)
/// unless `include_nested` is set — needed when the root is a plain folder
/// holding several independent repos, which otherwise collects nothing.
/// Skips generated files found by walking (not a file passed as `path`) unless
/// `include_generated` is set; returns how many were skipped.
fn collect_files(
    path: &Path,
    lang_filter: Option<Language>,
    include_nested: bool,
    include_generated: bool,
) -> Result<(Vec<(std::path::PathBuf, Language)>, usize), String> {
    walk_files(path, lang_filter, include_nested, include_generated, true)
}

/// `collect_files`, with `.mcaignore` optional (`now_skipped_files` walks without it).
fn walk_files(
    path: &Path,
    lang_filter: Option<Language>,
    include_nested: bool,
    include_generated: bool,
    use_mcaignore: bool,
) -> Result<(Vec<(std::path::PathBuf, Language)>, usize), String> {
    let mut files = Vec::new();
    let mut generated_skipped = 0;

    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()));
    }

    // Get the root path to identify nested repos
    let root_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // Build walker that respects .gitignore
    let mut builder = WalkBuilder::new(path);
    builder
        .hidden(false)         // Don't skip hidden files (let .gitignore handle it)
        .git_ignore(true)      // Respect .gitignore
        .git_global(true)      // Respect global gitignore
        .git_exclude(true)     // Respect .git/info/exclude
        .require_git(false);   // Work even if not a git repo
    if use_mcaignore {
        builder.add_custom_ignore_filename(".mcaignore");
    }
    let walker = builder
        .filter_entry(move |entry| {
            // Skip nested git repositories (directories with their own .git)
            let entry_path = entry.path();
            if !include_nested && entry_path.is_dir() {
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

        // Check file extension (and, for `.h`, its content)
        if let Some(lang) = Language::from_path(path) {
            if lang_filter.is_none() || lang_filter == Some(lang) {
                if !include_generated && entry.depth() > 0 && is_generated_file(path) {
                    generated_skipped += 1;
                    continue;
                }
                // Canonicalize to resolve ../ and symlinks so stored paths work for `code show`
                let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
                files.push((resolved, lang));
            }
        }
    }

    Ok((files, generated_skipped))
}

/// True if one of the first 5 lines carries a generated-code marker: Go's
/// `// Code generated ... DO NOT EDIT.` convention (also protobuf, stringer,
/// mockgen, ANTLR-Go), `@generated`, or .NET's `<auto-generated>` header.
fn is_generated_file(path: &Path) -> bool {
    use std::io::BufRead;
    static GO_MARKER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let go_marker = GO_MARKER.get_or_init(|| regex::Regex::new(r"^// Code generated .* DO NOT EDIT\.$").unwrap());
    let Ok(file) = std::fs::File::open(path) else { return false };
    std::io::BufReader::new(file)
        .lines()
        .take(5)
        .map_while(Result::ok)
        .any(|line| {
            let line = line.trim_end_matches('\r');
            go_marker.is_match(line) || line.contains("@generated") || line.contains("<auto-generated")
        })
}

/// Public wrapper to get list of files that would be imported.
/// Returns paths only (no language info) for use by CLI --update mode,
/// plus the number of generated files skipped.
pub fn collect_code_files(
    path: &Path,
    lang_filter: Option<&str>,
    include_nested: bool,
    include_generated: bool,
) -> Result<(Vec<std::path::PathBuf>, usize), String> {
    let lang = lang_filter.and_then(Language::from_str);
    let (files, generated_skipped) = collect_files(path, lang, include_nested, include_generated)?;
    Ok((files.into_iter().map(|(p, _)| p).collect(), generated_skipped))
}

/// Files under `path` that are on disk but skipped only because they are
/// generated or `.mcaignore`d: what a walk without either filter collects,
/// minus what `collect_code_files` collects with the same settings.
pub fn now_skipped_files(
    path: &Path,
    lang_filter: Option<&str>,
    include_nested: bool,
    include_generated: bool,
) -> Result<Vec<std::path::PathBuf>, String> {
    let lang = lang_filter.and_then(Language::from_str);
    let (kept, _) = collect_files(path, lang, include_nested, include_generated)?;
    let kept: HashSet<std::path::PathBuf> = kept.into_iter().map(|(p, _)| p).collect();
    let (all, _) = walk_files(path, lang, include_nested, true, false)?;
    Ok(all.into_iter().map(|(p, _)| p).filter(|p| !kept.contains(p)).collect())
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

/// Run one file's parse, turning a panic inside it into that file's error.
///
/// A file that fails to parse is already isolated: the caller counts it in
/// `files_skipped` and records the reason in `errors`, and the other files still
/// import. A *panic* unwound straight past all of that and took the whole run
/// with it — one Go file with a CJK string literal in a `const` stopped
/// `mca update` for every agent on this box, and would have kept stopping it for
/// as long as the file existed. Per-file isolation is the design; this makes a
/// panicking parser obey it instead of exempting itself.
///
/// The result counters may already have counted some items from the file that
/// panicked. That is approximate rather than corrupt, and the alternative is
/// importing nothing at all.
fn catch_parser_panic<F>(f: F) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String>,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            Err(format!("parser panicked: {}", msg))
        }
    }
}

/// Process a single source file.
fn process_file(
    db: &Database,
    path: &Path,
    language: Language,
    existing_ids: &HashSet<String>,
    result: &mut CodeImportResult,
    root_path: &str,
) -> Result<(), String> {
    // Handle documentation files separately (single doc node, not code items)
    if language == Language::Markdown || language == Language::Rst {
        process_doc_file(db, path, language, existing_ids, result, root_path)?;
        return Ok(());
    }

    let items = parse_code_file(path, language)?;

    // Create module node for the file itself
    let file_module_id = create_file_module_node(db, path, &language, root_path)?;
    result.modules += 1;
    result.node_ids.push(file_module_id.clone());

    let mut seen_ids: HashSet<String> = HashSet::new();
    let file_path = rel_path(path, root_path);

    for mut item in items {
        item.file_path = file_path.clone();
        let node_id = item.generate_id();

        if existing_ids.contains(&node_id) || seen_ids.contains(&node_id) {
            continue;
        }

        match create_code_node(db, &item, &node_id, language.source()) {
            Ok(()) => {
                seen_ids.insert(node_id.clone());
                result.node_ids.push(node_id.clone());
                result.increment(&item.item_type);

                if let Err(e) = create_defined_in_edge(db, &node_id, &file_module_id) {
                    eprintln!("[Code] Warning: {}", e);
                } else {
                    result.edges_created += 1;
                }
            }
            Err(e) if e.contains("UNIQUE constraint") => {
                continue;
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

/// Parse the code items of a source file (not Markdown/RST).
pub fn parse_code_file(path: &Path, language: Language) -> Result<Vec<CodeItem>, String> {
    Ok(match language {
        Language::Rust => rust_parser::parse_rust_file(path)?,
        Language::TypeScript | Language::JavaScript => ts_parser::parse_ts_file(path)?,
        Language::Python => python_parser::parse_py_file(path)?,
        Language::C => c_parser::parse_c_file(path)?,
        Language::Go => go_parser::parse_go_file(path)?,
        Language::Swift => swift_parser::parse_swift_file(path)?,
        Language::Kotlin => kotlin_parser::parse_kotlin_file(path)?,
        Language::CSharp => csharp_parser::parse_csharp_file(path)?,
        Language::Cpp => cpp_parser::parse_cpp_file(path)?,
        Language::Qml => qml_parser::parse_qml_file(path)?,
        Language::Markdown | Language::Rst => return Err("not a code file".to_string()),
    })
}

/// Process a documentation file (Markdown or RST) as a documentation node.
fn process_doc_file(
    db: &Database,
    path: &Path,
    language: Language,
    existing_ids: &HashSet<String>,
    result: &mut CodeImportResult,
    root_path: &str,
) -> Result<(), String> {
    let file_path_str = rel_path(path, root_path);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let node_id = types::code_id(&file_path_str, &["doc"]);

    // Skip if already exists
    if existing_ids.contains(&node_id) {
        return Ok(());
    }

    // Read file content
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    // Extract title based on language
    let title = match language {
        Language::Markdown => extract_markdown_title(&content),
        Language::Rst => extract_rst_title(&content),
        _ => None,
    }.unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(file_name)
            .to_string()
    });

    let now = chrono::Utc::now().timestamp_millis();

    let (source_str, lang_str) = match language {
        Language::Markdown => ("code-markdown", "code-markdown"),
        Language::Rst => ("code-rst", "code-rst"),
        _ => ("code-doc", "code-doc"),
    };

    // Create metadata JSON (same format as code nodes)
    let metadata = serde_json::json!({
        "file_path": file_path_str,
        "language": lang_str
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
        source: Some(source_str.to_string()),
        pdf_available: None,
        content_type: Some("code_doc".to_string()),
        associated_idea_id: None,
        privacy: None,
        human_edited: None,
        human_created: false,
        author: None,
        agent_id: None,
        node_class: None,
        meta_type: None,
    };

    db.insert_node(&node)
        .map_err(|e| format!("Failed to insert doc node: {}", e))?;
    result.docs += 1;
    result.node_ids.push(node_id);

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

/// Extract title from RST: first heading with underline
/// RST headings are a line of text followed by a line of = - ~ ^ " etc.
fn extract_rst_title(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let underline_chars = ['=', '-', '`', ':', '\'', '"', '~', '^', '_', '*', '+', '#', '<', '>'];

    for i in 0..lines.len().saturating_sub(1) {
        let line = lines[i].trim();
        let next_line = lines[i + 1].trim();

        // Skip empty lines and directives
        if line.is_empty() || line.starts_with("..") {
            continue;
        }

        // Check if next line is an underline (all same char, at least 3 chars)
        if next_line.len() >= 3 {
            let first_char = next_line.chars().next().unwrap();
            if underline_chars.contains(&first_char)
                && next_line.chars().all(|c| c == first_char)
                && next_line.len() >= line.len()
            {
                return Some(line.to_string());
            }
        }
    }

    // Also check for overline + title + underline pattern
    for i in 1..lines.len().saturating_sub(1) {
        let prev_line = lines[i - 1].trim();
        let line = lines[i].trim();
        let next_line = lines[i + 1].trim();

        if line.is_empty() || line.starts_with("..") {
            continue;
        }

        // Check if both prev and next are matching underlines
        if prev_line.len() >= 3 && next_line.len() >= 3 {
            let prev_char = prev_line.chars().next().unwrap();
            let next_char = next_line.chars().next().unwrap();

            if underline_chars.contains(&prev_char)
                && prev_char == next_char
                && prev_line.chars().all(|c| c == prev_char)
                && next_line.chars().all(|c| c == next_char)
            {
                return Some(line.to_string());
            }
        }
    }

    None
}

/// Create a module node for a source file.
fn create_file_module_node(db: &Database, path: &Path, language: &Language, root_path: &str) -> Result<String, String> {
    let file_path = rel_path(path, root_path);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let node_id = types::code_id(&file_path, &["module"]);

    // Check if already exists
    if db.get_node(&node_id).ok().flatten().is_some() {
        return Ok(node_id);
    }

    // Get or create the import container for this language
    let container_id = get_or_create_import_container(db, language);

    let now = chrono::Utc::now().timestamp_millis();
    let source = language.source();

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
        depth: 2,  // Container is depth 1, file modules are depth 2
        is_item: false, // File modules are containers, not leaf items
        is_universe: false,
        parent_id: Some(container_id),
        child_count: 0,
        ai_title: None,
        summary: None,
        tags: Some(serde_json::json!({ "file_path": file_path, "language": source }).to_string()),
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
        human_edited: None,
        human_created: false,
        author: None,
        agent_id: None,
        node_class: None,
        meta_type: None,
    };

    db.insert_node(&node)
        .map_err(|e| format!("Failed to insert module node: {}", e))?;

    Ok(node_id)
}

/// Create a node for a code item.
/// `source` comes from the file's `Language`, not its extension: a `.h` file
/// may be C or C++.
fn create_code_node(db: &Database, item: &CodeItem, node_id: &str, source: &str) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();

    // Build title: include signature for functions
    let title = if let Some(ref sig) = item.signature {
        sig.clone()
    } else if item.item_type == "impl" {
        item.name.clone()
    } else {
        format!("{} {}", item.item_type, item.name)
    };

    // Truncate title if too long. Language-agnostic, so this one broke for any
    // signature over 100 bytes whose 97th byte landed inside a character.
    let title = if title.len() > 100 {
        format!("{}...", crate::utils::safe_truncate(&title, 97))
    } else {
        title
    };

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
        source: Some(source.to_string()),
        pdf_available: None,
        content_type: Some(item.content_type()),
        associated_idea_id: None,
        privacy: None,
        human_edited: None,
        human_created: false,
        author: None,
        agent_id: None,
        node_class: None,
        meta_type: None,
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
        updated_at: Some(chrono::Utc::now().timestamp_millis()),
        author: None,
        reason: None,
        content: None,
        agent_id: None,
        superseded_by: None,
        metadata: None,
    };

    db.insert_edge(&edge)
        .map_err(|e| format!("Failed to insert edge: {}", e))?;

    Ok(())
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

/// A function node, with the context needed to tell two same-named ones apart.
struct FnDef {
    id: String,
    lang: String,
    file_path: String,
    /// Type this is a method on (Go receiver type, `impl_for` in metadata).
    impl_for: Option<String>,
    /// Go receiver *variable* — `s` in `func (s *Server) Foo()`. A call whose
    /// receiver text is this variable is a call on the same type.
    recv_var: Option<String>,
    /// Rust typed parameters — `db` → `["Database"]` for `db: &Database`. A call
    /// whose receiver is one of these is a call on a type its annotation names.
    params: Vec<(String, Vec<String>)>,
}

impl FnDef {
    /// Whether `recv` is a parameter whose type annotation mentions `ty`.
    fn param_mentions(&self, recv: &str, ty: &str) -> bool {
        self.params.iter().any(|(var, idents)| var == recv && idents.iter().any(|i| i == ty))
    }
}

/// Result of a call-edge resolution pass.
#[derive(Debug, Default, Clone, Copy)]
pub struct CallEdgeStats {
    pub created: usize,
    /// Call references whose name matched more than one function node and could
    /// not be narrowed to exactly one by receiver, file, or directory. These get
    /// no edge at all: a missing edge is honest, a guessed one is not.
    pub ambiguous_skipped: usize,
}

/// Directory part of a repo-relative path (`""` for a bare filename).
fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

/// Names a qualifier could plausibly be referring to for this file: the
/// containing directory (a Go package name, a Rust module dir) and the file
/// stem (a Rust module file). Used to resolve `session.NewManager()` /
/// `parser::parse()` to the definition in `.../session/` or `parser.rs`.
fn module_hints(path: &str) -> [&str; 2] {
    let dir = dir_of(path);
    let dir_name = dir.rsplit_once('/').map(|(_, d)| d).unwrap_or(dir);
    let base = path.rsplit_once('/').map(|(_, f)| f).unwrap_or(path);
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    [dir_name, stem]
}

/// The Go receiver variable in a method signature: `func (s *Server) Foo()` → `s`.
fn go_receiver_var(signature: &str) -> Option<String> {
    let rest = signature.strip_prefix("func ")?.trim_start();
    let inner = rest.strip_prefix('(')?;
    let inner = &inner[..inner.find(')')?];
    let first = inner.split_whitespace().next()?;
    // `(s *Server)` has a name; `(*Server)` does not.
    if first.starts_with('*') || first.is_empty() {
        return None;
    }
    Some(first.trim_end_matches(',').to_string())
}

/// The typed parameters of a Rust signature, each with the identifiers its type
/// mentions: `fn f(db : & Database, ids : Vec < NodeId >)` →
/// `[("db", ["Database"]), ("ids", ["Vec", "NodeId"])]`. `self` and destructuring
/// patterns are left out. Empty for any other language's signature.
fn rust_param_types(signature: &str) -> Vec<(String, Vec<String>)> {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';

    // Skip to the parameter list: past `fn name` and any `<generics>`. The first
    // `(` will not do — `pub(crate) fn` and `fn f<F : Fn(A)>(..)` both have one earlier.
    let Some(fn_at) = signature
        .match_indices("fn ")
        .map(|(i, _)| i)
        .find(|&i| i == 0 || signature[..i].ends_with(' '))
    else {
        return Vec::new();
    };
    let rest = signature[fn_at + 3..].trim_start();
    let rest = rest.trim_start_matches(is_ident).trim_start();
    let mut chars = rest.char_indices().peekable();
    let mut prev = ' ';
    if rest.starts_with('<') {
        let mut depth = 0i32;
        while let Some(&(_, c)) = chars.peek() {
            match c {
                '<' => depth += 1,
                '>' if prev != '-' => depth -= 1,
                _ => {}
            }
            prev = c;
            chars.next();
            if depth == 0 {
                break;
            }
        }
        while chars.peek().is_some_and(|&(_, c)| c == ' ') {
            chars.next();
        }
    }
    if chars.next().map(|(_, c)| c) != Some('(') {
        return Vec::new();
    }

    // Split the parameter list on its top-level commas.
    let mut parts: Vec<&str> = Vec::new();
    let mut depth = 0i32;
    let mut start = chars.peek().map(|&(i, _)| i).unwrap_or(rest.len());
    prev = ' ';
    for (i, c) in chars {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' if depth == 0 => {
                parts.push(&rest[start..i]);
                break;
            }
            ')' | ']' | '}' => depth -= 1,
            '>' if prev != '-' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&rest[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        prev = c;
    }

    parts
        .into_iter()
        .filter_map(|part| {
            let (var, ty) = part.split_once(':')?;
            let var = var.trim();
            let var = var.strip_prefix("mut ").unwrap_or(var).trim();
            if var.is_empty() || !var.chars().all(is_ident) || ty.starts_with(':') {
                return None;
            }
            let idents = ty
                .split(|c: char| !is_ident(c))
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            Some((var.to_string(), idents))
        })
        .collect()
}

/// Whether a Rust call may refer to this candidate at all.
///
/// A free function always may. A method needs something tying the call to its
/// type: `self.f()` / `Self::f()` in the same type, `Type::f()`, or a receiver
/// that is a parameter annotated with that type. Without it a uniquely named
/// method collects every std call of the same name — one `fn len` in the repo
/// and every `v.len()` points at it.
fn rust_call_may_refer(cand: &FnDef, caller: &FnDef, call: &CallRef) -> bool {
    let Some(ty) = cand.impl_for.as_deref() else {
        return true;
    };
    match call.receiver.as_deref() {
        None => false,
        Some("self") => caller.impl_for.as_deref() == Some(ty),
        Some(recv) => recv == ty || caller.param_mentions(recv, ty),
    }
}

/// Pick the one function a call refers to, or `None` if the name stays ambiguous.
///
/// Narrows the same-named candidates by receiver, then by file, then by
/// directory, and only commits when exactly one survives.
fn pick_callee(defs: &[FnDef], cands: &[usize], caller: &FnDef, call: &CallRef) -> Option<usize> {
    if cands.len() == 1 {
        return Some(cands[0]);
    }
    let mut set: Vec<usize> = cands.to_vec();

    if let Some(recv) = call.receiver.as_deref() {
        // `s.Foo()` inside `func (s *Server) Bar()` — and `self.foo()` — is a
        // call on the caller's own type. Anything else is a variable or a
        // package qualifier.
        let own_method = recv == "self" || caller.recv_var.as_deref() == Some(recv);
        let by_type: Vec<usize> = if own_method {
            match caller.impl_for.as_deref() {
                Some(t) => set.iter().copied().filter(|&i| defs[i].impl_for.as_deref() == Some(t)).collect(),
                None => Vec::new(),
            }
        } else {
            set.iter()
                .copied()
                .filter(|&i| {
                    defs[i].impl_for.as_deref().is_some_and(|t| t == recv || caller.param_mentions(recv, t))
                })
                .collect()
        };
        if !by_type.is_empty() {
            set = by_type;
        } else if !own_method {
            let by_module: Vec<usize> = set
                .iter()
                .copied()
                .filter(|&i| module_hints(&defs[i].file_path).contains(&recv))
                .collect();
            if !by_module.is_empty() {
                set = by_module;
            }
        }
        if set.len() == 1 {
            return Some(set[0]);
        }
    }

    // An unparseable node has no file_path, and "both have no file_path" is not
    // evidence of anything.
    if caller.file_path.is_empty() {
        return None;
    }

    let same_file: Vec<usize> = set.iter().copied().filter(|&i| defs[i].file_path == caller.file_path).collect();
    if same_file.len() == 1 {
        return Some(same_file[0]);
    }
    if !same_file.is_empty() {
        set = same_file;
    }

    let caller_dir = dir_of(&caller.file_path);
    let same_dir: Vec<usize> = set.iter().copied().filter(|&i| dir_of(&defs[i].file_path) == caller_dir).collect();
    if same_dir.len() == 1 {
        return Some(same_dir[0]);
    }

    None
}

/// Resolve call edges from stored CallRefs in node metadata.
/// Deletes all existing ast-analysis call edges, then re-resolves from ALL
/// function nodes in the DB. This ensures incremental imports don't lose
/// incoming edges from unchanged files.
///
/// A name can belong to several functions (139 nodes share the basename
/// `app.go`; `View`/`Update` are methods on twenty different Go types). Those
/// candidates are disambiguated by receiver, file, then directory; when that
/// still leaves more than one, no edge is written and the skip is counted.
pub fn resolve_call_edges(db: &Database) -> CallEdgeStats {
    use std::collections::{HashMap, HashSet};

    let _ = db.delete_edges_by_type_and_source("calls", "ast-analysis");

    let all_items = match db.get_items() {
        Ok(items) => items,
        Err(_) => return CallEdgeStats::default(),
    };

    let mut defs: Vec<FnDef> = Vec::new();
    // (language, name) → every function node with that name
    let mut name_map: HashMap<(String, String), Vec<usize>> = HashMap::new();
    // (index into defs, calls) from stored metadata
    let mut all_calls: Vec<(usize, Vec<CallRef>)> = Vec::new();

    for item in &all_items {
        if item.content_type.as_deref() != Some("code_function") {
            continue;
        }
        let lang = item.source.as_deref().unwrap_or("").to_string();
        let meta = item
            .tags
            .as_deref()
            .and_then(|tags| serde_json::from_str::<types::CodeItemMetadata>(tags).ok());

        let idx = defs.len();
        defs.push(FnDef {
            id: item.id.clone(),
            lang: lang.clone(),
            file_path: meta.as_ref().map(|m| m.file_path.clone()).unwrap_or_default(),
            impl_for: meta.as_ref().and_then(|m| m.impl_for.clone()),
            recv_var: meta
                .as_ref()
                .and_then(|m| m.signature.as_deref())
                .and_then(go_receiver_var),
            params: meta
                .as_ref()
                .filter(|_| lang == "code-rust")
                .and_then(|m| m.signature.as_deref())
                .map(rust_param_types)
                .unwrap_or_default(),
        });

        if let Some(name) = extract_code_name(&item.title, "code_function") {
            name_map.entry((lang, name)).or_default().push(idx);
        }
        if let Some(meta) = meta {
            if !meta.calls.is_empty() {
                all_calls.push((idx, meta.calls));
            }
        }
    }

    if all_calls.is_empty() {
        return CallEdgeStats::default();
    }

    // Resolve and deduplicate (caller_id, callee_id) pairs
    let mut stats = CallEdgeStats::default();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    // Ambiguous names counted once per (caller, name), so the count reads as
    // "edges not written", not "call sites visited".
    let mut skipped: HashSet<(usize, String)> = HashSet::new();
    let mut resolved = Vec::new();
    for (caller_idx, calls) in &all_calls {
        let caller = &defs[*caller_idx];
        for call in calls {
            let key = (caller.lang.clone(), call.name.clone());
            let cands = match name_map.get(&key) {
                Some(c) => c,
                None => continue,
            };
            // Rust only: drop method candidates nothing ties this call to, before
            // any narrowing. A call left with no candidate is not ours to link,
            // and not an ambiguity either.
            let evidenced: Vec<usize>;
            let cands: &[usize] = if caller.lang == "code-rust" {
                evidenced = cands.iter().copied().filter(|&i| rust_call_may_refer(&defs[i], caller, call)).collect();
                if evidenced.is_empty() {
                    continue;
                }
                &evidenced
            } else {
                cands
            };
            match pick_callee(&defs, cands, caller, call) {
                Some(callee_idx) => {
                    if defs[callee_idx].id == caller.id {
                        continue;
                    }
                    let pair = (caller.id.clone(), defs[callee_idx].id.clone());
                    if seen.insert(pair.clone()) {
                        resolved.push(pair);
                    }
                }
                None => {
                    if skipped.insert((*caller_idx, call.name.clone())) {
                        stats.ambiguous_skipped += 1;
                    }
                }
            }
        }
    }

    let now = chrono::Utc::now().timestamp_millis();

    for (caller_id, callee_id) in &resolved {
        let edge = Edge {
            id: format!("calls-{}-{}", caller_id, callee_id),
            source: caller_id.clone(),
            target: callee_id.clone(),
            edge_type: EdgeType::Calls,
            label: None,
            weight: Some(1.0),
            edge_source: Some("ast-analysis".to_string()),
            evidence_id: None,
            confidence: Some(1.0),
            created_at: now,
            updated_at: Some(now),
            author: None,
            reason: None,
            content: None,
            agent_id: None,
            superseded_by: None,
            metadata: None,
        };
        if db.insert_edge(&edge).is_ok() {
            stats.created += 1;
        }
    }

    if stats.ambiguous_skipped > 0 {
        eprintln!(
            "[CallEdges] {} edges created, {} references skipped as ambiguous",
            stats.created, stats.ambiguous_skipped
        );
    }
    stats
}

/// A symbol a doc can point at, with the context to tell same-named ones apart.
struct SymbolDef {
    id: String,
    file_path: String,
    impl_for: Option<String>,
}

/// Result of a doc→code extraction pass.
#[derive(Debug, Default, Clone, Copy)]
pub struct DocEdgeStats {
    pub created: usize,
    /// Doc references naming a symbol that exists more than once, with no
    /// qualifier to pick between them. No edge is written for these.
    pub ambiguous_skipped: usize,
    /// Inserts the DB rejected. Previously swallowed by `.is_ok()`.
    pub insert_failed: usize,
}

/// Extract doc→code edges from backtick references and function call patterns in documentation.
/// Scans all doc nodes and creates Documents edges.
/// Handles patterns like:
/// - Backticks: `function`, `function()`, `Type::method`, `module::func()`
/// - Plain text: function_name(), method_call(args)
///
/// A name that belongs to several symbols is only linked when a qualifier
/// (`Server::View`, `session.Manager`) narrows it to one; otherwise no edge is
/// written and the skip is counted, because a doc edge to an arbitrary
/// same-named node is read as fact by everything that walks the graph.
fn extract_doc_code_edges(db: &Database) -> DocEdgeStats {
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
        Err(_) => return DocEdgeStats::default(),
    };

    // Build name→symbols map for functions and structs. Every same-named
    // symbol is kept: dropping all but the last made 255 symbols unreachable
    // and pointed their docs at an arbitrary namesake.
    let mut name_to_ids: HashMap<String, Vec<SymbolDef>> = HashMap::new();
    for item in &all_items {
        let content_type = item.content_type.as_deref().unwrap_or("");
        if content_type == "code_function" || content_type == "code_struct"
            || content_type == "code_enum" || content_type == "code_trait"
            || content_type == "code_interface" || content_type == "code_class"
            || content_type == "code_const" || content_type == "code_type"
            || content_type == "code_macro" || content_type == "code_var" {
            // Extract name from title
            if let Some(name) = extract_code_name(&item.title, content_type) {
                let meta = item
                    .tags
                    .as_deref()
                    .and_then(|tags| serde_json::from_str::<types::CodeItemMetadata>(tags).ok());
                name_to_ids.entry(name).or_default().push(SymbolDef {
                    id: item.id.clone(),
                    file_path: meta.as_ref().map(|m| m.file_path.clone()).unwrap_or_default(),
                    impl_for: meta.and_then(|m| m.impl_for),
                });
            }
        }
    }

    if name_to_ids.is_empty() {
        eprintln!("[DocEdges] No code names found to index");
        return DocEdgeStats::default();
    }

    // Get doc nodes
    let doc_nodes: Vec<_> = all_items.iter()
        .filter(|n| n.content_type.as_deref() == Some("code_doc"))
        .collect();

    eprintln!("[DocEdges] {} code names indexed, {} docs found", name_to_ids.len(), doc_nodes.len());

    if doc_nodes.is_empty() {
        return DocEdgeStats::default();
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
    // Match source file paths a doc names: spore/internal/tui/details.go
    let path_re = Regex::new(r"\b[\w.-]+(?:/[\w.-]+)*\.(?:go|rs|ts|tsx|js|jsx|py|c|h)\b").unwrap();

    let skip_set: HashSet<&str> = SKIP_KEYWORDS.iter().copied().collect();
    let mut stats = DocEdgeStats::default();

    for doc in doc_nodes {
        let content = match &doc.content {
            Some(c) => c,
            None => continue,
        };

        // Source files this doc names outright — the strongest hint it carries
        // about which of several same-named symbols it means.
        let doc_paths: HashSet<&str> = path_re.find_iter(content).map(|m| m.as_str()).collect();

        // Targets already linked from this doc. Keyed on the resolved node, not
        // the bare name: a doc naming both `Alpha.View` and `Beta.View` links
        // both, where a name-keyed set let only the first one through.
        let mut seen_in_doc: HashSet<String> = HashSet::new();
        // Names left unresolved in this doc, so a later qualified mention
        // (`Server::View` after a bare `View`) still gets its chance, and so
        // the skip is counted once per doc.
        let mut unresolved_in_doc: HashSet<String> = HashSet::new();

        // Helper closure to try creating an edge for a name, given the segment
        // that preceded it (`Server` in `Server::View`), if any.
        let mut try_create_edge = |name: String, qualifier: Option<&str>| {
            // Check if this name maps to a code item
            let cands = match name_to_ids.get(&name) {
                Some(c) => c,
                None => return,
            };
            let code_id = match pick_documented(cands, qualifier, &doc_paths) {
                Some(id) => id,
                None => {
                    if unresolved_in_doc.insert(name) {
                        stats.ambiguous_skipped += 1;
                    }
                    return;
                }
            };
            // Skip if already linked from this doc
            if !seen_in_doc.insert(code_id.clone()) {
                return;
            }
            // Skip if edge already exists
            if existing_edges.contains(&(doc.id.clone(), code_id.clone())) {
                return;
            }

            // Create Documents edge: doc → code. Both ids in full: an id built
            // from the first 8 chars carried 12 bits of node id, and the UNIQUE
            // violations that caused were swallowed silently.
            let edge = Edge {
                id: format!("edge-doc-{}-{}", doc.id, code_id),
                source: doc.id.clone(),
                target: code_id.clone(),
                edge_type: EdgeType::Documents,
                label: None,
                weight: Some(1.0),
                edge_source: Some("code-import".to_string()),
                evidence_id: None,
                confidence: Some(0.9),
                created_at: chrono::Utc::now().timestamp_millis(),
                updated_at: Some(chrono::Utc::now().timestamp_millis()),
                author: None,
                reason: None,
                content: None,
                agent_id: None,
                superseded_by: None,
                metadata: None,
            };

            match db.insert_edge(&edge) {
                Ok(()) => stats.created += 1,
                Err(e) => {
                    stats.insert_failed += 1;
                    if stats.insert_failed <= 5 {
                        eprintln!("[DocEdges] insert failed {} → {}: {}", doc.id, code_id, e);
                    }
                }
            }
        };

        // Pattern 1: Backtick references (existing)
        for cap in backtick_re.captures_iter(content) {
            let backtick_content = &cap[1];
            let identifiers = extract_backtick_identifiers(backtick_content);
            for (i, name) in identifiers.iter().enumerate() {
                // `Server::View` / `db.insert_edge` — the preceding segment is
                // the qualifier that can tell same-named symbols apart.
                let qualifier = if i > 0 { Some(identifiers[i - 1].as_str()) } else { None };
                try_create_edge(name.clone(), qualifier);
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

            try_create_edge(name, None);
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

            try_create_edge(name, None);
        }
    }

    eprintln!(
        "[DocEdges] Created {} doc→code edges, skipped {} ambiguous references",
        stats.created, stats.ambiguous_skipped
    );
    if stats.insert_failed > 0 {
        eprintln!("[DocEdges] {} edges FAILED to insert", stats.insert_failed);
    }
    stats
}

/// True if `file_path` is one of the source files this doc names outright.
fn path_mentioned(file_path: &str, doc_paths: &HashSet<&str>) -> bool {
    doc_paths
        .iter()
        .any(|p| *p == file_path || file_path.ends_with(&format!("/{}", p)) || p.ends_with(&format!("/{}", file_path)))
}

/// Pick the one symbol a doc reference names, or `None` if it stays ambiguous.
///
/// Narrows by qualifier — the type the symbol is a method on (`Server::View`),
/// then its module or file (`session.Manager`, `parser::parse`) — and then by
/// the source files the doc names outright, which is what a wiki page mentioning
/// `spore/internal/tui/details.go` is telling you about the `truncate` next to
/// it. When nothing narrows it to one, the only honest answer is no edge.
fn pick_documented<'a>(
    cands: &'a [SymbolDef],
    qualifier: Option<&str>,
    doc_paths: &HashSet<&str>,
) -> Option<&'a String> {
    if cands.len() == 1 {
        return Some(&cands[0].id);
    }

    let mut set: Vec<&SymbolDef> = cands.iter().collect();
    if let Some(q) = qualifier {
        let mut narrowed: Vec<&SymbolDef> =
            set.iter().copied().filter(|c| c.impl_for.as_deref() == Some(q)).collect();
        if narrowed.is_empty() {
            narrowed = set.iter().copied().filter(|c| module_hints(&c.file_path).contains(&q)).collect();
        }
        if !narrowed.is_empty() {
            set = narrowed;
        }
        if set.len() == 1 {
            return Some(&set[0].id);
        }
    }

    let by_path: Vec<&SymbolDef> = set.iter().copied().filter(|c| path_mentioned(&c.file_path, doc_paths)).collect();
    if by_path.len() == 1 {
        return Some(&by_path[0].id);
    }
    None
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

const KOTLIN_FN_NAME: &str =
    r"\bfun\s+(?:<[^>]*>\s*)?(?:[\w.<>?,\[\] ]+\.)?([a-zA-Z_][a-zA-Z0-9_]*)\s*\(";

/// C#: "[modifiers] ReturnType Name[<T>](" and constructors "[modifiers]
/// Name(". Group 1 is everything before the name, a run of space-separated
/// tokens, where a token may hold a parenthesized tuple type with spaces
/// (`IEnumerable<(Type A, int B)>`); group 2 the name. An explicit interface
/// prefix (`IFoo<T>.Name`) is skipped. Type arguments may hold spaces only
/// after commas, so a generic return type (`Task<List<T>> Send<T>(`) is never
/// read as the name. The name touches its `(`: with a space allowed, the
/// modifier before a tuple return type (`public (int, int) F(`) reads as one.
const CSHARP_FN_NAME: &str =
    r"^((?:(?:[^\s(=]|\([^()]*\))+\s+)*?)(?:[^\s(=]+\.)?([A-Za-z_][A-Za-z0-9_]*)(?:<[^\s()]*(?:,\s+[^\s()]*)*>)?\(";

/// C++ titles no earlier arm can read: an out-of-line definition, whose name
/// follows `::` (`void ns::Foo::bar(`, `Foo::Foo(`), and a pointer or
/// reference return glued to the name (`int *make(`). The first such name
/// touching its `(` wins; a destructor's `~` is not in the set, so `Foo::~Foo(`
/// stays unread.
const CPP_FN_NAME: &str = r"(?:::|[*&])\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(";

/// Extract code item name from title based on content_type.
/// Supports Rust, Python, C, Go, Swift, Kotlin, C# and C++ patterns.
///
/// Kotlin spells its keywords differently enough (`fun`, `val`, `typealias`,
/// `enum class`) that its patterns cannot collide with the others; each is
/// placed so the existing behaviour for every other language is unchanged.
fn extract_code_name(title: &str, content_type: &str) -> Option<String> {
    match content_type {
        "code_function" => {
            // Rust: "fn name(...)" or "pub fn name(...)" or "pub async fn name(...)"
            if let Some(caps) = regex::Regex::new(r"(?:pub\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            // Python: "def name(" or "async def name("
            if let Some(caps) = regex::Regex::new(r"(?:async\s+)?def\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            // Go: "func Name(...)" or "func (r Type) Name(...)"
            if let Some(caps) = regex::Regex::new(r"func\s+(?:\([^)]*\)\s+)?([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            // C: "type name(" where type is void/int/char/bool/size_t/etc or pointer (*) or _t suffix
            // Match: "void foo(", "int bar(", "PyObject func(", "char *str(", "size_t len("
            if let Some(caps) = regex::Regex::new(r"^(?:static\s+)?(?:inline\s+)?(?:const\s+)?(?:unsigned\s+)?(?:signed\s+)?(?:long\s+)?(?:short\s+)?(?:void|int|char|bool|float|double|size_t|ssize_t|uint\d*_t|int\d*_t|[A-Z][a-zA-Z0-9_]*|\w+\s*\*+)\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\(").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            // Kotlin: "fun name(", "suspend fun name(", "fun <T> name(",
            // "fun Recv.name(" — an extension function's receiver is not its
            // name. Last, so no other language's title can reach it.
            if let Some(caps) = regex::Regex::new(KOTLIN_FN_NAME).ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            // C#, last of all. Its shape is generic enough to read other
            // languages' unnamed titles, so it refuses the three that reach
            // it: TS/JS `function f(`, a Swift `init(`, and a C# conversion
            // operator, whose `operator Foo(` would otherwise pose as Foo's
            // constructor.
            if let Some(caps) = regex::Regex::new(CSHARP_FN_NAME).ok()?.captures(title) {
                let prefix: Vec<&str> = caps[1].split_whitespace().collect();
                if !prefix.contains(&"function") && prefix.last() != Some(&"operator") && &caps[2] != "init" {
                    return Some(caps[2].to_string());
                }
            }
            // C++ out-of-line and pointer-returning definitions, after C#,
            // whose arm reads every other C++ shape. `operator()(` is not a
            // name.
            if let Some(caps) = regex::Regex::new(CPP_FN_NAME).ok()?.captures(title) {
                if &caps[1] != "operator" {
                    return Some(caps[1].to_string());
                }
            }
            None
        }
        "code_struct" => {
            // Go: "type Name struct"
            if let Some(caps) = regex::Regex::new(r"type\s+([a-zA-Z_][a-zA-Z0-9_]*)\s+struct").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            // Rust: "struct Name" or "pub struct Name"
            if let Some(caps) = regex::Regex::new(r"(?:pub\s+)?struct\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            // C: "typedef struct { ... } Name" or "struct name"
            if let Some(caps) = regex::Regex::new(r"(?:typedef\s+)?struct\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            None
        }
        "code_enum" => {
            // Kotlin: "enum class Name". First, because the general pattern
            // below would otherwise capture the word `class`.
            if let Some(caps) = regex::Regex::new(r"enum\s+class\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            // Rust/C: "enum Name" or "pub enum Name"
            let re = regex::Regex::new(r"(?:pub\s+)?(?:typedef\s+)?enum\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?;
            re.captures(title).map(|c| c[1].to_string())
        }
        "code_trait" => {
            if let Some(caps) = regex::Regex::new(r"(?:pub\s+)?trait\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            // Kotlin: "interface Name" (Swift's protocol reaches here too).
            let re = regex::Regex::new(r"(?:interface|protocol)\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?;
            re.captures(title).map(|c| c[1].to_string())
        }
        "code_class" => {
            // Python: "class Name" or "class Name(Base)"
            if let Some(caps) = regex::Regex::new(r"class\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            // Kotlin: "object Name". An unnamed `companion object` has no name
            // in its signature and stays out of the index.
            let re = regex::Regex::new(r"\bobject\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?;
            re.captures(title).map(|c| c[1].to_string())
        }
        "code_interface" => {
            // Go: "type Name interface"
            let re = regex::Regex::new(r"type\s+([a-zA-Z_][a-zA-Z0-9_]*)\s+interface").ok()?;
            re.captures(title).map(|c| c[1].to_string())
        }
        "code_const" => {
            // Kotlin: "const val NAME = ..." — first, so the general pattern
            // below does not capture the word `val`.
            if let Some(caps) = regex::Regex::new(r"const\s+val\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            // Go/general: "const Name" or "const Name Type = value"
            if let Some(caps) = regex::Regex::new(r"const\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            // Kotlin `val` / Swift `let`: an immutable binding with no `const`.
            let re = regex::Regex::new(r"\b(?:val|let)\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?;
            re.captures(title).map(|c| c[1].to_string())
        }
        "code_var" => {
            // Go: "var Name" or "var Name Type = value"
            let re = regex::Regex::new(r"var\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?;
            re.captures(title).map(|c| c[1].to_string())
        }
        "code_type" => {
            // Kotlin: "typealias Name = ..."
            if let Some(caps) = regex::Regex::new(r"typealias\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            // Go: "type Name underlying"
            let re = regex::Regex::new(r"type\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?;
            re.captures(title).map(|c| c[1].to_string())
        }
        "code_macro" => {
            // C: "#define NAME" or "#define NAME("
            let re = regex::Regex::new(r"#define\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?;
            re.captures(title).map(|c| c[1].to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reimporting a changed file must not cost the graph the edges written to
    /// it from outside this importer — reasoningbank anchors a distilled
    /// strategy to a file node with `derives_from`, and nothing regenerates it.
    ///
    /// Runs the exact sequence `run_code_update` runs (snapshot → delete →
    /// reimport → restore) and asserts three things at once: the foreign edges
    /// come back, the doc node's content is the *new* file content (a doc node
    /// preserved in place would go stale — `process_doc_file` early-returns on
    /// an existing id), and an edge type the `EdgeType` enum has never heard of
    /// survives with its type string intact rather than collapsing to `related`.
    #[test]
    fn test_reimport_preserves_foreign_edges_and_refreshes_doc_content() {

        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("fe-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let pkg = base.join("pkg");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::create_dir_all(base.join(".git")).unwrap();

        let doc = base.join("notes.md");
        let code = pkg.join("x.go");
        std::fs::write(&doc, "# Notes\n\nfirst version\n").unwrap();
        std::fs::write(&code, "package pkg\n\nfunc Alpha() {}\n").unwrap();

        let files: Vec<String> = vec![
            doc.to_string_lossy().to_string(),
            code.to_string_lossy().to_string(),
        ];

        let db = db_at(&base);
        import_code_files(&db, &files, None).unwrap();

        let node_for = |file_path: &str, content_type: &str| -> String {
            let conn = db.raw_conn().lock().unwrap();
            conn.query_row(
                "SELECT id FROM nodes WHERE JSON_EXTRACT(tags, '$.file_path') = ?1 AND content_type = ?2",
                rusqlite::params![file_path, content_type],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
        };
        let doc_id = node_for("notes.md", "code_doc");
        let module_id = node_for("pkg/x.go", "code_module");

        // Two foreign edges onto those file nodes, written the way a producer
        // outside this crate writes them: a known type and an unknown one.
        {
            let conn = db.raw_conn().lock().unwrap();
            conn.execute(
                "INSERT INTO nodes (id, type, title, content, created_at, updated_at, meta_type) \
                 VALUES ('strategy-1', 'thought', 'strategy: keep the anchor', 'pattern', 0, 0, 'strategy')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO edges (id, source_id, target_id, type, created_at, agent_id) \
                 VALUES ('e-doc', 'strategy-1', ?1, 'derives_from', 7, 'reasoningbank')",
                rusqlite::params![doc_id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO edges (id, source_id, target_id, type, created_at) \
                 VALUES ('e-code', 'strategy-1', ?1, 'not_yet_invented', 7)",
                rusqlite::params![module_id],
            )
            .unwrap();
        }

        // The file changed on disk; now run the update cycle over both paths.
        std::fs::write(&doc, "# Notes\n\nsecond version\n").unwrap();
        let db_paths = vec!["notes.md".to_string(), "pkg/x.go".to_string()];

        let snapshot = db
            .snapshot_foreign_edges_for_files(&db_paths, IMPORTER_OWNED_EDGE_TYPES)
            .unwrap();
        // Exactly the two foreign edges — `defined_in`/`documents` are the
        // importer's own and must not be carried over stale.
        assert_eq!(snapshot.len(), 2, "snapshot should hold only non-importer edges");

        for p in &db_paths {
            db.delete_nodes_by_file_path(p).unwrap();
        }
        assert_eq!(
            edge_types_from(&db, "strategy-1"),
            Vec::<String>::new(),
            "the delete should have taken the anchors out, or this test proves nothing"
        );

        import_code_files(&db, &files, None).unwrap();
        let restored = db.restore_edges(&snapshot).unwrap();
        assert_eq!(restored, 2, "both foreign edges should be restored");

        // The anchors point at the reimported nodes, types intact.
        let mut types = edge_types_from(&db, "strategy-1");
        types.sort();
        assert_eq!(types, vec!["derives_from".to_string(), "not_yet_invented".to_string()]);
        {
            let conn = db.raw_conn().lock().unwrap();
            let (target, agent): (String, Option<String>) = conn
                .query_row(
                    "SELECT target_id, agent_id FROM edges WHERE id = 'e-doc'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(target, doc_id, "the anchor should point at the reimported doc node");
            assert_eq!(agent.as_deref(), Some("reasoningbank"), "row restored verbatim");
        }

        // ...and the doc node carries the new file content, not the old.
        let content = db.get_node(&doc_id).unwrap().unwrap().content.unwrap();
        assert!(
            content.contains("second version") && !content.contains("first version"),
            "doc content went stale across the reimport: {:?}",
            content
        );

        std::fs::remove_dir_all(&base).ok();
    }

    /// Three Go types with a `View` method each, one caller per type, plus a
    /// name defined in two sibling packages and called unqualified.
    ///
    /// `calls` is what `mca nav edges` walks, so a call resolved to an
    /// arbitrary namesake is read as fact. Keyed on (language, name) with a
    /// plain insert, all three `a.View()` / `b.View()` / `c.View()` landed on
    /// whichever `View` was written last — at least two of the three assertions
    /// below fail whichever one that is. The unqualified `Shared()` has no
    /// honest answer at all and must produce no edge.
    #[test]
    fn test_call_edges_disambiguate_by_receiver_and_drop_when_ambiguous() {

        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("ce-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        std::fs::create_dir_all(base.join(".git")).unwrap();
        let pkg = base.join("pkg");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::create_dir_all(base.join("one")).unwrap();
        std::fs::create_dir_all(base.join("two")).unwrap();

        let mut files: Vec<String> = Vec::new();
        for (file, ty, recv) in [("a.go", "Alpha", "a"), ("b.go", "Beta", "b"), ("c.go", "Gamma", "c")] {
            let src = format!(
                "package pkg\n\n\
                 type {ty} struct{{}}\n\n\
                 func ({recv} *{ty}) View() string {{\n\treturn \"{ty}\"\n}}\n\n\
                 func ({recv} *{ty}) Render() string {{\n\treturn {recv}.View()\n}}\n"
            );
            std::fs::write(pkg.join(file), src).unwrap();
            files.push(pkg.join(file).to_string_lossy().to_string());
        }
        std::fs::write(pkg.join("use.go"), "package pkg\n\nfunc Use() {\n\tShared()\n}\n").unwrap();
        files.push(pkg.join("use.go").to_string_lossy().to_string());
        for dir in ["one", "two"] {
            let f = base.join(dir).join("x.go");
            std::fs::write(&f, format!("package {dir}\n\nfunc Shared() {{\n}}\n")).unwrap();
            files.push(f.to_string_lossy().to_string());
        }

        let db = db_at(&base);
        let result = import_code_files(&db, &files, None).unwrap();

        let id_of = |file_path: &str, title_like: &str| -> String {
            let conn = db.raw_conn().lock().unwrap();
            conn.query_row(
                "SELECT id FROM nodes WHERE JSON_EXTRACT(tags, '$.file_path') = ?1 AND title LIKE ?2",
                rusqlite::params![file_path, title_like],
                |r| r.get::<_, String>(0),
            )
            .unwrap_or_else(|e| panic!("no node for {} {}: {}", file_path, title_like, e))
        };
        let calls_from = |id: &str| -> Vec<String> {
            let conn = db.raw_conn().lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT target_id FROM edges WHERE source_id = ?1 AND type = 'calls'")
                .unwrap();
            let rows = stmt
                .query_map(rusqlite::params![id], |r| r.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        // Each Render() reaches its own type's View(), and nobody else's.
        for (file, ty) in [("pkg/a.go", "Alpha"), ("pkg/b.go", "Beta"), ("pkg/c.go", "Gamma")] {
            let render = id_of(file, "%) Render(%");
            let view = id_of(file, "%) View(%");
            assert_eq!(
                calls_from(&render),
                vec![view.clone()],
                "{ty}.Render should call exactly {ty}.View"
            );
        }

        // `Shared()` exists in two packages, called with no receiver from a
        // third: no narrowing is possible, so no edge — and it is counted.
        let use_fn = id_of("pkg/use.go", "func Use(%");
        assert!(
            calls_from(&use_fn).is_empty(),
            "an unresolvable call must produce no edge, got {:?}",
            calls_from(&use_fn)
        );
        assert!(
            result.call_edges_skipped >= 1,
            "the skipped call must be reported, not silently dropped"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    /// Imports one Rust file and returns a lookup: is there a `calls` edge from
    /// one function to another, each named as (`impl_for`, fn name).
    fn rust_call_edges(tag: &str, src: &str) -> impl Fn((Option<&str>, &str), (Option<&str>, &str)) -> bool {

        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("rce-{}-{}", tag, std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        std::fs::create_dir_all(base.join(".git")).unwrap();
        std::fs::create_dir_all(base.join("pkg")).unwrap();
        let file = base.join("pkg").join("lib.rs");
        std::fs::write(&file, src).unwrap();

        let db = db_at(&base);
        import_code_files(&db, &[file.to_string_lossy().to_string()], None).unwrap();
        std::fs::remove_dir_all(&base).ok();

        move |from, to| {
            let conn = db.raw_conn().lock().unwrap();
            let id_of = |(ty, name): (Option<&str>, &str)| -> String {
                conn.query_row(
                    "SELECT id FROM nodes WHERE content_type = 'code_function' \
                     AND JSON_EXTRACT(tags, '$.impl_for') IS ?1 AND (title LIKE ?2 OR title LIKE ?3)",
                    rusqlite::params![ty, format!("%fn {}(%", name), format!("%fn {}<%", name)],
                    |r| r.get::<_, String>(0),
                )
                .unwrap_or_else(|e| panic!("no function node for {:?} {}: {}", ty, name, e))
            };
            conn.query_row(
                "SELECT COUNT(*) FROM edges WHERE type = 'calls' AND source_id = ?1 AND target_id = ?2",
                rusqlite::params![id_of(from), id_of(to)],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
                > 0
        }
    }

    /// `self.f()` reaches the caller's own type. A `self.f()` the caller's type
    /// does not define (a trait or deref method) must not fall through to the
    /// only other `f` in the graph.
    #[test]
    fn test_rust_method_edge_by_self() {
        let edge = rust_call_edges("self", r#"
struct Store;
struct Other;
struct Bare;

impl Store {
    fn load(&self) {}
    fn run(&self) { self.load(); }
}

impl Other {
    fn load(&self) {}
    fn save(&self) {}
}

impl Bare {
    fn run_bare(&self) { self.save(); }
}
"#);
        assert!(edge((Some("Store"), "run"), (Some("Store"), "load")));
        assert!(!edge((Some("Store"), "run"), (Some("Other"), "load")));
        assert!(!edge((Some("Bare"), "run_bare"), (Some("Other"), "save")));
    }

    /// `Type::f()` reaches `f` on that type. `Vec::build()` must not reach the
    /// only `build` in the graph just because it is the only one.
    #[test]
    fn test_rust_method_edge_by_type_path() {
        let edge = rust_call_edges("type", r#"
struct Store;
struct Other;

impl Store {
    fn open() -> Store { Store }
    fn build() -> Store { Store }
}

impl Other {
    fn open() -> Other { Other }
}

fn start() {
    let s = Store::open();
    let v = Vec::build();
}
"#);
        assert!(edge((None, "start"), (Some("Store"), "open")));
        assert!(!edge((None, "start"), (Some("Other"), "open")));
        assert!(!edge((None, "start"), (Some("Store"), "build")));
    }

    /// A receiver that is a typed parameter reaches the method on the type its
    /// annotation names — through a reference, a wrapper, or behind `pub(crate)`.
    #[test]
    fn test_rust_method_edge_by_typed_param() {
        let edge = rust_call_edges("param", r#"
struct Store;
struct Other;

impl Store {
    fn load(&self) {}
}

impl Other {
    fn load(&self) {}
}

fn direct(db: &Store) { db.load(); }

pub(crate) fn wrapped<F: Fn(u8) -> bool>(f: F, mut db: std::sync::Arc<Store>, other: &Other) {
    db.load();
}
"#);
        assert!(edge((None, "direct"), (Some("Store"), "load")));
        assert!(!edge((None, "direct"), (Some("Other"), "load")));
        assert!(edge((None, "wrapped"), (Some("Store"), "load")));
        assert!(!edge((None, "wrapped"), (Some("Other"), "load")));
    }

    /// One `fn len` in the graph must not collect every `v.len()`: a local
    /// variable says nothing about its type.
    #[test]
    fn test_rust_untyped_receiver_writes_no_method_edge() {
        let edge = rust_call_edges("untyped", r#"
struct Snapshot;

impl Snapshot {
    fn len(&self) -> usize { 0 }
}

fn count(names: &[String]) -> usize {
    let v = vec![1, 2, 3];
    v.len() + names.len()
}
"#);
        assert!(!edge((None, "count"), (Some("Snapshot"), "len")));
    }

    /// `x.iter().count()` records no receiver for `count`, so it cannot be tied
    /// to a method. A free function of that name is still reachable.
    #[test]
    fn test_rust_chained_call_writes_no_method_edge() {
        let edge = rust_call_edges("chained", r#"
struct Index;

impl Index {
    fn count(&self) -> usize { 0 }
}

fn tally() -> usize { 1 }

fn total(xs: &[u8]) -> usize {
    xs.iter().count() + tally()
}
"#);
        assert!(!edge((None, "total"), (Some("Index"), "count")));
        assert!(edge((None, "total"), (None, "tally")));
    }

    #[test]
    fn test_rust_param_types() {
        assert_eq!(
            rust_param_types("pub(crate) fn f<F : Fn (u8) -> bool>(& self, f : F, mut db : & Arc < Store >, (a , b) : (u8 , u8)) -> Vec < u8 >"),
            vec![
                ("f".to_string(), vec!["F".to_string()]),
                ("db".to_string(), vec!["Arc".to_string(), "Store".to_string()]),
            ]
        );
        assert!(rust_param_types("func (s *Server) Foo(x int)").is_empty());
    }

    /// A doc naming `Alpha.View` must reach Alpha's `View`, and a doc naming a
    /// bare `View` — which three types define — must reach none of them.
    ///
    /// The bare-name map behind `documents` was also last-write-wins, so every
    /// mention of `View` in every doc pointed at one arbitrary namesake and the
    /// other two were unreachable. At least two of the three qualified
    /// assertions fail whichever namesake won.
    #[test]
    fn test_doc_edges_disambiguate_by_qualifier_and_drop_when_ambiguous() {

        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("de-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        std::fs::create_dir_all(base.join(".git")).unwrap();
        let pkg = base.join("pkg");
        std::fs::create_dir_all(&pkg).unwrap();

        let mut files: Vec<String> = Vec::new();
        for (file, ty, recv) in [("a.go", "Alpha", "a"), ("b.go", "Beta", "b"), ("c.go", "Gamma", "c")] {
            let src = format!(
                "package pkg\n\n\
                 type {ty} struct{{}}\n\n\
                 func ({recv} *{ty}) View() string {{\n\treturn \"{ty}\"\n}}\n"
            );
            std::fs::write(pkg.join(file), src).unwrap();
            files.push(pkg.join(file).to_string_lossy().to_string());
        }
        std::fs::write(
            base.join("qualified.md"),
            "# Qualified\n\nRendering goes through `Alpha.View`, `Beta.View` and `Gamma.View`.\n",
        )
        .unwrap();
        files.push(base.join("qualified.md").to_string_lossy().to_string());
        std::fs::write(base.join("bare.md"), "# Bare\n\nEverything ends up in `View`.\n").unwrap();
        files.push(base.join("bare.md").to_string_lossy().to_string());

        let db = db_at(&base);
        let result = import_code_files(&db, &files, None).unwrap();

        let id_of = |file_path: &str, ct: &str, title_like: &str| -> String {
            let conn = db.raw_conn().lock().unwrap();
            conn.query_row(
                "SELECT id FROM nodes WHERE JSON_EXTRACT(tags, '$.file_path') = ?1 \
                 AND content_type = ?2 AND title LIKE ?3",
                rusqlite::params![file_path, ct, title_like],
                |r| r.get::<_, String>(0),
            )
            .unwrap_or_else(|e| panic!("no node for {} {} {}: {}", file_path, ct, title_like, e))
        };
        let documents_from = |id: &str| -> Vec<String> {
            let conn = db.raw_conn().lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT target_id FROM edges WHERE source_id = ?1 AND type = 'documents'")
                .unwrap();
            let rows = stmt
                .query_map(rusqlite::params![id], |r| r.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        let qualified = documents_from(&id_of("qualified.md", "code_doc", "%"));
        for file in ["pkg/a.go", "pkg/b.go", "pkg/c.go"] {
            let view = id_of(file, "code_function", "%) View(%");
            assert!(
                qualified.contains(&view),
                "qualified doc should link the View in {file}, edges: {:?}",
                qualified
            );
        }

        // Nothing in `bare.md` says which View, so it links none of them.
        let bare = documents_from(&id_of("bare.md", "code_doc", "%"));
        for file in ["pkg/a.go", "pkg/b.go", "pkg/c.go"] {
            let view = id_of(file, "code_function", "%) View(%");
            assert!(
                !bare.contains(&view),
                "an ambiguous doc reference must produce no edge, got one to {file}"
            );
        }
        assert!(
            result.doc_edges_skipped >= 1,
            "the skipped doc reference must be reported, not silently dropped"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    /// One doc naming many symbols gets one edge per symbol.
    ///
    /// The edge id used to be `edge-doc-{first 8 of doc id}-{first 8 of code
    /// id}`. Ids are `code-{16 hex}`, so eight characters left three hex digits
    /// — 12 bits — and any two targets of one doc that collided in those bits
    /// hit the UNIQUE constraint, whose error `.is_ok()` threw away. Over 400
    /// targets that is not a tail risk, it is arithmetic.
    #[test]
    fn test_doc_edge_ids_do_not_collide_across_many_targets() {

        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("dw-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        std::fs::create_dir_all(base.join(".git")).unwrap();

        const N: usize = 400;
        let mut code = String::from("package pkg\n");
        let mut doc = String::from("# Symbols\n");
        for i in 0..N {
            code.push_str(&format!("\nfunc fixture_sym_{i:04}() {{\n}}\n"));
            doc.push_str(&format!("- `fixture_sym_{i:04}`\n"));
        }
        std::fs::create_dir_all(base.join("pkg")).unwrap();
        std::fs::write(base.join("pkg").join("syms.go"), &code).unwrap();
        std::fs::write(base.join("syms.md"), &doc).unwrap();
        let files = vec![
            base.join("pkg").join("syms.go").to_string_lossy().to_string(),
            base.join("syms.md").to_string_lossy().to_string(),
        ];

        let db = db_at(&base);
        let result = import_code_files(&db, &files, None).unwrap();

        let edges: usize = {
            let conn = db.raw_conn().lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM edges e JOIN nodes n ON n.id = e.source_id \
                 WHERE e.type = 'documents' AND n.content_type = 'code_doc'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap() as usize
        };
        assert_eq!(edges, N, "every named symbol should get its own doc→code edge");
        assert_eq!(result.doc_edges, N, "and the reported count should match reality");

        std::fs::remove_dir_all(&base).ok();
    }

    /// Titles over 100 bytes were cut with `&title[..97]`, so a signature whose
    /// 97th byte landed inside a character panicked. Unlike the per-parser
    /// truncations this one is in `create_code_node`, so it applied to every
    /// language.
    ///
    /// Sweeps name lengths so some const lands on each of the three offsets
    /// within a 3-byte character — hand-picking one length only proves the case
    /// you picked.
    #[test]
    fn test_long_multibyte_titles_do_not_panic() {
        use crate::db::Database;

        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("mb-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        std::fs::create_dir_all(base.join(".git")).unwrap();
        std::fs::create_dir_all(base.join("pkg")).unwrap();

        // Value stays under the parser's own 40-byte cut so this exercises the
        // title truncation, not the const one.
        let lengths: Vec<usize> = (70..=100).collect();
        let mut src = String::from("package pkg\n");
        for n in &lengths {
            src.push_str(&format!("\nconst {} = \"配置文件已更新\"\n", "N".repeat(*n)));
        }
        let f = base.join("pkg").join("consts.go");
        std::fs::write(&f, &src).unwrap();
        let files = vec![f.to_string_lossy().to_string()];

        let db = Database::in_memory().unwrap();
        let result = import_code_files(&db, &files, None).unwrap();

        assert!(result.errors.is_empty(), "import reported errors: {:?}", result.errors);
        let consts: i64 = {
            let conn = db.raw_conn().lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM nodes WHERE content_type = 'code_const'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(consts as usize, lengths.len(), "every const should have produced a node");

        std::fs::remove_dir_all(&base).ok();
    }

    /// A panicking parser must cost its own file, not the whole import.
    ///
    /// The importer already isolates a file that *fails* — the caller counts it
    /// and keeps going. A panic used to unwind past that and abort the run, so
    /// a single unparseable file stopped the graph import for everyone.
    #[test]
    fn test_catch_parser_panic_isolates_the_file() {
        assert_eq!(catch_parser_panic(|| Ok(())), Ok(()));
        assert_eq!(
            catch_parser_panic(|| Err("plain failure".to_string())),
            Err("plain failure".to_string())
        );

        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let str_panic = catch_parser_panic(|| panic!("boom"));
        let string_panic = catch_parser_panic(|| panic!("{}", format!("boom {}", 2)));
        std::panic::set_hook(hook);

        assert_eq!(str_panic, Err("parser panicked: boom".to_string()));
        assert_eq!(string_panic, Err("parser panicked: boom 2".to_string()));
    }

    /// Edge types on every edge incident to `node_id`.
    fn edge_types_from(db: &crate::db::Database, node_id: &str) -> Vec<String> {
        let conn = db.raw_conn().lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT type FROM edges WHERE source_id = ?1 OR target_id = ?1")
            .unwrap();
        let rows = stmt
            .query_map(rusqlite::params![node_id], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        rows
    }

    /// A subtree import must strip the repo root, not the subtree, so a file
    /// keeps one file_path — and therefore one node id — whichever path it was
    /// imported through. Fixture lives under target/ (gitignored, self-cleaning)
    /// rather than /tmp, which other suites here have been poisoned by.
    #[test]
    fn test_repo_root_prefix_is_repo_root_not_import_path() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("rrp-{}", std::process::id()));
        let sub = base.join("pkg").join("inner");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(base.join(".git")).unwrap();
        let file = sub.join("x.go");
        std::fs::write(&file, "package inner\n").unwrap();

        let want = format!("{}/", base.canonicalize().unwrap().to_string_lossy());

        // Whole repo, a subtree, and a single file all yield the repo root —
        // and the nested .git wins over the mco checkout enclosing it.
        assert_eq!(repo_root_prefix(&base), want);
        assert_eq!(repo_root_prefix(&sub), want);
        assert_eq!(repo_root_prefix(&file), want);

        // ...so the stored file_path is repo-relative for a subtree update too.
        let abs = file.canonicalize().unwrap().to_string_lossy().to_string();
        assert_eq!(
            abs.strip_prefix(&repo_root_prefix(&sub)).unwrap(),
            "pkg/inner/x.go"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    /// A database inside `base`, so its code root (`code_root`) is `base` when
    /// `base` holds a `.git` or sits outside any repo.
    fn db_at(base: &Path) -> crate::db::Database {
        crate::db::Database::new(base.join("graph.db")).unwrap()
    }

    fn fixture_dir(name: &str) -> std::path::PathBuf {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("{}-{}", name, std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn put(base: &Path, rel: &str, body: &str) {
        let p = base.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// `collect_files` output as sorted root-relative paths.
    fn collected(base: &Path, include_nested: bool) -> Vec<String> {
        let root = base.canonicalize().unwrap();
        let mut got: Vec<String> = collect_files(base, None, include_nested, false)
            .unwrap()
            .0
            .into_iter()
            .map(|(p, _)| p.strip_prefix(&root).unwrap().to_string_lossy().to_string())
            .collect();
        got.sort();
        got
    }

    /// CODEID-PREFIX-01: two repos holding the same in-repo path under one import
    /// root are two files, so two nodes each. The old id cut the path at `src/`
    /// and collapsed them into one.
    #[test]
    fn test_same_tail_files_in_sibling_repos_get_distinct_ids() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        for r in ["r1", "r2"] {
            std::fs::create_dir_all(base.join(r).join(".git")).unwrap();
            put(base, &format!("{r}/app/src/main/kotlin/com/x/Foo.kt"), "package com.x\nclass Foo { fun f() {} }\n");
        }
        let db = db_at(base);
        let result = import_code(&db, base.to_str().unwrap(), None, true, false).unwrap();
        assert_eq!(result.errors, Vec::<String>::new());
        let conn = db.raw_conn().lock().unwrap();
        let count = |ct: &str| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM nodes WHERE content_type = ?1", [ct], |r| r.get(0)).unwrap()
        };
        assert_eq!(count("code_class"), 2, "one class Foo per repo");
        assert_eq!(count("code_module"), 2, "one Foo.kt module per repo");
        assert_eq!(count("code_function"), 2);
    }

    /// A plain folder of separate repos (no `.git` at the top) collects
    /// nothing by default — every subrepo is skipped whole.
    #[test]
    fn test_collect_files_nested_repo_under_non_git_root() {
        let base = fixture_dir("cf-nongit");
        std::fs::create_dir_all(base.join("a/.git")).unwrap();
        put(&base, "a/src/Foo.kt", "class Foo\n");

        assert!(collected(&base, false).is_empty());
        assert_eq!(collected(&base, true), vec!["a/src/Foo.kt"]);

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn test_collect_files_nested_repo_under_git_root() {
        let base = fixture_dir("cf-git");
        std::fs::create_dir_all(base.join(".git")).unwrap();
        std::fs::create_dir_all(base.join("n/.git")).unwrap();
        put(&base, "src/Top.kt", "class Top\n");
        put(&base, "n/src/Bar.kt", "class Bar\n");

        assert_eq!(collected(&base, false), vec!["src/Top.kt"]);
        assert_eq!(collected(&base, true), vec!["n/src/Bar.kt", "src/Top.kt"]);

        std::fs::remove_dir_all(&base).ok();
    }

    /// With `include_nested`, a nested repo's own `.gitignore` still applies —
    /// a Gradle `build/` holds generated code that can dwarf the real source.
    /// `b` is the control: same layout, no `.gitignore`, so its `build/` file
    /// is collected. The root `!build/` cancels the mco checkout's `build/`
    /// rule, which (`require_git(false)`) reaches fixtures under target/ and
    /// would otherwise pass this without `a/.gitignore` doing anything.
    #[test]
    fn test_collect_files_include_nested_honours_nested_gitignore() {
        let base = fixture_dir("cf-gi");
        std::fs::write(base.join(".gitignore"), "!build/\n").unwrap();
        for repo in ["a", "b"] {
            std::fs::create_dir_all(base.join(repo).join(".git")).unwrap();
            put(&base, &format!("{}/src/Foo.kt", repo), "class Foo\n");
            put(&base, &format!("{}/build/Gen.kt", repo), "class Gen\n");
        }
        std::fs::write(base.join("a/.gitignore"), "build/\n").unwrap();

        assert_eq!(
            collected(&base, true),
            vec!["a/src/Foo.kt", "b/build/Gen.kt", "b/src/Foo.kt"]
        );

        std::fs::remove_dir_all(&base).ok();
    }

    /// A `.git` *file* (worktree / submodule checkout) marks a nested repo
    /// just like a `.git` directory.
    #[test]
    fn test_collect_files_git_file_marks_nested_repo() {
        let base = fixture_dir("cf-gitfile");
        put(&base, "a/.git", "gitdir: ../.git/worktrees/a\n");
        put(&base, "a/src/Foo.kt", "class Foo\n");

        assert!(collected(&base, false).is_empty());
        assert_eq!(collected(&base, true), vec!["a/src/Foo.kt"]);

        std::fs::remove_dir_all(&base).ok();
    }

    /// a.go is plain; gen.go is generated; .mcaignore lists ignored.go and the
    /// generated ignored_gen.go, so no flag can bring those two back.
    pub(crate) fn generated_fixture(name: &str) -> std::path::PathBuf {
        let base = fixture_dir(name);
        std::fs::create_dir_all(base.join(".git")).unwrap();
        put(&base, "a.go", "package a\n\nfunc A() {}\n");
        put(&base, "gen.go", "// Code generated by stringer. DO NOT EDIT.\n\npackage a\n\nfunc Gen() {}\n");
        put(&base, "ignored.go", "package a\n\nfunc Ignored() {}\n");
        put(&base, "ignored_gen.go", "// Code generated by mockgen. DO NOT EDIT.\n\npackage a\n\nfunc IgnoredGen() {}\n");
        put(&base, ".mcaignore", "ignored*.go\n");
        base
    }

    fn file_names(paths: &[std::path::PathBuf]) -> Vec<String> {
        let mut names: Vec<String> = paths.iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn test_collect_code_files_skips_generated_and_mcaignore() {
        let base = generated_fixture("cf-generated");

        let (files, skipped) = collect_code_files(&base, None, false, false).unwrap();
        assert_eq!(file_names(&files), vec!["a.go"]);
        assert_eq!(skipped, 1);

        let (files, skipped) = collect_code_files(&base, None, false, true).unwrap();
        assert_eq!(file_names(&files), vec!["a.go", "gen.go"]);
        assert_eq!(skipped, 0);

        // A generated file named directly is taken as-is.
        let (files, _) = collect_code_files(&base.join("gen.go"), None, false, false).unwrap();
        assert_eq!(file_names(&files), vec!["gen.go"]);

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn test_now_skipped_files() {
        let base = generated_fixture("now-skipped");
        put(&base, "ignored_doc.md", "# doc\n");
        put(&base, ".mcaignore", "ignored*.go\nignored_doc.md\n");
        let skipped = now_skipped_files(&base, None, false, false).unwrap();
        assert_eq!(file_names(&skipped), vec!["gen.go", "ignored.go", "ignored_doc.md", "ignored_gen.go"]);
        // With --include-generated the plain generated file is collected again;
        // the .mcaignored ones stay skipped.
        let skipped = now_skipped_files(&base, None, false, true).unwrap();
        assert_eq!(file_names(&skipped), vec!["ignored.go", "ignored_doc.md", "ignored_gen.go"]);
        // A language filter never reaches other languages.
        let skipped = now_skipped_files(&base, Some("go"), false, false).unwrap();
        assert_eq!(file_names(&skipped), vec!["gen.go", "ignored.go", "ignored_gen.go"]);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn test_import_code_skips_generated_and_mcaignore() {
        let base = generated_fixture("ic-generated");
        let imported = |include_generated: bool| -> (Vec<String>, usize) {
            let db = Database::in_memory().unwrap();
            let res = import_code(&db, base.to_str().unwrap(), None, false, include_generated).unwrap();
            let mut names: Vec<String> = db.get_all_code_file_paths().unwrap().into_iter()
                .map(|(p, _)| Path::new(&p).file_name().unwrap().to_string_lossy().to_string())
                .collect();
            names.sort();
            (names, res.generated_skipped)
        };

        assert_eq!(imported(false), (vec!["a.go".to_string()], 1));
        assert_eq!(imported(true), (vec!["a.go".to_string(), "gen.go".to_string()], 0));

        std::fs::remove_dir_all(&base).ok();
    }

    /// The module node's tags are JSON even when the path needs escaping.
    #[test]
    fn test_module_node_tags_escape_path() {
        let base = fixture_dir("module-tags");
        std::fs::create_dir_all(base.join(".git")).unwrap();
        put(&base, "we\"ird\\name.go", "package a\n\nfunc A() {}\n");
        let db = db_at(&base);
        import_code(&db, base.to_str().unwrap(), None, false, false).unwrap();
        let module = db.get_items().unwrap().into_iter().chain(db.get_all_nodes(false).unwrap())
            .find(|n| n.content_type.as_deref() == Some("code_module"))
            .expect("module node");
        let tags: serde_json::Value = serde_json::from_str(module.tags.as_deref().unwrap())
            .expect("module tags are not valid JSON");
        assert_eq!(tags["file_path"], "we\"ird\\name.go");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn test_is_generated_file_markers() {
        let base = fixture_dir("gen-markers");
        let check = |body: &str| {
            put(&base, "f.go", body);
            is_generated_file(&base.join("f.go"))
        };
        assert!(check("// Code generated by protoc-gen-go. DO NOT EDIT.\npackage p\n"));
        assert!(check("// Code generated from X.g4 by ANTLR 4.13.2. DO NOT EDIT.\r\npackage p\n"));
        assert!(check("/*\n * @generated by codegen\n */\n"));
        assert!(!check("// Code generated by hand, edit freely.\npackage p\n"));
        assert!(!check("package p\n\n\n\n\n// Code generated by x. DO NOT EDIT.\n"));
        // .NET tooling (resx designers, gRPC, EF migrations snapshots).
        assert!(check("//------------------------------------------------------------------------------\n// <auto-generated>\n//     This code was generated by a tool.\n"));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn test_language_from_extension() {
        assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("tsx"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("py"), Some(Language::Python));
        assert_eq!(Language::from_extension("c"), Some(Language::C));
        assert_eq!(Language::from_extension("h"), Some(Language::C));
        assert_eq!(Language::from_extension("go"), Some(Language::Go));
        assert_eq!(Language::from_extension("swift"), Some(Language::Swift));
        assert_eq!(Language::from_extension("kt"), Some(Language::Kotlin));
        assert_eq!(Language::from_extension("md"), Some(Language::Markdown));
        assert_eq!(Language::from_extension("rst"), Some(Language::Rst));
        assert_eq!(Language::from_extension("java"), None);

        // The rest of the dispatch tables a language has to appear in.
        assert_eq!(Language::from_str("kotlin"), Some(Language::Kotlin));
        assert_eq!(Language::from_str("kt"), Some(Language::Kotlin));
        assert_eq!(Language::Kotlin.extensions(), &["kt"]);
        assert_eq!(Language::Kotlin.source(), "code-kotlin");

        assert_eq!(Language::from_extension("cs"), Some(Language::CSharp));
        assert_eq!(Language::from_str("csharp"), Some(Language::CSharp));
        assert_eq!(Language::from_str("cs"), Some(Language::CSharp));
        assert_eq!(Language::from_str("c#"), Some(Language::CSharp));
        assert_eq!(Language::CSharp.extensions(), &["cs"]);
        assert_eq!(Language::CSharp.source(), "code-csharp");

        for ext in ["cpp", "cc", "cxx", "hpp", "hh", "hxx"] {
            assert_eq!(Language::from_extension(ext), Some(Language::Cpp), "{ext}");
        }
        assert_eq!(Language::from_str("cpp"), Some(Language::Cpp));
        assert_eq!(Language::from_str("c++"), Some(Language::Cpp));
        assert_eq!(Language::Cpp.source(), "code-cpp");

        assert_eq!(Language::from_extension("qml"), Some(Language::Qml));
        assert_eq!(Language::from_str("qml"), Some(Language::Qml));
        assert_eq!(Language::Qml.extensions(), &["qml"]);
        assert_eq!(Language::Qml.source(), "code-qml");
    }

    /// QML function and signal titles read back to their names; a signal
    /// handler's title does not, so nothing can call it by name.
    #[test]
    fn test_extract_code_name_qml() {
        let f = |t: &str| extract_code_name(t, "code_function");
        assert_eq!(f("root.reset(n)"), Some("reset".into()));
        assert_eq!(f("Main.helper()"), Some("helper".into()));
        assert_eq!(f("signal clicked(int x, string why)"), Some("clicked".into()));
        assert_eq!(f("root.onClicked: handler"), None);
        assert_eq!(f("root.Component.onCompleted: handler"), None);
    }

    /// QML calls resolve by id receiver (`txt.update()` to the `update` on the
    /// object with `id: txt`, not the one on `other`) and by file for bare calls.
    #[test]
    fn test_qml_call_edges() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("Main.qml");
        std::fs::write(
            &src,
            "import QtQuick\n\
             Item {\n\
                 id: root\n\
                 function reset() { txt.update(); helper() }\n\
                 function helper() {}\n\
                 Component.onCompleted: root.reset()\n\
                 Text { id: txt; function update() {} }\n\
                 Text { id: other; function update() {} }\n\
             }\n",
        )
        .unwrap();
        let db = db_at(dir.path());
        let result = import_code_files(&db, &[src.to_string_lossy().to_string()], None).unwrap();

        let conn = db.raw_conn().lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT s.title, t.title, JSON_EXTRACT(t.tags, '$.impl_for') FROM edges e \
                 JOIN nodes s ON s.id = e.source_id JOIN nodes t ON t.id = e.target_id \
                 WHERE e.type = 'calls' ORDER BY s.title, t.title",
            )
            .unwrap();
        let edges: Vec<(String, String, String)> =
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect();
        let e = |a: &str, b: &str, c: &str| (a.to_string(), b.to_string(), c.to_string());
        assert_eq!(
            edges,
            vec![
                e("root.Component.onCompleted: handler", "root.reset()", "root"),
                e("root.reset()", "root.helper()", "root"),
                e("root.reset()", "txt.update()", "txt"),
            ]
        );
        assert_eq!(result.call_edges_skipped, 0);
    }

    /// A `.h` file is C unless it looks like C++; its nodes carry the source
    /// of the language it was parsed as, file node and items alike, because
    /// call resolution only links within one source.
    #[test]
    fn test_header_source_follows_detected_language() {
        let dir = tempfile::tempdir().unwrap();
        let c_h = dir.path().join("pure.h");
        let cpp_h = dir.path().join("widget.h");
        std::fs::write(&c_h, "struct s { int a; };\n#define N 1\n").unwrap();
        std::fs::write(&cpp_h, "class Widget { void show() { draw(); } void draw() {} };\n").unwrap();
        assert_eq!(Language::from_path(&c_h), Some(Language::C));
        assert_eq!(Language::from_path(&cpp_h), Some(Language::Cpp));
        assert_eq!(Language::from_path(&dir.path().join("x.hpp")), Some(Language::Cpp));

        let db = db_at(dir.path());
        let files = vec![c_h.to_string_lossy().to_string(), cpp_h.to_string_lossy().to_string()];
        let result = import_code_files(&db, &files, None).unwrap();
        assert_eq!(result.call_edges, 1, "show -> draw resolves within code-cpp");

        let conn = db.raw_conn().lock().unwrap();
        let sources = |file: &str| -> Vec<String> {
            let mut stmt = conn
                .prepare("SELECT DISTINCT source FROM nodes WHERE JSON_EXTRACT(tags, '$.file_path') = ?1")
                .unwrap();
            stmt.query_map([file], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };
        assert_eq!(sources("pure.h"), vec!["code-c".to_string()]);
        assert_eq!(sources("widget.h"), vec!["code-cpp".to_string()]);
    }

    /// Every title shape the C++ parser writes for a function reads back to
    /// its bare name, whichever arm catches it.
    #[test]
    fn test_extract_code_name_cpp() {
        let f = |t: &str| extract_code_name(t, "code_function");
        assert_eq!(f("int add(int a, int b)"), Some("add".into()));
        assert_eq!(f("void Foo::bar(int a)"), Some("bar".into()));
        assert_eq!(f("void ns::Foo::baz() const"), Some("baz".into()));
        assert_eq!(f("QString Foo::name() const"), Some("name".into()));
        assert_eq!(f("std::vector<int> Foo::items() const"), Some("items".into()));
        assert_eq!(f("T Box<T>::get()"), Some("get".into()));
        assert_eq!(f("Foo* Foo::make()"), Some("make".into()));
        assert_eq!(f("Foo::Foo(int x)"), Some("Foo".into()));
        assert_eq!(f("Foo(int x)"), Some("Foo".into()));
        assert_eq!(f("explicit Foo(QObject *parent = nullptr)"), Some("Foo".into()));
        assert_eq!(f("int *ptr_fn()"), Some("ptr_fn".into()));
        assert_eq!(f("const std::string& ref_fn(const Foo& f)"), Some("ref_fn".into()));
        assert_eq!(f("static Foo* make()"), Some("make".into()));
        assert_eq!(f("virtual std::string name() const override"), Some("name".into()));
        assert_eq!(f("Q_INVOKABLE void doIt()"), Some("doIt".into()));
        assert_eq!(f("T max(T a, T b)"), Some("max".into()));
        // Destructors and operators are not called by name.
        assert_eq!(f("Foo::~Foo()"), None);
        assert_eq!(f("~Foo()"), None);
        assert_eq!(f("bool Foo::operator==(const Foo& o) const"), None);
        assert_eq!(f("bool operator==(const Foo& o) const"), None);
        assert_eq!(f("int Foo::operator()(int x)"), None);
    }

    /// C++ methods resolve through the shared receiver logic: `this->f()` to
    /// the caller's own class, `Class::f()` to that class, and out-of-line and
    /// pointer-returning titles are readable as call targets at all.
    #[test]
    fn test_cpp_call_edges_follow_receivers() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("widget.cpp");
        std::fs::write(
            &src,
            "void A::run() {}\n\
             void A::go() { this->run(); }\n\
             void B::run() {}\n\
             void B::go() { A::run(); helper(); }\n\
             static int *helper() { return 0; }\n",
        )
        .unwrap();
        let db = db_at(dir.path());
        let result = import_code_files(&db, &[src.to_string_lossy().to_string()], None).unwrap();

        let conn = db.raw_conn().lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT s.title, t.title FROM edges e JOIN nodes s ON s.id = e.source_id \
                 JOIN nodes t ON t.id = e.target_id WHERE e.type = 'calls' ORDER BY s.title, t.title",
            )
            .unwrap();
        let edges: Vec<(String, String)> =
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        assert_eq!(
            edges,
            vec![
                ("void A::go()".to_string(), "void A::run()".to_string()),
                ("void B::go()".to_string(), "static int *helper()".to_string()),
                ("void B::go()".to_string(), "void A::run()".to_string()),
            ]
        );
        assert_eq!(result.call_edges_skipped, 0);
    }

    #[test]
    fn test_extract_code_name_csharp() {
        let f = |t: &str| extract_code_name(t, "code_function");
        assert_eq!(f("public void Handle(Ping request)"), Some("Handle".into()));
        assert_eq!(f("void Helper()"), Some("Helper".into()));
        assert_eq!(f("public async Task<Unit> Handle(Ping request, CancellationToken ct)"), Some("Handle".into()));
        // A generic return type is not the name; a generic method's own type
        // parameters (with a space after the comma) are skipped.
        assert_eq!(f("public Task<IEnumerable<T>> Send<T>(IRequest<T> r)"), Some("Send".into()));
        assert_eq!(f("public static Dictionary<string, int> Build<TKey, TValue>(TKey k)"), Some("Build".into()));
        assert_eq!(f("internal static string? Describe(object o)"), Some("Describe".into()));
        assert_eq!(f("protected internal virtual int[] Items()"), Some("Items".into()));
        // Tuple types, bare or inside a generic (found on MediatR).
        assert_eq!(f("private IEnumerable<(Type ExceptionType, object Handler)> GetHandlers(Type t)"), Some("GetHandlers".into()));
        assert_eq!(f("public (int Count, string Name) Stats()"), Some("Stats".into()));
        // Constructor.
        assert_eq!(f("public Mediator(IServiceProvider serviceProvider)"), Some("Mediator".into()));
        assert_eq!(f("Mediator()"), Some("Mediator".into()));
        // Explicit interface implementation: the interface is not the name.
        assert_eq!(f("Task<Unit> IRequestHandler<Ping, Unit>.Handle(Ping request)"), Some("Handle".into()));
        // A title cut at 100 bytes keeps its name as long as `(` survived.
        assert_eq!(f("public static IServiceCollection AddMediatR(this IServiceCollection services, Action<Medi..."), Some("AddMediatR".into()));
        // Operators are not callable by name; a conversion operator must not
        // pose as its target type's constructor.
        assert_eq!(f("public static Unit operator +(Unit a, Unit b)"), None);
        assert_eq!(f("public static implicit operator Unit(int x)"), None);
        assert_eq!(f("~Mediator()"), None);
        // Properties carry calls only and are never call targets.
        assert_eq!(f("public int Count { get }"), None);
    }

    /// `resolve_call_edges` indexes definitions by the name it can read back
    /// out of a node *title*, which is the signature. A language with no arm
    /// here parses fine and then silently produces zero call edges.
    #[test]
    fn test_extract_code_name_kotlin() {
        let f = |t: &str| extract_code_name(t, "code_function");
        assert_eq!(f("fun create(key: String): AccountViewModel"), Some("create".into()));
        assert_eq!(f("suspend fun loadInstanceProfile()"), Some("loadInstanceProfile".into()));
        assert_eq!(f("@Composable fun MemoCard(memo: Memo)"), Some("MemoCard".into()));
        assert_eq!(f("private fun refresh()"), Some("refresh".into()));
        assert_eq!(f("fun <T> generic(x: T): T"), Some("generic".into()));
        // An extension receiver is not the function name.
        assert_eq!(f("fun Memo.describe(): String"), Some("describe".into()));
        assert_eq!(f("fun List<String>.joinAll(): String"), Some("joinAll".into()));
        assert_eq!(f("suspend fun CoroutineScope.ext()"), Some("ext".into()));

        assert_eq!(extract_code_name("enum class Kind", "code_enum"), Some("Kind".into()));
        assert_eq!(extract_code_name("interface MemoService", "code_trait"), Some("MemoService".into()));
        assert_eq!(extract_code_name("object Registry", "code_class"), Some("Registry".into()));
        assert_eq!(extract_code_name("data class Memo(val id: String)", "code_class"), Some("Memo".into()));
        assert_eq!(extract_code_name("const val TAG = \"x\"", "code_const"), Some("TAG".into()));
        assert_eq!(extract_code_name("private val cache = mutableListOf()", "code_const"), Some("cache".into()));
        assert_eq!(extract_code_name("var cursor: Int = 0", "code_var"), Some("cursor".into()));
        assert_eq!(extract_code_name("typealias MemoList = List<Memo>", "code_type"), Some("MemoList".into()));
    }

    /// The Kotlin arms above are additive: every other language reads the same.
    #[test]
    fn test_extract_code_name_other_languages_unchanged() {
        assert_eq!(extract_code_name("pub async fn run(x: u8)", "code_function"), Some("run".into()));
        assert_eq!(extract_code_name("async def handler(req)", "code_function"), Some("handler".into()));
        assert_eq!(extract_code_name("func (s *Server) Serve()", "code_function"), Some("Serve".into()));
        assert_eq!(extract_code_name("void free_all(void *p)", "code_function"), Some("free_all".into()));
        assert_eq!(extract_code_name("pub enum Kind", "code_enum"), Some("Kind".into()));
        assert_eq!(extract_code_name("pub trait Store", "code_trait"), Some("Store".into()));
        assert_eq!(extract_code_name("class Widget(Base)", "code_class"), Some("Widget".into()));
        assert_eq!(extract_code_name("const MaxRetries = 3", "code_const"), Some("MaxRetries".into()));
        assert_eq!(extract_code_name("type Handler func(int)", "code_type"), Some("Handler".into()));
        // Titles no arm read before the C# one, which must stay unread.
        assert_eq!(extract_code_name("function lsGet<T>(key: string, fallback: T): T", "code_function"), None);
        assert_eq!(extract_code_name("export async function load(id: string)", "code_function"), None);
        assert_eq!(extract_code_name("init(id: String)", "code_function"), None);
        assert_eq!(extract_code_name("public convenience init(name: String)", "code_function"), None);
        assert_eq!(extract_code_name("const resize = () => ...", "code_function"), None);
    }

    #[test]
    fn test_extract_rst_title() {
        // Simple underline style
        assert_eq!(
            extract_rst_title("My Title\n========\n\nContent here"),
            Some("My Title".to_string())
        );

        // Dashes underline
        assert_eq!(
            extract_rst_title("Another Title\n-------------\n"),
            Some("Another Title".to_string())
        );

        // Overline + underline style
        assert_eq!(
            extract_rst_title("==========\nBig Title\n==========\n"),
            Some("Big Title".to_string())
        );

        // No title
        assert_eq!(extract_rst_title("Just some text\nwithout underlines"), None);

        // Skip directives
        assert_eq!(
            extract_rst_title(".. note::\n\nReal Title\n==========\n"),
            Some("Real Title".to_string())
        );
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
