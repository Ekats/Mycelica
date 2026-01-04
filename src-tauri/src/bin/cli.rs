//! Mycelica CLI - Full command-line interface for knowledge graph operations
//!
//! Usage: mycelica-cli [OPTIONS] <COMMAND>
//!
//! A first-class CLI for power users. Supports JSON output for scripting.

use clap::{Parser, Subcommand, CommandFactory};
use clap_complete::{generate, Shell};
use mycelica_lib::{db::{Database, Node, NodeType}, settings, import, hierarchy, similarity, clustering, openaire};
use std::path::PathBuf;
use std::sync::Arc;
use std::io::Write;
use chrono::Utc;

// ============================================================================
// Main CLI Structure
// ============================================================================

#[derive(Parser)]
#[command(name = "mycelica-cli")]
#[command(version, about = "Mycelica knowledge graph CLI", long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Database path (default: auto-detect)
    #[arg(long, global = true)]
    db: Option<String>,

    /// Output as JSON for scripting
    #[arg(long, global = true)]
    json: bool,

    /// Suppress progress output
    #[arg(long, short, global = true)]
    quiet: bool,

    /// Detailed logging
    #[arg(long, short, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Database operations
    Db {
        #[command(subcommand)]
        cmd: DbCommands,
    },
    /// Import data from various sources
    Import {
        #[command(subcommand)]
        cmd: ImportCommands,
    },
    /// Node operations
    Node {
        #[command(subcommand)]
        cmd: NodeCommands,
    },
    /// Hierarchy operations
    Hierarchy {
        #[command(subcommand)]
        cmd: HierarchyCommands,
    },
    /// AI processing operations
    Process {
        #[command(subcommand)]
        cmd: ProcessCommands,
    },
    /// Clustering operations
    Cluster {
        #[command(subcommand)]
        cmd: ClusterCommands,
    },
    /// Embedding operations
    Embeddings {
        #[command(subcommand)]
        cmd: EmbeddingsCommands,
    },
    /// Privacy analysis and export
    Privacy {
        #[command(subcommand)]
        cmd: PrivacyCommands,
    },
    /// Paper/document operations
    Paper {
        #[command(subcommand)]
        cmd: PaperCommands,
    },
    /// Configuration settings
    Config {
        #[command(subcommand)]
        cmd: ConfigCommands,
    },
    /// Recent nodes
    Recent {
        #[command(subcommand)]
        cmd: RecentCommands,
    },
    /// Pinned nodes
    Pinned {
        #[command(subcommand)]
        cmd: PinnedCommands,
    },
    /// Graph navigation (non-interactive)
    Nav {
        #[command(subcommand)]
        cmd: NavCommands,
    },
    /// Interactive TUI mode
    Tui,
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}

// ============================================================================
// Subcommand Enums
// ============================================================================

#[derive(Subcommand)]
enum DbCommands {
    /// Show database statistics
    Stats,
    /// Print database path
    Path,
    /// Select database interactively
    Select,
    /// Export trimmed database
    Export {
        /// Output path for exported database
        path: String,
    },
    /// Tidy database (vacuum, fix counts, prune edges)
    Tidy,
}

#[derive(Subcommand)]
enum ImportCommands {
    /// Import papers from OpenAIRE
    Openaire {
        /// Search query (required)
        #[arg(long, short)]
        query: String,
        /// Country filter (ISO code: EE, US, etc)
        #[arg(long, short)]
        country: Option<String>,
        /// Field of science filter
        #[arg(long)]
        fos: Option<String>,
        /// From publication year
        #[arg(long)]
        from_year: Option<String>,
        /// To publication year
        #[arg(long)]
        to_year: Option<String>,
        /// Maximum papers to import
        #[arg(long, short, default_value = "100")]
        max: u32,
        /// Download PDFs
        #[arg(long)]
        download_pdfs: bool,
        /// Maximum PDF size in MB
        #[arg(long, default_value = "20")]
        max_pdf_size: u32,
    },
    /// Import markdown files
    Markdown {
        /// Path to markdown file or directory
        path: String,
    },
    /// Import Claude conversation export (JSON)
    Claude {
        /// Path to Claude export JSON file
        path: String,
    },
    /// Import Google Keep (from Takeout ZIP)
    Keep {
        /// Path to Google Takeout ZIP file
        path: String,
    },
}

#[derive(Subcommand)]
enum NodeCommands {
    /// List nodes
    List {
        /// Filter by type: item or category
        #[arg(long, short = 't')]
        node_type: Option<String>,
        /// Maximum results
        #[arg(long, short, default_value = "50")]
        limit: usize,
        /// Only processed nodes
        #[arg(long)]
        processed: bool,
        /// Only unprocessed nodes
        #[arg(long)]
        unprocessed: bool,
    },
    /// Get a single node by ID
    Get {
        /// Node ID
        id: String,
    },
    /// Search nodes (full-text)
    Search {
        /// Search query
        query: String,
        /// Maximum results
        #[arg(long, short, default_value = "20")]
        limit: usize,
    },
    /// Find semantically similar nodes
    Similar {
        /// Node ID to find similar nodes for
        id: String,
        /// Number of results
        #[arg(long, short, default_value = "10")]
        top: usize,
        /// Minimum similarity threshold (0.0-1.0)
        #[arg(long, short = 'm', default_value = "0.6")]
        threshold: f32,
    },
    /// Create a new node
    Create {
        /// Node title
        #[arg(long, short)]
        title: String,
        /// Node content
        #[arg(long, short)]
        content: Option<String>,
        /// Node type: page, thought, context
        #[arg(long, short = 't', default_value = "thought")]
        node_type: String,
    },
    /// Delete a node
    Delete {
        /// Node ID to delete
        id: String,
    },
}

#[derive(Subcommand)]
enum HierarchyCommands {
    /// Build hierarchy (initial)
    Build,
    /// Rebuild hierarchy from scratch
    Rebuild,
    /// Rebuild without AI (keyword-based)
    RebuildLite,
    /// Flatten single-child chains
    Flatten,
    /// Show hierarchy statistics
    Stats,
}

#[derive(Subcommand)]
enum ProcessCommands {
    /// Process unprocessed nodes with AI
    Run {
        /// Maximum nodes to process
        #[arg(long, short)]
        limit: Option<usize>,
        /// Model to use: haiku or sonnet
        #[arg(long, short, default_value = "haiku")]
        model: String,
    },
    /// Show processing status
    Status,
    /// Reset all AI processing flags
    Reset,
}

#[derive(Subcommand)]
enum ClusterCommands {
    /// Run clustering on new items
    Run,
    /// Recluster all items
    All,
    /// Reset clustering data
    Reset,
    /// Get or set clustering thresholds
    Thresholds {
        /// Primary threshold (0.0-1.0)
        #[arg(long)]
        primary: Option<f32>,
        /// Secondary threshold (0.0-1.0)
        #[arg(long)]
        secondary: Option<f32>,
    },
}

#[derive(Subcommand)]
enum EmbeddingsCommands {
    /// Show embedding statistics
    Status,
    /// Regenerate all embeddings
    Regenerate,
    /// Clear all embeddings
    Clear,
    /// Toggle local embeddings (on/off)
    Local {
        /// Enable or disable local embeddings
        #[arg(value_parser = ["on", "off"])]
        state: Option<String>,
    },
}

