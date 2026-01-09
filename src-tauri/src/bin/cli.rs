//! Mycelica CLI - Full command-line interface for knowledge graph operations
//!
//! Usage: mycelica-cli [OPTIONS] <COMMAND>
//!
//! A first-class CLI for power users. Supports JSON output for scripting.

use clap::{Parser, Subcommand, CommandFactory};
use clap_complete::{generate, Shell};
use mycelica_lib::{db::{Database, Node, NodeType, EdgeType}, settings, import, hierarchy, similarity, clustering, openaire, ai_client, utils};
use serde_json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::io::Write;
use chrono::{Timelike, Utc};

// TUI imports
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};

// Privacy scoring imports
use serde::{Deserialize, Serialize};

// ============================================================================
// Privacy Scoring Constants and Structs
// ============================================================================

const PRIVACY_SCORING_PROMPT: &str = r#"Score each item 0.0-1.0 for public shareability:

0.0-0.2: Highly private — real names, health/mental state, finances, relationships, personal struggles, private contact info
0.3-0.4: Personal — work grievances, emotional venting, private project details, identifiable personal situations
0.5-0.6: Semi-private — named companies/projects in neutral context, work discussions, some identifiable context
0.7-0.8: Low risk — technical content with minor project context, professional discussions
0.9-1.0: Public — generic concepts, public knowledge, tutorials, no identifying context

When content spans multiple levels, use the LOWEST applicable score.

Items to score:
{items_json}

Return ONLY a JSON array:
[{"id": "...", "privacy": 0.7}, {"id": "...", "privacy": 0.3}]"#;

#[derive(Serialize)]
struct PrivacyApiRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<PrivacyApiMessage>,
}

#[derive(Serialize)]
struct PrivacyApiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct PrivacyApiResponse {
    content: Vec<PrivacyContentBlock>,
}

#[derive(Deserialize)]
struct PrivacyContentBlock {
    text: String,
}