#[derive(Subcommand)]
enum PrivacyCommands {
    /// Scan all nodes for privacy
    Scan,
    /// Score privacy for all items (continuous 0-1)
    ScanItems {
        /// Force rescore all items
        #[arg(long)]
        force: bool,
    },
    /// Show privacy statistics
    Stats,
    /// Reset all privacy flags
    Reset,
    /// Export shareable database
    Export {
        /// Output path
        path: String,
        /// Minimum privacy threshold (0-100)
        #[arg(long, short, default_value = "50")]
        threshold: u32,
    },
    /// Set node privacy level
    Set {
        /// Node ID
        id: String,
        /// Privacy level: public, private, sensitive
        #[arg(value_parser = ["public", "private", "sensitive"])]
        level: String,
    },
}

#[derive(Subcommand)]
enum PaperCommands {
    /// List imported papers
    List {
        /// Maximum results
        #[arg(long, short, default_value = "50")]
        limit: usize,
    },
    /// Get paper metadata
    Get {
        /// Node ID of paper
        id: String,
    },
    /// Download paper PDF on demand
    Download {
        /// Node ID of paper
        id: String,
    },
    /// Open paper in external viewer
    Open {
        /// Node ID of paper
        id: String,
    },
    /// Sync PDF availability status
    SyncPdfs,
    /// Reformat all paper abstracts
    ReformatAbstracts,
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// List all settings
    List,
    /// Get a setting value
    Get {
        /// Setting key
        key: String,
    },
    /// Set a setting value
    Set {
        /// Setting key
        key: String,
        /// Setting value
        value: String,
    },
}

#[derive(Subcommand)]
enum RecentCommands {
    /// List recent nodes
    List {
        /// Maximum results
        #[arg(long, short, default_value = "20")]
        limit: usize,
    },
    /// Clear recent history
    Clear,
}

#[derive(Subcommand)]
enum PinnedCommands {
    /// List pinned nodes
    List,
    /// Pin a node
    Add {
        /// Node ID to pin
        id: String,
    },
    /// Unpin a node
    Remove {
        /// Node ID to unpin
        id: String,
    },
}

#[derive(Subcommand)]
enum NavCommands {
    /// List children of a node
    Ls {
        /// Node ID (use "root" for Universe)
        id: String,
        /// Long format with details
        #[arg(long, short)]
        long: bool,
    },
    /// Show subtree
    Tree {
        /// Node ID (use "root" for Universe)
        id: String,
        /// Maximum depth
        #[arg(long, short, default_value = "3")]
        depth: usize,
    },
    /// Find path between nodes
    Path {
        /// Source node ID
        from: String,
        /// Target node ID
        to: String,
    },
    /// Show edges for a node
    Edges {
        /// Node ID
        id: String,
    },
    /// Find similar nodes by embedding
    Similar {
        /// Node ID
        id: String,
        /// Number of results
        #[arg(long, short, default_value = "10")]
        top: usize,
    },
}

// ============================================================================
// Main Entry Point
// ============================================================================

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(e) = run_cli(cli).await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

async fn run_cli(cli: Cli) -> Result<(), String> {
    // Initialize settings first (needed for custom db path)
    let app_data_dir = dirs::data_dir()
        .map(|p| p.join("com.mycelica.app"))
        .unwrap_or_else(|| PathBuf::from("."));
    settings::init(app_data_dir);

    // Handle completions first (no DB needed)
    if let Commands::Completions { shell } = &cli.command {
        generate(*shell, &mut Cli::command(), "mycelica-cli", &mut std::io::stdout());
        return Ok(());
    }

    // Find database
    let db_path = cli.db.map(PathBuf::from).unwrap_or_else(find_database);

    if cli.verbose {
        eprintln!("[verbose] Using database: {:?}", db_path);
    }

    let db = Arc::new(Database::new(&db_path).map_err(|e| format!("Failed to open database: {}", e))?);


    match cli.command {
        Commands::Db { cmd } => handle_db(cmd, &db, cli.json).await,
        Commands::Import { cmd } => handle_import(cmd, &db, cli.json, cli.quiet).await,
        Commands::Node { cmd } => handle_node(cmd, &db, cli.json).await,
        Commands::Hierarchy { cmd } => handle_hierarchy(cmd, &db, cli.json, cli.quiet).await,
        Commands::Process { cmd } => handle_process(cmd, &db, cli.json, cli.quiet).await,
        Commands::Cluster { cmd } => handle_cluster(cmd, &db, cli.json, cli.quiet).await,
        Commands::Embeddings { cmd } => handle_embeddings(cmd, &db, cli.json).await,
        Commands::Privacy { cmd } => handle_privacy(cmd, &db, cli.json, cli.quiet).await,
        Commands::Paper { cmd } => handle_paper(cmd, &db, cli.json).await,
        Commands::Config { cmd } => handle_config(cmd, cli.json),
        Commands::Recent { cmd } => handle_recent(cmd, &db, cli.json),
        Commands::Pinned { cmd } => handle_pinned(cmd, &db, cli.json),
        Commands::Nav { cmd } => handle_nav(cmd, &db, cli.json).await,
        Commands::Tui => run_tui(&db).await,
        Commands::Completions { .. } => unreachable!(),
    }
}

// ============================================================================
// Database Commands
// ============================================================================

async fn handle_db(cmd: DbCommands, db: &Database, json: bool) -> Result<(), String> {
    match cmd {
        DbCommands::Stats => {
            // Get items and count stats from them directly (most accurate)
            let items = db.get_items().map_err(|e| e.to_string())?;
            let edges = db.get_all_edges().map_err(|e| e.to_string())?;

            let total_items = items.len();
            let processed_items = items.iter().filter(|n| n.is_processed).count();

            // Count items with embeddings by checking each item
            let items_with_embeddings = {
                let mut count = 0;
                for item in &items {
                    if db.get_node_embedding(&item.id).ok().flatten().is_some() {
                        count += 1;
                    }
                }
                count
            };

            // Get hierarchy stats
            let universe = db.get_universe().map_err(|e| e.to_string())?;
            let max_depth = db.get_max_depth().map_err(|e| e.to_string())?;

            // Count categories (non-items)
            let mut categories = 0;
            for depth in 0..=max_depth {
                let nodes_at_depth = db.get_nodes_at_depth(depth).map_err(|e| e.to_string())?;
                categories += nodes_at_depth.iter().filter(|n| !n.is_item).count();
            }

            if json {
                println!(r#"{{"path":"{}","items":{},"categories":{},"edges":{},"processed":{},"embeddings":{}}}"#,
                    db.get_path(), total_items, categories, edges.len(), processed_items, items_with_embeddings);
            } else {
                println!("Database:   {}", db.get_path());
                println!("Items:      {:>6}", total_items);
                println!("Categories: {:>6}", categories);
                println!("Edges:      {:>6}", edges.len());
                println!("Processed:  {:>6} / {}", processed_items, total_items);
                println!("Embeddings: {:>6} / {}", items_with_embeddings, total_items);
                if let Some(u) = universe {
                    println!("Hierarchy:  {} levels, root=\"{}\"", max_depth, u.title);
                }
            }
        }
        DbCommands::Path => {
            let path = db.get_path();
            if json {
                println!(r#"{{"path":"{}"}}"#, path);
            } else {
                println!("{}", path);
            }
        }
        DbCommands::Select => {
            // Find databases in common locations
            let mut databases: Vec<PathBuf> = Vec::new();

            // Check repo directory and parent
            let cwd = std::env::current_dir().unwrap_or_default();
            for dir in [&cwd, &cwd.parent().unwrap_or(&cwd).to_path_buf()] {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map(|e| e == "db").unwrap_or(false)
                           && path.file_name().map(|n| n.to_string_lossy().contains("mycelica")).unwrap_or(false) {
                            databases.push(path);
                        }
                    }
                }
            }

            // Check app data directory
            let app_data = dirs::data_dir()
                .map(|p| p.join("com.mycelica.app"))
                .unwrap_or_default();
            if let Ok(entries) = std::fs::read_dir(&app_data) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map(|e| e == "db").unwrap_or(false) {
                        databases.push(path);
                    }
                }
            }

            // Check Downloads
            if let Some(downloads) = dirs::download_dir() {
                if let Ok(entries) = std::fs::read_dir(&downloads) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map(|e| e == "db").unwrap_or(false)
                           && path.file_name().map(|n| n.to_string_lossy().contains("mycelica")).unwrap_or(false) {
                            databases.push(path);
                        }
                    }
                }
            }

            // Deduplicate and sort
            databases.sort();
            databases.dedup();

            if databases.is_empty() {
                println!("No databases found.");
                return Ok(());
            }

            // Get current selection
            let current = settings::get_custom_db_path();

            // Display options
            println!("Available databases:\n");
            let home = std::env::var("HOME").unwrap_or_default();
            for (i, path) in databases.iter().enumerate() {
                let display = path.to_string_lossy();
                let display = if !home.is_empty() && display.starts_with(&home) {
                    display.replacen(&home, "~", 1)
                } else {
                    display.to_string()
                };
                let marker = if current.as_ref() == Some(&path.to_string_lossy().to_string()) { " *" } else { "" };
                println!("  [{}] {}{}", i + 1, display, marker);
            }
            println!("\n  [0] Use default (auto-detect)");

            // Read selection
            print!("\nSelect database (0-{}): ", databases.len());
            std::io::stdout().flush().ok();

            let mut input = String::new();
            std::io::stdin().read_line(&mut input).map_err(|e| e.to_string())?;

            let choice: usize = input.trim().parse().unwrap_or(999);

            if choice == 0 {
                settings::set_custom_db_path(None).map_err(|e| e.to_string())?;
                println!("Reset to default database.");
            } else if choice <= databases.len() {
                let selected = &databases[choice - 1];
                settings::set_custom_db_path(Some(selected.to_string_lossy().to_string()))
                    .map_err(|e| e.to_string())?;
                println!("Selected: {}", selected.display());
            } else {
                println!("Invalid selection.");
            }
        }
        DbCommands::Export { path } => {
            // Copy database
            let src = db.get_path();
            std::fs::copy(&src, &path).map_err(|e| format!("Failed to copy database: {}", e))?;

            if json {
                println!(r#"{{"exported":"{}"}}"#, path);
            } else {
                println!("Exported to: {}", path);
            }
        }
        DbCommands::Tidy => {
            eprintln!("Tidying database...");
            db.fix_all_child_counts().map_err(|e| e.to_string())?;
            db.prune_dead_edges().map_err(|e| e.to_string())?;

            if json {
                println!(r#"{{"status":"ok"}}"#);
            } else {
                println!("Database tidied successfully");
            }
        }
    }
    Ok(())
}

// ============================================================================
// Import Commands
// ============================================================================