#[derive(Deserialize)]
struct PrivacyScoreResult {
    id: String,
    privacy: f64,
}

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
    /// Interactive setup wizard
    Setup {
        /// Skip the processing pipeline
        #[arg(long)]
        skip_pipeline: bool,
        /// Use FOS (Field of Science) pre-grouping for papers before clustering
        #[arg(long)]
        fos: bool,
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
    /// Database maintenance operations (use with caution)
    Maintenance {
        #[command(subcommand)]
        cmd: MaintenanceCommands,
    },
    /// Export data in various formats
    Export {
        #[command(subcommand)]
        cmd: ExportCommands,
    },
    /// Analyze code relationships
    Analyze {
        #[command(subcommand)]
        cmd: AnalyzeCommands,
    },
    /// Code intelligence commands
    Code {
        #[command(subcommand)]
        cmd: CodeCommands,
    },
    /// Global search across all nodes
    Search {
        /// Search query
        query: String,
        /// Filter by type (item, category, paper, all)
        #[arg(long, short = 't', default_value = "all")]
        type_filter: String,
        /// Maximum results
        #[arg(long, short, default_value = "20")]
        limit: u32,
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
    /// Create a new database
    New {
        /// Path for the new database
        path: String,
        /// Don't set as default database
        #[arg(long)]
        no_select: bool,
    },
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
    /// Import source code files (Rust, TypeScript, Markdown)
    /// Respects .gitignore automatically
    Code {
        /// Path to source file or directory
        path: String,
        /// Language filter: rust, typescript, markdown (default: auto-detect all)
        #[arg(long, short)]
        language: Option<String>,
        /// Update mode: delete existing nodes from file(s) before import
        #[arg(long, short)]
        update: bool,
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
    /// Fix Recent Notes position (move to Universe)
    FixRecentNotes,
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
    /// Cluster with FOS (Field of Science) pre-grouping for papers
    Fos,
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
    /// Sync paper dates from publication_date to nodes.created_at
    SyncDates,
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
        /// Filter by edge type (calls, defined_in, uses_type, implements, related, etc.)
        #[arg(long = "type", short = 't')]
        edge_type: Option<String>,
        /// Filter by direction: incoming, outgoing, or both (default: both)
        #[arg(long, short, default_value = "both")]
        direction: String,
    },
    /// Find similar nodes by embedding
    Similar {
        /// Node ID
        id: String,
        /// Number of results
        #[arg(long, short, default_value = "10")]
        top: usize,
    },
    /// View code nodes as folder tree (extracted from file_path metadata)
    Folder {
        /// Filter by path prefix (e.g., "src/", "src-tauri/src/db")
        path: Option<String>,
        /// Maximum depth
        #[arg(long, short, default_value = "10")]
        depth: usize,
        /// Show item counts only, not individual items
        #[arg(long, short)]
        summary: bool,
    },
}

#[derive(Subcommand)]
enum MaintenanceCommands {
    /// Delete ALL data (nodes, edges, everything)
    Wipe {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Reset AI processing (titles, summaries, tags)
    ResetAi {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Reset clustering data
    ResetClusters {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Reset privacy scores
    ResetPrivacy {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Clear all embeddings
    ClearEmbeddings {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Clear hierarchy (flatten to universe)
    ClearHierarchy {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Clear all tags
    ClearTags {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Delete nodes with empty content
    DeleteEmpty {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Vacuum database (reclaim space)
    Vacuum,
    /// Fix child counts
    FixCounts {
        /// Show details
        #[arg(long, short)]
        verbose: bool,
    },
    /// Fix node depths
    FixDepths {
        /// Show details
        #[arg(long, short)]
        verbose: bool,
    },
    /// Prune dead edges
    PruneEdges {
        /// Show details
        #[arg(long, short)]
        verbose: bool,
    },
    /// Precompute FOS edge sets for fast view loading
    PrecomputeFosEdges,
    /// Index edges by parent for fast per-view loading
    IndexEdges,
}

#[derive(Subcommand)]
enum ExportCommands {
    /// Export papers as BibTeX
    Bibtex {
        /// Output file path
        #[arg(short, long)]
        output: String,
        /// Only export subtree of this node
        #[arg(long)]
        node: Option<String>,
        /// Include children recursively
        #[arg(long)]
        subtree: bool,
    },
    /// Export nodes as Markdown
    Markdown {
        /// Output file path
        #[arg(short, long)]
        output: String,
        /// Only export subtree of this node
        #[arg(long)]
        node: Option<String>,
        /// Include children recursively
        #[arg(long)]
        subtree: bool,
        /// Include full content (not just summaries)
        #[arg(long)]
        full: bool,
    },
    /// Export nodes as JSON
    Json {
        /// Output file path
        #[arg(short, long)]
        output: String,
        /// Only export subtree of this node
        #[arg(long)]
        node: Option<String>,
        /// Include children recursively
        #[arg(long)]
        subtree: bool,
        /// Pretty-print JSON
        #[arg(long)]
        pretty: bool,
    },
    /// Export graph structure (DOT or GraphML)
    Graph {
        /// Output file path
        #[arg(short, long)]
        output: String,
        /// Format: dot or graphml
        #[arg(long, short, default_value = "dot")]
        format: String,
        /// Only export subtree of this node
        #[arg(long)]
        node: Option<String>,
        /// Include similarity edges
        #[arg(long)]
        edges: bool,
    },
    /// Export a subtree as a new database
    Subgraph {
        /// Root node ID for subtree
        node: String,
        /// Output database path
        #[arg(short, long)]
        output: String,
        /// Maximum depth to export
        #[arg(long, short, default_value = "10")]
        depth: usize,
    },
}

#[derive(Subcommand)]
enum AnalyzeCommands {
    /// Analyze code and create "Calls" edges between functions
    CodeEdges {
        /// Only analyze functions from this path prefix
        #[arg(long)]
        path: Option<String>,
        /// Dry run - show what would be created without inserting
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum CodeCommands {
    /// Show source code for a code node (reads actual file)
    Show {
        /// Node ID (e.g., code-abc123...)
        id: String,
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
        Commands::Setup { skip_pipeline, fos } => handle_setup(&db, skip_pipeline, fos, cli.quiet).await,
        Commands::Recent { cmd } => handle_recent(cmd, &db, cli.json),
        Commands::Pinned { cmd } => handle_pinned(cmd, &db, cli.json),
        Commands::Nav { cmd } => handle_nav(cmd, &db, cli.json).await,
        Commands::Maintenance { cmd } => handle_maintenance(cmd, &db, cli.json).await,
        Commands::Export { cmd } => handle_export(cmd, &db, cli.quiet).await,
        Commands::Analyze { cmd } => handle_analyze(cmd, &db, cli.json, cli.quiet).await,
        Commands::Code { cmd } => handle_code(cmd, &db, &db_path).await,
        Commands::Search { query, type_filter, limit } => handle_search(&query, &type_filter, limit, &db, cli.json).await,
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

            // Check current directory and parent
            let cwd = std::env::current_dir().unwrap_or_default();
            for dir in [&cwd, &cwd.parent().unwrap_or(&cwd).to_path_buf()] {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map(|e| e == "db").unwrap_or(false) {
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
                        if path.extension().map(|e| e == "db").unwrap_or(false) {
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
        DbCommands::New { path, no_select } => {
            let db_path = PathBuf::from(&path);

            // Check if file already exists
            if db_path.exists() {
                return Err(format!("Database already exists at: {}", path));
            }

            // Create parent directory if needed
            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {}", e))?;
            }

            // Create new database
            let new_db = Database::new(&db_path)
                .map_err(|e| format!("Failed to create database: {}", e))?;

            // Build hierarchy for new database
            if let Err(e) = hierarchy::build_hierarchy(&new_db) {
                eprintln!("Warning: Failed to build initial hierarchy: {}", e);
            }

            // Set as default unless --no-select
            if !no_select {
                settings::set_custom_db_path(Some(path.clone()))
                    .map_err(|e| format!("Failed to save database path: {}", e))?;
            }

            if json {
                println!(r#"{{"created":"{}","selected":{}}}"#, path, !no_select);
            } else {
                println!("Created: {}", path);
                if !no_select {
                    println!("Set as default database.");
                }
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
        ImportCommands::Code { path, language, update } => {
            use mycelica_lib::code;
            use mycelica_lib::ai_client;

            if !quiet {
                if update {
                    eprintln!("[Code] Update mode: {} (will replace existing nodes)", path);
                } else {
                    eprintln!("[Code] Scanning: {} (respects .gitignore)", path);
                }
                if let Some(ref lang) = language {
                    eprintln!("[Code]   Language filter: {}", lang);
                }
            }

            // In update mode, collect files first and delete existing nodes
            let mut total_deleted = 0;
            if update {
                let file_path = std::path::Path::new(&path);
                let files_to_delete: Vec<String> = if file_path.is_file() {
                    vec![path.clone()]
                } else {
                    // Get list of files we'll be importing (respects .gitignore)
                    code::collect_code_files(file_path, language.as_deref())
                        .unwrap_or_default()
                        .into_iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect()
                };

                for file in &files_to_delete {
                    match db.delete_nodes_by_file_path(file) {
                        Ok(deleted) if deleted > 0 => {
                            if !quiet {
                                eprintln!("  Deleted {} nodes from {}", deleted, file);
                            }
                            total_deleted += deleted;
                        }
                        _ => {}
                    }
                }
                if !quiet && total_deleted > 0 {
                    eprintln!("[Code] Deleted {} existing nodes", total_deleted);
                }
            }

            let result = code::import_code(db, &path, language.as_deref())?;

            // In update mode, generate embeddings for new nodes and refresh Calls edges
            let mut embeddings_generated = 0;
            let mut calls_edges_created = 0;
            if update && result.total_items() > 0 {
                // Get the newly imported node IDs
                let file_path = std::path::Path::new(&path);
                let imported_files: Vec<String> = if file_path.is_file() {
                    vec![path.clone()]
                } else {
                    code::collect_code_files(file_path, language.as_deref())
                        .unwrap_or_default()
                        .into_iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect()
                };

                // Collect all newly imported node IDs
                let mut new_node_ids: Vec<String> = Vec::new();
                for file in &imported_files {
                    if let Ok(ids) = db.get_node_ids_by_file_path(file) {
                        new_node_ids.extend(ids);
                    }
                }

                // Generate embeddings for nodes without them
                if !new_node_ids.is_empty() {
                    if !quiet {
                        eprintln!("[Code] Generating embeddings for {} nodes...", new_node_ids.len());
                    }
                    for node_id in &new_node_ids {
                        if db.get_node_embedding(node_id).ok().flatten().is_none() {
                            if let Ok(Some(node)) = db.get_node(node_id) {
                                let text = format!("{}\n{}", node.title, node.content.as_deref().unwrap_or(""));
                                let embed_text = utils::safe_truncate(&text, 1000);
                                if let Ok(embedding) = ai_client::generate_embedding(embed_text).await {
                                    db.update_node_embedding(node_id, &embedding).ok();
                                    embeddings_generated += 1;
                                }
                            }
                        }
                    }
                }

                // Refresh Calls edges for functions
                let functions: Vec<_> = new_node_ids.iter()
                    .filter_map(|id| db.get_node(id).ok().flatten())
                    .filter(|n| n.content_type.as_deref() == Some("code_function"))
                    .collect();

                if !functions.is_empty() {
                    if !quiet {
                        eprintln!("[Code] Refreshing Calls edges for {} functions...", functions.len());
                    }

                    // Build function name index
                    let all_functions: Vec<_> = db.get_items()
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .filter(|n| n.content_type.as_deref() == Some("code_function"))
                        .collect();

                    let fn_name_to_id: std::collections::HashMap<String, String> = all_functions.iter()
                        .filter_map(|f| {
                            // Extract function name from title (e.g., "pub fn foo(...)" -> "foo")
                            let title = &f.title;
                            let name = title.split('(').next()
                                .and_then(|s| s.split_whitespace().last())
                                .map(|s| s.to_string());
                            name.map(|n| (n, f.id.clone()))
                        })
                        .collect();

                    // For each function, find calls in its body
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as i64;

                    for func in &functions {
                        if let Some(content) = &func.content {
                            for (fn_name, target_id) in &fn_name_to_id {
                                if target_id != &func.id && content.contains(&format!("{}(", fn_name)) {
                                    // Create Calls edge
                                    let edge = mycelica_lib::db::Edge {
                                        id: format!("calls-{}-{}", func.id, target_id),
                                        source: func.id.clone(),
                                        target: target_id.clone(),
                                        edge_type: mycelica_lib::db::EdgeType::Calls,
                                        label: None,
                                        weight: Some(1.0),
                                        edge_source: Some("code-analysis".to_string()),
                                        evidence_id: None,
                                        confidence: None,
                                        created_at: now,
                                    };
                                    if db.insert_edge(&edge).is_ok() {
                                        calls_edges_created += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if json {
                println!("{}", serde_json::to_string(&result).map_err(|e| e.to_string())?);
            } else {
                if update && total_deleted > 0 {
                    println!("Updated {} files (replaced {} existing nodes):", result.files_processed, total_deleted);
                } else {
                    println!("Imported {} items from {} files:", result.total_items(), result.files_processed);
                }
                if result.functions > 0 { println!("  Functions: {}", result.functions); }
                if result.structs > 0 { println!("  Structs: {}", result.structs); }
                if result.enums > 0 { println!("  Enums: {}", result.enums); }
                if result.traits > 0 { println!("  Traits: {}", result.traits); }
                if result.impls > 0 { println!("  Impl blocks: {}", result.impls); }
                if result.modules > 0 { println!("  Modules: {}", result.modules); }
                if result.macros > 0 { println!("  Macros: {}", result.macros); }
                if result.docs > 0 { println!("  Docs: {}", result.docs); }
                println!("  Edges created: {}", result.edges_created);
                if result.doc_edges > 0 { println!("  Doc→code edges: {}", result.doc_edges); }
                if update {
                    if embeddings_generated > 0 { println!("  Embeddings generated: {}", embeddings_generated); }
                    if calls_edges_created > 0 { println!("  Calls edges refreshed: {}", calls_edges_created); }
                }
                if result.files_skipped > 0 {
                    println!("  Files skipped: {}", result.files_skipped);
                }
                if !result.errors.is_empty() {
                    eprintln!("\nErrors ({}):", result.errors.len());
                    for err in &result.errors[..result.errors.len().min(5)] {
                        eprintln!("  {}", err);
                    }
                    if result.errors.len() > 5 {
                        eprintln!("  ... and {} more", result.errors.len() - 5);
                    }
                }
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
        HierarchyCommands::FixRecentNotes => {
            let recent_notes_id = settings::RECENT_NOTES_CONTAINER_ID;

            // Check if Recent Notes exists
            if let Some(recent_notes) = db.get_node(recent_notes_id).map_err(|e| e.to_string())? {
                // Get Universe
                let universe = db.get_universe().map_err(|e| e.to_string())?
                    .ok_or("Universe not found - run 'hierarchy build' first")?;

                let old_parent = recent_notes.parent_id.clone().unwrap_or_else(|| "none".to_string());

                if old_parent == universe.id {
                    if json {
                        println!(r#"{{"status":"already_correct","parent":"{}"}}"#, universe.id);
                    } else {
                        println!("Recent Notes is already under Universe");
                    }
                } else {
                    // Move Recent Notes to Universe at depth 1
                    db.update_parent(recent_notes_id, &universe.id).map_err(|e| e.to_string())?;
                    db.set_node_depth(recent_notes_id, 1).map_err(|e| e.to_string())?;

                    // Update child counts
                    db.recalculate_child_count(&old_parent).map_err(|e| e.to_string())?;
                    db.recalculate_child_count(&universe.id).map_err(|e| e.to_string())?;

                    // Propagate latest dates
                    db.propagate_latest_dates().map_err(|e| e.to_string())?;

                    if json {
                        println!(r#"{{"status":"fixed","old_parent":"{}","new_parent":"{}"}}"#, old_parent, universe.id);
                    } else {
                        println!("Moved Recent Notes from '{}' to Universe (depth 1)", old_parent);
                    }
                }
            } else {
                if json {
                    println!(r#"{{"status":"not_found"}}"#);
                } else {
                    println!("Recent Notes container not found");
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
                        // Preserve code_* content_types (from code import)
                        let final_content_type = if node.content_type
                            .as_ref()
                            .map(|ct| ct.starts_with("code_"))
                            .unwrap_or(false)
                        {
                            node.content_type.clone().unwrap_or_default()
                        } else {
                            result.content_type.clone()
                        };

                        db.update_node_ai(
                            &node.id,
                            &result.title,
                            &result.summary,
                            &serde_json::to_string(&result.tags).unwrap_or_default(),
                            &final_content_type,
                        ).map_err(|e| e.to_string())?;

                        // Generate embedding for the node
                        let embed_text = utils::safe_truncate(content, 1000);
                        if let Ok(embedding) = ai_client::generate_embedding(embed_text).await {
                            db.update_node_embedding(&node.id, &embedding).ok();
                        }

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
        ClusterCommands::Fos => {
            if !quiet { eprintln!("Clustering with FOS pre-grouping for papers..."); }
            let result = clustering::cluster_with_fos_pregrouping(db).await?;
            if json {
                println!(r#"{{"items_processed":{},"clusters_created":{},"items_assigned":{},"edges_created":{}}}"#,
                    result.items_processed, result.clusters_created, result.items_assigned, result.edges_created);
            } else {
                println!("Processed {} items, created {} clusters, assigned {} items, {} edges",
                    result.items_processed, result.clusters_created, result.items_assigned, result.edges_created);
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
        PrivacyCommands::ScanItems { force } => {
            let api_key = settings::get_api_key().ok_or("ANTHROPIC_API_KEY not set. Set it with: mycelica-cli config set anthropic-key YOUR_KEY")?;

            // Get items needing scoring
            let items = if force {
                db.get_items().map_err(|e| e.to_string())?
            } else {
                db.get_items_needing_privacy_scoring().map_err(|e| e.to_string())?
            };

            if items.is_empty() {
                if json {
                    println!(r#"{{"items_scored":0,"batches_processed":0,"error_count":0}}"#);
                } else {
                    println!("No items need privacy scoring");
                }
                return Ok(());
            }

            const BATCH_SIZE: usize = 25;
            let batches: Vec<_> = items.chunks(BATCH_SIZE).collect();
            let total_batches = batches.len();
            let total_items = items.len();

            if !quiet {
                eprintln!("Scoring {} items in {} batches...", total_items, total_batches);
            }

            let mut items_scored = 0;
            let mut error_count = 0;
            let client = reqwest::Client::new();

            for (batch_idx, batch) in batches.iter().enumerate() {
                if !quiet {
                    eprint!("\r[{}/{}] Processing batch...", batch_idx + 1, total_batches);
                    std::io::stderr().flush().ok();
                }

                // Build items JSON for prompt
                let items_for_prompt: Vec<serde_json::Value> = batch.iter().map(|item| {
                    let title = item.ai_title.as_deref().unwrap_or(&item.title);
                    let summary = item.summary.as_deref().unwrap_or("");
                    let content = item.content.as_deref().unwrap_or("");
                    let content_preview = utils::safe_truncate(content, 500);

                    serde_json::json!({
                        "id": item.id,
                        "title": title,
                        "summary": summary,
                        "content_preview": content_preview
                    })
                }).collect();

                let items_json = serde_json::to_string_pretty(&items_for_prompt)
                    .unwrap_or_else(|_| "[]".to_string());

                let prompt = PRIVACY_SCORING_PROMPT.replace("{items_json}", &items_json);

                let request = PrivacyApiRequest {
                    model: "claude-haiku-4-5-20251001".to_string(),
                    max_tokens: 2000,
                    messages: vec![PrivacyApiMessage {
                        role: "user".to_string(),
                        content: prompt,
                    }],
                };

                // Make API call
                match client
                    .post("https://api.anthropic.com/v1/messages")
                    .header("x-api-key", &api_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(&request)
                    .send()
                    .await
                {
                    Ok(response) => {
                        if response.status().is_success() {
                            if let Ok(api_response) = response.json::<PrivacyApiResponse>().await {
                                let text = api_response
                                    .content
                                    .first()
                                    .map(|c| c.text.clone())
                                    .unwrap_or_default();

                                match parse_privacy_scoring_response(&text) {
                                    Ok(scores) => {
                                        for score in scores {
                                            if let Err(e) = db.update_privacy_score(&score.id, score.privacy) {
                                                if !quiet {
                                                    eprintln!("\n  Failed to update privacy for {}: {}", &score.id[..8], e);
                                                }
                                                error_count += 1;
                                            } else {
                                                items_scored += 1;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        if !quiet {
                                            eprintln!("\n  Batch {} parse error: {}", batch_idx + 1, e);
                                        }
                                        error_count += batch.len();
                                    }
                                }
                            } else {
                                if !quiet {
                                    eprintln!("\n  Batch {} failed to parse API response", batch_idx + 1);
                                }
                                error_count += batch.len();
                            }
                        } else {
                            if !quiet {
                                eprintln!("\n  Batch {} API error: {}", batch_idx + 1, response.status());
                            }
                            error_count += batch.len();
                        }
                    }
                    Err(e) => {
                        if !quiet {
                            eprintln!("\n  Batch {} request failed: {}", batch_idx + 1, e);
                        }
                        error_count += batch.len();
                    }
                }

                // Small delay to avoid rate limits
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            }

            if !quiet { eprintln!(); }

            if json {
                println!(r#"{{"items_scored":{},"batches_processed":{},"error_count":{}}}"#,
                    items_scored, total_batches, error_count);
            } else {
                println!("Scored {} items ({} errors)", items_scored, error_count);
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
        PaperCommands::SyncDates => {
            let (updated, unknown) = db.sync_paper_dates().map_err(|e| e.to_string())?;
            // Also propagate latest dates to clusters
            db.propagate_latest_dates().map_err(|e| e.to_string())?;
            if json {
                println!(r#"{{"updated":{},"unknown":{}}}"#, updated, unknown);
            } else {
                println!("Synced dates: {} updated, {} set to unknown", updated, unknown);
                println!("Latest dates propagated to clusters");
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
// Setup Command
// ============================================================================

async fn handle_setup(db: &Database, skip_pipeline: bool, use_fos: bool, quiet: bool) -> Result<(), String> {
    use std::io::{Write, BufRead};

    println!("=== Mycelica Setup ===\n");

    // Show current database
    println!("Database: {}\n", db.get_path());

    // Check and prompt for API keys
    let has_openai = settings::has_openai_api_key();
    let has_anthropic = settings::has_api_key();

    println!("API Keys:");
    println!("  OpenAI:    {}", if has_openai { "configured" } else { "not set" });
    println!("  Anthropic: {}", if has_anthropic { "configured" } else { "not set" });
    println!();

    // Prompt for OpenAI key if not set (required for embeddings)
    if !has_openai {
        print!("Enter OpenAI API key (for embeddings, or press Enter to skip): ");
        std::io::stdout().flush().ok();

        let mut key = String::new();
        std::io::stdin().lock().read_line(&mut key).ok();
        let key = key.trim().to_string();
        if !key.is_empty() {
            settings::set_openai_api_key(key)?;
            println!("  OpenAI key saved.\n");
        } else {
            println!("  Skipped.\n");
        }
    }

    // Prompt for Anthropic key if not set (required for AI processing)
    if !has_anthropic {
        print!("Enter Anthropic API key (for AI processing, or press Enter to skip): ");
        std::io::stdout().flush().ok();

        let mut key = String::new();
        std::io::stdin().lock().read_line(&mut key).ok();
        let key = key.trim().to_string();
        if !key.is_empty() {
            settings::set_api_key(key)?;
            println!("  Anthropic key saved.\n");
        } else {
            println!("  Skipped.\n");
        }
    }

    // Show database stats
    let items = db.get_items().map_err(|e| e.to_string())?;
    let total = items.len();
    let papers = items.iter().filter(|n| n.content_type.as_deref() == Some("paper")).count();
    let non_papers = total - papers;
    let processed = items.iter().filter(|n| n.is_processed).count();
    let non_paper_processed = items.iter()
        .filter(|n| n.is_processed && n.content_type.as_deref() != Some("paper"))
        .count();
    let with_embeddings = items.iter().filter(|n| {
        db.get_node_embedding(&n.id).ok().flatten().is_some()
    }).count();

    println!("Database Stats:");
    println!("  Items:      {} ({} papers, {} other)", total, papers, non_papers);
    println!("  Processed:  {} / {} (papers skip AI processing)", non_paper_processed, non_papers);
    println!("  Embeddings: {} / {}", with_embeddings, total);
    println!();

    if skip_pipeline {
        println!("Setup complete. (Pipeline skipped)");
        return Ok(());
    }

    // Ask to run pipeline if there's work to do
    // Papers don't need AI processing (they have metadata from OpenAIRE)
    let needs_processing = non_paper_processed < non_papers;
    let needs_embeddings = with_embeddings < total;

    if !needs_processing && !needs_embeddings {
        println!("All items are processed and have embeddings. Nothing to do!");
        return Ok(());
    }

    print!("Run processing pipeline? [Y/n]: ");
    std::io::stdout().flush().ok();

    let mut input = String::new();
    std::io::stdin().lock().read_line(&mut input).ok();
    let run_pipeline = input.trim().is_empty() || input.trim().to_lowercase().starts_with('y');

    if !run_pipeline {
        println!("Setup complete. Run 'mycelica-cli process run' later to process items.");
        return Ok(());
    }

    println!();
    println!("═══════════════════════════════════════════════════════════");
    println!("Starting Mycelica Pipeline");
    println!("═══════════════════════════════════════════════════════════");

    // Step 1: AI Processing + Embeddings
    println!();
    println!("▶ STEP 1/3: AI Processing + Embeddings");
    println!("───────────────────────────────────────────────────────────");

    // 1a: AI process non-paper, non-code items
    // Papers have metadata from OpenAIRE, code items use their signature as title
    let unprocessed_non_papers: Vec<_> = items.iter()
        .filter(|n| !n.is_processed)
        .filter(|n| n.content_type.as_deref() != Some("paper"))
        .filter(|n| !n.content_type.as_ref().map(|ct| ct.starts_with("code_")).unwrap_or(false))
        .collect();

    if !unprocessed_non_papers.is_empty() && settings::has_api_key() {
        println!("[1a] AI Processing {} items...", unprocessed_non_papers.len());

        let mut success_count = 0;
        let mut error_count = 0;
        let step_start = std::time::Instant::now();

        for (i, node) in unprocessed_non_papers.iter().enumerate() {
            if !quiet {
                let title_preview: String = node.title.chars().take(50).collect();
                println!("  [{}/{}] {}{}",
                    i + 1,
                    unprocessed_non_papers.len(),
                    title_preview,
                    if node.title.len() > 50 { "..." } else { "" }
                );
            }

            let content = node.content.as_deref().unwrap_or(&node.title);
            match mycelica_lib::ai_client::analyze_node(&node.title, content).await {
                Ok(result) => {
                    db.update_node_ai(
                        &node.id,
                        &result.title,
                        &result.summary,
                        &serde_json::to_string(&result.tags).unwrap_or_default(),
                        &result.content_type,
                    ).ok();

                    // Generate embedding for the node
                    let embed_text = utils::safe_truncate(content, 1000);
                    if let Ok(embedding) = ai_client::generate_embedding(embed_text).await {
                        db.update_node_embedding(&node.id, &embedding).ok();
                    }

                    if !quiet {
                        println!("    → \"{}\" ({})", result.title, result.content_type);
                    }
                    success_count += 1;
                }
                Err(e) => {
                    if !quiet {
                        eprintln!("    ✗ Error: {}", e);
                    }
                    error_count += 1;
                }
            }
        }
        let elapsed = step_start.elapsed().as_secs_f64();
        println!("  ✓ Processed {} items in {:.1}s ({} errors)", success_count, elapsed, error_count);
    } else if !unprocessed_non_papers.is_empty() {
        println!("[1a] AI Processing... ⊘ SKIPPED (no Anthropic API key)");
    } else {
        println!("[1a] AI Processing... ✓ already complete");
    }

    // 1b: Generate embeddings for papers (they have metadata but need embeddings)
    let papers_needing_embeddings: Vec<_> = items.iter()
        .filter(|n| n.content_type.as_deref() == Some("paper"))
        .filter(|n| db.get_node_embedding(&n.id).ok().flatten().is_none())
        .collect();

    if !papers_needing_embeddings.is_empty() && settings::has_openai_api_key() {
        println!("[1b] Embedding {} papers...", papers_needing_embeddings.len());

        let mut success_count = 0;
        let step_start = std::time::Instant::now();

        for (i, node) in papers_needing_embeddings.iter().enumerate() {
            if !quiet && (i % 10 == 0 || i == papers_needing_embeddings.len() - 1) {
                println!("  [{}/{}] {}", i + 1, papers_needing_embeddings.len(),
                    node.title.chars().take(50).collect::<String>());
            }

            let text = format!("{} {}",
                node.ai_title.as_deref().unwrap_or(&node.title),
                node.summary.as_deref().unwrap_or("")
            );

            if let Ok(embedding) = ai_client::generate_embedding(&text).await {
                db.update_node_embedding(&node.id, &embedding).ok();
                success_count += 1;
            }
        }
        let elapsed = step_start.elapsed().as_secs_f64();
        println!("  ✓ Embedded {} papers in {:.1}s", success_count, elapsed);
    } else if !papers_needing_embeddings.is_empty() {
        println!("[1b] Paper embeddings... ⊘ SKIPPED (no OpenAI API key)");
    } else if papers > 0 {
        println!("[1b] Paper embeddings... ✓ already complete");
    }

    // 1c: Generate embeddings for code items (skip AI, just embeddings)
    let code_needing_embeddings: Vec<_> = items.iter()
        .filter(|n| n.content_type.as_ref().map(|ct| ct.starts_with("code_")).unwrap_or(false))
        .filter(|n| db.get_node_embedding(&n.id).ok().flatten().is_none())
        .collect();

    if !code_needing_embeddings.is_empty() {
        println!("[1c] Embedding {} code items (skipping AI)...", code_needing_embeddings.len());

        let mut success_count = 0;
        let step_start = std::time::Instant::now();

        for (i, node) in code_needing_embeddings.iter().enumerate() {
            if !quiet && (i % 25 == 0 || i == code_needing_embeddings.len() - 1) {
                println!("  [{}/{}] {}", i + 1, code_needing_embeddings.len(),
                    node.title.chars().take(50).collect::<String>());
            }

            // Embed signature + content for semantic search
            let text = format!("{}\n{}",
                node.title,
                node.content.as_deref().unwrap_or("")
            );
            let embed_text = utils::safe_truncate(&text, 1000);

            if let Ok(embedding) = ai_client::generate_embedding(embed_text).await {
                db.update_node_embedding(&node.id, &embedding).ok();
                success_count += 1;
            }
        }
        let elapsed = step_start.elapsed().as_secs_f64();
        println!("  ✓ Embedded {} code items in {:.1}s", success_count, elapsed);
    } else {
        let code_count = items.iter()
            .filter(|n| n.content_type.as_ref().map(|ct| ct.starts_with("code_")).unwrap_or(false))
            .count();
        if code_count > 0 {
            println!("[1c] Code embeddings... ✓ already complete");
        }
    }

    // Step 2: Clustering & Hierarchy
    // Note: hierarchy::build_full_hierarchy has its own verbose logging via emit_log()
    println!();
    println!("▶ STEP 2/3: Clustering & Hierarchy");
    println!("───────────────────────────────────────────────────────────");

    // If FOS flag is set, run full FOS pipeline: FOS grouping → clustering → hierarchy
    if use_fos {
        println!("Running FOS (Field of Science) pipeline...");
        println!();

        // Step 2a: Create FOS parent nodes
        println!("  [2a] Creating FOS parent nodes...");
        match clustering::cluster_with_fos_pregrouping(db).await {
            Ok(result) => {
                println!("  ✓ FOS grouping: {} papers → {} FOS categories",
                    result.items_assigned, result.clusters_created);
            }
            Err(e) => {
                eprintln!("  ✗ FOS pre-grouping failed: {}", e);
            }
        }

        // Step 2b: Run clustering within each FOS group (assigns cluster_id)
        println!("  [2b] Clustering within FOS groups...");
        match clustering::run_clustering(db, true).await {
            Ok(result) => {
                println!("  ✓ Clustered: {} items → {} clusters",
                    result.items_assigned, result.clusters_created);
            }
            Err(e) => {
                eprintln!("  ✗ Clustering failed: {}", e);
            }
        }

        // Step 2c: Build full hierarchy with recursive AI grouping
        println!("  [2c] Building hierarchy with AI grouping...");
        println!();
        match hierarchy::build_full_hierarchy(db, false, None).await {
            Ok(result) => {
                println!();
                println!("  ✓ Hierarchy complete: {} levels, {} items organized",
                    result.hierarchy_result.max_depth, result.hierarchy_result.items_organized);
            }
            Err(e) => {
                eprintln!("  ✗ Hierarchy build failed: {}", e);
            }
        }

        // Step 2d: Precompute FOS edges for fast view loading
        println!("  [2d] Precomputing FOS edge sets...");
        match db.precompute_fos_edges() {
            Ok(count) => println!("  ✓ FOS edges: {} edges precomputed", count),
            Err(e) => eprintln!("  ✗ FOS edge precomputation failed: {}", e),
        }
    } else {
        // Non-FOS path: Use full 7-step hierarchy build with recursive AI grouping
        println!("Running full hierarchy build (7 steps)...");
        println!();
        match hierarchy::build_full_hierarchy(db, true, None).await {
            Ok(result) => {
                println!();
                println!("✓ Hierarchy complete: {} levels, {} items organized",
                    result.hierarchy_result.max_depth, result.hierarchy_result.items_organized);
            }
            Err(e) => {
                eprintln!("✗ Hierarchy build failed: {}", e);
            }
        }
    }

    // Step 3: Code edges (only if code nodes exist)
    // Note: Category embeddings handled by Hierarchy Step 6/7
    println!();
    println!("▶ STEP 3/3: Code Analysis");
    println!("───────────────────────────────────────────────────────────");

    let code_functions: Vec<_> = items.iter()
        .filter(|n| n.content_type.as_deref() == Some("code_function"))
        .collect();

    if !code_functions.is_empty() {
        println!("Analyzing {} code functions for call relationships...", code_functions.len());
        // Run analyze_code_edges inline (simplified version)
        use std::collections::{HashMap, HashSet};

        // Build function name -> id map
        let mut name_to_id: HashMap<String, String> = HashMap::new();
        for func in &code_functions {
            if let Some(name) = extract_function_name(&func.title) {
                name_to_id.insert(name, func.id.clone());
            }
        }

        // Get existing Calls edges
        let existing_edges: HashSet<(String, String)> = db
            .get_all_edges()
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.edge_type == EdgeType::Calls)
            .map(|e| (e.source, e.target))
            .collect();

        let mut edges_created = 0;
        for func in &code_functions {
            let content = match &func.content {
                Some(c) => c,
                None => continue,
            };

            let caller_name = extract_function_name(&func.title).unwrap_or_default();
            let called_names = find_called_functions(content, &name_to_id);

            for called_name in called_names {
                if called_name == caller_name {
                    continue;
                }

                if let Some(callee_id) = name_to_id.get(&called_name) {
                    if existing_edges.contains(&(func.id.clone(), callee_id.clone())) {
                        continue;
                    }

                    let edge = mycelica_lib::db::Edge {
                        id: format!("edge-call-{}-{}", &func.id[..8.min(func.id.len())], &callee_id[..8.min(callee_id.len())]),
                        source: func.id.clone(),
                        target: callee_id.clone(),
                        edge_type: EdgeType::Calls,
                        label: None,
                        weight: Some(1.0),
                        edge_source: Some("code-analysis".to_string()),
                        evidence_id: None,
                        confidence: Some(0.8),
                        created_at: chrono::Utc::now().timestamp_millis(),
                    };

                    if db.insert_edge(&edge).is_ok() {
                        edges_created += 1;
                    }
                }
            }
        }

        println!("  Indexed {} function names", name_to_id.len());
        println!("  Found {} existing call edges", existing_edges.len());

        if edges_created > 0 {
            println!("✓ Created {} new call edges", edges_created);
        } else {
            println!("✓ No new edges needed (already analyzed or no calls found)");
        }
    } else {
        println!("⊘ SKIPPED (no code_function nodes found)");
        println!("  Run 'mycelica-cli import code <path>' to import code first");
    }

    println!();
    println!("═══════════════════════════════════════════════════════════");
    println!("✓ Setup Complete!");
    println!("═══════════════════════════════════════════════════════════");
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
        NavCommands::Edges { id, edge_type, direction } => {
            let all_edges = db.get_edges_for_node(&id).map_err(|e| e.to_string())?;

            // Filter by edge type if specified
            let edges: Vec<_> = all_edges.into_iter()
                .filter(|e| {
                    if let Some(ref type_filter) = edge_type {
                        let edge_type_str = format!("{:?}", e.edge_type).to_lowercase();
                        edge_type_str == type_filter.to_lowercase()
                    } else {
                        true
                    }
                })
                .filter(|e| {
                    match direction.to_lowercase().as_str() {
                        "outgoing" | "out" => e.source == id,
                        "incoming" | "in" => e.target == id,
                        _ => true, // "both" or anything else
                    }
                })
                .collect();

            if json {
                let items: Vec<String> = edges.iter().map(|e| {
                    format!(r#"{{"source":"{}","target":"{}","type":"{}","weight":{}}}"#,
                        e.source, e.target, format!("{:?}", e.edge_type).to_lowercase(),
                        e.weight.unwrap_or(1.0))
                }).collect();
                println!("[{}]", items.join(","));
            } else {
                if edges.is_empty() {
                    println!("No edges found{}",
                        edge_type.as_ref().map(|t| format!(" of type '{}'", t)).unwrap_or_default());
                } else {
                    for edge in &edges {
                        let dir_marker = if edge.source == id { "→" } else { "←" };
                        let other = if edge.source == id { &edge.target } else { &edge.source };
                        if let Ok(Some(node)) = db.get_node(other) {
                            let weight_str = edge.weight.map(|w| format!(" ({:.0}%)", w * 100.0)).unwrap_or_default();
                            println!("{} {:?} {}{}", dir_marker, edge.edge_type, node.title, weight_str);
                        } else {
                            println!("{} {:?} {}", dir_marker, edge.edge_type, other);
                        }
                    }
                    println!("\n{} edge(s)", edges.len());
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
        NavCommands::Folder { path, depth, summary } => {
            // Get all code nodes
            let items = db.get_items().map_err(|e| e.to_string())?;
            let code_nodes: Vec<_> = items.iter()
                .filter(|n| n.source.as_ref().map(|s| s.starts_with("code-")).unwrap_or(false))
                .collect();

            // Extract file paths from tags metadata
            #[derive(serde::Deserialize)]
            struct CodeMeta {
                file_path: Option<String>,
            }

            // Build folder tree: path -> (files_here, subdirs)
            use std::collections::BTreeMap;

            #[derive(Default)]
            struct FolderNode {
                items: Vec<(String, String)>,  // (node_id, title)
                children: BTreeMap<String, FolderNode>,
            }

            let mut root = FolderNode::default();
            let path_filter = path.as_deref().unwrap_or("");

            for node in &code_nodes {
                let file_path = node.tags.as_ref()
                    .and_then(|t| serde_json::from_str::<CodeMeta>(t).ok())
                    .and_then(|m| m.file_path)
                    .unwrap_or_default();

                if file_path.is_empty() { continue; }

                // Apply path filter
                if !path_filter.is_empty() && !file_path.contains(path_filter) {
                    continue;
                }

                // Split path into components
                let parts: Vec<&str> = file_path.split('/').collect();

                // Navigate/create tree structure
                let mut current = &mut root;
                for (i, part) in parts.iter().enumerate() {
                    if i == parts.len() - 1 {
                        // This is the file - add item here
                        current.items.push((node.id.clone(), node.title.clone()));
                    } else {
                        // This is a directory - descend
                        current = current.children.entry(part.to_string()).or_default();
                    }
                }
            }

            // Count totals
            fn count_items(node: &FolderNode) -> usize {
                node.items.len() + node.children.values().map(count_items).sum::<usize>()
            }

            // Print tree
            fn print_folder_tree(
                name: &str,
                node: &FolderNode,
                prefix: &str,
                current_depth: usize,
                max_depth: usize,
                summary: bool,
                json: bool,
            ) {
                if current_depth > max_depth { return; }

                let item_count = count_items(node);
                let has_children = !node.children.is_empty();
                let has_items = !node.items.is_empty();

                if !json {
                    if has_children || has_items {
                        if summary {
                            println!("{}📁 {} ({} items)", prefix, name, item_count);
                        } else {
                            println!("{}📁 {}", prefix, name);
                        }
                    }

                    // Print items in this folder (unless summary mode)
                    if !summary {
                        for (i, (id, title)) in node.items.iter().enumerate() {
                            let is_last_item = i == node.items.len() - 1 && node.children.is_empty();
                            let item_prefix = if is_last_item { "└─" } else { "├─" };
                            // Truncate title for display
                            let display_title = if title.len() > 60 {
                                format!("{}...", &title[..57])
                            } else {
                                title.clone()
                            };
                            println!("{}  {} 📄 {} ({})", prefix, item_prefix, display_title, &id[..12]);
                        }
                    }

                    // Print subdirectories
                    let child_count = node.children.len();
                    for (i, (child_name, child_node)) in node.children.iter().enumerate() {
                        let is_last = i == child_count - 1;
                        let child_prefix = format!("{}  {}", prefix, if is_last { "└─" } else { "├─" });
                        let next_prefix = format!("{}  {}", prefix, if is_last { "  " } else { "│ " });
                        print!("{}", child_prefix.trim_end());
                        print_folder_tree(
                            child_name,
                            child_node,
                            &next_prefix,
                            current_depth + 1,
                            max_depth,
                            summary,
                            json,
                        );
                    }
                }
            }

            let total_items = count_items(&root);
            let total_files = root.children.len();

            if json {
                // JSON output: flat list of paths with counts
                fn collect_paths(node: &FolderNode, current_path: &str, paths: &mut Vec<(String, usize)>) {
                    if !node.items.is_empty() {
                        paths.push((current_path.to_string(), node.items.len()));
                    }
                    for (name, child) in &node.children {
                        let child_path = if current_path.is_empty() {
                            name.clone()
                        } else {
                            format!("{}/{}", current_path, name)
                        };
                        collect_paths(child, &child_path, paths);
                    }
                }
                let mut paths = Vec::new();
                collect_paths(&root, "", &mut paths);
                let items: Vec<String> = paths.iter()
                    .map(|(p, c)| format!(r#"{{"path":"{}","count":{}}}"#, p, c))
                    .collect();
                println!("[{}]", items.join(","));
            } else {
                if path_filter.is_empty() {
                    println!("Code folder tree ({} items in {} top-level dirs):\n", total_items, total_files);
                } else {
                    println!("Code folder tree (filter: '{}', {} items):\n", path_filter, total_items);
                }

                // Print from root children (skip empty root)
                for (name, child_node) in &root.children {
                    print_folder_tree(name, child_node, "", 0, depth, summary, false);
                }

                if total_items == 0 {
                    println!("No code nodes found. Run: mycelica-cli import code <PATH>");
                }
            }
        }
    }
    Ok(())
}

// ============================================================================
// Search Command
// ============================================================================

async fn handle_search(query: &str, type_filter: &str, limit: u32, db: &Database, json: bool) -> Result<(), String> {
    let results = db.search_nodes(query).map_err(|e| e.to_string())?;

    // Filter by type
    let filtered: Vec<_> = results.into_iter()
        .filter(|node| {
            match type_filter {
                "item" => node.is_item,
                "category" => !node.is_item,
                "paper" => node.source.as_deref() == Some("openaire"),
                _ => true, // "all"
            }
        })
        .take(limit as usize)
        .collect();

    if json {
        let items: Vec<String> = filtered.iter().map(|node| {
            format!(
                r#"{{"id":"{}","title":"{}","type":"{}","depth":{}}}"#,
                node.id,
                escape_json(node.ai_title.as_ref().unwrap_or(&node.title)),
                if node.is_item { "item" } else { "category" },
                node.depth
            )
        }).collect();
        println!("[{}]", items.join(","));
    } else {
        if filtered.is_empty() {
            println!("No results for '{}'", query);
        } else {
            println!("Found {} results for '{}':\n", filtered.len(), query);
            for node in &filtered {
                let emoji = node.emoji.as_deref().unwrap_or(if node.is_item { "📄" } else { "📁" });
                let title = node.ai_title.as_ref().unwrap_or(&node.title);
                let type_str = if node.is_item { "item" } else { "cat " };
                println!("{} {} [{}] {}", emoji, title, type_str, &node.id);
            }
        }
    }

    Ok(())
}

// ============================================================================
// Maintenance Commands
// ============================================================================

async fn handle_maintenance(cmd: MaintenanceCommands, db: &Database, _json: bool) -> Result<(), String> {
    match cmd {
        MaintenanceCommands::Wipe { force } => {
            // get_stats returns (total_nodes, total_items, processed, with_embeddings, unprocessed, unclustered, orphan_items, topics)
            let (total, _, _, _, _, _, _, _) = db.get_stats().map_err(|e| e.to_string())?;

            if !force {
                println!("\n⚠️  WARNING: This will permanently delete {} nodes!", total);
                println!("This action CANNOT be undone.\n");
                print!("Type 'yes' to confirm: ");
                std::io::stdout().flush().ok();

                let mut input = String::new();
                std::io::stdin().read_line(&mut input).map_err(|e| e.to_string())?;

                if input.trim() != "yes" {
                    return Err("Operation cancelled".into());
                }
            }

            db.delete_all_nodes().map_err(|e| e.to_string())?;
            db.delete_all_edges().map_err(|e| e.to_string())?;
            println!("✓ Deleted {} nodes and all edges", total);
        }

        MaintenanceCommands::ResetAi { force } => {
            if !force && !confirm_action("reset AI processing (titles, summaries, tags)")? {
                return Err("Operation cancelled".into());
            }
            let count = db.reset_ai_processing().map_err(|e| e.to_string())?;
            println!("✓ Reset AI processing for {} nodes", count);
        }

        MaintenanceCommands::ResetClusters { force } => {
            if !force && !confirm_action("reset clustering data")? {
                return Err("Operation cancelled".into());
            }
            // Reset cluster assignments for all items
            db.clear_item_parents().map_err(|e| e.to_string())?;
            println!("✓ Reset clustering (cleared item parents)");
        }

        MaintenanceCommands::ResetPrivacy { force } => {
            if !force && !confirm_action("reset privacy scores")? {
                return Err("Operation cancelled".into());
            }
            let count = db.reset_all_privacy_flags().map_err(|e| e.to_string())?;
            println!("✓ Reset privacy scores for {} nodes", count);
        }

        MaintenanceCommands::ClearEmbeddings { force } => {
            if !force && !confirm_action("clear all embeddings")? {
                return Err("Operation cancelled".into());
            }
            let count = db.clear_all_embeddings().map_err(|e| e.to_string())?;
            println!("✓ Cleared {} embeddings", count);
        }

        MaintenanceCommands::ClearHierarchy { force } => {
            if !force && !confirm_action("clear hierarchy (delete intermediate nodes, keep items)")? {
                return Err("Operation cancelled".into());
            }
            // Clear parent_id on items
            db.clear_item_parents().map_err(|e| e.to_string())?;
            // Delete intermediate hierarchy nodes (clusters, categories)
            let deleted = db.delete_hierarchy_nodes().map_err(|e| e.to_string())?;
            println!("✓ Cleared hierarchy: {} intermediate nodes deleted", deleted);
        }

        MaintenanceCommands::ClearTags { force } => {
            if !force && !confirm_action("clear all tags")? {
                return Err("Operation cancelled".into());
            }
            db.delete_all_tags().map_err(|e| e.to_string())?;
            println!("✓ Cleared all tags");
        }

        MaintenanceCommands::DeleteEmpty { force } => {
            // Use delete_empty_items which returns count
            if !force && !confirm_action("delete nodes with empty content")? {
                return Err("Operation cancelled".into());
            }

            let deleted = db.delete_empty_items().map_err(|e| e.to_string())?;
            println!("✓ Deleted {} empty items", deleted);
        }

        MaintenanceCommands::Vacuum => {
            // Fix counts, depths, and prune edges
            println!("Tidying database...");
            db.fix_all_child_counts().map_err(|e| e.to_string())?;
            db.fix_all_depths().map_err(|e| e.to_string())?;
            db.prune_dead_edges().map_err(|e| e.to_string())?;
            println!("✓ Database tidied (for VACUUM, use: sqlite3 <db> 'VACUUM')");
        }

        MaintenanceCommands::FixCounts { verbose } => {
            let fixed = db.fix_all_child_counts().map_err(|e| e.to_string())?;
            if verbose {
                println!("Fixed {} node child counts", fixed);
            } else {
                println!("✓ Fixed child counts");
            }
        }

        MaintenanceCommands::FixDepths { verbose } => {
            let fixed = db.fix_all_depths().map_err(|e| e.to_string())?;
            if verbose {
                println!("Fixed {} node depths", fixed);
            } else {
                println!("✓ Fixed depths");
            }
        }

        MaintenanceCommands::PruneEdges { verbose } => {
            let pruned = db.prune_dead_edges().map_err(|e| e.to_string())?;
            if verbose {
                println!("Pruned {} dead edges", pruned);
            } else {
                println!("✓ Pruned dead edges");
            }
        }

        MaintenanceCommands::PrecomputeFosEdges => {
            println!("Precomputing FOS edge sets...");
            let count = db.precompute_fos_edges().map_err(|e| e.to_string())?;
            println!("✓ Precomputed {} edges across FOS categories", count);
        }

        MaintenanceCommands::IndexEdges => {
            println!("Indexing edges by parent for fast per-view loading...");
            let count = db.update_edge_parents().map_err(|e| e.to_string())?;
            println!("✓ Indexed {} edges", count);
        }
    }
    Ok(())
}

/// Parse privacy scoring JSON array response from AI
fn parse_privacy_scoring_response(text: &str) -> Result<Vec<PrivacyScoreResult>, String> {
    let text = text.trim();

    // Handle markdown code blocks
    let json_text = if text.starts_with("```") {
        text.lines()
            .skip(1)
            .take_while(|line| !line.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.to_string()
    };

    // Find array start/end
    let start = json_text.find('[').ok_or("No JSON array found")?;
    let end = json_text.rfind(']').ok_or("No JSON array end found")?;
    let array_text = &json_text[start..=end];

    // Fix common AI JSON issues: trailing commas before ]
    let mut cleaned = array_text.to_string();
    while cleaned.contains(",]") || cleaned.contains(", ]") || cleaned.contains(",\n]") || cleaned.contains(",\r\n]") {
        cleaned = cleaned
            .replace(",\r\n]", "]")
            .replace(",\n]", "]")
            .replace(", ]", "]")
            .replace(",]", "]");
    }
    // Handle comma followed by whitespace until ]
    if let Some(last_comma) = cleaned.rfind(',') {
        let after_comma = &cleaned[last_comma + 1..];
        if after_comma.trim() == "]" {
            cleaned = format!("{}]", &cleaned[..last_comma]);
        }
    }

    serde_json::from_str::<Vec<PrivacyScoreResult>>(&cleaned)
        .map_err(|e| format!("JSON parse error: {}", e))
}

fn confirm_action(action: &str) -> Result<bool, String> {
    println!("\n⚠️  Are you sure you want to {}?", action);
    print!("Type 'yes' to confirm: ");
    std::io::stdout().flush().ok();

    let mut input = String::new();
    std::io::stdin().read_line(&mut input).map_err(|e| e.to_string())?;

    Ok(input.trim() == "yes")
}

// ============================================================================
// Export Commands
// ============================================================================

async fn handle_export(cmd: ExportCommands, db: &Database, quiet: bool) -> Result<(), String> {
    match cmd {
        ExportCommands::Bibtex { output, node, subtree } => {
            let nodes = get_export_nodes(db, node.as_deref(), subtree)?;
            let papers: Vec<_> = nodes.iter()
                .filter(|n| n.source.as_deref() == Some("openaire"))
                .collect();

            if papers.is_empty() {
                return Err("No papers found to export".into());
            }

            let mut bibtex = String::new();
            for paper in &papers {
                let key = format!("paper_{}", &paper.id[..8]);
                let title = paper.ai_title.as_ref().unwrap_or(&paper.title);
                let year = paper.created_at / 31536000 + 1970; // Rough year from timestamp

                bibtex.push_str(&format!(
                    "@article{{{},\n  title = {{{}}},\n  year = {{{}}},\n",
                    key, title, year
                ));

                if let Some(ref tags) = paper.tags {
                    bibtex.push_str(&format!("  keywords = {{{}}},\n", tags));
                }
                if let Some(ref summary) = paper.summary {
                    let abstract_text = summary.replace('\n', " ").replace('{', "\\{").replace('}', "\\}");
                    bibtex.push_str(&format!("  abstract = {{{}}},\n", abstract_text));
                }
                bibtex.push_str("}\n\n");
            }

            std::fs::write(&output, bibtex).map_err(|e| e.to_string())?;
            if !quiet {
                println!("✓ Exported {} papers to {}", papers.len(), output);
            }
        }

        ExportCommands::Markdown { output, node, subtree, full } => {
            let nodes = get_export_nodes(db, node.as_deref(), subtree)?;

            let mut markdown = String::new();
            markdown.push_str("# Mycelica Export\n\n");

            for node in &nodes {
                let emoji = node.emoji.as_deref().unwrap_or("");
                let title = node.ai_title.as_ref().unwrap_or(&node.title);
                let level = "#".repeat((node.depth + 1).min(6) as usize);

                markdown.push_str(&format!("{} {} {}\n\n", level, emoji, title));

                if let Some(ref tags) = node.tags {
                    markdown.push_str(&format!("**Tags:** {}\n\n", tags));
                }

                if let Some(ref summary) = node.summary {
                    markdown.push_str(&format!("{}\n\n", summary));
                }

                if full {
                    if let Some(ref content) = node.content {
                        markdown.push_str("---\n\n");
                        markdown.push_str(content);
                        markdown.push_str("\n\n");
                    }
                }

                markdown.push_str("---\n\n");
            }

            std::fs::write(&output, markdown).map_err(|e| e.to_string())?;
            if !quiet {
                println!("✓ Exported {} nodes to {}", nodes.len(), output);
            }
        }

        ExportCommands::Json { output, node, subtree, pretty } => {
            let nodes = get_export_nodes(db, node.as_deref(), subtree)?;

            let json_nodes: Vec<serde_json::Value> = nodes.iter().map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "title": n.ai_title.as_ref().unwrap_or(&n.title),
                    "emoji": n.emoji,
                    "depth": n.depth,
                    "is_item": n.is_item,
                    "tags": n.tags,
                    "summary": n.summary,
                    "content": n.content,
                    "created_at": n.created_at,
                    "source": n.source,
                })
            }).collect();

            let json_str = if pretty {
                serde_json::to_string_pretty(&json_nodes).map_err(|e| e.to_string())?
            } else {
                serde_json::to_string(&json_nodes).map_err(|e| e.to_string())?
            };

            std::fs::write(&output, json_str).map_err(|e| e.to_string())?;
            if !quiet {
                println!("✓ Exported {} nodes to {}", nodes.len(), output);
            }
        }

        ExportCommands::Graph { output, format, node, edges } => {
            let nodes = get_export_nodes(db, node.as_deref(), true)?;

            let graph_str = match format.as_str() {
                "dot" => export_dot(&nodes, db, edges)?,
                "graphml" => export_graphml(&nodes, db, edges)?,
                _ => return Err(format!("Unknown format: {}. Use 'dot' or 'graphml'", format)),
            };

            std::fs::write(&output, graph_str).map_err(|e| e.to_string())?;
            if !quiet {
                println!("✓ Exported graph ({} nodes) to {}", nodes.len(), output);
            }
        }

        ExportCommands::Subgraph { node, output, depth } => {
            // For subgraph export, we'd need to create a new database
            // This is a simplified version that exports to JSON
            let root = db.get_node(&node)
                .map_err(|e| e.to_string())?
                .ok_or("Node not found")?;

            let mut all_nodes = vec![root];
            collect_descendants(db, &node, depth, &mut all_nodes)?;

            let json_nodes: Vec<serde_json::Value> = all_nodes.iter().map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "parent_id": n.parent_id,
                    "title": n.title,
                    "ai_title": n.ai_title,
                    "emoji": n.emoji,
                    "depth": n.depth,
                    "is_item": n.is_item,
                    "tags": n.tags,
                    "summary": n.summary,
                    "content": n.content,
                })
            }).collect();

            let json_str = serde_json::to_string_pretty(&json_nodes).map_err(|e| e.to_string())?;
            std::fs::write(&output, json_str).map_err(|e| e.to_string())?;
            if !quiet {
                println!("✓ Exported subgraph ({} nodes, depth {}) to {}", all_nodes.len(), depth, output);
            }
        }
    }

    Ok(())
}

fn get_export_nodes(db: &Database, node_id: Option<&str>, subtree: bool) -> Result<Vec<Node>, String> {
    match node_id {
        Some(id) => {
            let root = db.get_node(id)
                .map_err(|e| e.to_string())?
                .ok_or("Node not found")?;

            if subtree {
                let mut nodes = vec![root];
                collect_descendants(db, id, 100, &mut nodes)?;
                Ok(nodes)
            } else {
                Ok(vec![root])
            }
        }
        None => {
            // Export all items
            db.get_items().map_err(|e| e.to_string())
        }
    }
}

fn collect_descendants(db: &Database, parent_id: &str, max_depth: usize, nodes: &mut Vec<Node>) -> Result<(), String> {
    if max_depth == 0 {
        return Ok(());
    }

    let children = db.get_children(parent_id).map_err(|e| e.to_string())?;
    for child in children {
        let child_id = child.id.clone();
        nodes.push(child);
        collect_descendants(db, &child_id, max_depth - 1, nodes)?;
    }

    Ok(())
}

// ============================================================================
// Analyze Commands
// ============================================================================

async fn handle_analyze(cmd: AnalyzeCommands, db: &Database, json: bool, quiet: bool) -> Result<(), String> {
    match cmd {
        AnalyzeCommands::CodeEdges { path, dry_run } => {
            analyze_code_edges(db, path.as_deref(), dry_run, json, quiet)?;
        }
    }
    Ok(())
}

/// Analyze code and create "Calls" edges between functions.
/// Uses simple heuristic: find identifiers in function bodies that match known function names.
fn analyze_code_edges(
    db: &Database,
    path_filter: Option<&str>,
    dry_run: bool,
    json: bool,
    quiet: bool,
) -> Result<(), String> {
    use std::collections::{HashMap, HashSet};
    use mycelica_lib::db::EdgeType;

    // Get all code function nodes
    let all_nodes = db.get_items().map_err(|e| e.to_string())?;
    let code_functions: Vec<_> = all_nodes
        .iter()
        .filter(|n| {
            n.content_type.as_deref() == Some("code_function")
                && n.source.as_deref().map(|s| s.starts_with("code-")).unwrap_or(false)
        })
        .filter(|n| {
            // Apply path filter if provided
            if let Some(filter) = path_filter {
                if let Some(ref tags) = n.tags {
                    tags.contains(filter)
                } else {
                    false
                }
            } else {
                true
            }
        })
        .collect();

    if !quiet {
        eprintln!("[Analyze] Found {} functions to analyze", code_functions.len());
    }

    // Build a map of function name -> node ID
    // Extract function name from title (e.g., "pub fn foo_bar(...)" -> "foo_bar")
    let mut name_to_id: HashMap<String, String> = HashMap::new();
    for func in &code_functions {
        if let Some(name) = extract_function_name(&func.title) {
            name_to_id.insert(name, func.id.clone());
        }
    }

    if !quiet {
        eprintln!("[Analyze] Built index of {} function names", name_to_id.len());
    }

    // Get existing edges to avoid duplicates
    let existing_edges: HashSet<(String, String)> = db
        .get_all_edges()
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.edge_type == EdgeType::Calls)
        .map(|e| (e.source, e.target))
        .collect();

    let mut edges_created = 0;
    let mut edges_found = Vec::new();

    // For each function, scan its body for calls to other functions
    for func in &code_functions {
        let content = match &func.content {
            Some(c) => c,
            None => continue,
        };

        let caller_id = &func.id;
        let caller_name = extract_function_name(&func.title).unwrap_or_default();

        // Find all identifiers in the content that match known function names
        let called_names = find_called_functions(content, &name_to_id);

        for called_name in called_names {
            // Don't create self-edges
            if called_name == caller_name {
                continue;
            }

            if let Some(callee_id) = name_to_id.get(&called_name) {
                // Skip if edge already exists
                if existing_edges.contains(&(caller_id.clone(), callee_id.clone())) {
                    continue;
                }

                edges_found.push((caller_name.clone(), called_name.clone(), caller_id.clone(), callee_id.clone()));

                if !dry_run {
                    let edge = mycelica_lib::db::Edge {
                        id: format!("calls-{}-{}", &caller_id[..8.min(caller_id.len())], &callee_id[..8.min(callee_id.len())]),
                        source: caller_id.clone(),
                        target: callee_id.clone(),
                        edge_type: EdgeType::Calls,
                        label: Some(format!("{} -> {}", caller_name, called_name)),
                        weight: Some(1.0),
                        edge_source: Some("code-analysis".to_string()),
                        evidence_id: None,
                        confidence: Some(0.8), // Heuristic confidence
                        created_at: chrono::Utc::now().timestamp_millis(),
                    };

                    if db.insert_edge(&edge).is_ok() {
                        edges_created += 1;
                    }
                }
            }
        }
    }

    // Output results
    if json {
        let result = serde_json::json!({
            "functions_analyzed": code_functions.len(),
            "edges_found": edges_found.len(),
            "edges_created": edges_created,
            "dry_run": dry_run,
        });
        println!("{}", serde_json::to_string(&result).unwrap());
    } else {
        println!("Analyzed {} functions", code_functions.len());
        println!("Found {} call relationships", edges_found.len());

        if dry_run {
            println!("\nDry run - edges that would be created:");
            for (caller, callee, _, _) in edges_found.iter().take(20) {
                println!("  {} -> {}", caller, callee);
            }
            if edges_found.len() > 20 {
                println!("  ... and {} more", edges_found.len() - 20);
            }
        } else {
            println!("Created {} new Calls edges", edges_created);
        }
    }

    Ok(())
}

/// Extract function name from a title like "pub fn foo_bar(...)" or "fn baz<T>(...)"
fn extract_function_name(title: &str) -> Option<String> {
    // Look for "fn name" pattern
    let fn_idx = title.find("fn ")?;
    let after_fn = &title[fn_idx + 3..];

    // Find the end of the function name (first non-identifier char)
    let name_end = after_fn
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(after_fn.len());

    let name = &after_fn[..name_end];
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Find function names that appear to be called in the given code content.
/// Uses simple heuristic: identifier followed by '(' that matches a known function name.
fn find_called_functions(content: &str, known_functions: &std::collections::HashMap<String, String>) -> Vec<String> {
    let mut called = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Simple tokenization: find word boundaries and check if followed by '('
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Skip non-identifier starts
        if !chars[i].is_alphabetic() && chars[i] != '_' {
            i += 1;
            continue;
        }

        // Collect identifier
        let start = i;
        while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
            i += 1;
        }
        let ident: String = chars[start..i].iter().collect();

        // Skip whitespace
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }

        // Check if followed by '(' - indicates a function call
        if i < chars.len() && chars[i] == '(' {
            // Also accept method calls like .foo() and turbofish foo::<T>()
            if known_functions.contains_key(&ident) && !seen.contains(&ident) {
                called.push(ident.clone());
                seen.insert(ident);
            }
        }

        // Skip any colons for turbofish
        while i < chars.len() && (chars[i] == ':' || chars[i] == '<') {
            // Skip through generic params
            if chars[i] == '<' {
                let mut depth = 1;
                i += 1;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '<' { depth += 1; }
                    if chars[i] == '>' { depth -= 1; }
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
    }

    called
}

// ============================================================================
// Code Commands
// ============================================================================

async fn handle_code(cmd: CodeCommands, db: &Database, db_path: &PathBuf) -> Result<(), String> {
    match cmd {
        CodeCommands::Show { id } => {
            // Get the node
            let node = db.get_node(&id).map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Node not found: {}", id))?;

            // Parse tags JSON to get file_path, line_start, line_end
            let tags = node.tags.as_ref()
                .ok_or("Node has no tags metadata")?;

            #[derive(serde::Deserialize)]
            struct CodeMetadata {
                file_path: String,
                line_start: usize,
                line_end: usize,
            }

            let metadata: CodeMetadata = serde_json::from_str(tags)
                .map_err(|e| format!("Failed to parse tags JSON: {}. Tags: {}", e, tags))?;

            // Resolve file path - if relative, resolve from database directory
            let file_path = PathBuf::from(&metadata.file_path);
            let file_path = if file_path.is_relative() {
                // Get the directory containing the database
                let db_dir = db_path.parent().unwrap_or(Path::new("."));
                db_dir.join(&file_path)
            } else {
                file_path
            };

            // Read the actual file
            let file_content = std::fs::read_to_string(&file_path)
                .map_err(|e| format!("Failed to read file '{}': {}", file_path.display(), e))?;

            let lines: Vec<&str> = file_content.lines().collect();

            // Validate line range
            if metadata.line_start == 0 || metadata.line_start > lines.len() {
                return Err(format!("Invalid line_start: {} (file has {} lines)", metadata.line_start, lines.len()));
            }
            let line_end = metadata.line_end.min(lines.len());

            // Extract the code range (1-indexed to 0-indexed)
            let code_lines = &lines[metadata.line_start - 1..line_end];

            // Print header
            println!("=== {} ===", node.title);
            println!("File: {}", metadata.file_path);
            println!("Lines: {}-{}", metadata.line_start, line_end);
            println!();

            // Print code with line numbers
            for (i, line) in code_lines.iter().enumerate() {
                let line_num = metadata.line_start + i;
                println!("{:4} | {}", line_num, line);
            }
        }
    }
    Ok(())
}

fn export_dot(nodes: &[Node], db: &Database, include_edges: bool) -> Result<String, String> {
    let mut dot = String::from("digraph Mycelica {\n");
    dot.push_str("  rankdir=TB;\n");
    dot.push_str("  node [shape=box, style=rounded];\n\n");

    // Add nodes
    for node in nodes {
        let label = node.ai_title.as_ref().unwrap_or(&node.title);
        let label_escaped = label.replace('"', "\\\"");
        let color = if node.is_item { "lightblue" } else { "lightyellow" };
        dot.push_str(&format!(
            "  \"{}\" [label=\"{}\", fillcolor={}, style=filled];\n",
            &node.id[..8], label_escaped, color
        ));
    }

    dot.push_str("\n");

    // Add hierarchy edges
    for node in nodes {
        if let Some(ref parent_id) = node.parent_id {
            // Check if parent is in our export set
            if nodes.iter().any(|n| n.id == *parent_id) {
                dot.push_str(&format!(
                    "  \"{}\" -> \"{}\" [color=gray];\n",
                    &parent_id[..8.min(parent_id.len())], &node.id[..8]
                ));
            }
        }
    }

    // Add similarity edges if requested
    if include_edges {
        let node_ids: std::collections::HashSet<_> = nodes.iter().map(|n| n.id.as_str()).collect();

        for node in nodes {
            if let Ok(edges) = db.get_edges_for_node(&node.id) {
                for edge in edges {
                    let other_id = if edge.source == node.id { &edge.target } else { &edge.source };
                    if node_ids.contains(other_id.as_str()) && edge.source == node.id {
                        let weight = edge.weight.unwrap_or(0.0);
                        if weight > 0.7 {
                            dot.push_str(&format!(
                                "  \"{}\" -> \"{}\" [color=red, style=dashed, label=\"{:.0}%\"];\n",
                                &node.id[..8], &other_id[..8.min(other_id.len())], weight * 100.0
                            ));
                        }
                    }
                }
            }
        }
    }

    dot.push_str("}\n");
    Ok(dot)
}

fn export_graphml(nodes: &[Node], db: &Database, include_edges: bool) -> Result<String, String> {
    let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?>
<graphml xmlns="http://graphml.graphdrawing.org/xmlns">
  <key id="label" for="node" attr.name="label" attr.type="string"/>
  <key id="type" for="node" attr.name="type" attr.type="string"/>
  <key id="weight" for="edge" attr.name="weight" attr.type="double"/>
  <graph id="G" edgedefault="directed">
"#);

    // Add nodes
    for node in nodes {
        let label = node.ai_title.as_ref().unwrap_or(&node.title);
        let label_escaped = label.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
        let node_type = if node.is_item { "item" } else { "category" };
        xml.push_str(&format!(
            "    <node id=\"{}\">\n      <data key=\"label\">{}</data>\n      <data key=\"type\">{}</data>\n    </node>\n",
            &node.id[..8], label_escaped, node_type
        ));
    }

    // Add hierarchy edges
    let mut edge_id = 0;
    for node in nodes {
        if let Some(ref parent_id) = node.parent_id {
            if nodes.iter().any(|n| n.id == *parent_id) {
                xml.push_str(&format!(
                    "    <edge id=\"e{}\" source=\"{}\" target=\"{}\"/>\n",
                    edge_id, &parent_id[..8.min(parent_id.len())], &node.id[..8]
                ));
                edge_id += 1;
            }
        }
    }

    // Add similarity edges if requested
    if include_edges {
        let node_ids: std::collections::HashSet<_> = nodes.iter().map(|n| n.id.as_str()).collect();

        for node in nodes {
            if let Ok(edges) = db.get_edges_for_node(&node.id) {
                for edge in edges {
                    let other_id = if edge.source == node.id { &edge.target } else { &edge.source };
                    if node_ids.contains(other_id.as_str()) && edge.source == node.id {
                        let weight = edge.weight.unwrap_or(0.0);
                        if weight > 0.7 {
                            xml.push_str(&format!(
                                "    <edge id=\"e{}\" source=\"{}\" target=\"{}\">\n      <data key=\"weight\">{:.3}</data>\n    </edge>\n",
                                edge_id, &node.id[..8], &other_id[..8.min(other_id.len())], weight
                            ));
                            edge_id += 1;
                        }
                    }
                }
            }
        }
    }

    xml.push_str("  </graph>\n</graphml>\n");
    Ok(xml)
}

// ============================================================================
// TUI Mode
// ============================================================================

/// TUI operating mode
#[derive(Clone, Copy, PartialEq)]
enum TuiMode {
    Navigation,  // Browsing hierarchy (tree view)
    LeafView,    // Viewing item content (full screen)
    Edit,        // Editing content
    Search,      // Search mode
    Maintenance, // Maintenance menu
    Settings,    // Settings screen
    Jobs,        // Job status popup
}

/// Focus state for Navigation mode (3-column layout)
#[derive(Clone, Copy, PartialEq)]
enum NavFocus {
    Tree,
    Pins,
    Recents,
}

/// Focus state for Leaf View mode
#[derive(Clone, Copy, PartialEq)]
enum LeafFocus {
    Content,
    Similar,
    Calls,  // Only shown for code nodes
}

/// Tree node for TUI display
#[derive(Clone)]
struct TreeNode {
    id: String,
    parent_id: Option<String>,
    title: String,
    emoji: Option<String>,
    depth: i32,
    child_count: i32,
    is_item: bool,
    is_expanded: bool,
    is_universe: bool,
    children_loaded: bool,
    created_at: i64,
    latest_child_date: Option<i64>,
}

/// Similar node for leaf view sidebar
#[derive(Clone)]
struct SimilarNodeInfo {
    id: String,
    title: String,
    emoji: Option<String>,
    similarity: f32,
    parent_title: Option<String>,
}

/// TUI Application state
struct TuiApp {
    // Mode
    mode: TuiMode,

    // Tree data
    nodes: Vec<TreeNode>,
    visible_nodes: Vec<usize>,  // Indices into nodes that are currently visible
    list_state: ListState,

    // CD-style navigation
    current_root_id: String,        // Current "directory" being viewed
    breadcrumb_path: Vec<(String, String)>,  // (id, title) pairs from Universe to current

    // Navigation mode focus (which pane is active)
    nav_focus: NavFocus,
    pins_selected: usize,
    recents_selected: usize,

    // Selected node in navigation
    selected_node: Option<Node>,

    // Leaf view state
    leaf_node_id: Option<String>,
    leaf_content: Option<String>,
    leaf_scroll_offset: u16,
    leaf_focus: LeafFocus,  // Which section is focused: Content, Similar, or Edges

    // Similar nodes (loaded on leaf view entry)
    similar_nodes: Vec<SimilarNodeInfo>,
    similar_selected: usize,

    // Calls for current leaf (only for code nodes)
    calls_for_node: Vec<(String, String, bool)>,  // (target_id, title, is_outgoing)
    calls_selected: usize,
    is_code_node: bool,  // Whether current leaf is a code node

    // Pins and recents
    pinned_nodes: Vec<Node>,
    recent_nodes: Vec<Node>,

    // Search
    search_mode: bool,
    search_query: String,
    search_results: Vec<Node>,

    // Edit mode
    edit_buffer: String,
    edit_cursor_line: usize,
    edit_cursor_col: usize,
    edit_scroll_offset: usize,
    edit_dirty: bool,  // Track if content has been modified

    // Status
    status_message: String,

    // Date range for color gradient
    date_min: i64,
    date_max: i64,
}

impl TuiApp {
    fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            mode: TuiMode::Navigation,
            nodes: Vec::new(),
            visible_nodes: Vec::new(),
            list_state,
            current_root_id: String::new(),
            breadcrumb_path: Vec::new(),
            nav_focus: NavFocus::Tree,
            pins_selected: 0,
            recents_selected: 0,
            selected_node: None,
            leaf_node_id: None,
            leaf_content: None,
            leaf_scroll_offset: 0,
            leaf_focus: LeafFocus::Content,
            similar_nodes: Vec::new(),
            similar_selected: 0,
            calls_for_node: Vec::new(),
            calls_selected: 0,
            is_code_node: false,
            pinned_nodes: Vec::new(),
            recent_nodes: Vec::new(),
            search_mode: false,
            search_query: String::new(),
            search_results: Vec::new(),
            edit_buffer: String::new(),
            edit_cursor_line: 0,
            edit_cursor_col: 0,
            edit_scroll_offset: 0,
            edit_dirty: false,
            status_message: String::new(),
            date_min: 0,
            date_max: i64::MAX,
        }
    }

    fn load_tree(&mut self, db: &Database) -> Result<(), String> {
        self.nodes.clear();
        self.visible_nodes.clear();

        // Get universe as root
        if let Some(universe) = db.get_universe().map_err(|e| e.to_string())? {
            // If no current root, start at Universe
            if self.current_root_id.is_empty() {
                self.current_root_id = universe.id.clone();
                self.breadcrumb_path = vec![(universe.id.clone(), "Universe".to_string())];
            }

            // Load children of current root directly (flat list, not tree)
            let children = db.get_children(&self.current_root_id).map_err(|e| e.to_string())?;

            // Calculate date range for color gradient (use effective dates)
            // Exclude Recent Notes from date range calculation (it skews the gradient)
            let recent_notes_id = settings::RECENT_NOTES_CONTAINER_ID;
            if !children.is_empty() {
                self.date_min = children.iter()
                    .filter(|n| n.id != recent_notes_id)
                    .map(|n| n.latest_child_date.unwrap_or(n.created_at))
                    .min()
                    .unwrap_or(0);
                self.date_max = children.iter()
                    .filter(|n| n.id != recent_notes_id)
                    .map(|n| n.latest_child_date.unwrap_or(n.created_at))
                    .max()
                    .unwrap_or(i64::MAX);
            }

            for child in children {
                self.nodes.push(TreeNode {
                    id: child.id.clone(),
                    parent_id: Some(self.current_root_id.clone()),
                    title: child.ai_title.clone().unwrap_or(child.title.clone()),
                    emoji: child.emoji.clone(),
                    depth: 0,  // Relative depth from current root
                    child_count: child.child_count,
                    is_item: child.is_item,
                    is_expanded: false,
                    is_universe: false,
                    children_loaded: false,
                    created_at: child.created_at,
                    latest_child_date: child.latest_child_date,
                });
            }
        }

        self.update_visible_nodes();

        // Load pins and recents
        self.pinned_nodes = db.get_pinned_nodes().unwrap_or_default();
        self.recent_nodes = db.get_recent_nodes(10).unwrap_or_default();

        Ok(())
    }

    /// CD into a cluster (make it the new root)
    fn cd_into(&mut self, db: &Database, node_id: &str) -> Result<(), String> {
        let node = db.get_node(node_id)
            .map_err(|e| e.to_string())?
            .ok_or("Node not found")?;

        // Add to breadcrumb
        let title = node.ai_title.clone().unwrap_or(node.title.clone());
        self.breadcrumb_path.push((node_id.to_string(), title));

        // Set as new root
        self.current_root_id = node_id.to_string();

        // Reload tree from new root
        self.nodes.clear();
        self.list_state.select(Some(0));

        let children = db.get_children(node_id).map_err(|e| e.to_string())?;

        // Update date range (use effective dates)
        // Exclude Recent Notes from date range calculation (it skews the gradient)
        let recent_notes_id = settings::RECENT_NOTES_CONTAINER_ID;
        if !children.is_empty() {
            self.date_min = children.iter()
                .filter(|n| n.id != recent_notes_id)
                .map(|n| n.latest_child_date.unwrap_or(n.created_at))
                .min()
                .unwrap_or(0);
            self.date_max = children.iter()
                .filter(|n| n.id != recent_notes_id)
                .map(|n| n.latest_child_date.unwrap_or(n.created_at))
                .max()
                .unwrap_or(i64::MAX);
        }

        for child in children {
            self.nodes.push(TreeNode {
                id: child.id.clone(),
                parent_id: Some(node_id.to_string()),
                title: child.ai_title.clone().unwrap_or(child.title.clone()),
                emoji: child.emoji.clone(),
                depth: 0,
                child_count: child.child_count,
                is_item: child.is_item,
                is_expanded: false,
                is_universe: false,
                children_loaded: false,
                created_at: child.created_at,
                latest_child_date: child.latest_child_date,
            });
        }

        self.update_visible_nodes();
        self.status_message = format!("Entered {} ({} items)",
            self.breadcrumb_path.last().map(|(_, t)| t.as_str()).unwrap_or("?"),
            self.nodes.len()
        );
        Ok(())
    }

    /// Go up one level (cd ..)
    fn cd_up(&mut self, db: &Database) -> Result<(), String> {
        if self.breadcrumb_path.len() <= 1 {
            self.status_message = "Already at root".to_string();
            return Ok(());
        }

        // Remove current from breadcrumb
        self.breadcrumb_path.pop();

        // Get parent ID
        let parent_id = self.breadcrumb_path.last()
            .map(|(id, _)| id.clone())
            .unwrap_or_default();

        self.current_root_id = parent_id.clone();

        // Reload tree from parent
        self.nodes.clear();
        self.list_state.select(Some(0));

        let children = db.get_children(&parent_id).map_err(|e| e.to_string())?;

        for child in children {
            self.nodes.push(TreeNode {
                id: child.id.clone(),
                parent_id: Some(parent_id.clone()),
                title: child.ai_title.clone().unwrap_or(child.title.clone()),
                emoji: child.emoji.clone(),
                depth: 0,
                child_count: child.child_count,
                is_item: child.is_item,
                is_expanded: false,
                is_universe: false,
                children_loaded: false,
                created_at: child.created_at,
                latest_child_date: child.latest_child_date,
            });
        }

        self.update_visible_nodes();
        self.status_message = format!("Back to {} ({} items)",
            self.breadcrumb_path.last().map(|(_, t)| t.as_str()).unwrap_or("Universe"),
            self.nodes.len()
        );
        Ok(())
    }

    /// Enter leaf view mode for an item
    fn enter_leaf_view(&mut self, db: &Database, node_id: &str) -> Result<(), String> {
        let node = db.get_node(node_id)
            .map_err(|e| e.to_string())?
            .ok_or("Node not found")?;

        self.mode = TuiMode::LeafView;
        self.leaf_node_id = Some(node_id.to_string());
        self.leaf_content = node.content.clone();
        self.leaf_scroll_offset = 0;
        self.leaf_focus = LeafFocus::Content;
        self.calls_selected = 0;
        self.similar_nodes.clear();
        self.similar_selected = 0;

        // Check if this is a code node
        self.is_code_node = node.content_type.as_ref().map(|ct| ct.starts_with("code_")).unwrap_or(false);

        // Store selected node for header display
        self.selected_node = Some(node.clone());

        // Load similar nodes using embeddings
        if let Some(target_emb) = db.get_node_embedding(node_id).ok().flatten() {
            if let Ok(all_embeddings) = db.get_nodes_with_embeddings() {
                let similar = similarity::find_similar(&target_emb, &all_embeddings, node_id, 15, 0.5);

                for (sim_id, score) in similar {
                    if let Ok(Some(sim_node)) = db.get_node(&sim_id) {
                        // Get parent title for grouping display
                        let parent_title = if let Some(ref pid) = sim_node.parent_id {
                            db.get_node(pid).ok().flatten().map(|p| p.ai_title.unwrap_or(p.title))
                        } else {
                            None
                        };

                        self.similar_nodes.push(SimilarNodeInfo {
                            id: sim_id,
                            title: sim_node.ai_title.unwrap_or(sim_node.title),
                            emoji: sim_node.emoji,
                            similarity: score,
                            parent_title,
                        });
                    }
                }
            }
        }

        // Load Calls edges for code nodes only
        self.calls_for_node.clear();
        if self.is_code_node {
            if let Ok(edges) = db.get_edges_for_node(node_id) {
                for edge in edges {
                    // Only process Calls edges
                    if edge.edge_type != EdgeType::Calls {
                        continue;
                    }
                    // Determine direction: outgoing if this node is source
                    let is_outgoing = edge.source == node_id;
                    let other_id = if is_outgoing { &edge.target } else { &edge.source };
                    if let Ok(Some(other_node)) = db.get_node(other_id) {
                        let title = other_node.ai_title.unwrap_or(other_node.title);
                        self.calls_for_node.push((other_id.to_string(), title, is_outgoing));
                    }
                }
            }
        }

        // Touch node to update recent
        let _ = db.touch_node(node_id);

        // Reload recents
        self.recent_nodes = db.get_recent_nodes(10).unwrap_or_default();

        self.status_message = format!("Viewing: {} ({} similar) [q/Esc to go back]",
            node.ai_title.unwrap_or(node.title),
            self.similar_nodes.len());
        Ok(())
    }

    /// Exit leaf view, return to navigation
    fn exit_leaf_view(&mut self) {
        self.mode = TuiMode::Navigation;
        self.leaf_node_id = None;
        self.leaf_content = None;
        self.similar_nodes.clear();
        self.calls_for_node.clear();
        self.is_code_node = false;
        self.status_message = "Back to navigation".to_string();
    }

    /// Enter edit mode from leaf view
    fn enter_edit_mode(&mut self) {
        if let Some(ref content) = self.leaf_content {
            self.edit_buffer = content.clone();
        } else {
            self.edit_buffer = String::new();
        }
        self.edit_cursor_line = 0;
        self.edit_cursor_col = 0;
        self.edit_scroll_offset = 0;
        self.edit_dirty = false;
        self.mode = TuiMode::Edit;
        self.status_message = "Edit mode: Ctrl+S save, Esc cancel".to_string();
    }

    /// Save edited content and return to leaf view
    fn save_edit(&mut self, db: &Database) -> Result<(), String> {
        if let Some(ref node_id) = self.leaf_node_id {
            db.update_node_content(node_id, &self.edit_buffer)
                .map_err(|e| e.to_string())?;

            // Update the leaf content with saved buffer
            self.leaf_content = Some(self.edit_buffer.clone());
            self.leaf_scroll_offset = 0;
            self.mode = TuiMode::LeafView;
            self.edit_dirty = false;
            self.status_message = "Content saved".to_string();
            Ok(())
        } else {
            Err("No node to save".to_string())
        }
    }

    /// Cancel edit and return to leaf view
    fn cancel_edit(&mut self) {
        let was_dirty = self.edit_dirty;
        self.mode = TuiMode::LeafView;
        self.edit_buffer.clear();
        self.edit_dirty = false;
        self.status_message = if was_dirty {
            "Edit cancelled (changes discarded)".to_string()
        } else {
            "Edit cancelled".to_string()
        };
    }

    /// Get the lines of the edit buffer
    fn edit_lines(&self) -> Vec<&str> {
        self.edit_buffer.lines().collect()
    }

    /// Get total line count in edit buffer
    fn edit_line_count(&self) -> usize {
        self.edit_buffer.lines().count().max(1)
    }

    /// Get the current line content
    fn current_edit_line(&self) -> &str {
        self.edit_buffer.lines().nth(self.edit_cursor_line).unwrap_or("")
    }

    /// Insert a character at cursor position
    fn edit_insert_char(&mut self, c: char) {
        let byte_pos = self.cursor_byte_position();
        self.edit_buffer.insert(byte_pos, c);
        if c == '\n' {
            self.edit_cursor_line += 1;
            self.edit_cursor_col = 0;
        } else {
            self.edit_cursor_col += 1;
        }
        self.edit_dirty = true;
    }

    /// Delete character before cursor (backspace)
    fn edit_backspace(&mut self) {
        if self.edit_cursor_col > 0 {
            // Delete character before cursor on current line
            let byte_pos = self.cursor_byte_position();
            if byte_pos > 0 {
                // Find the byte position of the previous character
                let prev_char_start = self.edit_buffer[..byte_pos]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.edit_buffer.remove(prev_char_start);
                self.edit_cursor_col = self.edit_cursor_col.saturating_sub(1);
                self.edit_dirty = true;
            }
        } else if self.edit_cursor_line > 0 {
            // At start of line, merge with previous line
            let byte_pos = self.cursor_byte_position();
            if byte_pos > 0 {
                // Remove the newline before current position
                self.edit_buffer.remove(byte_pos - 1);
                self.edit_cursor_line -= 1;
                // Set cursor to end of the now-merged line
                self.edit_cursor_col = self.edit_buffer
                    .lines()
                    .nth(self.edit_cursor_line)
                    .map(|l| l.chars().count())
                    .unwrap_or(0);
                self.edit_dirty = true;
            }
        }
    }

    /// Delete character at cursor (delete key)
    fn edit_delete(&mut self) {
        let byte_pos = self.cursor_byte_position();
        if byte_pos < self.edit_buffer.len() {
            self.edit_buffer.remove(byte_pos);
            self.edit_dirty = true;
        }
    }

    /// Move cursor left
    fn edit_cursor_left(&mut self) {
        if self.edit_cursor_col > 0 {
            self.edit_cursor_col -= 1;
        } else if self.edit_cursor_line > 0 {
            self.edit_cursor_line -= 1;
            self.edit_cursor_col = self.current_edit_line().chars().count();
        }
    }

    /// Move cursor right
    fn edit_cursor_right(&mut self) {
        let line_len = self.current_edit_line().chars().count();
        if self.edit_cursor_col < line_len {
            self.edit_cursor_col += 1;
        } else if self.edit_cursor_line < self.edit_line_count().saturating_sub(1) {
            self.edit_cursor_line += 1;
            self.edit_cursor_col = 0;
        }
    }

    /// Move cursor up
    fn edit_cursor_up(&mut self) {
        if self.edit_cursor_line > 0 {
            self.edit_cursor_line -= 1;
            // Clamp column to line length
            let line_len = self.current_edit_line().chars().count();
            self.edit_cursor_col = self.edit_cursor_col.min(line_len);
        }
    }

    /// Move cursor down
    fn edit_cursor_down(&mut self) {
        let line_count = self.edit_line_count();
        if self.edit_cursor_line < line_count.saturating_sub(1) {
            self.edit_cursor_line += 1;
            // Clamp column to line length
            let line_len = self.current_edit_line().chars().count();
            self.edit_cursor_col = self.edit_cursor_col.min(line_len);
        }
    }

    /// Move cursor to start of line
    fn edit_cursor_home(&mut self) {
        self.edit_cursor_col = 0;
    }

    /// Move cursor to end of line
    fn edit_cursor_end(&mut self) {
        self.edit_cursor_col = self.current_edit_line().chars().count();
    }

    /// Calculate byte position from line/col
    fn cursor_byte_position(&self) -> usize {
        let mut byte_pos = 0;
        for (line_idx, line) in self.edit_buffer.lines().enumerate() {
            if line_idx == self.edit_cursor_line {
                // Add bytes up to cursor column
                for (col, c) in line.chars().enumerate() {
                    if col >= self.edit_cursor_col {
                        break;
                    }
                    byte_pos += c.len_utf8();
                }
                return byte_pos;
            }
            byte_pos += line.len() + 1; // +1 for newline
        }
        self.edit_buffer.len()
    }

    /// Update scroll offset to keep cursor visible
    fn edit_ensure_cursor_visible(&mut self, visible_lines: usize) {
        if self.edit_cursor_line < self.edit_scroll_offset {
            self.edit_scroll_offset = self.edit_cursor_line;
        } else if self.edit_cursor_line >= self.edit_scroll_offset + visible_lines {
            self.edit_scroll_offset = self.edit_cursor_line.saturating_sub(visible_lines) + 1;
        }
    }

    fn load_children_for_node(&mut self, db: &Database, node_idx: usize) -> Result<(), String> {
        if self.nodes[node_idx].children_loaded {
            return Ok(());
        }

        let parent_id = self.nodes[node_idx].id.clone();
        let children = db.get_children(&parent_id).map_err(|e| e.to_string())?;

        // Insert children right after the parent node
        let insert_pos = node_idx + 1;

        for (i, child) in children.into_iter().enumerate() {
            self.nodes.insert(insert_pos + i, TreeNode {
                id: child.id.clone(),
                parent_id: Some(parent_id.clone()),
                title: child.ai_title.clone().unwrap_or(child.title.clone()),
                emoji: child.emoji.clone(),
                depth: child.depth,
                child_count: child.child_count,
                is_item: child.is_item,
                is_expanded: false,
                is_universe: false,
                children_loaded: false,
                created_at: child.created_at,
                latest_child_date: child.latest_child_date,
            });
        }

        self.nodes[node_idx].children_loaded = true;
        Ok(())
    }

    fn update_visible_nodes(&mut self) {
        self.visible_nodes.clear();

        // With CD-style navigation, all direct children of current_root are at depth 0
        // They are always visible. Only their expanded children need ancestor checking.

        // Build set of expanded node IDs for quick lookup
        let expanded_ids: std::collections::HashSet<String> = self.nodes.iter()
            .filter(|n| n.is_expanded)
            .map(|n| n.id.clone())
            .collect();

        for (idx, node) in self.nodes.iter().enumerate() {
            // Depth 0 nodes are direct children of current root - always visible
            if node.depth == 0 {
                self.visible_nodes.push(idx);
                continue;
            }

            // For deeper nodes, check if all ancestors are expanded
            if self.is_ancestor_chain_expanded(idx, &expanded_ids) {
                self.visible_nodes.push(idx);
            }
        }
    }

    fn is_ancestor_chain_expanded(&self, idx: usize, expanded_ids: &std::collections::HashSet<String>) -> bool {
        let node = &self.nodes[idx];

        // Check if parent is expanded
        if let Some(ref parent_id) = node.parent_id {
            // If parent is the current root, it's implicitly expanded
            if *parent_id == self.current_root_id {
                return true;
            }

            if !expanded_ids.contains(parent_id) {
                return false;
            }
            // Recursively check parent's ancestors
            for (i, n) in self.nodes.iter().enumerate() {
                if n.id == *parent_id {
                    return self.is_ancestor_chain_expanded(i, expanded_ids);
                }
            }
        }
        true
    }

    fn toggle_expand(&mut self, db: &Database) {
        if let Some(selected) = self.list_state.selected() {
            if selected < self.visible_nodes.len() {
                let node_idx = self.visible_nodes[selected];
                let node = &self.nodes[node_idx];

                if !node.is_item && node.child_count > 0 {
                    // Toggle expansion
                    let was_expanded = self.nodes[node_idx].is_expanded;
                    self.nodes[node_idx].is_expanded = !was_expanded;

                    if !was_expanded {
                        // Load children if not already loaded
                        let _ = self.load_children_for_node(db, node_idx);
                    }

                    self.update_visible_nodes();

                    // Adjust selection if needed (visible_nodes may have changed)
                    if selected >= self.visible_nodes.len() {
                        self.list_state.select(Some(self.visible_nodes.len().saturating_sub(1)));
                    }
                }
            }
        }
    }

    fn select_next(&mut self) {
        if let Some(selected) = self.list_state.selected() {
            if selected < self.visible_nodes.len().saturating_sub(1) {
                self.list_state.select(Some(selected + 1));
            }
        }
    }

    fn select_prev(&mut self) {
        if let Some(selected) = self.list_state.selected() {
            if selected > 0 {
                self.list_state.select(Some(selected - 1));
            }
        }
    }

    fn get_selected_node(&self, db: &Database) -> Option<Node> {
        if let Some(selected) = self.list_state.selected() {
            if selected < self.visible_nodes.len() {
                let node_idx = self.visible_nodes[selected];
                let tree_node = &self.nodes[node_idx];
                return db.get_node(&tree_node.id).ok().flatten();
            }
        }
        None
    }
}

async fn run_tui(db: &Database) -> Result<(), String> {
    // Setup terminal
    enable_raw_mode().map_err(|e| e.to_string())?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(|e| e.to_string())?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;

    // Create app and load data
    let mut app = TuiApp::new();
    app.load_tree(db)?;
    app.status_message = format!("Loaded {} nodes. Press ? for help, q to quit.", app.nodes.len());

    // Main loop
    let result = run_tui_loop(&mut terminal, &mut app, db);

    // Restore terminal
    disable_raw_mode().map_err(|e| e.to_string())?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    ).map_err(|e| e.to_string())?;
    terminal.show_cursor().map_err(|e| e.to_string())?;

    result
}

fn run_tui_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut TuiApp,
    db: &Database,
) -> Result<(), String> {
    loop {
        // Update selected node details (only in Navigation mode)
        if app.mode == TuiMode::Navigation && !app.search_mode {
            app.selected_node = app.get_selected_node(db);
        }

        // Draw UI
        terminal.draw(|f| draw_ui(f, app)).map_err(|e| e.to_string())?;

        // Handle input
        if event::poll(std::time::Duration::from_millis(100)).map_err(|e| e.to_string())? {
            if let Event::Key(key) = event::read().map_err(|e| e.to_string())? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Handle search mode (overlay on Navigation)
                if app.search_mode {
                    match key.code {
                        KeyCode::Esc => {
                            app.search_mode = false;
                            app.search_query.clear();
                            app.status_message = "Search cancelled".to_string();
                        }
                        KeyCode::Enter => {
                            app.search_mode = false;
                            // Perform search
                            if !app.search_query.is_empty() {
                                if let Ok(results) = db.search_nodes(&app.search_query) {
                                    app.status_message = format!("Found {} results for '{}'", results.len(), app.search_query);
                                    app.search_results = results.clone();
                                    // Jump to first result if found in current view
                                    if let Some(first) = results.first() {
                                        for (i, &idx) in app.visible_nodes.iter().enumerate() {
                                            if app.nodes[idx].id == first.id {
                                                app.list_state.select(Some(i));
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            app.search_query.clear();
                        }
                        KeyCode::Backspace => {
                            app.search_query.pop();
                        }
                        KeyCode::Char(c) => {
                            app.search_query.push(c);
                        }
                        _ => {}
                    }
                    continue;
                }

                // Handle input based on current mode
                match app.mode {
                    TuiMode::Navigation => {
                        match key.code {
                            KeyCode::Char('q') => return Ok(()),

                            // Tab: Cycle focus between Tree → Pins → Recents
                            KeyCode::Tab => {
                                app.nav_focus = match app.nav_focus {
                                    NavFocus::Tree => NavFocus::Pins,
                                    NavFocus::Pins => NavFocus::Recents,
                                    NavFocus::Recents => NavFocus::Tree,
                                };
                                app.status_message = match app.nav_focus {
                                    NavFocus::Tree => "Focus: Tree".to_string(),
                                    NavFocus::Pins => "Focus: Pins".to_string(),
                                    NavFocus::Recents => "Focus: Recents".to_string(),
                                };
                            }
                            // Shift+Tab: Cycle focus in reverse
                            KeyCode::BackTab => {
                                app.nav_focus = match app.nav_focus {
                                    NavFocus::Tree => NavFocus::Recents,
                                    NavFocus::Pins => NavFocus::Tree,
                                    NavFocus::Recents => NavFocus::Pins,
                                };
                                app.status_message = match app.nav_focus {
                                    NavFocus::Tree => "Focus: Tree".to_string(),
                                    NavFocus::Pins => "Focus: Pins".to_string(),
                                    NavFocus::Recents => "Focus: Recents".to_string(),
                                };
                            }

                            // j/k: Navigate in focused pane
                            KeyCode::Char('j') | KeyCode::Down => {
                                match app.nav_focus {
                                    NavFocus::Tree => app.select_next(),
                                    NavFocus::Pins => {
                                        if !app.pinned_nodes.is_empty() {
                                            app.pins_selected = (app.pins_selected + 1).min(app.pinned_nodes.len() - 1);
                                        }
                                    }
                                    NavFocus::Recents => {
                                        if !app.recent_nodes.is_empty() {
                                            app.recents_selected = (app.recents_selected + 1).min(app.recent_nodes.len() - 1);
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                match app.nav_focus {
                                    NavFocus::Tree => app.select_prev(),
                                    NavFocus::Pins => {
                                        app.pins_selected = app.pins_selected.saturating_sub(1);
                                    }
                                    NavFocus::Recents => {
                                        app.recents_selected = app.recents_selected.saturating_sub(1);
                                    }
                                }
                            }

                            // Enter: Action depends on focused pane
                            KeyCode::Enter => {
                                match app.nav_focus {
                                    NavFocus::Tree => {
                                        if let Some(selected) = app.list_state.selected() {
                                            if selected < app.visible_nodes.len() {
                                                let node_idx = app.visible_nodes[selected];
                                                let node = &app.nodes[node_idx];

                                                if node.is_item {
                                                    let node_id = node.id.clone();
                                                    if let Err(e) = app.enter_leaf_view(db, &node_id) {
                                                        app.status_message = format!("Error: {}", e);
                                                    }
                                                } else if node.child_count > 0 {
                                                    let node_id = node.id.clone();
                                                    if let Err(e) = app.cd_into(db, &node_id) {
                                                        app.status_message = format!("Error: {}", e);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    NavFocus::Pins => {
                                        if app.pins_selected < app.pinned_nodes.len() {
                                            let node = &app.pinned_nodes[app.pins_selected];
                                            let node_id = node.id.clone();
                                            let is_item = node.is_item;
                                            if is_item {
                                                if let Err(e) = app.enter_leaf_view(db, &node_id) {
                                                    app.status_message = format!("Error: {}", e);
                                                }
                                            } else {
                                                if let Err(e) = app.cd_into(db, &node_id) {
                                                    app.status_message = format!("Error: {}", e);
                                                }
                                            }
                                        }
                                    }
                                    NavFocus::Recents => {
                                        if app.recents_selected < app.recent_nodes.len() {
                                            let node = &app.recent_nodes[app.recents_selected];
                                            let node_id = node.id.clone();
                                            let is_item = node.is_item;
                                            if is_item {
                                                if let Err(e) = app.enter_leaf_view(db, &node_id) {
                                                    app.status_message = format!("Error: {}", e);
                                                }
                                            } else {
                                                if let Err(e) = app.cd_into(db, &node_id) {
                                                    app.status_message = format!("Error: {}", e);
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // l/Right: Expand children inline (toggle) - only in Tree focus
                            KeyCode::Char('l') | KeyCode::Right => {
                                if app.nav_focus == NavFocus::Tree {
                                    app.toggle_expand(db);
                                }
                            }

                            // h/Left: Collapse children - only in Tree focus
                            KeyCode::Char('h') | KeyCode::Left => {
                                if app.nav_focus == NavFocus::Tree {
                                    if let Some(selected) = app.list_state.selected() {
                                        if selected < app.visible_nodes.len() {
                                            let node_idx = app.visible_nodes[selected];
                                            if app.nodes[node_idx].is_expanded {
                                                app.nodes[node_idx].is_expanded = false;
                                                app.update_visible_nodes();
                                            }
                                        }
                                    }
                                }
                            }

                            // Backspace/-/Esc: Go up one level (cd ..)
                            KeyCode::Backspace | KeyCode::Char('-') | KeyCode::Esc => {
                                if let Err(e) = app.cd_up(db) {
                                    app.status_message = format!("Error: {}", e);
                                }
                            }

                            KeyCode::Char('/') => {
                                app.search_mode = true;
                                app.search_query.clear();
                                app.status_message = "Search: ".to_string();
                            }
                            KeyCode::Char('?') => {
                                app.status_message = "Tab:focus  Enter:cd/view  l:expand  h:collapse  -:up  /:search  q:quit".to_string();
                            }
                            KeyCode::Char('g') => {
                                match app.nav_focus {
                                    NavFocus::Tree => app.list_state.select(Some(0)),
                                    NavFocus::Pins => app.pins_selected = 0,
                                    NavFocus::Recents => app.recents_selected = 0,
                                }
                            }
                            KeyCode::Char('G') => {
                                match app.nav_focus {
                                    NavFocus::Tree => {
                                        if !app.visible_nodes.is_empty() {
                                            app.list_state.select(Some(app.visible_nodes.len() - 1));
                                        }
                                    }
                                    NavFocus::Pins => {
                                        if !app.pinned_nodes.is_empty() {
                                            app.pins_selected = app.pinned_nodes.len() - 1;
                                        }
                                    }
                                    NavFocus::Recents => {
                                        if !app.recent_nodes.is_empty() {
                                            app.recents_selected = app.recent_nodes.len() - 1;
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('r') => {
                                let _ = app.load_tree(db);
                                app.status_message = format!("Reloaded {} nodes", app.nodes.len());
                            }
                            KeyCode::Char('p') => {
                                // Toggle pin for selected node (works in any focus)
                                if let Some(ref node) = app.selected_node {
                                    let new_pinned = !node.is_pinned;
                                    if db.set_node_pinned(&node.id, new_pinned).is_ok() {
                                        app.pinned_nodes = db.get_pinned_nodes().unwrap_or_default();
                                        app.status_message = if new_pinned {
                                            format!("Pinned: {}", node.ai_title.as_ref().unwrap_or(&node.title))
                                        } else {
                                            format!("Unpinned: {}", node.ai_title.as_ref().unwrap_or(&node.title))
                                        };
                                    }
                                }
                            }
                            _ => {}
                        }
                    }

                    TuiMode::LeafView => {
                        match key.code {
                            // q/Esc: Back to navigation
                            KeyCode::Char('q') | KeyCode::Esc => {
                                app.exit_leaf_view();
                            }

                            // j/k: Scroll content OR navigate in sidebar based on focus
                            KeyCode::Char('j') | KeyCode::Down => {
                                match app.leaf_focus {
                                    LeafFocus::Content => {
                                        // Calculate visual line count for bounds checking
                                        if let Some(content) = &app.leaf_content {
                                            let size = terminal.size().unwrap_or(ratatui::layout::Rect::new(0, 0, 80, 24));
                                            let visible_lines = size.height.saturating_sub(5) as usize;
                                            // Content width: 60% of terminal - borders(2)
                                            let content_width = ((size.width as usize * 60) / 100).saturating_sub(2).max(1);

                                            // Calculate total visual lines after wrapping
                                            let total_visual_lines: usize = content.lines()
                                                .map(|line| {
                                                    let len = line.chars().count();
                                                    if len == 0 { 1 } else { (len + content_width - 1) / content_width }
                                                })
                                                .sum();

                                            // Only scroll if there's more content below
                                            let max_scroll = total_visual_lines.saturating_sub(visible_lines);
                                            if (app.leaf_scroll_offset as usize) < max_scroll {
                                                app.leaf_scroll_offset = app.leaf_scroll_offset.saturating_add(1);
                                            }
                                        }
                                    }
                                    LeafFocus::Similar => {
                                        if !app.similar_nodes.is_empty() {
                                            app.similar_selected = (app.similar_selected + 1).min(app.similar_nodes.len() - 1);
                                        }
                                    }
                                    LeafFocus::Calls => {
                                        if !app.calls_for_node.is_empty() {
                                            app.calls_selected = (app.calls_selected + 1).min(app.calls_for_node.len() - 1);
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                match app.leaf_focus {
                                    LeafFocus::Content => {
                                        app.leaf_scroll_offset = app.leaf_scroll_offset.saturating_sub(1);
                                    }
                                    LeafFocus::Similar => {
                                        app.similar_selected = app.similar_selected.saturating_sub(1);
                                    }
                                    LeafFocus::Calls => {
                                        app.calls_selected = app.calls_selected.saturating_sub(1);
                                    }
                                }
                            }

                            // Page down/up (only in Content focus)
                            KeyCode::Char('d') => {
                                if app.leaf_focus == LeafFocus::Content {
                                    if let Some(content) = &app.leaf_content {
                                        let size = terminal.size().unwrap_or(ratatui::layout::Rect::new(0, 0, 80, 24));
                                        let visible_lines = size.height.saturating_sub(5) as usize;
                                        let content_width = ((size.width as usize * 60) / 100).saturating_sub(2).max(1);

                                        let total_visual_lines: usize = content.lines()
                                            .map(|line| {
                                                let len = line.chars().count();
                                                if len == 0 { 1 } else { (len + content_width - 1) / content_width }
                                            })
                                            .sum();

                                        let max_scroll = total_visual_lines.saturating_sub(visible_lines);
                                        let new_offset = (app.leaf_scroll_offset as usize).saturating_add(visible_lines / 2).min(max_scroll);
                                        app.leaf_scroll_offset = new_offset as u16;
                                    }
                                }
                            }
                            KeyCode::Char('u') => {
                                if app.leaf_focus == LeafFocus::Content {
                                    let size = terminal.size().unwrap_or(ratatui::layout::Rect::new(0, 0, 80, 24));
                                    let visible_lines = size.height.saturating_sub(5) as usize;
                                    app.leaf_scroll_offset = app.leaf_scroll_offset.saturating_sub(visible_lines as u16 / 2);
                                }
                            }

                            // Tab: Cycle focus Content → Similar → Calls (if code node)
                            KeyCode::Tab => {
                                app.leaf_focus = match app.leaf_focus {
                                    LeafFocus::Content => LeafFocus::Similar,
                                    LeafFocus::Similar => {
                                        // Only cycle to Calls if this is a code node with call edges
                                        if app.is_code_node && !app.calls_for_node.is_empty() {
                                            LeafFocus::Calls
                                        } else {
                                            LeafFocus::Content
                                        }
                                    }
                                    LeafFocus::Calls => LeafFocus::Content,
                                };
                                app.status_message = match app.leaf_focus {
                                    LeafFocus::Content => "Focus: Content".to_string(),
                                    LeafFocus::Similar => "Focus: Similar".to_string(),
                                    LeafFocus::Calls => "Focus: Calls".to_string(),
                                };
                            }
                            // Shift+Tab: Cycle focus in reverse
                            KeyCode::BackTab => {
                                app.leaf_focus = match app.leaf_focus {
                                    LeafFocus::Content => {
                                        // Only cycle to Calls if this is a code node with call edges
                                        if app.is_code_node && !app.calls_for_node.is_empty() {
                                            LeafFocus::Calls
                                        } else {
                                            LeafFocus::Similar
                                        }
                                    }
                                    LeafFocus::Similar => LeafFocus::Content,
                                    LeafFocus::Calls => LeafFocus::Similar,
                                };
                                app.status_message = match app.leaf_focus {
                                    LeafFocus::Content => "Focus: Content".to_string(),
                                    LeafFocus::Similar => "Focus: Similar".to_string(),
                                    LeafFocus::Calls => "Focus: Calls".to_string(),
                                };
                            }

                            // n/N: Navigate similar nodes (quick access from any focus)
                            KeyCode::Char('n') => {
                                if !app.similar_nodes.is_empty() {
                                    app.similar_selected = (app.similar_selected + 1) % app.similar_nodes.len();
                                }
                            }
                            KeyCode::Char('N') => {
                                if !app.similar_nodes.is_empty() {
                                    app.similar_selected = if app.similar_selected == 0 {
                                        app.similar_nodes.len() - 1
                                    } else {
                                        app.similar_selected - 1
                                    };
                                }
                            }

                            // Enter: Navigate to selected similar/call node
                            KeyCode::Enter => {
                                let target_id = match app.leaf_focus {
                                    LeafFocus::Similar if !app.similar_nodes.is_empty() => {
                                        Some(app.similar_nodes[app.similar_selected].id.clone())
                                    }
                                    LeafFocus::Calls if !app.calls_for_node.is_empty() => {
                                        Some(app.calls_for_node[app.calls_selected].0.clone())
                                    }
                                    _ => None,
                                };
                                if let Some(id) = target_id {
                                    app.exit_leaf_view();
                                    if let Err(e) = app.enter_leaf_view(db, &id) {
                                        app.status_message = format!("Error: {}", e);
                                    }
                                }
                            }

                            // e: Enter edit mode
                            KeyCode::Char('e') => {
                                app.enter_edit_mode();
                            }

                            // v: View PDF in external viewer
                            KeyCode::Char('v') => {
                                if let Some(ref node) = app.selected_node {
                                    if node.pdf_available == Some(true) {
                                        match db.get_paper_document(&node.id) {
                                            Ok(Some((doc_data, format))) => {
                                                let title = node.ai_title.as_ref().unwrap_or(&node.title);
                                                let safe_name: String = title.chars()
                                                    .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
                                                    .take(50)
                                                    .collect();
                                                let safe_name = safe_name.trim().replace(' ', "_");

                                                let temp_dir = std::env::temp_dir();
                                                let file_path = temp_dir.join(format!("{}.{}", safe_name, format));

                                                match std::fs::File::create(&file_path) {
                                                    Ok(mut file) => {
                                                        if let Err(e) = file.write_all(&doc_data) {
                                                            app.status_message = format!("Failed to write temp file: {}", e);
                                                        } else {
                                                            #[cfg(target_os = "linux")]
                                                            let result = std::process::Command::new("xdg-open")
                                                                .arg(&file_path)
                                                                .spawn();
                                                            #[cfg(target_os = "macos")]
                                                            let result = std::process::Command::new("open")
                                                                .arg(&file_path)
                                                                .spawn();
                                                            #[cfg(target_os = "windows")]
                                                            let result = std::process::Command::new("cmd")
                                                                .args(["/C", "start", "", &file_path.to_string_lossy()])
                                                                .spawn();

                                                            match result {
                                                                Ok(_) => app.status_message = format!("Opening {}...", format.to_uppercase()),
                                                                Err(e) => app.status_message = format!("Failed to open viewer: {}", e),
                                                            }
                                                        }
                                                    }
                                                    Err(e) => app.status_message = format!("Failed to create temp file: {}", e),
                                                }
                                            }
                                            Ok(None) => app.status_message = "PDF not available (not downloaded)".to_string(),
                                            Err(e) => app.status_message = format!("Database error: {}", e),
                                        }
                                    } else {
                                        app.status_message = "No PDF available for this paper".to_string();
                                    }
                                }
                            }

                            // o: Open URL in browser
                            KeyCode::Char('o') => {
                                if let Some(ref node) = app.selected_node {
                                    if let Some(ref url) = node.url {
                                        #[cfg(target_os = "linux")]
                                        let result = std::process::Command::new("xdg-open")
                                            .arg(url)
                                            .spawn();
                                        #[cfg(target_os = "macos")]
                                        let result = std::process::Command::new("open")
                                            .arg(url)
                                            .spawn();
                                        #[cfg(target_os = "windows")]
                                        let result = std::process::Command::new("cmd")
                                            .args(["/C", "start", "", url])
                                            .spawn();

                                        match result {
                                            Ok(_) => app.status_message = format!("Opening {}...", url),
                                            Err(e) => app.status_message = format!("Failed to open browser: {}", e),
                                        }
                                    } else {
                                        app.status_message = "No URL available for this node".to_string();
                                    }
                                }
                            }

                            KeyCode::Char('?') => {
                                app.status_message = "Tab:focus  j/k:nav  v:pdf  o:url  e:edit  n/N:similar  Enter:goto  q:back".to_string();
                            }
                            _ => {}
                        }
                    }

                    TuiMode::Edit => {
                        use crossterm::event::KeyModifiers;

                        match key.code {
                            // Ctrl+S: Save
                            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                if let Err(e) = app.save_edit(db) {
                                    app.status_message = format!("Save error: {}", e);
                                }
                            }

                            // Esc: Cancel edit
                            KeyCode::Esc => {
                                app.cancel_edit();
                            }

                            // Arrow keys: Move cursor
                            KeyCode::Left => app.edit_cursor_left(),
                            KeyCode::Right => app.edit_cursor_right(),
                            KeyCode::Up => app.edit_cursor_up(),
                            KeyCode::Down => app.edit_cursor_down(),

                            // Home/End: Jump to line start/end
                            KeyCode::Home => app.edit_cursor_home(),
                            KeyCode::End => app.edit_cursor_end(),

                            // Backspace: Delete character before cursor
                            KeyCode::Backspace => app.edit_backspace(),

                            // Delete: Delete character at cursor
                            KeyCode::Delete => app.edit_delete(),

                            // Enter: Insert newline
                            KeyCode::Enter => app.edit_insert_char('\n'),

                            // Tab: Insert 4 spaces (or actual tab)
                            KeyCode::Tab => {
                                for _ in 0..4 {
                                    app.edit_insert_char(' ');
                                }
                            }

                            // Regular character input
                            KeyCode::Char(c) => {
                                app.edit_insert_char(c);
                            }

                            _ => {}
                        }

                        // Keep cursor visible (estimate ~20 visible lines)
                        let visible_lines = 20;
                        app.edit_ensure_cursor_visible(visible_lines);

                        // Update status with cursor position
                        let dirty_marker = if app.edit_dirty { " [modified]" } else { "" };
                        app.status_message = format!(
                            "Edit mode: Ln {}, Col {} {} | Ctrl+S save, Esc cancel",
                            app.edit_cursor_line + 1,
                            app.edit_cursor_col + 1,
                            dirty_marker
                        );
                    }

                    // Other modes (Maintenance, Settings, Jobs) - placeholder
                    _ => {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                app.mode = TuiMode::Navigation;
                                app.status_message = "Back to navigation".to_string();
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

fn draw_ui(f: &mut Frame, app: &TuiApp) {
    match app.mode {
        TuiMode::Navigation => draw_navigation_mode(f, app),
        TuiMode::LeafView => draw_leaf_view_mode(f, app),
        TuiMode::Edit => draw_edit_mode(f, app),
        _ => draw_navigation_mode(f, app), // Fallback for unimplemented modes
    }
}

fn draw_navigation_mode(f: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // Breadcrumb bar
            Constraint::Min(0),     // Main content
            Constraint::Length(1),  // Status bar
        ])
        .split(f.size());

    // Breadcrumb bar
    draw_breadcrumb(f, app, chunks[0]);

    // 3-column layout: Tree (50%) | Pins+Recents (25%) | Preview (25%)
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(chunks[1]);

    // Tree view
    draw_tree(f, app, main_chunks[0]);

    // Pins + Recents pane
    draw_pins_recents(f, app, main_chunks[1]);

    // Preview pane
    draw_preview(f, app, main_chunks[2]);

    // Status bar
    let status = if app.search_mode {
        format!("Search: {}_", app.search_query)
    } else {
        app.status_message.clone()
    };
    let status_bar = Paragraph::new(status)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White));
    f.render_widget(status_bar, chunks[2]);
}

fn draw_pins_recents(f: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),  // Pinned
            Constraint::Percentage(50),  // Recent
        ])
        .split(area);

    // Border styles based on focus
    let pins_border = if app.nav_focus == NavFocus::Pins {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let recents_border = if app.nav_focus == NavFocus::Recents {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Pinned nodes with selection highlight
    let pinned_items: Vec<ListItem> = app.pinned_nodes.iter().enumerate().take(10).map(|(i, node)| {
        let emoji = node.emoji.as_deref().unwrap_or("📌");
        let title = node.ai_title.as_ref().unwrap_or(&node.title);
        let truncated = if title.len() > 20 {
            format!("{}...", &title[..17])
        } else {
            title.clone()
        };
        let content = format!("{} {}", emoji, truncated);

        // Highlight selected item when Pins pane is focused
        if app.nav_focus == NavFocus::Pins && i == app.pins_selected {
            ListItem::new(content).style(Style::default().bg(Color::Blue).fg(Color::White))
        } else {
            ListItem::new(content)
        }
    }).collect();

    let pinned_list = List::new(pinned_items)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(pins_border)
            .title(format!(" 📌 Pinned ({}) ", app.pinned_nodes.len())));
    f.render_widget(pinned_list, chunks[0]);

    // Recent nodes with selection highlight
    let recent_items: Vec<ListItem> = app.recent_nodes.iter().enumerate().take(10).map(|(i, node)| {
        let emoji = node.emoji.as_deref().unwrap_or("📄");
        let title = node.ai_title.as_ref().unwrap_or(&node.title);
        let truncated = if title.len() > 20 {
            format!("{}...", &title[..17])
        } else {
            title.clone()
        };
        let content = format!("{} {}", emoji, truncated);

        // Highlight selected item when Recents pane is focused
        if app.nav_focus == NavFocus::Recents && i == app.recents_selected {
            ListItem::new(content).style(Style::default().bg(Color::Blue).fg(Color::White))
        } else {
            ListItem::new(content)
        }
    }).collect();

    let recent_list = List::new(recent_items)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(recents_border)
            .title(format!(" 🕐 Recent ({}) ", app.recent_nodes.len())));
    f.render_widget(recent_list, chunks[1]);
}

fn draw_preview(f: &mut Frame, app: &TuiApp, area: Rect) {
    let content = if let Some(ref node) = app.selected_node {
        let emoji = node.emoji.as_deref().unwrap_or("");
        let title = node.ai_title.as_ref().unwrap_or(&node.title);
        let node_type = if node.is_item { "Item" } else { "Category" };

        let mut lines = vec![
            Line::from(vec![
                Span::styled(emoji, Style::default()),
                Span::raw(" "),
                Span::styled(title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Type: ", Style::default().fg(Color::Yellow)),
                Span::raw(node_type),
            ]),
            Line::from(vec![
                Span::styled("Children: ", Style::default().fg(Color::Yellow)),
                Span::raw(node.child_count.to_string()),
            ]),
        ];

        // Add date (use derived date for clusters, own date for items)
        let effective_date = node.latest_child_date.unwrap_or(node.created_at);
        let date_str = format_date_time(effective_date);
        let date_color = date_color(effective_date, app.date_min, app.date_max);
        lines.push(Line::from(vec![
            Span::styled("Date: ", Style::default().fg(Color::Yellow)),
            Span::styled(date_str, Style::default().fg(date_color)),
        ]));

        // Add tags if present
        if let Some(ref tags) = node.tags {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("Tags: ", Style::default().fg(Color::Cyan)),
            ]));
            // Wrap tags
            for chunk in tags.chars().collect::<Vec<_>>().chunks(25) {
                lines.push(Line::from(chunk.iter().collect::<String>()));
            }
        }

        // Add summary if present
        if let Some(ref summary) = node.summary {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Summary:", Style::default().fg(Color::Magenta))));
            // Wrap summary
            let preview = if summary.len() > 500 {
                format!("{}...", &summary[..497])
            } else {
                summary.clone()
            };
            for chunk in preview.chars().collect::<Vec<_>>().chunks(45) {
                lines.push(Line::from(chunk.iter().collect::<String>()));
            }
        }

        Text::from(lines)
    } else {
        Text::from("No node selected")
    };

    let preview = Paragraph::new(content)
        .block(Block::default().borders(Borders::ALL).title(" Preview "))
        .wrap(Wrap { trim: false });

    f.render_widget(preview, area);
}

fn draw_breadcrumb(f: &mut Frame, app: &TuiApp, area: Rect) {
    let mut spans = vec![Span::styled(" ", Style::default().bg(Color::Rgb(40, 40, 60)))];

    for (i, (_, title)) in app.breadcrumb_path.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" > ", Style::default().fg(Color::DarkGray).bg(Color::Rgb(40, 40, 60))));
        }

        let style = if i == app.breadcrumb_path.len() - 1 {
            // Current location (highlighted)
            Style::default().fg(Color::Cyan).bg(Color::Rgb(40, 40, 60)).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray).bg(Color::Rgb(40, 40, 60))
        };

        // Truncate long titles
        let display_title = if title.len() > 20 {
            format!("{}...", &title[..17])
        } else {
            title.clone()
        };
        spans.push(Span::styled(display_title, style));
    }

    // Add hint for going back
    if app.breadcrumb_path.len() > 1 {
        spans.push(Span::styled(
            "   [Esc/Backspace: up]",
            Style::default().fg(Color::DarkGray).bg(Color::Rgb(40, 40, 60))
        ));
    }

    let breadcrumb = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(Color::Rgb(40, 40, 60)));
    f.render_widget(breadcrumb, area);
}

fn draw_leaf_view_mode(f: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),  // Header
            Constraint::Min(0),     // Main content
            Constraint::Length(1),  // Status bar
        ])
        .split(f.size());

    // Header with title and back hint
    let title = if let Some(ref node) = app.selected_node {
        let emoji = node.emoji.as_deref().unwrap_or("");
        let title = node.ai_title.as_ref().unwrap_or(&node.title);
        format!("{} {} ", emoji, title)
    } else {
        "Content".to_string()
    };

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(" ← [q/Esc] Back", Style::default().fg(Color::Yellow)),
            Span::raw("   "),
            Span::styled(&title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]),
    ])
    .style(Style::default().bg(Color::Rgb(30, 30, 50)));
    f.render_widget(header, chunks[0]);

    // Main content area: Content (60%) | Sidebar (40%)
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(60),
            Constraint::Percentage(40),
        ])
        .split(chunks[1]);

    // Content pane
    draw_leaf_content(f, app, main_chunks[0]);

    // Sidebar
    draw_leaf_sidebar(f, app, main_chunks[1]);

    // Status bar
    let status_bar = Paragraph::new(&*app.status_message)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White));
    f.render_widget(status_bar, chunks[2]);
}

fn draw_leaf_content(f: &mut Frame, app: &TuiApp, area: Rect) {
    let content = app.leaf_content.as_deref().unwrap_or("No content");

    let border_style = if app.leaf_focus == LeafFocus::Content {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let scroll_info = format!(" Content (line {}) ", app.leaf_scroll_offset + 1);

    // Use Paragraph's native scroll - this handles wrapped text properly
    // by scrolling visual lines, not raw newline-separated lines
    let paragraph = Paragraph::new(content)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(scroll_info))
        .wrap(Wrap { trim: false })
        .scroll((app.leaf_scroll_offset, 0));

    f.render_widget(paragraph, area);
}

fn draw_leaf_sidebar(f: &mut Frame, app: &TuiApp, area: Rect) {
    // Only show Calls section for code nodes with call edges
    let show_calls = app.is_code_node && !app.calls_for_node.is_empty();

    let chunks = if show_calls {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(50),  // Similar nodes
                Constraint::Percentage(50),  // Calls
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(100)])  // Only Similar
            .split(area)
    };

    // Border style for Similar section
    let similar_border = if app.leaf_focus == LeafFocus::Similar {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Similar nodes section with selection highlight
    // Calculate min/max for normalization (like GraphCanvas.tsx line 346)
    let (min_sim, max_sim) = if app.similar_nodes.is_empty() {
        (0.5, 1.0)
    } else {
        let min = app.similar_nodes.iter().map(|s| s.similarity).fold(f32::MAX, f32::min);
        let max = app.similar_nodes.iter().map(|s| s.similarity).fold(f32::MIN, f32::max);
        (min as f64, max as f64)
    };

    let similar_items: Vec<ListItem> = app.similar_nodes.iter().enumerate().map(|(i, sim)| {
        let emoji = sim.emoji.as_deref().unwrap_or("📄");
        let similarity_pct = (sim.similarity * 100.0) as i32;
        // Normalized gradient: spreads colors across visible range (red→yellow | blue→cyan)
        let color = similarity_color_normalized(sim.similarity as f64, min_sim, max_sim);

        let content = format!("{} {} {}%", emoji, &sim.title[..sim.title.len().min(25)], similarity_pct);

        // Highlight selected item when Similar section is focused
        if i == app.similar_selected && app.leaf_focus == LeafFocus::Similar {
            ListItem::new(Span::styled(content, Style::default().bg(Color::Blue).fg(Color::White)))
        } else {
            ListItem::new(Span::styled(content, Style::default().fg(color)))
        }
    }).collect();

    let similar_list = List::new(similar_items)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(similar_border)
            .title(format!(" Similar ({}) ", app.similar_nodes.len())));
    f.render_widget(similar_list, chunks[0]);

    // Calls section - only shown for code nodes with call edges
    if show_calls {
        let calls_border = if app.leaf_focus == LeafFocus::Calls {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let call_items: Vec<ListItem> = app.calls_for_node.iter().enumerate().map(|(i, (_, title, is_outgoing))| {
            // Direction indicator: → for outgoing (calls), ← for incoming (called by)
            let arrow = if *is_outgoing { "→" } else { "←" };
            let content = format!("{} {}", arrow, &title[..title.len().min(30)]);

            // Highlight selected item when Calls section is focused
            if i == app.calls_selected && app.leaf_focus == LeafFocus::Calls {
                ListItem::new(Span::styled(content, Style::default().bg(Color::Blue).fg(Color::White)))
            } else {
                // Color by direction: outgoing = green, incoming = yellow
                let color = if *is_outgoing { Color::Green } else { Color::Yellow };
                ListItem::new(Span::styled(content, Style::default().fg(color)))
            }
        }).collect();

        let calls_list = List::new(call_items)
            .block(Block::default()
                .borders(Borders::ALL)
                .border_style(calls_border)
                .title(format!(" Calls ({}) ", app.calls_for_node.len())));
        f.render_widget(calls_list, chunks[1]);
    }
}

fn draw_edit_mode(f: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),  // Header
            Constraint::Min(0),     // Editor
            Constraint::Length(1),  // Status bar
        ])
        .split(f.size());

    // Header with title and mode indicator
    let title = if let Some(ref node) = app.selected_node {
        let emoji = node.emoji.as_deref().unwrap_or("");
        let title = node.ai_title.as_ref().unwrap_or(&node.title);
        format!("{} {} ", emoji, title)
    } else {
        "Editing".to_string()
    };

    let dirty_indicator = if app.edit_dirty { " [modified]" } else { "" };
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(" ✏️  EDIT MODE", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(dirty_indicator, Style::default().fg(Color::Red)),
            Span::raw("   "),
            Span::styled(&title, Style::default().fg(Color::White)),
        ]),
    ])
    .style(Style::default().bg(Color::Rgb(50, 30, 30)));
    f.render_widget(header, chunks[0]);

    // Calculate visible area for the editor
    let editor_area = chunks[1];
    let inner_height = editor_area.height.saturating_sub(2) as usize; // Account for borders

    // Build editor lines with line numbers and cursor
    let lines: Vec<&str> = app.edit_buffer.lines().collect();
    let total_lines = lines.len().max(1);

    // Calculate line number width (for alignment)
    let line_num_width = total_lines.to_string().len();

    // Build styled lines
    let mut styled_lines: Vec<Line> = Vec::new();

    // Handle empty buffer case
    if app.edit_buffer.is_empty() {
        let line_num = format!("{:>width$} │ ", 1, width = line_num_width);
        styled_lines.push(Line::from(vec![
            Span::styled(line_num, Style::default().fg(Color::DarkGray)),
            Span::styled("█", Style::default().bg(Color::White).fg(Color::Black)), // Cursor
        ]));
    } else {
        for (line_idx, line_content) in lines.iter().enumerate() {
            // Skip lines before scroll offset
            if line_idx < app.edit_scroll_offset {
                continue;
            }
            // Stop if we've filled the visible area
            if styled_lines.len() >= inner_height {
                break;
            }

            let line_num = format!("{:>width$} │ ", line_idx + 1, width = line_num_width);
            let is_cursor_line = line_idx == app.edit_cursor_line;

            if is_cursor_line {
                // Build line with cursor
                let chars: Vec<char> = line_content.chars().collect();
                let mut spans = vec![
                    Span::styled(line_num, Style::default().fg(Color::Yellow)),
                ];

                // Characters before cursor
                if app.edit_cursor_col > 0 {
                    let before: String = chars[..app.edit_cursor_col.min(chars.len())].iter().collect();
                    spans.push(Span::raw(before));
                }

                // Cursor character (or space if at end of line)
                if app.edit_cursor_col < chars.len() {
                    let cursor_char = chars[app.edit_cursor_col].to_string();
                    spans.push(Span::styled(cursor_char, Style::default().bg(Color::White).fg(Color::Black)));
                } else {
                    // Cursor at end of line
                    spans.push(Span::styled(" ", Style::default().bg(Color::White).fg(Color::Black)));
                }

                // Characters after cursor
                if app.edit_cursor_col + 1 < chars.len() {
                    let after: String = chars[app.edit_cursor_col + 1..].iter().collect();
                    spans.push(Span::raw(after));
                }

                styled_lines.push(Line::from(spans));
            } else {
                // Regular line without cursor
                styled_lines.push(Line::from(vec![
                    Span::styled(line_num, Style::default().fg(Color::DarkGray)),
                    Span::raw(*line_content),
                ]));
            }
        }
    }

    // Editor pane
    let scroll_info = format!(
        " Editor - Ln {}/{}, Col {} ",
        app.edit_cursor_line + 1,
        total_lines,
        app.edit_cursor_col + 1
    );

    let editor = Paragraph::new(styled_lines)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(scroll_info))
        .wrap(Wrap { trim: false });
    f.render_widget(editor, editor_area);

    // Status bar with keybindings
    let status_bar = Paragraph::new(&*app.status_message)
        .style(Style::default().bg(Color::Rgb(60, 40, 40)).fg(Color::White));
    f.render_widget(status_bar, chunks[2]);
}

fn draw_tree(f: &mut Frame, app: &TuiApp, area: Rect) {
    // Fixed date width: "01 Jan 2020" = 11 chars
    const DATE_WIDTH: usize = 11;
    // Account for: borders (2), highlight symbol "→ " (3), separator space (1)
    let usable_width = area.width.saturating_sub(6) as usize;
    // Reserve space for date at the end
    let title_max_width = usable_width.saturating_sub(DATE_WIDTH + 1);

    let items: Vec<ListItem> = app.visible_nodes.iter().map(|&idx| {
        let node = &app.nodes[idx];
        let indent = "  ".repeat(node.depth as usize);

        // Use emoji if available, otherwise default icons
        let prefix = if node.is_item {
            node.emoji.as_deref().unwrap_or("📄").to_string()
        } else if node.is_expanded {
            "▼".to_string()
        } else if node.child_count > 0 {
            node.emoji.as_deref().unwrap_or("▶").to_string()
        } else {
            node.emoji.as_deref().unwrap_or("○").to_string()
        };

        let count = if !node.is_item && node.child_count > 0 {
            format!(" ({})", node.child_count)
        } else {
            String::new()
        };

        // Use effective date (derived from children for clusters, own date for items)
        let effective_date = node.latest_child_date.unwrap_or(node.created_at);

        // Calculate date color using graph-matching gradient (red=old → cyan=new)
        let node_date_color = date_color(effective_date, app.date_min, app.date_max);

        // Format date (date only, no time, for hierarchy view)
        let date_str = format_date_only(effective_date);

        // Build title with indent, prefix, title, and count
        let full_title = format!("{}{} {}{}", indent, prefix, node.title, count);

        // Truncate title if needed, accounting for unicode graphemes
        let title_chars: Vec<char> = full_title.chars().collect();
        let truncated_title = if title_chars.len() > title_max_width {
            let truncate_at = title_max_width.saturating_sub(1);
            let truncated: String = title_chars.iter().take(truncate_at).collect();
            format!("{}…", truncated)
        } else {
            full_title
        };

        // Pad title to align dates (left-aligned dates at fixed position)
        let display_width = truncated_title.chars().count();
        let padding = title_max_width.saturating_sub(display_width);
        let padded_title = format!("{}{}", truncated_title, " ".repeat(padding));

        // Create styled content with colored date
        let content = Line::from(vec![
            Span::raw(padded_title),
            Span::raw(" "),
            Span::styled(date_str, Style::default().fg(node_date_color)),
        ]);

        ListItem::new(content)
    }).collect();

    // Border style based on focus
    let border_style = if app.nav_focus == NavFocus::Tree {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let tree = List::new(items)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(" Hierarchy "))
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White).add_modifier(Modifier::BOLD))
        .highlight_symbol("→ ");

    f.render_stateful_widget(tree, area, &mut app.list_state.clone());
}

/// Convert HSL to RGB Color
fn hsl_to_color(hue: f64, saturation: f64, lightness: f64) -> Color {
    let h = (hue % 360.0) / 360.0;
    let s = saturation.clamp(0.0, 1.0);
    let l = lightness.clamp(0.0, 1.0);

    let (r, g, b) = if s == 0.0 {
        let v = (l * 255.0) as u8;
        (v, v, v)
    } else {
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;

        let hue_to_rgb = |p: f64, q: f64, mut t: f64| -> f64 {
            if t < 0.0 { t += 1.0; }
            if t > 1.0 { t -= 1.0; }
            if t < 1.0 / 6.0 { return p + (q - p) * 6.0 * t; }
            if t < 1.0 / 2.0 { return q; }
            if t < 2.0 / 3.0 { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
            p
        };

        let r = (hue_to_rgb(p, q, h + 1.0 / 3.0) * 255.0) as u8;
        let g = (hue_to_rgb(p, q, h) * 255.0) as u8;
        let b = (hue_to_rgb(p, q, h - 1.0 / 3.0) * 255.0) as u8;
        (r, g, b)
    };

    Color::Rgb(r, g, b)
}

/// Get similarity color with range normalization (like GraphCanvas.tsx line 346)
/// Normalizes similarity to visible range, then applies two-segment gradient.
/// RED (0°) → YELLOW (60°) | BLUE (210°) → CYAN (180°)
fn similarity_color_normalized(similarity: f64, min_sim: f64, max_sim: f64) -> Color {
    // Normalize to 0-1 based on visible range (exactly like graph edges)
    let range = (max_sim - min_sim).max(0.01); // avoid div by zero
    let t = ((similarity - min_sim) / range).clamp(0.0, 1.0);

    // Two-segment gradient from getEdgeColor
    let hue = if t < 0.5 {
        t * 2.0 * 60.0              // RED (0°) → YELLOW (60°)
    } else {
        210.0 - (t - 0.5) * 2.0 * 30.0  // BLUE (210°) → CYAN (180°)
    };

    hsl_to_color(hue, 0.80, 0.50)
}

/// Get similarity color with default 0.5-1.0 range normalization
fn similarity_color(similarity: f64) -> Color {
    similarity_color_normalized(similarity, 0.5, 1.0)
}

/// Get date color using EXACT formula from GraphCanvas.tsx getDateColor
/// RED (0°) → YELLOW (60°) at 50% | BLUE (210°) → CYAN (180°) at 100%
/// NO GREEN anywhere. NO saturation tricks.
fn date_color(timestamp: i64, min_date: i64, max_date: i64) -> Color {
    if max_date <= min_date {
        return Color::Gray;
    }
    let t = (timestamp - min_date) as f64 / (max_date - min_date) as f64;

    // EXACT formula from GraphCanvas.tsx getEdgeColor (lines 160-168):
    let hue = if t <= 0.5 {
        t * 2.0 * 60.0              // 0→0°, 50%→60° (red to yellow)
    } else {
        210.0 - (t - 0.5) * 2.0 * 30.0  // 50%→210°, 100%→180° (blue to cyan)
    };

    // Match GraphStatusBar.tsx legend: hsl(h, 75%, 65%)
    hsl_to_color(hue, 0.75, 0.65)
}

fn format_date_time(timestamp: i64) -> String {
    // timestamp is in milliseconds; 0 = unknown date
    if timestamp == 0 {
        return "Unknown".to_string();
    }
    chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|dt| {
            // Only show time if not midnight (papers only have dates, shown as 00:00)
            if dt.hour() == 0 && dt.minute() == 0 {
                dt.format("%d %b %Y").to_string()
            } else {
                dt.format("%d %b %Y %H:%M").to_string()
            }
        })
        .unwrap_or_else(|| "Unknown".to_string())
}

/// Format date only (no time) - fixed width of 11 chars for alignment
fn format_date_only(timestamp: i64) -> String {
    // timestamp is in milliseconds; 0 = unknown date
    if timestamp == 0 {
        return "    Unknown".to_string(); // Pad to 11 chars
    }
    chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|dt| dt.format("%d %b %Y").to_string()) // Always 11 chars: "01 Jan 2020"
        .unwrap_or_else(|| "    Unknown".to_string())
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Auto-discover project-specific database by walking up from current directory.
/// Looks for .mycelica.db in each directory up to root.
fn find_project_db() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        // Check for .mycelica.db in this directory
        let db_path = dir.join(".mycelica.db");
        if db_path.exists() {
            return Some(db_path);
        }
        // Check docs/.mycelica.db (common location)
        let docs_db = dir.join("docs/.mycelica.db");
        if docs_db.exists() {
            return Some(docs_db);
        }
        // Go up one directory
        if !dir.pop() {
            return None;
        }
    }
}

fn find_database() -> PathBuf {
    // 1. Auto-discover project-specific database (highest priority when in a repo)
    if let Some(project_db) = find_project_db() {
        return project_db;
    }

    // 2. Check custom path from settings
    if let Some(custom_path) = settings::get_custom_db_path() {
        let path = PathBuf::from(&custom_path);
        if path.exists() {
            return path;
        }
    }

    // 3. Check specific known system paths
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

    // 4. Fall back to app data dir
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