async fn handle_import(cmd: ImportCommands, db: &Database, json: bool, quiet: bool) -> Result<(), String> {
    match cmd {
        ImportCommands::Openaire { query, country, fos, from_year, to_year, max, download_pdfs, max_pdf_size } => {
            let api_key = settings::get_openaire_api_key();

            if !quiet {
                eprintln!("[OpenAIRE] Searching: \"{}\"", query);
                if let Some(ref c) = country {
                    eprintln!("[OpenAIRE]   Country: {}", c);
                }
            }

            let on_progress = |current: usize, total: usize| {
                if !quiet && current % 10 == 0 {
                    eprint!("\r[OpenAIRE] Progress: {}/{}", current, total);
                    std::io::stderr().flush().ok();
                }
            };

            let result = import::import_openaire_papers(
                db,
                query,
                country,
                fos,
                from_year,
                to_year,
                max,
                download_pdfs,
                max_pdf_size,
                api_key,
                on_progress,
            ).await?;

            if !quiet {
                eprintln!();
            }

            if json {
                println!(r#"{{"papers_imported":{},"pdfs_downloaded":{},"duplicates_skipped":{},"errors":{}}}"#,
                    result.papers_imported, result.pdfs_downloaded, result.duplicates_skipped, result.errors.len());
            } else {
                println!("Imported {} papers, {} PDFs, {} duplicates skipped",
                    result.papers_imported, result.pdfs_downloaded, result.duplicates_skipped);
                if !result.errors.is_empty() {
                    println!("Errors: {}", result.errors.len());
                    for (i, err) in result.errors.iter().take(5).enumerate() {
                        println!("  {}. {}", i + 1, err);
                    }
                }
            }
        }
        ImportCommands::Markdown { path } => {
            let files = if std::path::Path::new(&path).is_dir() {
                std::fs::read_dir(&path)
                    .map_err(|e| format!("Failed to read directory: {}", e))?
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
                    .map(|e| e.path().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
            } else {
                vec![path]
            };

            let result = import::import_markdown_files(db, &files)?;

            if json {
                println!(r#"{{"conversations_imported":{},"exchanges_imported":{},"skipped":{},"errors":{}}}"#,
                    result.conversations_imported, result.exchanges_imported, result.skipped, result.errors.len());
            } else {
                println!("Imported {} files, {} items", result.conversations_imported, result.exchanges_imported);
            }
        }
        ImportCommands::Claude { path } => {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read file: {}", e))?;

            let result = import::import_claude_conversations(db, &content)?;

            if json {
                println!(r#"{{"conversations_imported":{},"exchanges_imported":{},"skipped":{},"errors":{}}}"#,
                    result.conversations_imported, result.exchanges_imported, result.skipped, result.errors.len());
            } else {
                println!("Imported {} conversations, {} exchanges",
                    result.conversations_imported, result.exchanges_imported);
            }
        }
        ImportCommands::Keep { path } => {
            let result = import::import_google_keep(db, &path)?;

            if json {
                println!(r#"{{"notes_imported":{},"skipped":{},"errors":{}}}"#,
                    result.notes_imported, result.skipped, result.errors.len());
            } else {
                println!("Imported {} notes, {} skipped", result.notes_imported, result.skipped);
            }
        }
    }
    Ok(())
}

// ============================================================================
// Node Commands
// ============================================================================

async fn handle_node(cmd: NodeCommands, db: &Database, json: bool) -> Result<(), String> {
    match cmd {
        NodeCommands::List { node_type, limit, processed, unprocessed } => {
            let nodes = if node_type.as_deref() == Some("item") {
                db.get_items().map_err(|e| e.to_string())?
            } else {
                db.get_all_nodes().map_err(|e| e.to_string())?
            };

            let filtered: Vec<_> = nodes.into_iter()
                .filter(|n| {
                    if processed && !n.is_processed { return false; }
                    if unprocessed && n.is_processed { return false; }
                    if node_type.as_deref() == Some("category") && n.is_item { return false; }
                    true
                })
                .take(limit)
                .collect();

            if json {
                let items: Vec<String> = filtered.iter().map(|n| {
                    format!(r#"{{"id":"{}","title":"{}","is_item":{},"is_processed":{},"depth":{}}}"#,
                        n.id, escape_json(&n.title), n.is_item, n.is_processed, n.depth)
                }).collect();
                println!("[{}]", items.join(","));
            } else {
                for node in &filtered {
                    let marker = if node.is_item { "📄" } else { "📁" };
                    let processed_marker = if node.is_processed { "✓" } else { "○" };
                    println!("{} {} {} {}", marker, processed_marker, &node.id[..8], node.title);
                }
                println!("\n{} nodes", filtered.len());
            }
        }
        NodeCommands::Get { id } => {
            let node = db.get_node(&id).map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Node not found: {}", id))?;

            if json {
                println!(r#"{{"id":"{}","title":"{}","content":{},"is_item":{},"is_processed":{},"depth":{},"child_count":{},"parent_id":{}}}"#,
                    node.id,
                    escape_json(&node.title),
                    node.content.as_ref().map(|c| format!("\"{}\"", escape_json(c))).unwrap_or("null".to_string()),
                    node.is_item,
                    node.is_processed,
                    node.depth,
                    node.child_count,
                    node.parent_id.as_ref().map(|p| format!("\"{}\"", p)).unwrap_or("null".to_string())
                );
            } else {
                println!("ID:       {}", node.id);
                println!("Title:    {}", node.title);
                println!("Type:     {}", if node.is_item { "Item" } else { "Category" });
                println!("Depth:    {}", node.depth);
                println!("Children: {}", node.child_count);
                if let Some(ref parent) = node.parent_id {
                    println!("Parent:   {}", parent);
                }
                if let Some(ref summary) = node.summary {
                    println!("\nSummary:\n{}", summary);
                }
                if let Some(ref content) = node.content {
                    let preview = if content.len() > 500 { &content[..500] } else { content };
                    println!("\nContent:\n{}", preview);
                }
            }
        }
        NodeCommands::Search { query, limit } => {
            let results = db.search_nodes(&query).map_err(|e| e.to_string())?;
            let limited: Vec<_> = results.into_iter().take(limit).collect();

            if json {
                let items: Vec<String> = limited.iter().map(|n| {
                    format!(r#"{{"id":"{}","title":"{}","is_item":{}}}"#,
                        n.id, escape_json(&n.title), n.is_item)
                }).collect();
                println!("[{}]", items.join(","));
            } else {
                for node in &limited {
                    let marker = if node.is_item { "📄" } else { "📁" };
                    println!("{} {} {}", marker, &node.id[..8], node.title);
                }
                println!("\n{} results", limited.len());
            }
        }
        NodeCommands::Similar { id, top, threshold } => {
            // Get target embedding
            let target_emb = db.get_node_embedding(&id).map_err(|e| e.to_string())?
                .ok_or_else(|| format!("No embedding for node: {}", id))?;

            // Get all embeddings
            let all_embeddings = db.get_nodes_with_embeddings().map_err(|e| e.to_string())?;

            // Find similar
            let similar = similarity::find_similar(&target_emb, &all_embeddings, &id, top, threshold);

            if json {
                let items: Vec<String> = similar.iter().map(|(node_id, score)| {
                    format!(r#"{{"id":"{}","similarity":{:.3}}}"#, node_id, score)
                }).collect();
                println!("[{}]", items.join(","));
            } else {
                for (node_id, score) in &similar {
                    if let Ok(Some(node)) = db.get_node(node_id) {
                        println!("{:.0}% {} {}", score * 100.0, &node_id[..8], node.title);
                    }
                }
            }
        }
        NodeCommands::Create { title, content, node_type } => {
            let id = uuid::Uuid::new_v4().to_string();
            let nt = match node_type.as_str() {
                "page" => NodeType::Page,
                "context" => NodeType::Context,
                _ => NodeType::Thought,
            };

            let now = Utc::now().timestamp_millis();
            let node = Node {
                id: id.clone(),
                node_type: nt,
                title: title.clone(),
                url: None,
                content,
                position: mycelica_lib::db::Position { x: 0.0, y: 0.0 },
                created_at: now,
                updated_at: now,
                cluster_id: None,
                cluster_label: None,
                depth: 0,
                is_item: true,
                is_universe: false,
                parent_id: None,
                child_count: 0,
                ai_title: None,
                summary: None,
                tags: None,
                emoji: None,
                is_processed: false,
                conversation_id: None,
                sequence_index: None,
                is_pinned: false,
                last_accessed_at: None,
                latest_child_date: None,
                is_private: None,
                privacy_reason: None,
                privacy: None,
                source: Some("cli".to_string()),
                pdf_available: None,
                content_type: None,
                associated_idea_id: None,
            };

            db.insert_node(&node).map_err(|e| e.to_string())?;

            if json {
                println!(r#"{{"id":"{}","title":"{}"}}"#, id, escape_json(&title));
            } else {
                println!("Created node: {}", id);
            }
        }
        NodeCommands::Delete { id } => {
            db.delete_node(&id).map_err(|e| e.to_string())?;

            if json {
                println!(r#"{{"deleted":"{}"}}"#, id);
            } else {
                println!("Deleted node: {}", id);
            }
        }
    }
    Ok(())
}

// ============================================================================
// Hierarchy Commands
// ============================================================================

async fn handle_hierarchy(cmd: HierarchyCommands, db: &Database, json: bool, quiet: bool) -> Result<(), String> {
    match cmd {
        HierarchyCommands::Build => {
            if !quiet { eprintln!("Building hierarchy..."); }
            hierarchy::build_hierarchy(db)?;
            if json {
                println!(r#"{{"status":"ok"}}"#);
            } else {
                println!("Hierarchy built successfully");
            }
        }
        HierarchyCommands::Rebuild => {
            if !quiet { eprintln!("Rebuilding hierarchy (this may take a while)..."); }
            db.delete_hierarchy_nodes().map_err(|e| e.to_string())?;
            hierarchy::build_hierarchy(db)?;
            if json {
                println!(r#"{{"status":"ok"}}"#);
            } else {
                println!("Hierarchy rebuilt successfully");
            }
        }
        HierarchyCommands::RebuildLite => {
            if !quiet { eprintln!("Rebuilding hierarchy (lite, no AI)..."); }
            // Use clustering with lite mode
            let result = clustering::cluster_with_embeddings_lite(db).await?;
            if json {
                println!(r#"{{"clusters_created":{},"items_assigned":{}}}"#,
                    result.clusters_created, result.items_assigned);
            } else {
                println!("Created {} clusters, assigned {} items",
                    result.clusters_created, result.items_assigned);
            }
        }
        HierarchyCommands::Flatten => {
            if !quiet { eprintln!("Flattening single-child chains..."); }
            db.flatten_single_child_chains().map_err(|e| e.to_string())?;
            if json {
                println!(r#"{{"status":"ok"}}"#);
            } else {
                println!("Flattened successfully");
            }
        }
        HierarchyCommands::Stats => {
            let max_depth = db.get_max_depth().map_err(|e| e.to_string())?;
            let universe = db.get_universe().map_err(|e| e.to_string())?;

            if json {
                println!(r#"{{"max_depth":{},"has_universe":{}}}"#, max_depth, universe.is_some());
            } else {
                println!("Max depth: {}", max_depth);
                if let Some(u) = universe {
                    println!("Universe:  {} ({} children)", u.title, u.child_count);
                } else {
                    println!("Universe:  None (run 'hierarchy build')");
                }

                // Show counts per depth
                for d in 0..=max_depth {
                    if let Ok(nodes) = db.get_nodes_at_depth(d) {
                        println!("  Depth {}: {} nodes", d, nodes.len());
                    }
                }
            }
        }
    }
    Ok(())
}

// ============================================================================
// Process Commands
// ============================================================================

async fn handle_process(cmd: ProcessCommands, db: &Database, json: bool, quiet: bool) -> Result<(), String> {
    match cmd {
        ProcessCommands::Run { limit, model: _ } => {
            let unprocessed = db.get_unprocessed_nodes().map_err(|e| e.to_string())?;
            let to_process: Vec<_> = if let Some(l) = limit {
                unprocessed.into_iter().take(l).collect()
            } else {
                unprocessed
            };

            if to_process.is_empty() {
                if json {
                    println!(r#"{{"processed":0,"message":"No unprocessed nodes"}}"#);
                } else {
                    println!("No unprocessed nodes");
                }
                return Ok(());
            }

            if !quiet {
                eprintln!("Processing {} nodes...", to_process.len());
            }

            // Process nodes one by one
            let mut processed_count = 0;
            for (i, node) in to_process.iter().enumerate() {
                if !quiet && i % 10 == 0 {
                    eprint!("\r[{}/{}] Processing...", i, to_process.len());
                    std::io::stderr().flush().ok();
                }

                // Use AI to analyze node
                let content = node.content.as_deref().unwrap_or(&node.title);
                match mycelica_lib::ai_client::analyze_node(&node.title, content).await {
                    Ok(result) => {
                        db.update_node_ai(
                            &node.id,
                            &result.title,
                            &result.summary,
                            &result.tags.join(","),
                            &result.content_type,
                        ).map_err(|e| e.to_string())?;
                        processed_count += 1;
                    }
                    Err(e) => {
                        if !quiet {
                            eprintln!("\nError processing {}: {}", &node.id[..8], e);
                        }
                    }
                }
            }

            if !quiet { eprintln!(); }

            if json {
                println!(r#"{{"processed":{}}}"#, processed_count);
            } else {
                println!("Processed {} nodes", processed_count);
            }
        }
        ProcessCommands::Status => {
            let unprocessed = db.get_unprocessed_nodes().map_err(|e| e.to_string())?;
            let stats = db.get_stats().map_err(|e| e.to_string())?;
            let items = stats.1;

            if json {
                println!(r#"{{"unprocessed":{},"total":{}}}"#, unprocessed.len(), items);
            } else {
                println!("Unprocessed: {} / {} items", unprocessed.len(), items);
            }
        }
        ProcessCommands::Reset => {
            db.reset_ai_processing().map_err(|e| e.to_string())?;
            if json {
                println!(r#"{{"status":"ok"}}"#);
            } else {
                println!("AI processing reset");
            }
        }
    }
    Ok(())
}

// ============================================================================
// Cluster Commands
// ============================================================================

async fn handle_cluster(cmd: ClusterCommands, db: &Database, json: bool, quiet: bool) -> Result<(), String> {
    match cmd {
        ClusterCommands::Run => {
            if !quiet { eprintln!("Running clustering..."); }
            let result = clustering::run_clustering(db, true).await?;
            if json {
                println!(r#"{{"items_processed":{},"clusters_created":{},"items_assigned":{}}}"#,
                    result.items_processed, result.clusters_created, result.items_assigned);
            } else {
                println!("Processed {} items, created {} clusters, assigned {} items",
                    result.items_processed, result.clusters_created, result.items_assigned);
            }
        }
        ClusterCommands::All => {
            if !quiet { eprintln!("Reclustering all items..."); }
            let result = clustering::recluster_all(db, true).await?;
            if json {
                println!(r#"{{"items_processed":{},"clusters_created":{},"items_assigned":{}}}"#,
                    result.items_processed, result.clusters_created, result.items_assigned);
            } else {
                println!("Processed {} items, created {} clusters, assigned {} items",
                    result.items_processed, result.clusters_created, result.items_assigned);
            }
        }
        ClusterCommands::Reset => {
            // Reset clustering by clearing cluster assignments
            // Note: update_node_cluster requires an ID, so we'll just report status
            let items = db.get_items().map_err(|e| e.to_string())?;
            let clustered: Vec<_> = items.iter().filter(|n| n.cluster_id.is_some()).collect();
            if json {
                println!(r#"{{"status":"info","clustered_items":{},"message":"Use GUI to reset clustering"}}"#, clustered.len());
            } else {
                println!("{} items have cluster assignments", clustered.len());
                println!("Use 'hierarchy rebuild' to recluster from scratch");
            }
        }
        ClusterCommands::Thresholds { primary, secondary } => {
            if primary.is_some() || secondary.is_some() {
                settings::set_clustering_thresholds(primary, secondary)?;
                if json {
                    println!(r#"{{"status":"ok"}}"#);
                } else {
                    println!("Thresholds updated");
                }
            } else {
                let (p, s) = settings::get_clustering_thresholds();
                if json {
                    println!(r#"{{"primary":{},"secondary":{}}}"#,
                        p.map(|v| v.to_string()).unwrap_or("null".to_string()),
                        s.map(|v| v.to_string()).unwrap_or("null".to_string()));
                } else {
                    println!("Primary:   {}", p.map(|v| format!("{:.2}", v)).unwrap_or("default (0.75)".to_string()));
                    println!("Secondary: {}", s.map(|v| format!("{:.2}", v)).unwrap_or("default (0.60)".to_string()));
                }
            }
        }
    }
    Ok(())
}

// ============================================================================
// Embeddings Commands
// ============================================================================

async fn handle_embeddings(cmd: EmbeddingsCommands, db: &Database, json: bool) -> Result<(), String> {
    match cmd {
        EmbeddingsCommands::Status => {
            let count = db.count_nodes_with_embeddings().map_err(|e| e.to_string())?;
            let stats = db.get_stats().map_err(|e| e.to_string())?;
            let items = stats.1;
            let pct = if items > 0 { (count as f64 / items as f64 * 100.0) as u32 } else { 0 };
            let local = settings::use_local_embeddings();

            if json {
                println!(r#"{{"with_embeddings":{},"total":{},"percent":{},"local_embeddings":{}}}"#,
                    count, items, pct, local);
            } else {
                println!("With embeddings: {} / {} ({}%)", count, items, pct);
                println!("Local embeddings: {}", if local { "enabled" } else { "disabled" });
            }
        }
        EmbeddingsCommands::Regenerate => {
            eprintln!("Regenerating all embeddings (this may take a while)...");

            let items = db.get_items().map_err(|e| e.to_string())?;
            let mut generated = 0;

            for (i, node) in items.iter().enumerate() {
                if i % 50 == 0 {
                    eprint!("\r[{}/{}] Generating embeddings...", i, items.len());
                    std::io::stderr().flush().ok();
                }

                let text = format!("{}\n{}", node.title, node.content.as_deref().unwrap_or(""));
                match mycelica_lib::ai_client::generate_embedding(&text).await {
                    Ok(emb) => {
                        db.update_node_embedding(&node.id, &emb).map_err(|e| e.to_string())?;
                        generated += 1;
                    }
                    Err(e) => {
                        eprintln!("\nError generating embedding for {}: {}", &node.id[..8], e);
                    }
                }
            }
            eprintln!();

            if json {
                println!(r#"{{"generated":{}}}"#, generated);
            } else {
                println!("Generated {} embeddings", generated);
            }
        }
        EmbeddingsCommands::Clear => {
            db.clear_all_embeddings().map_err(|e| e.to_string())?;
            if json {
                println!(r#"{{"status":"ok"}}"#);
            } else {
                println!("Embeddings cleared");
            }
        }
        EmbeddingsCommands::Local { state } => {
            if let Some(s) = state {
                let enabled = s == "on";
                settings::set_use_local_embeddings(enabled)?;
                if json {
                    println!(r#"{{"local_embeddings":{}}}"#, enabled);
                } else {
                    println!("Local embeddings: {}", if enabled { "enabled" } else { "disabled" });
                }
            } else {
                let current = settings::use_local_embeddings();
                if json {
                    println!(r#"{{"local_embeddings":{}}}"#, current);
                } else {
                    println!("Local embeddings: {}", if current { "enabled" } else { "disabled" });
                }
            }
        }
    }
    Ok(())
}

// ============================================================================
// Privacy Commands
// ============================================================================

async fn handle_privacy(cmd: PrivacyCommands, db: &Database, json: bool, quiet: bool) -> Result<(), String> {
    match cmd {
        PrivacyCommands::Scan => {
            if !quiet { eprintln!("Scanning nodes for privacy..."); }

            // Get items that need privacy scanning (is_private is NULL)
            let items = db.get_items().map_err(|e| e.to_string())?;
            let unscanned: Vec<_> = items.into_iter().filter(|n| n.is_private.is_none()).collect();

            if json {
                println!(r#"{{"unscanned":{},"message":"Use GUI for AI-powered privacy scanning"}}"#, unscanned.len());
            } else {
                println!("{} items need privacy scanning", unscanned.len());
                println!("Note: AI-powered privacy analysis requires the GUI interface");
            }
        }
        PrivacyCommands::ScanItems { force: _ } => {
            if !quiet { eprintln!("Checking privacy scoring status..."); }
            let items = db.get_items_needing_privacy_scoring().map_err(|e| e.to_string())?;
            if json {
                println!(r#"{{"items_to_score":{}}}"#, items.len());
            } else {
                println!("{} items need privacy scoring", items.len());
            }
        }
        PrivacyCommands::Stats => {
            // get_privacy_stats returns (total, scanned, unscanned, private, safe)
            let stats = db.get_privacy_stats().map_err(|e| e.to_string())?;
            let (total, scanned, unscanned, private, safe) = stats;
            if json {
                println!(r#"{{"total":{},"scanned":{},"unscanned":{},"private":{},"safe":{}}}"#,
                    total, scanned, unscanned, private, safe);
            } else {
                println!("Total:     {}", total);
                println!("Scanned:   {}", scanned);
                println!("Unscanned: {}", unscanned);
                println!("Private:   {}", private);
                println!("Safe:      {}", safe);
            }
        }
        PrivacyCommands::Reset => {
            db.reset_all_privacy_flags().map_err(|e| e.to_string())?;
            if json {
                println!(r#"{{"status":"ok"}}"#);
            } else {
                println!("Privacy flags reset");
            }
        }
        PrivacyCommands::Export { path, threshold } => {
            let min_privacy = threshold as f64 / 100.0;

            // Copy database
            let src = db.get_path();
            std::fs::copy(&src, &path).map_err(|e| format!("Failed to copy: {}", e))?;

            // Open and filter
            let export_db = Database::new(&PathBuf::from(&path)).map_err(|e| e.to_string())?;

            // Delete private nodes
            let items = export_db.get_items().map_err(|e| e.to_string())?;
            let mut deleted = 0;
            for node in items {
                if node.privacy.unwrap_or(0.0) < min_privacy {
                    export_db.delete_node(&node.id).map_err(|e| e.to_string())?;
                    deleted += 1;
                }
            }

            export_db.prune_dead_edges().map_err(|e| e.to_string())?;

            if json {
                println!(r#"{{"exported":"{}","removed":{}}}"#, path, deleted);
            } else {
                println!("Exported to: {} ({} private nodes removed)", path, deleted);
            }
        }
        PrivacyCommands::Set { id, level } => {
            let is_private = level != "public";
            let reason = match level.as_str() {
                "sensitive" => Some("marked sensitive"),
                "private" => Some("marked private"),
                _ => None,
            };
            db.update_node_privacy(&id, is_private, reason).map_err(|e| e.to_string())?;

            if json {
                println!(r#"{{"id":"{}","privacy":"{}"}}"#, id, level);
            } else {
                println!("Set {} to {}", &id[..8], level);
            }
        }
    }
    Ok(())
}

// ============================================================================
// Paper Commands
// ============================================================================

async fn handle_paper(cmd: PaperCommands, db: &Database, json: bool) -> Result<(), String> {
    match cmd {
        PaperCommands::List { limit } => {
            let nodes = db.get_items().map_err(|e| e.to_string())?;
            let papers: Vec<_> = nodes.into_iter()
                .filter(|n| n.node_type == NodeType::Paper)
                .take(limit)
                .collect();

            if json {
                let items: Vec<String> = papers.iter().map(|n| {
                    format!(r#"{{"id":"{}","title":"{}","pdf_available":{}}}"#,
                        n.id, escape_json(&n.title), n.pdf_available.unwrap_or(false))
                }).collect();
                println!("[{}]", items.join(","));
            } else {
                for paper in &papers {
                    let pdf = if paper.pdf_available.unwrap_or(false) { "📄" } else { "○" };
                    println!("{} {} {}", pdf, &paper.id[..8], paper.title);
                }
                println!("\n{} papers", papers.len());
            }
        }
        PaperCommands::Get { id } => {
            let paper = db.get_paper_by_node_id(&id).map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Paper not found: {}", id))?;

            if json {
                println!(r#"{{"id":"{}","title":"{}","doi":{},"pdf_available":{}}}"#,
                    id,
                    paper.authors.as_deref().unwrap_or(""),
                    paper.doi.as_ref().map(|d| format!("\"{}\"", d)).unwrap_or("null".to_string()),
                    paper.pdf_available);
            } else {
                if let Some(ref doi) = paper.doi {
                    println!("DOI:     {}", doi);
                }
                if let Some(ref authors) = paper.authors {
                    println!("Authors: {}", authors);
                }
                if let Some(ref date) = paper.publication_date {
                    println!("Date:    {}", date);
                }
                if let Some(ref journal) = paper.journal {
                    println!("Journal: {}", journal);
                }
                println!("PDF:     {}", if paper.pdf_available { "Available" } else { "Not available" });
                if let Some(ref abstract_text) = paper.abstract_text {
                    let preview = if abstract_text.len() > 500 { &abstract_text[..500] } else { abstract_text };
                    println!("\nAbstract:\n{}", preview);
                }
            }
        }
        PaperCommands::Download { id } => {
            let paper = db.get_paper_by_node_id(&id).map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Paper not found: {}", id))?;

            if let Some(ref url) = paper.pdf_url {
                eprintln!("Downloading PDF...");
                let client = openaire::OpenAireClient::new();
                match client.download_pdf(url, 50).await {
                    Ok(Some(data)) => {
                        db.update_paper_pdf(&id, &data).map_err(|e| e.to_string())?;
                        if json {
                            println!(r#"{{"downloaded":true,"size":{}}}"#, data.len());
                        } else {
                            println!("Downloaded {} bytes", data.len());
                        }
                    }
                    Ok(None) => {
                        if json {
                            println!(r#"{{"downloaded":false,"reason":"too large or not found"}}"#);
                        } else {
                            println!("PDF too large or not found");
                        }
                    }
                    Err(e) => return Err(format!("Download failed: {}", e)),
                }
            } else {
                return Err("No PDF URL available".to_string());
            }
        }
        PaperCommands::Open { id } => {
            let pdf_data = db.get_paper_pdf(&id).map_err(|e| e.to_string())?
                .ok_or_else(|| "No PDF available for this paper".to_string())?;

            // Write to temp file and open
            let temp_path = std::env::temp_dir().join(format!("mycelica-{}.pdf", &id[..8]));
            std::fs::write(&temp_path, &pdf_data).map_err(|e| format!("Failed to write temp file: {}", e))?;

            #[cfg(target_os = "linux")]
            std::process::Command::new("xdg-open").arg(&temp_path).spawn().ok();

            #[cfg(target_os = "macos")]
            std::process::Command::new("open").arg(&temp_path).spawn().ok();

            #[cfg(target_os = "windows")]
            std::process::Command::new("cmd").args(["/C", "start", ""]).arg(&temp_path).spawn().ok();

            if json {
                println!(r#"{{"opened":"{}"}}"#, temp_path.display());
            } else {
                println!("Opened: {}", temp_path.display());
            }
        }
        PaperCommands::SyncPdfs => {
            db.sync_paper_pdf_status().map_err(|e| e.to_string())?;
            if json {
                println!(r#"{{"status":"ok"}}"#);
            } else {
                println!("PDF status synced");
            }
        }
        PaperCommands::ReformatAbstracts => {
            let count = db.reformat_all_paper_abstracts().map_err(|e| e.to_string())?;
            if json {
                println!(r#"{{"reformatted":{}}}"#, count);
            } else {
                println!("Reformatted {} abstracts", count);
            }
        }
    }
    Ok(())
}

// ============================================================================
// Config Commands
// ============================================================================

fn handle_config(cmd: ConfigCommands, json: bool) -> Result<(), String> {
    match cmd {
        ConfigCommands::List => {
            let anthropic = settings::has_api_key();
            let openai = settings::has_openai_api_key();
            let openaire = settings::has_openaire_api_key();
            let (cluster_p, cluster_s) = settings::get_clustering_thresholds();
            let privacy = settings::get_privacy_threshold();
            let local_emb = settings::use_local_embeddings();
            let protect = settings::is_recent_notes_protected();
            let tips = settings::show_tips();

            if json {
                println!(r#"{{"anthropic_api_key":{},"openai_api_key":{},"openaire_api_key":{},"clustering_primary":{},"clustering_secondary":{},"privacy_threshold":{},"local_embeddings":{},"protect_recent_notes":{},"show_tips":{}}}"#,
                    anthropic, openai, openaire,
                    cluster_p.map(|v| v.to_string()).unwrap_or("null".to_string()),
                    cluster_s.map(|v| v.to_string()).unwrap_or("null".to_string()),
                    privacy, local_emb, protect, tips);
            } else {
                println!("anthropic-api-key:    {}", if anthropic { "set" } else { "not set" });
                println!("openai-api-key:       {}", if openai { "set" } else { "not set" });
                println!("openaire-api-key:     {}", if openaire { "set" } else { "not set" });
                println!("clustering-primary:   {}", cluster_p.map(|v| format!("{:.2}", v)).unwrap_or("default".to_string()));
                println!("clustering-secondary: {}", cluster_s.map(|v| format!("{:.2}", v)).unwrap_or("default".to_string()));
                println!("privacy-threshold:    {:.2}", privacy);
                println!("local-embeddings:     {}", local_emb);
                println!("protect-recent-notes: {}", protect);
                println!("show-tips:            {}", tips);
            }
        }
        ConfigCommands::Get { key } => {
            let value: String = match key.as_str() {
                "anthropic-api-key" => settings::get_masked_api_key().unwrap_or_else(|| "not set".to_string()),
                "openai-api-key" => settings::get_masked_openai_api_key().unwrap_or_else(|| "not set".to_string()),
                "openaire-api-key" => settings::get_masked_openaire_api_key().unwrap_or_else(|| "not set".to_string()),
                "clustering-primary" => settings::get_clustering_thresholds().0.map(|v| v.to_string()).unwrap_or("default".to_string()),
                "clustering-secondary" => settings::get_clustering_thresholds().1.map(|v| v.to_string()).unwrap_or("default".to_string()),
                "privacy-threshold" => settings::get_privacy_threshold().to_string(),
                "local-embeddings" => settings::use_local_embeddings().to_string(),
                "protect-recent-notes" => settings::is_recent_notes_protected().to_string(),
                "show-tips" => settings::show_tips().to_string(),
                _ => return Err(format!("Unknown config key: {}", key)),
            };

            if json {
                println!(r#"{{"{}":"{}"}}"#, key, value);
            } else {
                println!("{}", value);
            }
        }
        ConfigCommands::Set { key, value } => {
            match key.as_str() {
                "anthropic-api-key" => settings::set_api_key(value.clone())?,
                "openai-api-key" => settings::set_openai_api_key(value.clone())?,
                "openaire-api-key" => settings::set_openaire_api_key(value.clone())?,
                "clustering-primary" => {
                    let v = value.parse::<f32>().map_err(|_| "Invalid number")?;
                    let (_, s) = settings::get_clustering_thresholds();
                    settings::set_clustering_thresholds(Some(v), s)?;
                }
                "clustering-secondary" => {
                    let v = value.parse::<f32>().map_err(|_| "Invalid number")?;
                    let (p, _) = settings::get_clustering_thresholds();
                    settings::set_clustering_thresholds(p, Some(v))?;
                }
                "privacy-threshold" => {
                    let v = value.parse::<f32>().map_err(|_| "Invalid number")?;
                    settings::set_privacy_threshold(v)?;
                }
                "local-embeddings" => {
                    let v = value.parse::<bool>().map_err(|_| "Invalid boolean (use true/false)")?;
                    settings::set_use_local_embeddings(v)?;
                }
                "protect-recent-notes" => {
                    let v = value.parse::<bool>().map_err(|_| "Invalid boolean (use true/false)")?;
                    settings::set_protect_recent_notes(v)?;
                }
                "show-tips" => {
                    let v = value.parse::<bool>().map_err(|_| "Invalid boolean (use true/false)")?;
                    settings::set_show_tips(v)?;
                }
                _ => return Err(format!("Unknown config key: {}", key)),
            }

            if json {
                println!(r#"{{"status":"ok"}}"#);
            } else {
                println!("Set {} = {}", key, value);
            }
        }
    }
    Ok(())
}

// ============================================================================
// Recent/Pinned Commands
// ============================================================================

fn handle_recent(cmd: RecentCommands, db: &Database, json: bool) -> Result<(), String> {
    match cmd {
        RecentCommands::List { limit } => {
            let recent = db.get_recent_nodes(limit as i32).map_err(|e| e.to_string())?;

            if json {
                let items: Vec<String> = recent.iter().map(|n| {
                    format!(r#"{{"id":"{}","title":"{}"}}"#, n.id, escape_json(&n.title))
                }).collect();
                println!("[{}]", items.join(","));
            } else {
                for node in &recent {
                    println!("{} {}", &node.id[..8], node.title);
                }
            }
        }
        RecentCommands::Clear => {
            // Clear recent by resetting last_accessed_at for all nodes
            // Note: The DB method clear_recent requires a node_id, so we'll clear all
            let recent = db.get_recent_nodes(1000).map_err(|e| e.to_string())?;
            for node in &recent {
                db.clear_recent(&node.id).map_err(|e| e.to_string())?;
            }
            if json {
                println!(r#"{{"status":"ok","cleared":{}}}"#, recent.len());
            } else {
                println!("Cleared {} recent entries", recent.len());
            }
        }
    }
    Ok(())
}

fn handle_pinned(cmd: PinnedCommands, db: &Database, json: bool) -> Result<(), String> {
    match cmd {
        PinnedCommands::List => {
            let pinned = db.get_pinned_nodes().map_err(|e| e.to_string())?;

            if json {
                let items: Vec<String> = pinned.iter().map(|n| {
                    format!(r#"{{"id":"{}","title":"{}"}}"#, n.id, escape_json(&n.title))
                }).collect();
                println!("[{}]", items.join(","));
            } else {
                for node in &pinned {
                    println!("📌 {} {}", &node.id[..8], node.title);
                }
                if pinned.is_empty() {
                    println!("No pinned nodes");
                }
            }
        }
        PinnedCommands::Add { id } => {
            db.set_node_pinned(&id, true).map_err(|e| e.to_string())?;
            if json {
                println!(r#"{{"pinned":"{}"}}"#, id);
            } else {
                println!("Pinned: {}", &id[..8]);
            }
        }
        PinnedCommands::Remove { id } => {
            db.set_node_pinned(&id, false).map_err(|e| e.to_string())?;
            if json {
                println!(r#"{{"unpinned":"{}"}}"#, id);
            } else {
                println!("Unpinned: {}", &id[..8]);
            }
        }
    }
    Ok(())
}

// ============================================================================
// Navigation Commands
// ============================================================================

async fn handle_nav(cmd: NavCommands, db: &Database, json: bool) -> Result<(), String> {
    match cmd {
        NavCommands::Ls { id, long } => {
            let parent_id = if id == "root" {
                db.get_universe().map_err(|e| e.to_string())?
                    .ok_or_else(|| "No universe found".to_string())?.id
            } else {
                id
            };

            let children = db.get_children(&parent_id).map_err(|e| e.to_string())?;

            if json {
                let items: Vec<String> = children.iter().map(|n| {
                    format!(r#"{{"id":"{}","title":"{}","is_item":{},"child_count":{}}}"#,
                        n.id, escape_json(&n.title), n.is_item, n.child_count)
                }).collect();
                println!("[{}]", items.join(","));
            } else {
                for (i, node) in children.iter().enumerate() {
                    let marker = if node.is_item { "📄" } else { "📁" };
                    if long {
                        println!("[{:2}] {} {:>4} {} {}", i + 1, marker, node.child_count, &node.id[..8], node.title);
                    } else {
                        println!("[{:2}] {} {}", i + 1, marker, node.title);
                    }
                }
            }
        }
        NavCommands::Tree { id, depth } => {
            let root_id = if id == "root" {
                db.get_universe().map_err(|e| e.to_string())?
                    .ok_or_else(|| "No universe found".to_string())?.id
            } else {
                id
            };

            fn print_tree(db: &Database, node_id: &str, depth: usize, max_depth: usize, prefix: &str, json: bool) {
                if depth > max_depth { return; }

                if let Ok(Some(node)) = db.get_node(node_id) {
                    let marker = if node.is_item { "📄" } else { if depth == 0 { "🌌" } else { "📁" } };
                    if !json {
                        println!("{}{} {}", prefix, marker, node.title);
                    }

                    if !node.is_item && depth < max_depth {
                        if let Ok(children) = db.get_children(node_id) {
                            for (i, child) in children.iter().take(10).enumerate() {
                                let is_last = i == children.len().min(10) - 1;
                                let new_prefix = format!("{}{}  ", prefix, if is_last { "└─" } else { "├─" });
                                print_tree(db, &child.id, depth + 1, max_depth, &new_prefix, json);
                            }
                            if children.len() > 10 {
                                println!("{}   ... and {} more", prefix, children.len() - 10);
                            }
                        }
                    }
                }
            }

            if json {
                // For JSON, just return flat structure
                println!(r#"{{"tree":"use --no-json for tree view"}}"#);
            } else {
                print_tree(db, &root_id, 0, depth, "", false);
            }
        }
        NavCommands::Path { from, to } => {
            // Simple path finding - walk up from both and find common ancestor
            let from_path = hierarchy::build_hierarchy_path(db, &from)?;
            let to_path = hierarchy::build_hierarchy_path(db, &to)?;

            if json {
                println!(r#"{{"from_path":{:?},"to_path":{:?}}}"#, from_path, to_path);
            } else {
                println!("From: {}", from_path.join(" > "));
                println!("To:   {}", to_path.join(" > "));
            }
        }
        NavCommands::Edges { id } => {
            let edges = db.get_edges_for_node(&id).map_err(|e| e.to_string())?;

            if json {
                let items: Vec<String> = edges.iter().map(|e| {
                    format!(r#"{{"source":"{}","target":"{}","type":"{}","weight":{}}}"#,
                        e.source, e.target, format!("{:?}", e.edge_type).to_lowercase(),
                        e.weight.unwrap_or(1.0))
                }).collect();
                println!("[{}]", items.join(","));
            } else {
                for edge in &edges {
                    let direction = if edge.source == id { "→" } else { "←" };
                    let other = if edge.source == id { &edge.target } else { &edge.source };
                    if let Ok(Some(node)) = db.get_node(other) {
                        println!("{} {:?} {} ({})", direction, edge.edge_type, node.title,
                            edge.weight.map(|w| format!("{:.0}%", w * 100.0)).unwrap_or_default());
                    }
                }
            }
        }
        NavCommands::Similar { id, top } => {
            if let Some(target_emb) = db.get_node_embedding(&id).map_err(|e| e.to_string())? {
                let all_embeddings = db.get_nodes_with_embeddings().map_err(|e| e.to_string())?;
                let similar = similarity::find_similar(&target_emb, &all_embeddings, &id, top, 0.5);

                if json {
                    let items: Vec<String> = similar.iter().map(|(node_id, score)| {
                        format!(r#"{{"id":"{}","similarity":{:.3}}}"#, node_id, score)
                    }).collect();
                    println!("[{}]", items.join(","));
                } else {
                    for (node_id, score) in &similar {
                        if let Ok(Some(node)) = db.get_node(node_id) {
                            println!("{:.0}% {}", score * 100.0, node.title);
                        }
                    }
                }
            } else {
                return Err("No embedding for this node".to_string());
            }
        }
    }
    Ok(())
}

// ============================================================================
// TUI Mode (placeholder)
// ============================================================================

async fn run_tui(_db: &Database) -> Result<(), String> {
    eprintln!("TUI mode coming soon!");
    eprintln!("For now, use the nav commands for non-interactive navigation:");
    eprintln!("  mycelica-cli nav ls root");
    eprintln!("  mycelica-cli nav tree root --depth 3");
    Ok(())
}

// ============================================================================
// Utility Functions
// ============================================================================

fn find_database() -> PathBuf {
    // Check custom path from settings first
    if let Some(custom_path) = settings::get_custom_db_path() {
        let path = PathBuf::from(&custom_path);
        if path.exists() {
            return path;
        }
    }

    // Check specific known paths
    let known_paths = [
        dirs::data_dir().map(|p| p.join("com.mycelica.app").join("mycelica.db")),
        dirs::data_dir().map(|p| p.join("com.mycelica.dev").join("mycelica.db")),
        Some(PathBuf::from("data/mycelica.db")),
    ];

    for path_opt in known_paths.iter() {
        if let Some(path) = path_opt {
            if path.exists() {
                return path.clone();
            }
        }
    }

    // Fall back to app data dir
    dirs::data_dir()
        .map(|p| p.join("com.mycelica.dev").join("mycelica.db"))
        .unwrap_or_else(|| PathBuf::from("mycelica.db"))
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
