//! Mycelica CLI - Full command-line interface for knowledge graph operations
//!
//! Usage: mycelica-cli [OPTIONS] <COMMAND>
//!
//! A first-class CLI for power users. Supports JSON output for scripting.

use clap::{Parser, Subcommand, CommandFactory};
use clap_complete::{generate, Shell};
use mycelica_lib::{db::{Database, Node, NodeType, Edge, EdgeType, Position}, settings, import, hierarchy, similarity, openaire, ai_client, utils, classification, local_embeddings};
use serde_json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::collections::HashSet;
use std::io::{BufRead, Read as _, Write};
use chrono::{Timelike, Utc, Datelike, Local};
use futures::{stream, StreamExt};

// ============================================================================
// Logging Infrastructure
// ============================================================================

use std::sync::Mutex;
use std::fs::{self, File, OpenOptions};

static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

/// Initialize logging - creates log file and cleans old logs
fn init_logging() -> Option<PathBuf> {
    let log_dir = dirs::data_dir()
        .map(|p| p.join("com.mycelica.app").join("logs"))
        .unwrap_or_else(|| PathBuf::from("logs"));

    if fs::create_dir_all(&log_dir).is_err() {
        return None;
    }

    // Clean logs older than 7 days
    if let Ok(entries) = fs::read_dir(&log_dir) {
        let cutoff = Local::now() - chrono::Duration::days(7);
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("mycelica-") && name.ends_with(".log") {
                    // Parse date from filename: mycelica-YYYY-MM-DD.log
                    if let Some(date_str) = name.strip_prefix("mycelica-").and_then(|s| s.strip_suffix(".log")) {
                        if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                            if date < cutoff.date_naive() {
                                let _ = fs::remove_file(&path);
                            }
                        }
                    }
                }
            }
        }
    }

    // Create today's log file
    let today = Local::now();
    let log_filename = format!("mycelica-{:04}-{:02}-{:02}.log", today.year(), today.month(), today.day());
    let log_path = log_dir.join(&log_filename);

    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&log_path) {
        *LOG_FILE.lock().unwrap() = Some(file);
        Some(log_path)
    } else {
        None
    }
}

/// Log to both terminal and file
#[allow(unused)]
fn log_both(msg: &str) {
    let now = Local::now();
    let timestamp = format!("[{:02}:{:02}:{:02}]", now.hour(), now.minute(), now.second());

    // Print to terminal
    println!("{}", msg);

    // Write to log file
    if let Ok(mut guard) = LOG_FILE.lock() {
        if let Some(ref mut file) = *guard {
            let _ = writeln!(file, "{} {}", timestamp, msg);
        }
    }
}

/// Log error to both terminal and file
#[allow(unused)]
fn elog_both(msg: &str) {
    let now = Local::now();
    let timestamp = format!("[{:02}:{:02}:{:02}]", now.hour(), now.minute(), now.second());

    // Print to terminal
    eprintln!("{}", msg);

    // Write to log file
    if let Ok(mut guard) = LOG_FILE.lock() {
        if let Some(ref mut file) = *guard {
            let _ = writeln!(file, "{} [ERROR] {}", timestamp, msg);
        }
    }
}

/// Check if text is predominantly English using ASCII ratio heuristic.
/// Returns true if >90% of characters are ASCII (letters, numbers, punctuation).
/// Used to detect non-English text that local LLMs may hallucinate on.
fn is_predominantly_english(texts: &[String]) -> bool {
    if texts.is_empty() {
        return true;
    }

    let combined: String = texts.join(" ");
    if combined.is_empty() {
        return true;
    }

    let total_chars = combined.chars().count();
    let ascii_chars = combined.chars().filter(|c| c.is_ascii()).count();

    let ascii_ratio = ascii_chars as f64 / total_chars as f64;
    ascii_ratio > 0.90
}

/// Macro for logging to both terminal and file
macro_rules! log {
    ($($arg:tt)*) => {
        log_both(&format!($($arg)*))
    };
}

/// Macro for error logging to both terminal and file
macro_rules! elog {
    ($($arg:tt)*) => {
        elog_both(&format!($($arg)*))
    };
}

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

    /// Remote server URL (route commands to team server instead of local DB)
    #[arg(long, global = true)]
    remote: Option<String>,

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
        /// Include code items in AI processing (generate summaries)
        #[arg(long)]
        include_code: bool,
        /// Hierarchy algorithm: adaptive (default) or dendrogram
        #[arg(long, default_value = "adaptive")]
        algorithm: String,
        /// Use keyword extraction instead of LLM for category naming (faster but lower quality)
        #[arg(long)]
        keywords_only: bool,
        /// Non-interactive mode: skip prompts, auto-confirm pipeline
        #[arg(long, short = 'y')]
        yes: bool,
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
        /// Filter by type (item, category, paper, concept, question, decision, reference, all)
        #[arg(long, short = 't', default_value = "all")]
        type_filter: String,
        /// Filter by author name
        #[arg(long)]
        author: Option<String>,
        /// Filter by recency (e.g., "7d", "24h", "30d")
        #[arg(long)]
        recent: Option<String>,
        /// Maximum results
        #[arg(long, short, default_value = "20")]
        limit: u32,
    },
    /// Add a link/resource to the knowledge graph
    Add {
        /// URL of the resource
        url: String,
        /// Optional note/description
        #[arg(long)]
        note: Option<String>,
        /// Comma-separated tags
        #[arg(long)]
        tag: Option<String>,
        /// Connect to existing nodes by term (comma-separated)
        #[arg(long, value_delimiter = ',')]
        connects_to: Option<Vec<String>>,
    },
    /// Record an open question
    Ask {
        /// The question text
        question: String,
        /// Connect to existing nodes by term (comma-separated)
        #[arg(long, value_delimiter = ',')]
        connects_to: Option<Vec<String>>,
    },
    /// Create a named concept
    Concept {
        /// Concept title
        title: String,
        /// Optional description/note
        #[arg(long)]
        note: Option<String>,
        /// Connect to existing nodes by term (comma-separated)
        #[arg(long, value_delimiter = ',')]
        connects_to: Option<Vec<String>>,
    },
    /// Record a decision
    Decide {
        /// Decision title
        title: String,
        /// Reasoning behind the decision
        #[arg(long)]
        reason: Option<String>,
        /// Connect to existing nodes by term (comma-separated)
        #[arg(long, value_delimiter = ',')]
        connects_to: Option<Vec<String>>,
    },
    /// Create a typed edge between existing nodes
    Link {
        /// Source node (ID prefix or title substring)
        source: String,
        /// Target node (ID prefix or title substring)
        target: String,
        /// Edge type: related, contradicts, supports, prerequisite, evolved_from, questions
        #[arg(long, short = 't', default_value = "related")]
        edge_type: String,
        /// Reason for this connection (short provenance)
        #[arg(long)]
        reason: Option<String>,
        /// Full reasoning/explanation text
        #[arg(long)]
        content: Option<String>,
        /// Agent attribution (default: human)
        #[arg(long, default_value = "human")]
        agent: String,
        /// Confidence score (0.0-1.0)
        #[arg(long)]
        confidence: Option<f64>,
        /// Edge ID to supersede
        #[arg(long)]
        supersedes: Option<String>,
    },
    /// List orphaned nodes (no valid parent)
    Orphans {
        /// Maximum results
        #[arg(long, short, default_value = "50")]
        limit: u32,
    },
    /// Run schema migrations
    Migrate {
        #[command(subcommand)]
        cmd: MigrateCommands,
    },
    /// Interactive TUI mode
    Tui,
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Spore agent coordination commands
    Spore {
        #[command(subcommand)]
        cmd: SporeCommands,
    },
    #[cfg(feature = "mcp")]
    /// Start MCP server for agent coordination
    McpServer {
        /// Agent role: human, ingestor, coder, verifier, planner, synthesizer, summarizer, docwriter
        #[arg(long)]
        agent_role: String,
        /// Agent ID for attribution (e.g. spore:coder-1)
        #[arg(long)]
        agent_id: String,
        /// Use stdio transport (required)
        #[arg(long)]
        stdio: bool,
        /// Run ID for tracking (UUID). All edges created in this session will include this in metadata.
        #[arg(long)]
        run_id: Option<String>,
    },
}

// ============================================================================
// Subcommand Enums
// ============================================================================

#[derive(Subcommand)]
enum SporeCommands {
    /// Query edges with multi-filter (type, agent, confidence, recency)
    QueryEdges {
        /// Filter by edge type
        #[arg(long = "type", short = 't')]
        edge_type: Option<String>,
        /// Filter by agent_id (edge creator)
        #[arg(long)]
        agent: Option<String>,
        /// Filter by target node's agent_id (whose work is being targeted)
        #[arg(long)]
        target_agent: Option<String>,
        /// Minimum confidence threshold (0.0-1.0)
        #[arg(long)]
        confidence_min: Option<f64>,
        /// Only edges created after this date (YYYY-MM-DD or relative: 1h, 2d, 1w)
        #[arg(long)]
        since: Option<String>,
        /// Exclude superseded edges
        #[arg(long)]
        not_superseded: bool,
        /// Maximum results
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Compact one-line output: edge_id type source -> target [confidence]
        #[arg(long)]
        compact: bool,
    },
    /// Explain an edge with full context (nodes, adjacents, supersession chain)
    ExplainEdge {
        /// Edge ID
        id: String,
        /// Depth of adjacent edge exploration (1 or 2)
        #[arg(long, default_value = "1")]
        depth: usize,
    },
    /// Find all paths between two nodes
    PathBetween {
        /// Source node (ID prefix or title substring)
        from: String,
        /// Target node (ID prefix or title substring)
        to: String,
        /// Maximum hops
        #[arg(long, default_value = "5")]
        max_hops: usize,
        /// Comma-separated edge types to follow (e.g. "contradicts,derives_from")
        #[arg(long = "edge-types")]
        edge_types: Option<String>,
    },
    /// Get the most relevant edges for a node, ranked by composite score
    EdgesForContext {
        /// Node ID prefix or title substring
        id: String,
        /// Number of top edges to return
        #[arg(long, default_value = "10")]
        top: usize,
        /// Exclude superseded edges
        #[arg(long)]
        not_superseded: bool,
    },
    /// Create a meta node (summary/contradiction/status) with edges to existing nodes
    CreateMeta {
        /// Meta type: summary, contradiction, status
        #[arg(long = "type", short = 't')]
        meta_type: String,
        /// Title for the meta node
        #[arg(long)]
        title: String,
        /// Content/body for the meta node
        #[arg(long)]
        content: Option<String>,
        /// Agent attribution
        #[arg(long, default_value = "human")]
        agent: String,
        /// Node IDs to connect to
        #[arg(long = "connects-to", num_args = 1..)]
        connects_to: Vec<String>,
        /// Edge type for connections (default: summarizes)
        #[arg(long = "edge-type", default_value = "summarizes")]
        edge_type: String,
    },
    /// Update an existing meta node
    UpdateMeta {
        /// Meta node ID
        id: String,
        /// New content
        #[arg(long)]
        content: Option<String>,
        /// New title
        #[arg(long)]
        title: Option<String>,
        /// Agent attribution
        #[arg(long, default_value = "human")]
        agent: String,
        /// Additional node IDs to connect to
        #[arg(long = "add-connects", num_args = 1..)]
        add_connects: Vec<String>,
        /// Edge type for new connections
        #[arg(long = "edge-type", default_value = "summarizes")]
        edge_type: String,
    },
    /// Spore status dashboard
    Status {
        /// Show all details (meta nodes, edge breakdown, contradictions)
        #[arg(long)]
        all: bool,
        /// Output format: compact (default) or full (adds top nodes, edge distribution, recent ops)
        #[arg(long, default_value = "compact")]
        format: String,
    },
    /// Create an edge between two existing nodes
    CreateEdge {
        /// Source node (ID prefix or title substring)
        #[arg(long)]
        from: String,
        /// Target node (ID prefix or title substring)
        #[arg(long)]
        to: String,
        /// Edge type (e.g. supports, contradicts, derives_from)
        #[arg(long = "type", short = 't')]
        edge_type: String,
        /// Full reasoning/explanation for this edge
        #[arg(long)]
        content: Option<String>,
        /// Short provenance
        #[arg(long)]
        reason: Option<String>,
        /// Agent attribution
        #[arg(long, default_value = "spore")]
        agent: String,
        /// Confidence (0.0-1.0)
        #[arg(long)]
        confidence: Option<f64>,
        /// Edge ID this supersedes
        #[arg(long)]
        supersedes: Option<String>,
    },
    /// Read full content of a node (no metadata noise)
    ReadContent {
        /// Node ID prefix or title substring
        id: String,
    },
    /// List all descendants of a category node
    ListRegion {
        /// Parent node ID prefix or title substring
        id: String,
        /// Filter by node_class (knowledge, meta, operational)
        #[arg(long)]
        class: Option<String>,
        /// Only show items (is_item=true)
        #[arg(long)]
        items_only: bool,
        /// Maximum results
        #[arg(long, default_value = "50")]
        limit: usize,
    },
    /// Check if summary meta-nodes are stale relative to summarized nodes
    CheckFreshness {
        /// Node ID prefix or title substring
        id: String,
    },
    /// Run tracking: list, inspect, or rollback orchestrator runs
    Runs {
        #[command(subcommand)]
        cmd: RunCommands,
    },
    /// Orchestrate a Coder → Verifier bounce loop for a task
    Orchestrate {
        /// Task description
        task: String,
        /// Maximum bounce iterations before escalation
        #[arg(long, default_value = "3")]
        max_bounces: usize,
        /// Max turns per agent invocation
        #[arg(long, default_value = "50")]
        max_turns: usize,
        /// Path to coder agent prompt
        #[arg(long, default_value = "docs/spore/agents/coder.md")]
        coder_prompt: std::path::PathBuf,
        /// Path to verifier agent prompt
        #[arg(long, default_value = "docs/spore/agents/verifier.md")]
        verifier_prompt: std::path::PathBuf,
        /// Show what would happen without running agents
        #[arg(long)]
        dry_run: bool,
        /// Print agent stdout/stderr
        #[arg(long, short)]
        verbose: bool,
    },
    /// Dijkstra context retrieval: find the N most relevant nodes by weighted graph proximity
    ContextForTask {
        /// Starting node (ID prefix, UUID, or title substring)
        id: String,
        /// Maximum number of context nodes to return
        #[arg(long, default_value = "20")]
        budget: usize,
        /// Maximum number of hops from source
        #[arg(long, default_value = "6")]
        max_hops: usize,
        /// Maximum cumulative path cost (lower = stricter)
        #[arg(long, default_value = "3.0")]
        max_cost: f64,
        /// Comma-separated edge types to follow (e.g. "supports,derives_from,calls")
        #[arg(long = "edge-types")]
        edge_types: Option<String>,
        /// Exclude superseded edges
        #[arg(long)]
        not_superseded: bool,
        /// Only include item nodes in results (categories still traversed)
        #[arg(long)]
        items_only: bool,
    },
}

#[derive(Subcommand)]
enum RunCommands {
    /// List recent runs
    List {
        /// Filter by agent ID
        #[arg(long)]
        agent: Option<String>,
        /// Only runs since this date (YYYY-MM-DD or relative: 1h, 2d, 1w)
        #[arg(long)]
        since: Option<String>,
        /// Maximum results
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Show all edges in a run
    Get {
        /// Run ID (UUID)
        run_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete all edges (and optionally nodes) from a run
    Rollback {
        /// Run ID (UUID)
        run_id: String,
        /// Also delete operational nodes created during the run
        #[arg(long)]
        delete_nodes: bool,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
        /// Show what would be deleted without deleting
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum MigrateCommands {
    /// Run Spore schema migration (adds agent_id, node_class, meta_type to nodes; content, agent_id, superseded_by, metadata to edges)
    SporeSchema {
        /// Skip creating a backup before migration
        #[arg(long)]
        no_backup: bool,
    },
}

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
        /// Unpaywall email for PDF lookup (fallback after arXiv/PMC)
        #[arg(long)]
        unpaywall_email: Option<String>,
        /// CORE API key for PDF lookup (fallback after Unpaywall)
        #[arg(long)]
        core_api_key: Option<String>,
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
    /// Import ChatGPT conversation export (JSON)
    Chatgpt {
        /// Path to ChatGPT conversations.json file
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
    /// Import messages from Signal Desktop (requires --features signal)
    #[cfg(feature = "signal")]
    Signal {
        /// Signal conversation ID. Use --list to discover IDs.
        #[arg(long)]
        conversation_id: Option<String>,
        /// List available conversations and exit
        #[arg(long)]
        list: bool,
        /// Map sourceServiceId UUIDs to names: "<uuid>=E,<uuid>=F". Use "self=E" to label your own outgoing messages.
        #[arg(long)]
        author_map: Option<String>,
        /// List participant UUIDs for a conversation (for building --author-map)
        #[arg(long)]
        list_authors: bool,
        /// Path to Signal database (default: ~/.config/Signal/sql/db.sqlite)
        #[arg(long)]
        db_path: Option<String>,
        /// Generate embeddings for imported nodes
        #[arg(long)]
        embed: bool,
        /// Allow importing into a database inside a git repository
        #[arg(long)]
        allow_repo_db: bool,
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
        /// Show full content (no truncation) and all spore fields
        #[arg(long)]
        full: bool,
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
    Rebuild {
        /// Algorithm: adaptive (default) or dendrogram
        #[arg(long, default_value = "adaptive")]
        algorithm: String,
        /// Target number of hierarchy levels (dendrogram only)
        #[arg(long, default_value = "4")]
        levels: usize,
        /// Threshold method: dynamic, gap, percentile, or fixed (dendrogram only)
        #[arg(long, default_value = "dynamic")]
        method: String,
        /// Minimum component size to create named category
        #[arg(long, default_value = "5")]
        min_size: usize,
        /// Custom thresholds for fixed method (comma-separated, e.g. "0.8,0.7,0.6,0.5")
        #[arg(long)]
        thresholds: Option<String>,
        /// [adaptive] Minimum intra/inter ratio for valid split (1.0-2.0)
        #[arg(long, default_value = "1.2")]
        cohesion_threshold: f64,
        /// [adaptive] Minimum gap between parent/child thresholds
        #[arg(long, default_value = "0.03")]
        delta_min: f64,
        /// [adaptive] Variance threshold below which group is tight (no split)
        #[arg(long, default_value = "0.001")]
        tight_threshold: f64,
        /// [adaptive] Auto-compute optimal parameters from edge statistics
        #[arg(long)]
        auto: bool,
        /// Use keyword extraction instead of LLM for category naming (faster but lower quality)
        #[arg(long)]
        keywords_only: bool,
        /// Fresh rebuild: delete all algorithm-generated categories, rebuild from scratch (preserves human modifications)
        #[arg(long)]
        fresh: bool,
    },
    /// Flatten single-child chains
    Flatten,
    /// Analyze edge weight distribution and recommend parameters
    Analyze {
        /// Show recommended auto-config parameters
        #[arg(long)]
        recommend: bool,
    },
    /// Show hierarchy statistics
    Stats,
    /// Fix Recent Notes position (move to Universe)
    FixRecentNotes,
    /// Smart add orphan items by finding similar existing items
    SmartAdd,
    /// Test dendrogram algorithm on current edges (analysis only, no changes)
    Dendrogram {
        /// Target number of hierarchy levels
        #[arg(long, default_value = "4")]
        levels: usize,
        /// Threshold method: gap, percentile, or fixed
        #[arg(long, default_value = "percentile")]
        method: String,
    },
    /// Consolidate root: group top-level categories into uber-categories
    Consolidate,
    /// Unconsolidate root: flatten uber-categories back to Universe
    Unconsolidate,
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
    /// Build HNSW index for fast similarity search
    BuildIndex,
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
        /// Filter by author name
        #[arg(long)]
        author: Option<String>,
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
    /// Index edges by parent for fast per-view loading
    IndexEdges,
    /// Merge small sibling categories by embedding similarity
    MergeSmallCategories {
        /// Cosine similarity threshold for clustering (0.0-1.0)
        #[arg(long, default_value = "0.7")]
        threshold: f32,
        /// Maximum children for a category to be considered "small"
        #[arg(long, default_value = "3")]
        max_size: i32,
    },
    /// Repair code node tags (restore file_path metadata from source files)
    RepairCodeTags {
        /// Path to source code directory
        #[arg(default_value = ".")]
        path: String,
        /// Show what would be repaired without making changes
        #[arg(long)]
        dry_run: bool,
    },
    /// Refine hierarchy using graph edges (papers move to connected subcategories)
    RefineGraph {
        /// Merge threshold (0.0-1.0). Sibling categories above this get merged.
        #[arg(long, default_value = "0.65")]
        merge_threshold: f32,

        /// Minimum papers to form a new subcategory from connected component
        #[arg(long, default_value = "3")]
        min_component: usize,

        /// Only analyze, don't make changes
        #[arg(long)]
        dry_run: bool,
    },
    /// Analyze coherence and sibling similarity distributions by depth
    DiagnoseCoherence {
        /// Maximum depth to analyze
        #[arg(long, default_value = "7")]
        max_depth: i32,
        /// Sample size per depth for coherence analysis
        #[arg(long, default_value = "50")]
        sample_size: usize,
    },
    /// Regenerate semantic edges at a different similarity threshold
    RegenerateEdges {
        /// Minimum similarity threshold for edges (0.0-1.0)
        #[arg(long, default_value = "0.3")]
        threshold: f32,
        /// Maximum edges per node
        #[arg(long, default_value = "15")]
        max_edges: usize,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Clean duplicate papers (same title, keep best metadata)
    CleanDuplicates {
        /// Only analyze, don't delete
        #[arg(long)]
        dry_run: bool,
    },
    /// Backfill content hashes for existing papers (for deduplication)
    BackfillHashes,
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
    /// Create "Documents" edges from docs to code they reference
    DocEdges,
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
    // Initialize logging
    if let Some(log_path) = init_logging() {
        eprintln!("Logging to: {}", log_path.display());
    }

    let cli = Cli::parse();

    if let Err(e) = run_cli(cli).await {
        elog!("Error: {}", e);
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

    // If --remote is set, route remotable commands through RemoteClient
    if let Some(remote_url) = cli.remote.clone() {
        return run_remote(cli, &remote_url).await;
    }

    // Find database
    let db_path = cli.db.map(PathBuf::from).unwrap_or_else(find_database);

    if cli.verbose {
        eprintln!("[verbose] Using database: {:?}", db_path);
    }

    // Pre-migration backup: must happen BEFORE Database::new() triggers auto-migration
    if let Commands::Migrate { cmd: MigrateCommands::SporeSchema { no_backup } } = &cli.command {
        if !no_backup {
            let backup_path = db_path.with_extension("db.pre-spore-backup");
            eprintln!("Creating backup at {:?}...", backup_path);
            std::fs::copy(&db_path, &backup_path)
                .map_err(|e| format!("Failed to create backup: {}", e))?;
            eprintln!("Backup created.");
        }
    }

    let db = Arc::new(Database::new(&db_path).map_err(|e| format!("Failed to open database: {}", e))?);


    match cli.command {
        Commands::Db { cmd } => handle_db(cmd, &db, cli.json).await,
        Commands::Import { cmd } => handle_import(cmd, &db, cli.json, cli.quiet).await,
        Commands::Node { cmd } => handle_node(cmd, &db, cli.json).await,
        Commands::Hierarchy { cmd } => handle_hierarchy(cmd, &db, cli.json, cli.quiet).await,
        Commands::Process { cmd } => handle_process(cmd, &db, cli.json, cli.quiet).await,
        Commands::Embeddings { cmd } => handle_embeddings(cmd, &db, cli.json).await,
        Commands::Privacy { cmd } => handle_privacy(cmd, &db, cli.json, cli.quiet).await,
        Commands::Paper { cmd } => handle_paper(cmd, &db, cli.json).await,
        Commands::Config { cmd } => handle_config(cmd, cli.json),
        Commands::Setup { skip_pipeline, include_code, algorithm, keywords_only, yes } => handle_setup(&db, skip_pipeline, include_code, &algorithm, keywords_only, yes, cli.quiet).await,
        Commands::Recent { cmd } => handle_recent(cmd, &db, cli.json),
        Commands::Pinned { cmd } => handle_pinned(cmd, &db, cli.json),
        Commands::Nav { cmd } => handle_nav(cmd, &db, cli.json).await,
        Commands::Maintenance { cmd } => handle_maintenance(cmd, &db, cli.json).await,
        Commands::Export { cmd } => handle_export(cmd, &db, cli.quiet).await,
        Commands::Analyze { cmd } => handle_analyze(cmd, &db, cli.json, cli.quiet).await,
        Commands::Code { cmd } => handle_code(cmd, &db, &db_path).await,
        Commands::Search { query, type_filter, author, recent, limit } => handle_search(&query, &type_filter, author, recent, limit, &db, cli.json).await,
        Commands::Add { url, note, tag, connects_to } => handle_add(&url, note, tag, connects_to, &db, cli.json).await,
        Commands::Ask { question, connects_to } => handle_ask(&question, connects_to, &db, cli.json).await,
        Commands::Concept { title, note, connects_to } => handle_concept(&title, note, connects_to, &db, cli.json).await,
        Commands::Decide { title, reason, connects_to } => handle_decide(&title, reason, connects_to, &db, cli.json).await,
        Commands::Link { source, target, edge_type, reason, content, agent, confidence, supersedes } => handle_link(&source, &target, &edge_type, reason, content, &agent, confidence, supersedes, "user", &db, cli.json).await,
        Commands::Orphans { limit } => handle_orphans(limit, &db, cli.json).await,
        Commands::Migrate { cmd } => handle_migrate(cmd, &db, &db_path, cli.json),
        Commands::Spore { cmd } => handle_spore(cmd, &db, cli.json).await,
        #[cfg(feature = "mcp")]
        Commands::McpServer { agent_role, agent_id, stdio, run_id } => {
            if !stdio {
                return Err("Only --stdio transport is currently supported".to_string());
            }
            let role = mycelica_lib::mcp::AgentRole::from_str(&agent_role)
                .ok_or_else(|| format!("Unknown role: '{}'. Valid: human, ingestor, coder, verifier, planner, synthesizer, summarizer, docwriter", agent_role))?;
            mycelica_lib::mcp::run_mcp_server(db.clone(), agent_id, role, run_id).await
        },
        Commands::Tui => run_tui(&db).await,
        Commands::Completions { .. } => unreachable!(),
    }
}

/// Route commands through RemoteClient when --remote is set.
async fn run_remote(cli: Cli, remote_url: &str) -> Result<(), String> {
    use mycelica_lib::remote_client::{RemoteClient, CreateNodeRequest, CreateEdgeRequest};

    let client = RemoteClient::new(remote_url);
    let json = cli.json;
    let author = Some(settings::get_author_or_default());

    match cli.command {
        Commands::Search { query, limit, .. } => {
            let nodes = client.search(&query, limit).await?;
            if json {
                println!("{}", serde_json::to_string(&nodes).unwrap_or_default());
            } else {
                for node in &nodes {
                    println!("{} {} [{}]",
                        &node.id[..8.min(node.id.len())],
                        node.ai_title.as_ref().unwrap_or(&node.title),
                        node.content_type.as_deref().unwrap_or("?"));
                }
                if !json { eprintln!("{} results", nodes.len()); }
            }
            Ok(())
        }
        Commands::Add { url, note, tag, connects_to } => {
            let req = CreateNodeRequest {
                title: note.clone().unwrap_or_else(|| url.clone()),
                content: Some(format!("{}\n\n{}", url, note.as_deref().unwrap_or(""))),
                url: Some(url),
                content_type: Some("reference".to_string()),
                tags: tag.map(|t| {
                    let tags: Vec<&str> = t.split(',').map(|s| s.trim()).collect();
                    serde_json::to_string(&tags).unwrap_or_default()
                }),
                author,
                connects_to,
                is_item: None,
            };
            let resp = client.create_node(&req).await?;
            if json {
                println!("{}", serde_json::to_string(&resp.node).unwrap_or_default());
            } else {
                println!("Added: {} {}", &resp.node.id[..8], resp.node.title);
                for edge in &resp.edges_created {
                    println!("  Linked to: {} {}", &edge.target_id[..8.min(edge.target_id.len())], edge.target_title);
                }
                for amb in &resp.ambiguous {
                    eprintln!("  Ambiguous '{}': {} candidates", amb.term, amb.candidates.len());
                }
            }
            Ok(())
        }
        Commands::Ask { question, connects_to } => {
            let req = CreateNodeRequest {
                title: question,
                content: None,
                url: None,
                content_type: Some("question".to_string()),
                tags: None,
                author,
                connects_to,
                is_item: None,
            };
            let resp = client.create_node(&req).await?;
            if json {
                println!("{}", serde_json::to_string(&resp.node).unwrap_or_default());
            } else {
                println!("Asked: {} {}", &resp.node.id[..8], resp.node.title);
            }
            Ok(())
        }
        Commands::Concept { title, note, connects_to } => {
            let req = CreateNodeRequest {
                title,
                content: note,
                url: None,
                content_type: Some("concept".to_string()),
                tags: None,
                author,
                connects_to,
                is_item: None,
            };
            let resp = client.create_node(&req).await?;
            if json {
                println!("{}", serde_json::to_string(&resp.node).unwrap_or_default());
            } else {
                println!("Concept: {} {}", &resp.node.id[..8], resp.node.title);
            }
            Ok(())
        }
        Commands::Decide { title, reason, connects_to } => {
            let req = CreateNodeRequest {
                title,
                content: reason,
                url: None,
                content_type: Some("decision".to_string()),
                tags: None,
                author,
                connects_to,
                is_item: None,
            };
            let resp = client.create_node(&req).await?;
            if json {
                println!("{}", serde_json::to_string(&resp.node).unwrap_or_default());
            } else {
                println!("Decision: {} {}", &resp.node.id[..8], resp.node.title);
            }
            Ok(())
        }
        Commands::Link { source, target, edge_type, reason, content: _, agent: _, confidence: _, supersedes: _ } => {
            let req = CreateEdgeRequest {
                source: source,
                target: target,
                edge_type: Some(edge_type),
                reason,
                author,
            };
            let resp = client.create_edge(&req).await?;
            if json {
                println!("{}", serde_json::to_string(&resp).unwrap_or_default());
            } else {
                println!("Linked: {} ({}) -> {} ({}) [{}]",
                    &resp.source_resolved.id[..8.min(resp.source_resolved.id.len())],
                    resp.source_resolved.title,
                    &resp.target_resolved.id[..8.min(resp.target_resolved.id.len())],
                    resp.target_resolved.title,
                    resp.edge.edge_type.as_str());
            }
            Ok(())
        }
        Commands::Orphans { limit } => {
            let nodes = client.get_orphans(limit).await?;
            if json {
                println!("{}", serde_json::to_string(&nodes).unwrap_or_default());
            } else {
                for node in &nodes {
                    println!("{} {} [{}]",
                        &node.id[..8.min(node.id.len())],
                        node.ai_title.as_ref().unwrap_or(&node.title),
                        node.content_type.as_deref().unwrap_or("?"));
                }
                eprintln!("{} orphans", nodes.len());
            }
            Ok(())
        }
        Commands::Recent { cmd } => {
            match cmd {
                RecentCommands::List { limit, .. } => {
                    let nodes = client.get_recent(limit as u32).await?;
                    if json {
                        println!("{}", serde_json::to_string(&nodes).unwrap_or_default());
                    } else {
                        for node in &nodes {
                            println!("{} {} [{}]",
                                &node.id[..8.min(node.id.len())],
                                node.ai_title.as_ref().unwrap_or(&node.title),
                                node.content_type.as_deref().unwrap_or("?"));
                        }
                    }
                    Ok(())
                }
                _ => Err("This recent subcommand is not available in remote mode".to_string()),
            }
        }
        _ => Err(format!("This command is not available in remote mode. Run without --remote for local operations.")),
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
                #[derive(Serialize)]
                struct DbStatsJson {
                    path: String,
                    items: usize,
                    categories: usize,
                    edges: usize,
                    processed: usize,
                    embeddings: usize,
                    hierarchy_levels: i32,
                    hierarchy_root: Option<String>,
                }
                let stats = DbStatsJson {
                    path: db.get_path(),
                    items: total_items,
                    categories,
                    edges: edges.len(),
                    processed: processed_items,
                    embeddings: items_with_embeddings,
                    hierarchy_levels: max_depth,
                    hierarchy_root: universe.as_ref().map(|u| u.title.clone()),
                };
                println!("{}", serde_json::to_string(&stats).unwrap());
            } else {
                log!("Database:   {}", db.get_path());
                log!("Items:      {:>6}", total_items);
                log!("Categories: {:>6}", categories);
                log!("Edges:      {:>6}", edges.len());
                log!("Processed:  {:>6} / {}", processed_items, total_items);
                log!("Embeddings: {:>6} / {}", items_with_embeddings, total_items);
                if let Some(u) = universe {
                    log!("Hierarchy:  {} levels, root=\"{}\"", max_depth, u.title);
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
        ImportCommands::Openaire { query, country, fos, from_year, to_year, max, download_pdfs, max_pdf_size, unpaywall_email, core_api_key } => {
            let api_key = settings::get_openaire_api_key();

            // Get Unpaywall email from CLI flag or settings
            let unpaywall_email = unpaywall_email.or_else(|| settings::get_unpaywall_email());

            // Get CORE API key from CLI flag or settings
            let core_api_key = core_api_key.or_else(|| settings::get_core_api_key());

            if !quiet {
                log!("[OpenAIRE] Searching: \"{}\"", query);
                if let Some(ref c) = country {
                    log!("[OpenAIRE]   Country: {}", c);
                }
                if download_pdfs {
                    log!("[PDF Resolver] Multi-source fallback enabled:");
                    log!("  1. arXiv (direct, ~95% success)");
                    log!("  2. PMC (direct, ~80% success)");
                    if unpaywall_email.is_some() {
                        log!("  3. Unpaywall (lookup, ~26% success)");
                    }
                    if core_api_key.is_some() {
                        log!("  4. CORE (lookup, ~9% success)");
                    }
                    log!("  5. OpenAIRE URLs (baseline, ~5% success)");
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
                unpaywall_email,
                core_api_key,
                on_progress,
            ).await?;

            if !quiet {
                log!("");
            }

            if json {
                println!(r#"{{"papers_imported":{},"pdfs_downloaded":{},"duplicates_skipped":{},"errors":{}}}"#,
                    result.papers_imported, result.pdfs_downloaded, result.duplicates_skipped, result.errors.len());
            } else {
                log!("Imported {} papers, {} PDFs, {} duplicates skipped",
                    result.papers_imported, result.pdfs_downloaded, result.duplicates_skipped);
                if !result.errors.is_empty() {
                    log!("Errors: {}", result.errors.len());
                    for (i, err) in result.errors.iter().take(5).enumerate() {
                        log!("  {}. {}", i + 1, err);
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
                log!("Imported {} files, {} items", result.conversations_imported, result.exchanges_imported);
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
                log!("Imported {} conversations, {} exchanges",
                    result.conversations_imported, result.exchanges_imported);
            }
        }
        ImportCommands::Chatgpt { path } => {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read file: {}", e))?;

            let result = import::import_chatgpt_conversations(db, &content)?;

            if json {
                println!(r#"{{"conversations_imported":{},"exchanges_imported":{},"skipped":{},"errors":{}}}"#,
                    result.conversations_imported, result.exchanges_imported, result.skipped, result.errors.len());
            } else {
                log!("Imported {} conversations, {} exchanges",
                    result.conversations_imported, result.exchanges_imported);
            }
        }
        ImportCommands::Keep { path } => {
            let result = import::import_google_keep(db, &path)?;

            if json {
                println!(r#"{{"notes_imported":{},"skipped":{},"errors":{}}}"#,
                    result.notes_imported, result.skipped, result.errors.len());
            } else {
                log!("Imported {} notes, {} skipped", result.notes_imported, result.skipped);
            }
        }
        ImportCommands::Code { path, language, update } => {
            use mycelica_lib::code;
            use mycelica_lib::ai_client;

            if !quiet {
                if update {
                    log!("[Code] Update mode: {} (will replace existing nodes)", path);
                } else {
                    log!("[Code] Scanning: {} (respects .gitignore)", path);
                }
                if let Some(ref lang) = language {
                    log!("[Code]   Language filter: {}", lang);
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
                                log!("  Deleted {} nodes from {}", deleted, file);
                            }
                            total_deleted += deleted;
                        }
                        _ => {}
                    }
                }
                if !quiet && total_deleted > 0 {
                    log!("[Code] Deleted {} existing nodes", total_deleted);
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
                        log!("[Code] Generating embeddings for {} nodes...", new_node_ids.len());
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
                        log!("[Code] Refreshing Calls edges for {} functions...", functions.len());
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
                                        updated_at: Some(now),
                                        author: None,
                                        reason: None,
                                        content: None,
                                        agent_id: None,
                                        superseded_by: None,
                                        metadata: None,
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
                    log!("Updated {} files (replaced {} existing nodes):", result.files_processed, total_deleted);
                } else {
                    log!("Imported {} items from {} files:", result.total_items(), result.files_processed);
                }
                if result.functions > 0 { log!("  Functions: {}", result.functions); }
                if result.structs > 0 { log!("  Structs: {}", result.structs); }
                if result.enums > 0 { log!("  Enums: {}", result.enums); }
                if result.traits > 0 { log!("  Traits: {}", result.traits); }
                if result.impls > 0 { log!("  Impl blocks: {}", result.impls); }
                if result.modules > 0 { log!("  Modules: {}", result.modules); }
                if result.macros > 0 { log!("  Macros: {}", result.macros); }
                if result.docs > 0 { log!("  Docs: {}", result.docs); }
                // TypeScript/JavaScript
                if result.classes > 0 { log!("  Classes: {}", result.classes); }
                if result.interfaces > 0 { log!("  Interfaces: {}", result.interfaces); }
                if result.types > 0 { log!("  Types: {}", result.types); }
                if result.consts > 0 { log!("  Consts: {}", result.consts); }
                log!("  Edges created: {}", result.edges_created);
                if result.doc_edges > 0 { log!("  Doc→code edges: {}", result.doc_edges); }
                if update {
                    if embeddings_generated > 0 { log!("  Embeddings generated: {}", embeddings_generated); }
                    if calls_edges_created > 0 { log!("  Calls edges refreshed: {}", calls_edges_created); }
                }
                if result.files_skipped > 0 {
                    log!("  Files skipped: {}", result.files_skipped);
                }
                if !result.errors.is_empty() {
                    elog!("\nErrors ({}):", result.errors.len());
                    for err in &result.errors[..result.errors.len().min(5)] {
                        elog!("  {}", err);
                    }
                    if result.errors.len() > 5 {
                        elog!("  ... and {} more", result.errors.len() - 5);
                    }
                }
            }
        }
        #[cfg(feature = "signal")]
        ImportCommands::Signal { conversation_id, list, author_map, list_authors, db_path, embed, allow_repo_db } => {
            use mycelica_lib::signal;

            // Determine Signal DB path
            let signal_db = db_path.unwrap_or_else(|| {
                dirs::config_dir()
                    .map(|p| p.join("Signal/sql/db.sqlite"))
                    .unwrap_or_else(|| std::path::PathBuf::from(
                        format!("{}/.config/Signal/sql/db.sqlite", std::env::var("HOME").unwrap_or_default())
                    ))
                    .to_string_lossy()
                    .to_string()
            });

            // Derive config.json path from DB path
            let config_path = std::path::PathBuf::from(&signal_db)
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("config.json"))
                .unwrap_or_else(|| {
                    dirs::config_dir()
                        .map(|p| p.join("Signal/config.json"))
                        .unwrap_or_else(|| std::path::PathBuf::from(
                            format!("{}/.config/Signal/config.json", std::env::var("HOME").unwrap_or_default())
                        ))
                });

            if !quiet {
                log!("[Signal] Reading key from {:?}", config_path);
            }

            let signal_key = signal::read_signal_key(&config_path.to_string_lossy())?;

            // List mode
            if list {
                let conversations = signal::list_signal_conversations(&signal_db, &signal_key)?;
                if json {
                    let items: Vec<serde_json::Value> = conversations.iter()
                        .map(|(id, name, count)| serde_json::json!({
                            "id": id,
                            "name": name,
                            "message_count": count
                        }))
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&items).unwrap_or_default());
                } else {
                    log!("Available Signal conversations:");
                    for (id, name, count) in &conversations {
                        log!("  {} — {} ({} messages)", id, name, count);
                    }
                }
                return Ok(());
            }

            // List authors mode: show sourceServiceId UUIDs for building --author-map
            if list_authors {
                let conv_id = conversation_id
                    .ok_or_else(|| "--conversation-id is required with --list-authors".to_string())?;
                let authors = signal::list_conversation_authors(&signal_db, &signal_key, &conv_id)?;
                if json {
                    let items: Vec<serde_json::Value> = authors.iter()
                        .map(|(uuid, count)| serde_json::json!({
                            "sourceServiceId": uuid,
                            "message_count": count
                        }))
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&items).unwrap_or_default());
                } else {
                    log!("Participants in {}:", conv_id);
                    for (uuid, count) in &authors {
                        log!("  {} ({} messages)", uuid, count);
                    }
                    let mappable: Vec<&(String, usize)> = authors.iter()
                        .filter(|(uuid, _)| !uuid.starts_with('('))
                        .collect();
                    if !mappable.is_empty() {
                        log!("\nBuild --author-map with:");
                        let example: Vec<String> = mappable.iter()
                            .map(|(uuid, _)| format!("{}=NAME", uuid))
                            .collect();
                        log!("  --author-map \"{}\"", example.join(","));
                    }
                }
                return Ok(());
            }

            // Guard: refuse to import into a database inside a git repo
            if !allow_repo_db {
                let db_abs = std::fs::canonicalize(db.get_path()).unwrap_or_else(|_| std::path::PathBuf::from(db.get_path()));
                let mut check_dir = db_abs.parent();
                while let Some(dir) = check_dir {
                    if dir.join(".git").exists() {
                        return Err(format!(
                            "Refusing to import Signal messages into {} — it is inside a git repository ({}).\n\
                             Private messages could be pushed to a remote.\n\
                             Use --db <path> to target a database outside the repo, or --allow-repo-db to override.",
                            db.get_path(), dir.display()
                        ));
                    }
                    check_dir = dir.parent();
                }
            }

            // First-run warning: Signal messages stored unencrypted
            {
                let ack_path = dirs::data_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("~/.local/share"))
                    .join("com.mycelica.app/signal-ack");
                if !ack_path.exists() {
                    let db_display = db.get_path();
                    eprintln!("Signal messages will be stored unencrypted in Mycelica's database at {}.", db_display);
                    eprintln!("They may be included in system backups.");
                    eprint!("Continue? [y/N] ");
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input)
                        .map_err(|e| format!("Failed to read input: {}", e))?;
                    if !input.trim().eq_ignore_ascii_case("y") {
                        return Err("Aborted.".to_string());
                    }
                    if let Some(parent) = ack_path.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                    std::fs::write(&ack_path, "acknowledged\n").ok();
                }
            }

            // Require conversation_id for actual import
            let conv_id = conversation_id
                .ok_or_else(|| "--conversation-id is required (use --list to find IDs)".to_string())?;

            // Parse author map
            let authors = author_map.as_deref()
                .map(signal::parse_author_map)
                .unwrap_or_default();

            if !quiet {
                log!("[Signal] Importing conversation {}...", conv_id);
                if !authors.is_empty() {
                    log!("[Signal] Author map: {} entries", authors.len());
                }
            }

            let result = signal::import_signal_conversation(db, &signal_db, &signal_key, &conv_id, &authors)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?);
            } else {
                log!("[Signal] Import complete:");
                log!("  Messages processed: {}", result.messages_processed);
                log!("  Nodes created: {}", result.nodes_created);
                if result.nodes_skipped_dedup > 0 { log!("  Nodes skipped (dedup): {}", result.nodes_skipped_dedup); }
                if result.nodes_skipped_filter > 0 { log!("  Nodes skipped (filter): {}", result.nodes_skipped_filter); }
                if result.metadata_attached > 0 { log!("  Reactions attached: {}", result.metadata_attached); }
                if result.edits_detected > 0 { log!("  Edits detected: {}", result.edits_detected); }
                log!("  Edges created: {}", result.edges_created);
                if result.replies_found > 0 { log!("  Replies: {}", result.replies_found); }
                if result.links_found > 0 { log!("  URLs found: {}", result.links_found); }
                if result.link_nodes_created > 0 { log!("  Link nodes: {}", result.link_nodes_created); }
                if result.temporal_threads > 0 { log!("  Temporal threads: {}", result.temporal_threads); }
                if result.decisions_detected > 0 { log!("  Decisions detected: {}", result.decisions_detected); }
                if !result.errors.is_empty() {
                    elog!("  Errors: {}", result.errors.len());
                    for err in &result.errors[..result.errors.len().min(5)] {
                        elog!("    {}", err);
                    }
                }
            }

            // Optional embedding generation
            if embed && result.nodes_created > 0 {
                use mycelica_lib::ai_client;

                if !quiet {
                    log!("[Signal] Generating embeddings for {} new nodes...", result.nodes_created);
                }

                let mut embeddings_generated = 0;
                // Get all signal nodes from this conversation that lack embeddings
                if let Ok(items) = db.get_items() {
                    let signal_nodes: Vec<_> = items.into_iter()
                        .filter(|n| n.source.as_deref() == Some("signal")
                            && n.conversation_id.as_deref() == Some(&format!("signal-conv-{}", &conv_id[..conv_id.len().min(8)])))
                        .collect();

                    for node in &signal_nodes {
                        if db.get_node_embedding(&node.id).ok().flatten().is_none() {
                            let text = format!("{}\n{}", node.title, node.content.as_deref().unwrap_or(""));
                            let embed_text = mycelica_lib::utils::safe_truncate(&text, 1000);
                            if let Ok(embedding) = ai_client::generate_embedding(embed_text).await {
                                db.update_node_embedding(&node.id, &embedding).ok();
                                embeddings_generated += 1;
                            }
                        }
                    }
                }

                if !quiet {
                    log!("[Signal] Generated {} embeddings", embeddings_generated);
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
                db.get_all_nodes(false).map_err(|e| e.to_string())?
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
                    let marker = if node.is_item { "[I]" } else { "[C]" };
                    let processed_marker = if node.is_processed { "+" } else { "o" };
                    log!("{} {} {} {}", marker, processed_marker, &node.id[..8], node.title);
                }
                log!("\n{} nodes", filtered.len());
            }
        }
        NodeCommands::Get { id, full } => {
            let node = db.get_node(&id).map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Node not found: {}", id))?;

            if json {
                println!(r#"{{"id":"{}","title":"{}","content":{},"is_item":{},"is_processed":{},"depth":{},"child_count":{},"parent_id":{},"tags":{},"node_class":{},"meta_type":{},"agent_id":{},"source":{}}}"#,
                    node.id,
                    escape_json(&node.title),
                    node.content.as_ref().map(|c| format!("\"{}\"", escape_json(c))).unwrap_or("null".to_string()),
                    node.is_item,
                    node.is_processed,
                    node.depth,
                    node.child_count,
                    node.parent_id.as_ref().map(|p| format!("\"{}\"", p)).unwrap_or("null".to_string()),
                    node.tags.as_ref().map(|t| format!("\"{}\"", escape_json(t))).unwrap_or("null".to_string()),
                    node.node_class.as_ref().map(|c| format!("\"{}\"", c)).unwrap_or("null".to_string()),
                    node.meta_type.as_ref().map(|m| format!("\"{}\"", m)).unwrap_or("null".to_string()),
                    node.agent_id.as_ref().map(|a| format!("\"{}\"", a)).unwrap_or("null".to_string()),
                    node.source.as_ref().map(|s| format!("\"{}\"", s)).unwrap_or("null".to_string()),
                );
            } else {
                log!("ID:       {}", node.id);
                log!("Title:    {}", node.title);
                log!("Type:     {}", if node.is_item { "Item" } else { "Category" });
                log!("Depth:    {}", node.depth);
                log!("Children: {}", node.child_count);
                if let Some(ref parent) = node.parent_id {
                    log!("Parent:   {}", parent);
                }
                if let Some(ref nc) = node.node_class {
                    log!("Class:    {}", nc);
                }
                if let Some(ref mt) = node.meta_type {
                    log!("MetaType: {}", mt);
                }
                if let Some(ref aid) = node.agent_id {
                    log!("Agent:    {}", aid);
                }
                if let Some(ref src) = node.source {
                    log!("Source:   {}", src);
                }
                if let Some(ref tags) = node.tags {
                    log!("Tags:     {}", tags);
                }
                if let Some(ref summary) = node.summary {
                    log!("\nSummary:\n{}", summary);
                }
                if let Some(ref content) = node.content {
                    if full {
                        log!("\nContent:\n{}", content);
                    } else {
                        let preview = if content.len() > 500 { &content[..500] } else { content };
                        log!("\nContent:\n{}", preview);
                    }
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
                human_edited: None,
                human_created: false,
                author: None,
                agent_id: None,
                node_class: None,
                meta_type: None,
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
            if !quiet { elog!("Building hierarchy..."); }
            hierarchy::build_hierarchy(db)?;
            if json {
                log!(r#"{{"status":"ok"}}"#);
            } else {
                log!("Hierarchy built successfully");
            }
        }
        HierarchyCommands::Rebuild { algorithm, levels, method, min_size, thresholds, cohesion_threshold, delta_min, tight_threshold, auto, keywords_only, fresh } => {
            match algorithm.as_str() {
                "adaptive" => {
                    if !quiet { elog!("Rebuilding hierarchy with adaptive v2 algorithm..."); }
                    let result = rebuild_hierarchy_adaptive(db, min_size, tight_threshold, cohesion_threshold, delta_min, auto, json, quiet, keywords_only, fresh).await?;
                    if json {
                        log!(r#"{{"status":"ok","categories":{},"papers_assigned":{},"sibling_edges":{},"bridges":{}}}"#,
                            result.categories, result.papers_assigned, result.sibling_edges, result.bridges);
                    } else {
                        log!("Hierarchy rebuilt: {} categories, {} papers assigned, {} sibling edges, {} bridge papers",
                            result.categories, result.papers_assigned, result.sibling_edges, result.bridges);
                    }
                }
                "dendrogram" => {
                    if !quiet { elog!("Rebuilding hierarchy with dendrogram algorithm..."); }
                    let result = rebuild_hierarchy_dendrogram(db, levels, &method, min_size, thresholds.as_deref(), json, quiet).await?;
                    if json {
                        log!(r#"{{"status":"ok","categories":{},"papers_assigned":{},"ai_calls":{}}}"#,
                            result.categories, result.papers_assigned, result.ai_calls);
                    } else {
                        log!("Hierarchy rebuilt: {} categories, {} papers assigned, {} AI naming calls",
                            result.categories, result.papers_assigned, result.ai_calls);
                    }
                }
                other => {
                    return Err(format!("Unknown algorithm '{}'. Valid options: adaptive, dendrogram", other));
                }
            }
        }
        HierarchyCommands::Flatten => {
            if !quiet { elog!("Flattening single-child chains..."); }
            db.flatten_single_child_chains().map_err(|e| e.to_string())?;
            if json {
                log!(r#"{{"status":"ok"}}"#);
            } else {
                log!("Flattened successfully");
            }
        }
        HierarchyCommands::Analyze { recommend } => {
            use mycelica_lib::dendrogram::{compute_edge_stats, auto_config};

            // Load edges
            let edges = db.get_all_item_edges_sorted().map_err(|e| e.to_string())?;
            if edges.is_empty() {
                return Err("No edges found. Run 'mycelica-cli setup' to create semantic edges first.".to_string());
            }

            // Count papers
            let mut paper_set = std::collections::HashSet::new();
            for (source, target, _) in &edges {
                paper_set.insert(source.clone());
                paper_set.insert(target.clone());
            }
            let n_papers = paper_set.len();

            // Compute statistics
            if let Some(stats) = compute_edge_stats(&edges) {
                if json {
                    log!(r#"{{"edges":{},"papers":{},"mean":{:.4},"std":{:.4},"min":{:.4},"max":{:.4},"p10":{:.4},"p25":{:.4},"p50":{:.4},"p75":{:.4},"p90":{:.4}}}"#,
                        stats.count, n_papers, stats.mean, stats.std_dev, stats.min, stats.max,
                        stats.p10, stats.p25, stats.p50, stats.p75, stats.p90);
                } else {
                    log!("Edge Weight Analysis:");
                    log!("  Total edges: {}", stats.count);
                    log!("  Total papers: {}", n_papers);
                    log!("  Mean: {:.4}, Std: {:.4}", stats.mean, stats.std_dev);
                    log!("  Range: [{:.4}, {:.4}]", stats.min, stats.max);
                    log!("  Percentiles: p10={:.4}, p25={:.4}, p50={:.4}, p75={:.4}, p90={:.4}",
                        stats.p10, stats.p25, stats.p50, stats.p75, stats.p90);

                    if recommend {
                        use mycelica_lib::dendrogram::{dynamic_min_ratio, dynamic_cohesion_threshold, reference};

                        let cfg = auto_config(&edges, n_papers);
                        log!("");
                        log!("Distribution Characteristics:");
                        log!("  IQR: {:.4} (reference: {:.4})", cfg.iqr, reference::IQR);
                        log!("  Density: {:.4} (reference: {:.4})", cfg.edge_density, reference::EDGE_DENSITY);
                        log!("");
                        log!("Dynamic Scaling (for 1000 papers):");
                        log!("  Balance ratio: {:.4} (base: {:.4})", dynamic_min_ratio(1000, cfg.iqr), 0.01);
                        log!("  Cohesion: {:.2} (base: {:.2})", dynamic_cohesion_threshold(1.0, cfg.edge_density), 1.0);
                        log!("");
                        log!("Recommended Auto-Config:");
                        log!("  --min-size {}", cfg.min_size);
                        log!("  --cohesion-threshold {:.2}", cfg.cohesion_threshold);
                        log!("  --delta-min {:.3}", cfg.delta_min);
                        log!("  --tight-threshold {:.4}", cfg.tight_threshold);
                        log!("");
                        log!("Or use: hierarchy rebuild --algorithm adaptive --auto");
                    }
                }
            } else {
                return Err("Could not compute edge statistics (no edges?)".to_string());
            }
        }
        HierarchyCommands::Stats => {
            let max_depth = db.get_max_depth().map_err(|e| e.to_string())?;
            let universe = db.get_universe().map_err(|e| e.to_string())?;

            if json {
                log!(r#"{{"max_depth":{},"has_universe":{}}}"#, max_depth, universe.is_some());
            } else {
                log!("Max depth: {}", max_depth);
                if let Some(u) = universe {
                    log!("Universe:  {} ({} children)", u.title, u.child_count);
                } else {
                    log!("Universe:  None (run 'hierarchy build')");
                }

                // Show counts per depth
                for d in 0..=max_depth {
                    if let Ok(nodes) = db.get_nodes_at_depth(d) {
                        log!("  Depth {}: {} nodes", d, nodes.len());
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
                        log!(r#"{{"status":"already_correct","parent":"{}"}}"#, universe.id);
                    } else {
                        log!("Recent Notes is already under Universe");
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
                        log!(r#"{{"status":"fixed","old_parent":"{}","new_parent":"{}"}}"#, old_parent, universe.id);
                    } else {
                        log!("Moved Recent Notes from '{}' to Universe (depth 1)", old_parent);
                    }
                }
            } else {
                if json {
                    log!(r#"{{"status":"not_found"}}"#);
                } else {
                    log!("Recent Notes container not found");
                }
            }
        }
        HierarchyCommands::SmartAdd => {
            handle_smart_add(&db, json).await?;
        }
        HierarchyCommands::Dendrogram { levels, method } => {
            handle_dendrogram_test(&db, levels, &method, json, quiet)?;
        }
        HierarchyCommands::Consolidate => {
            handle_consolidate(&db, json, quiet).await?;
        }
        HierarchyCommands::Unconsolidate => {
            handle_unconsolidate(&db, json, quiet)?;
        }
    }
    Ok(())
}

// ============================================================================
// Dendrogram Test
// ============================================================================

fn handle_dendrogram_test(db: &Database, target_levels: usize, method: &str, json: bool, quiet: bool) -> Result<(), String> {
    use mycelica_lib::dendrogram::{build_dendrogram, find_natural_thresholds, find_percentile_thresholds, fixed_thresholds, extract_levels, DendrogramConfig};
    use std::time::Instant;

    let start = Instant::now();

    // Step 1: Get all item edges sorted by weight
    log!("Loading edges...");
    let edges = db.get_all_item_edges_sorted().map_err(|e| e.to_string())?;
    log!("  {} edges loaded in {:.2}s", edges.len(), start.elapsed().as_secs_f64());

    if edges.is_empty() {
        if json {
            log!(r#"{{"error":"no_edges"}}"#);
        } else {
            log!("No item edges found. Run 'mycelica-cli setup' first to create semantic edges.");
        }
        return Ok(());
    }

    // Step 2: Get unique paper IDs from edges
    log!("Extracting paper IDs...");
    let mut paper_set = std::collections::HashSet::new();
    for (source, target, _) in &edges {
        paper_set.insert(source.clone());
        paper_set.insert(target.clone());
    }
    let papers: Vec<String> = paper_set.into_iter().collect();
    log!("  {} unique papers", papers.len());

    // Step 3: Build dendrogram
    log!("Building dendrogram...");
    let dendro_start = Instant::now();
    let dendrogram = build_dendrogram(papers.clone(), edges.clone());
    log!("  {} merges recorded in {:.3}s", dendrogram.merges.len(), dendro_start.elapsed().as_secs_f64());

    // Step 4: Find thresholds using selected method
    log!("Finding thresholds (method={}, target {} levels)...", method, target_levels);
    let thresholds = match method {
        "gap" => find_natural_thresholds(&dendrogram, target_levels),
        "percentile" => find_percentile_thresholds(&dendrogram, target_levels),
        "fixed" => fixed_thresholds(&[0.8, 0.7, 0.6, 0.5]),
        _ => {
            log!("Unknown method '{}', using percentile", method);
            find_percentile_thresholds(&dendrogram, target_levels)
        }
    };
    log!("  Thresholds: {:?}", thresholds);

    // Step 5: Extract levels
    log!("Extracting hierarchy levels...");
    let levels = extract_levels(&dendrogram, &thresholds);

    // Step 6: Report statistics
    log!("\n=== Dendrogram Analysis ===");
    log!("Papers:     {}", papers.len());
    log!("Edges:      {}", edges.len());
    log!("Merges:     {}", dendrogram.merges.len());
    log!("Levels:     {}", levels.levels.len());

    // Weight distribution
    if !dendrogram.merges.is_empty() {
        let weights: Vec<f64> = dendrogram.merges.iter().map(|m| m.weight).collect();
        let min_w = weights.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_w = weights.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let avg_w: f64 = weights.iter().sum::<f64>() / weights.len() as f64;
        log!("\nWeight distribution:");
        log!("  Min: {:.4}", min_w);
        log!("  Max: {:.4}", max_w);
        log!("  Avg: {:.4}", avg_w);
    }

    // Level breakdown
    log!("\nLevel breakdown:");
    for (i, level) in levels.levels.iter().enumerate() {
        let threshold = levels.thresholds.get(i).map(|t| format!("{:.3}", t)).unwrap_or_else(|| "root".to_string());
        let total_papers: usize = level.iter().map(|c| c.papers.len()).sum::<usize>();
        let sizes: Vec<usize> = level.iter().map(|c| c.papers.len()).collect();
        let min_size = sizes.iter().min().copied().unwrap_or(0);
        let max_size = sizes.iter().max().copied().unwrap_or(0);

        log!("  Level {} (threshold ≥ {}): {} components, {} papers (sizes: {}-{})",
            i, threshold, level.len(), total_papers, min_size, max_size);
    }

    // Large components (potential subdivision candidates)
    let config = DendrogramConfig::default();
    log!("\nLarge components (>{} papers):", config.max_component_size);
    let mut large_count = 0;
    for (level_idx, level) in levels.levels.iter().enumerate() {
        for comp in level {
            if comp.papers.len() > config.max_component_size {
                log!("  Level {}: {} papers", level_idx, comp.papers.len());
                large_count += 1;
            }
        }
    }
    if large_count == 0 {
        log!("  (none)");
    }

    // Small components (below naming threshold)
    log!("\nSmall components (<{} papers, would skip naming):", config.min_component_size);
    let mut small_count = 0;
    for level in &levels.levels {
        for comp in level {
            if comp.papers.len() < config.min_component_size && comp.papers.len() > 0 {
                small_count += 1;
            }
        }
    }
    log!("  {} components", small_count);

    let total_time = start.elapsed().as_secs_f64();
    log!("\nTotal time: {:.2}s", total_time);

    if json {
        log!(r#"{{"papers":{},"edges":{},"merges":{},"levels":{},"thresholds":{:?},"time_ms":{}}}"#,
            papers.len(), edges.len(), dendrogram.merges.len(), levels.levels.len(), thresholds, (total_time * 1000.0) as u64);
    }

    Ok(())
}

// ============================================================================
// Consolidate / Unconsolidate Root
// ============================================================================

async fn handle_consolidate(db: &Database, json: bool, quiet: bool) -> Result<(), String> {
    use mycelica_lib::ai_client::{self, TopicInfo};
    use mycelica_lib::hierarchy;
    use mycelica_lib::db::{Node, NodeType, Position};
    use mycelica_lib::similarity;
    use std::time::Instant;

    let start = Instant::now();

    // Get Universe
    let universe = db.get_universe()
        .map_err(|e| e.to_string())?
        .ok_or("No Universe node found")?;

    // Get Universe's direct children (excluding protected)
    let all_children = db.get_children(&universe.id).map_err(|e| e.to_string())?;
    let protected_ids = db.get_protected_node_ids();
    let children: Vec<_> = all_children
        .into_iter()
        .filter(|child| !protected_ids.contains(&child.id))
        .collect();

    if !protected_ids.is_empty() && !quiet {
        log!("Excluding {} protected nodes", protected_ids.len());
    }

    if children.is_empty() {
        return Err("Universe has no children to consolidate".to_string());
    }

    if children.len() <= 8 {
        return Err(format!("Universe only has {} children - already consolidated enough", children.len()));
    }

    if !quiet {
        log!("Grouping {} categories into uber-categories...", children.len());
    }

    // Build topic info for AI
    let categories: Vec<TopicInfo> = children
        .iter()
        .map(|child| TopicInfo {
            id: child.id.clone(),
            label: child.cluster_label
                .clone()
                .or_else(|| child.ai_title.clone())
                .unwrap_or_else(|| child.title.clone()),
            item_count: child.child_count.max(1),
        })
        .collect();

    // Pre-fetch embeddings for similarity-sorted batching
    let embeddings_map: std::collections::HashMap<String, Vec<f32>> = categories
        .iter()
        .filter_map(|c| {
            db.get_node_embedding(&c.id)
                .ok()
                .flatten()
                .map(|emb| (c.id.clone(), emb))
        })
        .collect();

    if !quiet {
        log!("Fetched {}/{} topic embeddings", embeddings_map.len(), categories.len());
    }

    // Call AI to group into uber-categories
    let groupings = ai_client::group_into_uber_categories(&categories, &embeddings_map, None).await?;

    if groupings.is_empty() {
        return Err("AI returned no uber-categories".to_string());
    }

    if !quiet {
        log!("AI created {} uber-categories", groupings.len());
    }

    // Create map from label -> child nodes
    let mut label_to_children: std::collections::HashMap<String, Vec<_>> = std::collections::HashMap::new();
    for child in &children {
        let label = child.cluster_label
            .as_ref()
            .or(child.ai_title.as_ref())
            .unwrap_or(&child.title)
            .clone();
        label_to_children.entry(label).or_default().push(child);
    }

    // Generate timestamp for unique IDs
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let now = chrono::Utc::now().timestamp_millis();

    let mut uber_categories_created = 0;
    let mut categories_reparented = 0;
    let mut uber_category_ids: Vec<String> = Vec::new();
    let mut all_children_to_update: Vec<String> = Vec::new();

    for (idx, grouping) in groupings.iter().enumerate() {
        let uber_id = format!("uber-{}-{}", timestamp, idx);

        // Find matching children for this grouping
        let mut matching_children: Vec<&Node> = Vec::new();
        for member_label in &grouping.children {
            if let Some(nodes) = label_to_children.get(member_label) {
                matching_children.extend(nodes.iter());
            }
        }

        if matching_children.is_empty() {
            continue;
        }

        // Create uber-category node
        let uber_node = Node {
            id: uber_id.clone(),
            node_type: NodeType::Cluster,
            title: grouping.name.clone(),
            url: None,
            content: grouping.description.clone(),
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: Some(grouping.name.clone()),
            depth: 1,
            is_item: false,
            is_universe: false,
            parent_id: Some(universe.id.clone()),
            child_count: matching_children.len() as i32,
            ai_title: None,
            summary: grouping.description.clone(),
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
            source: None,
            pdf_available: None,
            content_type: None,
            associated_idea_id: None,
            privacy: None,
            human_edited: None,
            human_created: false,
            author: None,
            agent_id: None,
            node_class: None,
            meta_type: None,
        };

        db.insert_node(&uber_node).map_err(|e| e.to_string())?;
        uber_category_ids.push(uber_id.clone());
        uber_categories_created += 1;

        // Reparent matching children
        for child in &matching_children {
            db.update_parent(&child.id, &uber_id).map_err(|e| e.to_string())?;
            all_children_to_update.push(child.id.clone());
            categories_reparented += 1;
        }

        if !quiet {
            log!("Created '{}' with {} children", grouping.name, matching_children.len());
        }
    }

    // Batch update depths
    if !all_children_to_update.is_empty() {
        db.increment_multiple_subtrees_depth(&all_children_to_update).map_err(|e| e.to_string())?;
    }

    // Update Universe's child count
    let new_child_count = db.get_children(&universe.id).map_err(|e| e.to_string())?.len();
    db.update_child_count(&universe.id, new_child_count as i32).map_err(|e| e.to_string())?;

    // Generate embeddings for uber-categories
    if !quiet {
        log!("Computing embeddings for uber-categories...");
    }
    for uber_id in &uber_category_ids {
        let uber_children = db.get_children(uber_id).map_err(|e| e.to_string())?;
        let child_embeddings: Vec<Vec<f32>> = uber_children
            .iter()
            .filter_map(|c| db.get_node_embedding(&c.id).ok().flatten())
            .collect();

        if !child_embeddings.is_empty() {
            let refs: Vec<&[f32]> = child_embeddings.iter().map(|e| e.as_slice()).collect();
            if let Some(centroid) = similarity::compute_centroid(&refs) {
                let _ = db.update_node_embedding(uber_id, &centroid);
            }
        }
    }

    // Create sibling edges between uber-categories based on embedding similarity
    if !quiet {
        log!("Creating sibling edges between uber-categories...");
    }
    let mut sibling_edges = 0;

    // Get embeddings for all uber-categories
    let uber_embeddings: Vec<(String, Vec<f32>)> = uber_category_ids.iter()
        .filter_map(|id| db.get_node_embedding(id).ok().flatten().map(|e| (id.clone(), e)))
        .collect();

    // Create edges between pairs with similarity > 0.3
    let edge_timestamp = chrono::Utc::now().timestamp_millis();
    for i in 0..uber_embeddings.len() {
        for j in (i + 1)..uber_embeddings.len() {
            let (id_a, emb_a) = &uber_embeddings[i];
            let (id_b, emb_b) = &uber_embeddings[j];

            let sim = similarity::cosine_similarity(emb_a, emb_b);
            if sim > 0.3 {
                let edge = mycelica_lib::db::Edge {
                    id: format!("sibling-{}-{}", edge_timestamp, sibling_edges),
                    source: id_a.clone(),
                    target: id_b.clone(),
                    edge_type: mycelica_lib::db::EdgeType::Sibling,
                    label: None,
                    weight: Some(sim as f64),
                    edge_source: Some("consolidate".to_string()),
                    evidence_id: None,
                    confidence: None,
                    created_at: edge_timestamp,
                    updated_at: Some(edge_timestamp),
                    author: None,
                    reason: None,
                    content: None,
                    agent_id: None,
                    superseded_by: None,
                    metadata: None,
                };
                if db.insert_edge(&edge).is_ok() {
                    sibling_edges += 1;
                }
            }
        }
    }

    // Index edges for view lookups
    if sibling_edges > 0 {
        let _ = db.update_edge_parents();
    }

    let duration = start.elapsed().as_secs_f64();

    if json {
        println!(r#"{{"uber_categories_created":{},"categories_reparented":{},"sibling_edges_created":{},"duration_ms":{}}}"#,
            uber_categories_created, categories_reparented, sibling_edges, (duration * 1000.0) as u64);
    } else if !quiet {
        log!("Consolidation complete:");
        log!("  {} uber-categories created", uber_categories_created);
        log!("  {} categories reparented", categories_reparented);
        log!("  {} sibling edges created", sibling_edges);
        log!("  Time: {:.2}s", duration);
    }

    Ok(())
}

fn handle_unconsolidate(db: &Database, json: bool, quiet: bool) -> Result<(), String> {
    use mycelica_lib::hierarchy;
    use std::time::Instant;

    let start = Instant::now();

    // Get Universe
    let universe = db.get_universe()
        .map_err(|e| e.to_string())?
        .ok_or("No Universe node found")?;

    // Find uber-category nodes (id starts with "uber-")
    let all_children = db.get_children(&universe.id).map_err(|e| e.to_string())?;
    let uber_categories: Vec<_> = all_children
        .into_iter()
        .filter(|n| n.id.starts_with("uber-"))
        .collect();

    if uber_categories.is_empty() {
        return Err("No uber-categories found to unconsolidate".to_string());
    }

    if !quiet {
        log!("Found {} uber-categories to flatten", uber_categories.len());
    }

    let mut categories_removed = 0;
    let mut children_reparented = 0;
    let mut all_children_to_update: Vec<String> = Vec::new();

    for uber in &uber_categories {
        let uber_children = db.get_children(&uber.id).map_err(|e| e.to_string())?;

        // Reparent children to Universe
        for child in &uber_children {
            db.update_parent(&child.id, &universe.id).map_err(|e| e.to_string())?;
            all_children_to_update.push(child.id.clone());
            children_reparented += 1;
        }

        // Delete the uber-category node
        db.delete_node(&uber.id).map_err(|e| e.to_string())?;
        categories_removed += 1;

        if !quiet {
            log!("Flattened '{}' ({} children)", uber.title, uber_children.len());
        }
    }

    // Decrement depths for reparented subtrees
    for child_id in &all_children_to_update {
        let _ = db.decrement_subtree_depth(child_id);
    }

    // Update Universe's child count
    let new_child_count = db.get_children(&universe.id).map_err(|e| e.to_string())?.len();
    db.update_child_count(&universe.id, new_child_count as i32).map_err(|e| e.to_string())?;

    // Delete old sibling edges
    let _ = db.delete_edges_by_type("sibling");

    // Recreate sibling edges for the new flat structure
    if !quiet {
        log!("Recreating sibling edges...");
    }
    let sibling_edges = hierarchy::create_category_edges_from_cross_counts(db, None).unwrap_or(0);

    // Index edges for view lookups
    if sibling_edges > 0 {
        let _ = db.update_edge_parents();
    }

    let duration = start.elapsed().as_secs_f64();

    if json {
        println!(r#"{{"categories_removed":{},"children_reparented":{},"sibling_edges_created":{},"duration_ms":{}}}"#,
            categories_removed, children_reparented, sibling_edges, (duration * 1000.0) as u64);
    } else if !quiet {
        log!("Unconsolidation complete:");
        log!("  {} uber-categories removed", categories_removed);
        log!("  {} children reparented", children_reparented);
        log!("  {} sibling edges recreated", sibling_edges);
        log!("  Time: {:.2}s", duration);
    }

    Ok(())
}

// ============================================================================
// Dendrogram Rebuild
// ============================================================================

struct DendrogramRebuildResult {
    categories: usize,
    papers_assigned: usize,
    ai_calls: usize,
}

struct AdaptiveRebuildResult {
    categories: usize,
    papers_assigned: usize,
    sibling_edges: usize,
    bridges: usize,
}

/// Rebuild hierarchy using v2 adaptive tree algorithm.
///
/// This algorithm:
/// 1. Builds tree recursively with per-subtree thresholds
/// 2. Validates each split (gap, balance, cohesion checks)
/// 3. Detects bridge papers (multi-parent membership)
/// 4. Creates sibling edges with bridge metadata
async fn rebuild_hierarchy_adaptive(
    db: &Database,
    min_size: usize,
    tight_threshold: f64,
    cohesion_threshold: f64,
    delta_min: f64,
    auto: bool,
    _json: bool,
    quiet: bool,
    keywords_only: bool,
    fresh: bool,
) -> Result<AdaptiveRebuildResult, String> {
    use mycelica_lib::dendrogram::{build_adaptive_tree, auto_config, AdaptiveTreeConfig, TreeNode};
    use mycelica_lib::ai_client;
    use mycelica_lib::db::{Node, NodeType, Position, Edge, EdgeType};
    use std::time::Instant;
    use std::collections::{HashMap, HashSet};

    let start = Instant::now();
    let now = chrono::Utc::now().timestamp_millis();

    // Step 1: Clear existing hierarchy
    if !quiet {
        if fresh {
            elog!("Step 1/7: Fresh rebuild — clearing algorithm-generated categories (preserving human modifications)...");
        } else {
            elog!("Step 1/7: Clearing existing hierarchy...");
        }
    }
    db.delete_hierarchy_nodes().map_err(|e| e.to_string())?;

    // Step 2: Load edges and check threshold
    if !quiet { elog!("Step 2/7: Loading edges..."); }
    let edges = db.get_all_item_edges_sorted().map_err(|e| e.to_string())?;
    if edges.is_empty() {
        return Err("No edges found. Run 'mycelica-cli setup' to create semantic edges first.".to_string());
    }
    if !quiet { elog!("  {} edges loaded", edges.len()); }

    // Check edge weight distribution
    let min_weight = edges.iter().map(|(_, _, w)| *w).fold(f64::INFINITY, f64::min);
    if min_weight > 0.4 {
        if !quiet {
            elog!("  Warning: Minimum edge weight is {:.2}. Consider regenerating edges at lower threshold for finer resolution.", min_weight);
        }
    }

    // Sovereignty: extract pinned items (human-edited parent_id)
    let pinned_items = db.get_pinned_items().map_err(|e| e.to_string())?;
    let pinned_ids: HashSet<String> = pinned_items.iter().map(|(id, _)| id.clone()).collect();
    if !pinned_ids.is_empty() && !quiet {
        elog!("  {} pinned items excluded from clustering (human-edited parent_id)", pinned_ids.len());
    }

    // Extract unique paper IDs, filtering out pinned items
    let mut paper_set = HashSet::new();
    for (source, target, _) in &edges {
        paper_set.insert(source.clone());
        paper_set.insert(target.clone());
    }
    let papers: Vec<String> = paper_set.into_iter()
        .filter(|id| !pinned_ids.contains(id))
        .collect();
    if !quiet { elog!("  {} unique papers (after excluding pinned)", papers.len()); }

    // Always compute edge statistics for dynamic scaling
    let auto_cfg = auto_config(&edges, papers.len());

    // Compute config (auto uses all auto values, manual uses CLI args but keeps iqr/density)
    let config = if auto {
        if !quiet {
            elog!("  Auto-config: min_size={}, cohesion={:.2}, delta_min={:.3}",
                auto_cfg.min_size, auto_cfg.cohesion_threshold, auto_cfg.delta_min);
            elog!("  Distribution: IQR={:.3}, density={:.4}", auto_cfg.iqr, auto_cfg.edge_density);
        }
        auto_cfg
    } else {
        AdaptiveTreeConfig {
            min_size,
            tight_threshold,
            cohesion_threshold,
            delta_min,
            // Use actual data statistics for dynamic scaling even in manual mode
            iqr: auto_cfg.iqr,
            edge_density: auto_cfg.edge_density,
        }
    };

    // Step 3: Build adaptive tree
    if !quiet { elog!("Step 3/7: Building adaptive tree..."); }
    let (root, sibling_edges) = build_adaptive_tree(papers.clone(), edges.clone(), Some(config.clone()));

    // Count tree structure
    fn count_nodes(node: &TreeNode) -> (usize, usize) {
        match node {
            TreeNode::Leaf { papers, .. } => (1, papers.len()),
            TreeNode::Internal { children, papers, .. } => {
                let mut leaves = 0;
                let mut total_papers = papers.len();
                for child in children {
                    let (l, p) = count_nodes(child);
                    leaves += l;
                    total_papers = total_papers.max(p);
                }
                (leaves, total_papers)
            }
        }
    }
    let (leaf_count, _) = count_nodes(&root);
    if !quiet { elog!("  {} leaf categories, {} sibling edges", leaf_count, sibling_edges.len()); }

    // Step 4: Create Universe and flatten tree to DB
    if !quiet { elog!("Step 4/7: Creating categories in database..."); }

    // Create or update Universe
    let universe_id = "universe".to_string();
    let universe_exists = db.get_node(&universe_id).map_err(|e| e.to_string())?.is_some();

    if !universe_exists {
        let universe = Node {
            id: universe_id.clone(),
            node_type: NodeType::Cluster,
            title: "All Knowledge".to_string(),
            url: None,
            content: None,
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: None,
            depth: 0,
            is_item: false,
            is_universe: true,
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
            source: None,
            is_private: None,
            privacy_reason: None,
            privacy: None,
            content_type: None,
            associated_idea_id: None,
            latest_child_date: None,
            pdf_available: None,
            human_edited: None,
            human_created: false,
            author: None,
            agent_id: None,
            node_class: None,
            meta_type: None,
        };
        db.insert_node(&universe).map_err(|e| e.to_string())?;
    }

    // Flatten tree to categories
    let mut categories_created = 0;
    let mut papers_assigned: HashSet<String> = HashSet::new();
    let mut bridge_count = 0;
    let mut category_id_counter = 0;
    let mut node_to_category: HashMap<String, String> = HashMap::new();

    fn flatten_tree(
        node: &TreeNode,
        parent_id: &str,
        depth: i32,
        db: &Database,
        now: i64,
        categories_created: &mut usize,
        papers_assigned: &mut HashSet<String>,
        bridge_count: &mut usize,
        id_counter: &mut usize,
        node_to_category: &mut HashMap<String, String>,
        quiet: bool,
    ) -> Result<String, String> {
        let category_id = format!("adaptive-{}", *id_counter);
        *id_counter += 1;

        // Map tree node ID to category ID
        node_to_category.insert(node.id().to_string(), category_id.clone());

        match node {
            TreeNode::Leaf { papers, .. } => {
                // Create category node
                let category = Node {
                    id: category_id.clone(),
                    node_type: NodeType::Cluster,
                    title: format!("Category {}", *id_counter - 1),
                    url: None,
                    content: None,
                    position: Position { x: 0.0, y: 0.0 },
                    created_at: now,
                    updated_at: now,
                    cluster_id: None,
                    cluster_label: None,
                    depth,
                    is_item: false,
                    is_universe: false,
                    parent_id: Some(parent_id.to_string()),
                    child_count: papers.len() as i32,
                    ai_title: None,
                    summary: None,
                    tags: None,
                    emoji: None,
                    is_processed: false,
                    conversation_id: None,
                    sequence_index: None,
                    is_pinned: false,
                    last_accessed_at: None,
                    source: None,
                    is_private: None,
                    privacy_reason: None,
                    privacy: None,
                    content_type: None,
                    associated_idea_id: None,
                    latest_child_date: None,
                    pdf_available: None,
                    human_edited: None,
                    human_created: false,
                    author: None,
                    agent_id: None,
                    node_class: None,
                    meta_type: None,
                };
                db.insert_node(&category).map_err(|e| e.to_string())?;
                *categories_created += 1;

                // Assign papers to this category
                for paper_id in papers {
                    db.update_node_hierarchy(paper_id, Some(&category_id), depth + 1)
                        .map_err(|e| e.to_string())?;
                    papers_assigned.insert(paper_id.clone());
                }
            }
            TreeNode::Internal { children, papers, bridges, .. } => {
                // Collect papers that made it into children
                let child_papers: std::collections::HashSet<&String> = children.iter()
                    .flat_map(|c| match c {
                        TreeNode::Leaf { papers, .. } => papers.iter(),
                        TreeNode::Internal { papers, .. } => papers.iter(),
                    })
                    .collect();

                // Papers in this node but not in any child (orphans from small components)
                let orphan_papers: Vec<&String> = papers.iter()
                    .filter(|p| !child_papers.contains(p))
                    .collect();

                // Create internal category node
                let category = Node {
                    id: category_id.clone(),
                    node_type: NodeType::Cluster,
                    title: format!("Category {}", *id_counter - 1),
                    url: None,
                    content: None,
                    position: Position { x: 0.0, y: 0.0 },
                    created_at: now,
                    updated_at: now,
                    cluster_id: None,
                    cluster_label: None,
                    depth,
                    is_item: false,
                    is_universe: false,
                    parent_id: Some(parent_id.to_string()),
                    child_count: (children.len() + if orphan_papers.is_empty() { 0 } else { orphan_papers.len() }) as i32,
                    ai_title: None,
                    summary: None,
                    tags: None,
                    emoji: None,
                    is_processed: false,
                    conversation_id: None,
                    sequence_index: None,
                    is_pinned: false,
                    last_accessed_at: None,
                    source: None,
                    is_private: None,
                    privacy_reason: None,
                    privacy: None,
                    content_type: None,
                    associated_idea_id: None,
                    latest_child_date: None,
                    pdf_available: None,
                    human_edited: None,
                    human_created: false,
                    author: None,
                    agent_id: None,
                    node_class: None,
                    meta_type: None,
                };
                db.insert_node(&category).map_err(|e| e.to_string())?;
                *categories_created += 1;

                // Assign orphan papers directly to this category
                for paper_id in &orphan_papers {
                    db.update_node_hierarchy(paper_id, Some(&category_id), depth + 1)
                        .map_err(|e| e.to_string())?;
                    papers_assigned.insert(paper_id.to_string());
                }

                // Track bridge count
                *bridge_count += bridges.len();

                // Recurse into children
                for child in children {
                    flatten_tree(
                        child,
                        &category_id,
                        depth + 1,
                        db,
                        now,
                        categories_created,
                        papers_assigned,
                        bridge_count,
                        id_counter,
                        node_to_category,
                        quiet,
                    )?;
                }
            }
        }

        Ok(category_id)
    }

    // Special case: if root is a leaf (all papers in one group), handle directly
    match &root {
        TreeNode::Leaf { papers, .. } => {
            // Assign all papers directly under Universe
            for paper_id in papers {
                db.update_node_hierarchy(paper_id, Some(&universe_id), 1)
                    .map_err(|e| e.to_string())?;
                papers_assigned.insert(paper_id.clone());
            }
            categories_created = 0; // No intermediate categories
        }
        TreeNode::Internal { children, papers, .. } => {
            // Collect papers that made it into children
            let child_papers: std::collections::HashSet<&String> = children.iter()
                .flat_map(|c| match c {
                    TreeNode::Leaf { papers, .. } => papers.iter(),
                    TreeNode::Internal { papers, .. } => papers.iter(),
                })
                .collect();

            // Papers at root that didn't make it into any child (orphans)
            let orphan_papers: Vec<&String> = papers.iter()
                .filter(|p| !child_papers.contains(p))
                .collect();

            // Flatten children under Universe
            for child in children {
                flatten_tree(
                    child,
                    &universe_id,
                    1,
                    db,
                    now,
                    &mut categories_created,
                    &mut papers_assigned,
                    &mut bridge_count,
                    &mut category_id_counter,
                    &mut node_to_category,
                    quiet,
                )?;
            }

            // Assign root-level orphan papers directly under Universe
            if !orphan_papers.is_empty() {
                if !quiet { elog!("  {} orphan papers (small components) assigned to Universe", orphan_papers.len()); }
                for paper_id in &orphan_papers {
                    db.update_node_hierarchy(paper_id, Some(&universe_id), 1)
                        .map_err(|e| e.to_string())?;
                    papers_assigned.insert(paper_id.to_string());
                }
            }
        }
    }

    // Update Universe child count
    let universe_children = db.get_children(&universe_id).map_err(|e| e.to_string())?;
    db.update_child_count(&universe_id, universe_children.len() as i32)
        .map_err(|e| e.to_string())?;

    if !quiet { elog!("  {} categories, {} papers assigned", categories_created, papers_assigned.len()); }

    // Step 5: Create sibling edges with bridge metadata
    if !quiet { elog!("Step 5/7: Creating sibling edges..."); }
    let mut sibling_edges_created = 0;

    for meta in &sibling_edges {
        // Map tree node IDs to category IDs
        let source_cat = node_to_category.get(&meta.source_id);
        let target_cat = node_to_category.get(&meta.target_id);

        if let (Some(source), Some(target)) = (source_cat, target_cat) {
            // Serialize bridges to JSON for label
            let bridges_json = serde_json::to_string(&meta.bridges).unwrap_or_else(|_| "[]".to_string());

            let edge = Edge {
                id: format!("sibling-{}-{}", source, target),
                source: source.clone(),
                target: target.clone(),
                edge_type: EdgeType::Sibling,
                label: Some(bridges_json),
                weight: Some(meta.weight),
                edge_source: Some("adaptive".to_string()),
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
                sibling_edges_created += 1;
            }
        }
    }

    if !quiet { elog!("  {} sibling edges created", sibling_edges_created); }

    // Sovereignty: validate pinned parents still exist (follow merges, orphan if deleted)
    if !pinned_ids.is_empty() {
        let warnings = db.validate_pinned_parents().map_err(|e| e.to_string())?;
        for w in &warnings {
            if !quiet { elog!("  Warning: {}", w); }
        }
        if warnings.is_empty() && !quiet {
            elog!("  {} pinned items validated — parents intact", pinned_ids.len());
        }
    }

    // Step 6: Name categories (bottom-up)
    // Use LLM by default for better quality names, --keywords-only for faster but lower quality
    // Local LLMs (Ollama) hallucinate on non-English text, so fall back to TF-IDF for those
    let llm_available = !keywords_only && (ai_client::ollama_available().await || ai_client::is_available());
    let is_local_llm = settings::get_llm_backend() == "ollama";
    if !quiet {
        if llm_available {
            if is_local_llm {
                elog!("Step 6/7: Naming categories with local LLM (keyword fallback for non-English)...");
            } else {
                elog!("Step 6/7: Naming categories with LLM...");
            }
        } else {
            elog!("Step 6/7: Naming categories with keyword extraction...");
        }
    }

    // Collect adaptive categories by depth (to process bottom-up)
    // Use high upper limit; loop breaks early when no categories found
    let mut categories_by_depth: Vec<Vec<Node>> = Vec::new();
    let mut max_cat_depth: i32 = 1;
    for depth in 1i32..=50 {
        let cats_at_depth: Vec<Node> = db.get_nodes_at_depth(depth)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|n| n.id.starts_with("adaptive-"))
            .collect();
        if cats_at_depth.is_empty() && depth > 1 {
            break;
        }
        if !cats_at_depth.is_empty() {
            max_cat_depth = depth;
        }
        categories_by_depth.push(cats_at_depth);
    }

    // Get all existing category names to avoid duplicates (use HashSet for live deduplication)
    let mut forbidden_names: std::collections::HashSet<String> = db.get_all_category_names()
        .unwrap_or_default()
        .into_iter()
        .collect();

    // Process bottom-up: deepest categories first (they have item children)
    // Then their parents can use the newly-named subcategory titles
    let mut named_count = 0;
    for cats_at_depth in categories_by_depth.iter().rev() {
        for category in cats_at_depth {
            let children = db.get_children(&category.id).map_err(|e| e.to_string())?;

            // Collect titles from children (items or already-named subcategories)
            let titles: Vec<String> = children.iter()
                .filter(|c| !c.title.is_empty() && c.title != "Category")
                .take(15)
                .map(|c| c.title.clone())
                .collect();

            if !titles.is_empty() {
                // Check if we should use LLM for this category
                // Local LLMs hallucinate on non-English text, so use keywords instead
                let use_llm_for_this = llm_available &&
                    (!is_local_llm || is_predominantly_english(&titles));

                let name = if use_llm_for_this {
                    // Get parent name for context
                    let parent_name: Option<String> = if let Some(parent_id) = &category.parent_id {
                        db.get_node(parent_id)
                            .ok()
                            .flatten()
                            .and_then(|p| if p.title != "Universe" && !p.title.is_empty() { Some(p.title.clone()) } else { None })
                    } else {
                        None
                    };

                    // Use LLM with parent context if available
                    // Convert HashSet to Vec for AI functions
                    let forbidden_vec: Vec<String> = forbidden_names.iter().cloned().collect();
                    let llm_result = if let Some(parent) = parent_name {
                        ai_client::name_cluster_with_parent(&titles, &parent, &forbidden_vec).await
                    } else {
                        ai_client::name_cluster_from_samples(&titles, &forbidden_vec).await
                    };

                    // Fall back to keyword extraction if LLM fails
                    match llm_result {
                        Ok(n) if !n.is_empty() => n,
                        _ => {
                            if titles.len() > 5 {
                                ai_client::extract_top_keywords(&titles, 4)
                            } else {
                                ai_client::extract_top_keywords(&titles, 3)
                            }
                        }
                    }
                } else {
                    // Keyword extraction only (either forced or non-English with local LLM)
                    if titles.len() > 5 {
                        ai_client::extract_top_keywords(&titles, 4)
                    } else {
                        ai_client::extract_top_keywords(&titles, 3)
                    }
                };

                if !name.is_empty() && name != "Category" {
                    db.update_node_title(&category.id, &name).map_err(|e| e.to_string())?;
                    forbidden_names.insert(name.clone());  // Live deduplication
                    named_count += 1;
                }
            }
        }
    }

    if !quiet { elog!("  {} categories named", named_count); }

    // Step 7.5: Merge binary cascades where AI couldn't differentiate siblings
    // This runs AFTER naming because it needs to compare the actual generated names
    if !quiet { elog!("Step 7.5/7: Merging similar binary siblings..."); }
    let merged = mycelica_lib::dendrogram::merge_similar_binary_siblings(db, 0.75)
        .map_err(|e| e.to_string())?;
    if merged > 0 {
        if !quiet { elog!("  Merged {} redundant binary splits", merged); }
        // Update parent child counts after merging
        let all_parents: Vec<String> = db.get_all_nodes(false)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|n| !n.is_item)
            .map(|n| n.id)
            .collect();
        for parent_id in &all_parents {
            let child_count = db.get_children(parent_id).map_err(|e| e.to_string())?.len();
            let _ = db.update_child_count(parent_id, child_count as i32);
        }
    }

    // Step 7: Final-pass orphan assignment using nearest-neighbor
    if !quiet { elog!("Step 7/7: Final-pass orphan assignment..."); }

    // Get orphans (items at depth=1 under Universe)
    let orphan_nodes: Vec<Node> = db.get_children(&universe_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|n| n.is_item)
        .collect();

    if !orphan_nodes.is_empty() {
        // Build edge index for O(1) weight lookups
        use mycelica_lib::dendrogram::EdgeIndex;
        let edge_index = EdgeIndex::new(&edges);

        // Get all leaf categories (categories whose children are items)
        // Collect categories across all depths (reuse max_cat_depth from naming phase)
        let mut all_categories: Vec<Node> = Vec::new();
        for depth in 1..=max_cat_depth {
            let cats_at_depth: Vec<Node> = db.get_nodes_at_depth(depth)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|n| !n.is_item && n.id.starts_with("adaptive-"))
                .collect();
            if cats_at_depth.is_empty() && depth > 1 {
                break; // No more categories at deeper levels
            }
            all_categories.extend(cats_at_depth);
        }

        // For each category, get its item children
        let mut leaf_categories: Vec<(String, Vec<String>, i32)> = Vec::new(); // (cat_id, member_ids, depth)
        for cat in &all_categories {
            let children = db.get_children(&cat.id).map_err(|e| e.to_string())?;
            let item_children: Vec<String> = children.iter()
                .filter(|c| c.is_item)
                .map(|c| c.id.clone())
                .collect();
            if !item_children.is_empty() {
                leaf_categories.push((cat.id.clone(), item_children, cat.depth));
            }
        }

        if !quiet { elog!("  {} orphans, {} leaf categories", orphan_nodes.len(), leaf_categories.len()); }

        let mut rescued = 0;
        let mut truly_isolated = 0;

        for orphan in &orphan_nodes {
            // Compute average edge weight to each leaf category
            let mut best_cat: Option<&str> = None;
            let mut best_avg: f64 = 0.0;
            let mut best_depth: i32 = 0;

            for (cat_id, members, depth) in &leaf_categories {
                let mut sum = 0.0;
                let mut count = 0;
                for member in members {
                    if let Some(w) = edge_index.weight(&orphan.id, member) {
                        sum += w;
                        count += 1;
                    }
                }
                if count > 0 {
                    // Use sum/count (actual average of edges found), not sum/members.len()
                    let avg = sum / count as f64;
                    if avg > best_avg {
                        best_avg = avg;
                        best_cat = Some(cat_id);
                        best_depth = *depth;
                    }
                }
            }

            if let Some(cat_id) = best_cat {
                // Assign orphan to best leaf category
                db.update_node_hierarchy(&orphan.id, Some(cat_id), best_depth + 1)
                    .map_err(|e| e.to_string())?;
                papers_assigned.insert(orphan.id.clone());
                rescued += 1;
            } else {
                // Truly isolated - no edges to any categorized paper
                truly_isolated += 1;
            }
        }

        // Update child counts for affected categories
        for (cat_id, _, _) in &leaf_categories {
            let children = db.get_children(cat_id).map_err(|e| e.to_string())?;
            db.update_child_count(cat_id, children.len() as i32).map_err(|e| e.to_string())?;
        }

        if !quiet {
            elog!("  {} orphans rescued via NN assignment", rescued);
            if truly_isolated > 0 {
                elog!("  {} truly isolated (no edges to any category)", truly_isolated);
            }
        }

    }

    // Final step: Recalculate child_count for all categories
    // This ensures graph view can drill into categories correctly
    let updated = db.recalculate_all_child_counts().map_err(|e| e.to_string())?;
    if !quiet { elog!("  Updated child_count for {} categories", updated); }

    // Update edge parent IDs so edges appear in the correct views
    let edge_updates = db.update_edge_parents().map_err(|e| e.to_string())?;
    if !quiet { elog!("  Updated parent IDs for {} edges", edge_updates); }

    let elapsed = start.elapsed();
    if !quiet { elog!("Completed in {:.1}s", elapsed.as_secs_f64()); }

    Ok(AdaptiveRebuildResult {
        categories: categories_created,
        papers_assigned: papers_assigned.len(),
        sibling_edges: sibling_edges_created,
        bridges: bridge_count,
    })
}

async fn rebuild_hierarchy_dendrogram(
    db: &Database,
    target_levels: usize,
    method: &str,
    min_component_size: usize,
    custom_thresholds: Option<&str>,
    _json: bool,
    quiet: bool,
) -> Result<DendrogramRebuildResult, String> {
    use mycelica_lib::dendrogram::{build_dendrogram, find_natural_thresholds, find_percentile_thresholds, fixed_thresholds, find_dynamic_thresholds, extract_levels, Component};
    use mycelica_lib::ai_client;
    use mycelica_lib::db::{Node, NodeType, Position};
    use std::time::Instant;
    use std::collections::{HashMap, HashSet};

    let start = Instant::now();
    let now = chrono::Utc::now().timestamp_millis();

    // Step 1: Clear existing hierarchy
    if !quiet { elog!("Step 1/6: Clearing existing hierarchy..."); }
    db.delete_hierarchy_nodes().map_err(|e| e.to_string())?;

    // Step 2: Load edges
    if !quiet { elog!("Step 2/6: Loading edges..."); }
    let edges = db.get_all_item_edges_sorted().map_err(|e| e.to_string())?;
    if edges.is_empty() {
        return Err("No edges found. Run 'mycelica-cli setup' to create semantic edges first.".to_string());
    }
    if !quiet { elog!("  {} edges loaded", edges.len()); }

    // Extract unique paper IDs
    let mut paper_set = std::collections::HashSet::new();
    for (source, target, _) in &edges {
        paper_set.insert(source.clone());
        paper_set.insert(target.clone());
    }
    let papers: Vec<String> = paper_set.into_iter().collect();
    if !quiet { elog!("  {} unique papers", papers.len()); }

    // Step 3: Build dendrogram
    if !quiet { elog!("Step 3/6: Building dendrogram..."); }
    let dendrogram = build_dendrogram(papers.clone(), edges.clone());
    if !quiet { elog!("  {} merges recorded", dendrogram.merges.len()); }

    // Step 4: Find thresholds and extract levels
    // Thresholds are returned DESCENDING (highest/tightest first)
    // We need to process ASCENDING (lowest/broadest first) for proper nesting
    if !quiet { elog!("Step 4/6: Finding thresholds (method={})...", method); }
    let mut thresholds = match method {
        "gap" => find_natural_thresholds(&dendrogram, target_levels),
        "percentile" => find_percentile_thresholds(&dendrogram, target_levels),
        "fixed" => {
            // Parse custom thresholds or use defaults
            if let Some(t_str) = custom_thresholds {
                let parsed: Vec<f64> = t_str.split(',')
                    .filter_map(|s| s.trim().parse::<f64>().ok())
                    .collect();
                if parsed.is_empty() {
                    return Err(format!("Invalid thresholds: '{}'. Use comma-separated decimals like '0.8,0.7,0.6,0.5'", t_str));
                }
                fixed_thresholds(&parsed)
            } else {
                fixed_thresholds(&[0.8, 0.7, 0.6, 0.5])
            }
        }
        "dynamic" => {
            // Dynamic: find thresholds where component count meaningfully changes
            // target_levels becomes max_levels (optional cap)
            let max_levels = if target_levels > 0 { Some(target_levels) } else { None };
            find_dynamic_thresholds(&dendrogram, max_levels, Some(0.05), Some(0.5), Some(0.95))
        }
        _ => find_percentile_thresholds(&dendrogram, target_levels),
    };
    if !quiet { elog!("  Thresholds (descending): {:?}", thresholds); }

    // Reverse thresholds to process broadest (lowest threshold) first
    thresholds.reverse();
    if !quiet { elog!("  Processing order (ascending): {:?}", thresholds); }

    let levels = extract_levels(&dendrogram, &thresholds);
    if !quiet { elog!("  {} levels extracted", levels.levels.len()); }

    // Step 5: Create nested category hierarchy
    if !quiet { elog!("Step 5/6: Creating nested category hierarchy..."); }

    // Get all paper nodes for naming
    let all_nodes = db.get_all_nodes(true).map_err(|e| e.to_string())?;
    let node_map: HashMap<String, &Node> = all_nodes.iter()
        .map(|n| (n.id.clone(), n))
        .collect();

    // Count edges per paper for selecting most-connected samples
    let mut edge_counts: HashMap<String, usize> = HashMap::new();
    for (source, target, _) in &edges {
        *edge_counts.entry(source.clone()).or_insert(0) += 1;
        *edge_counts.entry(target.clone()).or_insert(0) += 1;
    }

    let mut categories_created = 0;
    let mut papers_assigned = 0;
    let mut ai_calls = 0;

    // Create Universe node first
    let universe_id = format!("universe-{}", uuid::Uuid::new_v4());
    let universe_node = Node {
        id: universe_id.clone(),
        node_type: NodeType::Cluster,
        title: "Universe".to_string(),
        url: None,
        content: None,
        position: Position { x: 0.0, y: 0.0 },
        created_at: now,
        updated_at: now,
        cluster_id: None,
        cluster_label: None,
        depth: 0,
        is_item: false,
        is_universe: true,
        parent_id: None,
        child_count: 0, // Will update later
        ai_title: None,
        summary: None,
        tags: None,
        emoji: None,
        is_processed: true,
        conversation_id: None,
        sequence_index: None,
        is_pinned: false,
        last_accessed_at: None,
        latest_child_date: None,
        is_private: None,
        privacy_reason: None,
        source: None,
        pdf_available: None,
        content_type: None,
        associated_idea_id: None,
        privacy: None,
        human_edited: None,
        human_created: false,
        author: None,
        agent_id: None,
        node_class: None,
        meta_type: None,
    };
    db.insert_node(&universe_node).map_err(|e| e.to_string())?;

    // Track component -> category_id mapping for parent lookup
    // Key: (level_idx, component_idx), Value: category_id
    let mut component_to_category: HashMap<(usize, usize), String> = HashMap::new();

    // Track category_id -> name for parent context in naming
    let mut category_to_name: HashMap<String, String> = HashMap::new();
    category_to_name.insert(universe_id.clone(), "Universe".to_string());

    // Track which papers end up in which leaf category
    let mut paper_to_leaf_category: HashMap<String, String> = HashMap::new();

    let mut forbidden_names: Vec<String> = vec!["Universe".to_string()];

    // Helper: find parent component at previous level
    fn find_parent_component_idx(
        child: &Component,
        parent_level: &[Component],
    ) -> Option<usize> {
        let sample_paper = child.papers.first()?;
        parent_level.iter()
            .position(|parent| parent.papers.contains(sample_paper))
    }

    // Step 6: Process levels from broadest to most specific
    if !quiet { elog!("Step 6/6: Naming and creating categories..."); }

    let num_levels = levels.levels.len();
    for (level_idx, level_components) in levels.levels.iter().enumerate() {
        let threshold = thresholds.get(level_idx).copied().unwrap_or(0.0);
        let depth = level_idx + 1; // Universe is depth 0

        // Filter to nameable components at this level
        let nameable: Vec<(usize, &Component)> = level_components.iter()
            .enumerate()
            .filter(|(_, c)| c.papers.len() >= min_component_size)
            .collect();

        if !quiet {
            elog!("  Level {} (threshold {:.3}, depth {}): {} nameable components",
                level_idx, threshold, depth, nameable.len());
        }

        for (comp_idx, component) in &nameable {
            // Find parent category
            let parent_id = if level_idx == 0 {
                // First level: parent is Universe
                universe_id.clone()
            } else {
                // Find which component at previous level contains this component
                let prev_level = &levels.levels[level_idx - 1];
                if let Some(parent_comp_idx) = find_parent_component_idx(component, prev_level) {
                    // Look up the category we created for that parent component
                    component_to_category
                        .get(&(level_idx - 1, parent_comp_idx))
                        .cloned()
                        .unwrap_or_else(|| universe_id.clone())
                } else {
                    universe_id.clone()
                }
            };

            // Get papers in this component for naming
            let paper_nodes: Vec<&Node> = component.papers.iter()
                .filter_map(|id| node_map.get(id).copied())
                .collect();

            if paper_nodes.is_empty() {
                continue;
            }

            // Sort by edge count to get most-connected papers
            let mut papers_with_counts: Vec<_> = paper_nodes.iter()
                .map(|n| (*n, edge_counts.get(&n.id).copied().unwrap_or(0)))
                .collect();
            papers_with_counts.sort_by(|a, b| b.1.cmp(&a.1));

            // Sample top 10 most-connected papers for naming
            let sample: Vec<&Node> = papers_with_counts.iter()
                .take(10)
                .map(|(n, _)| *n)
                .collect();

            // Get ALL titles for keyword extraction (large components)
            let all_titles: Vec<String> = paper_nodes.iter()
                .map(|n| n.ai_title.clone()
                    .or_else(|| n.cluster_label.clone())
                    .unwrap_or_else(|| n.title.clone()))
                .collect();

            // Get sample titles for AI naming (small components)
            let sample_titles: Vec<String> = sample.iter()
                .map(|n| n.ai_title.clone()
                    .or_else(|| n.cluster_label.clone())
                    .unwrap_or_else(|| n.title.clone()))
                .collect();

            // Get parent name for context
            let parent_name = category_to_name.get(&parent_id)
                .cloned()
                .unwrap_or_else(|| "Universe".to_string());

            // Naming strategy: keyword extraction for large, AI with parent context for small
            let category_name = if component.papers.len() > 200 {
                // Large component: use keyword frequency naming with level suffix
                let keywords = ai_client::extract_top_keywords(&all_titles, 4);
                let name = format!("{} (L{})", keywords, level_idx);
                if !quiet && categories_created % 20 == 0 {
                    elog!("    {} categories created, keywords \"{}\" ({} papers)",
                        categories_created, name, component.papers.len());
                }
                forbidden_names.push(name.clone());
                name
            } else {
                // Small component: AI naming with parent context
                match ai_client::name_cluster_with_parent(&sample_titles, &parent_name, &forbidden_names).await {
                    Ok(name) => {
                        ai_calls += 1;
                        if !quiet && categories_created % 20 == 0 {
                            elog!("    {} categories created, naming \"{}\" ({} papers)",
                                categories_created, name, component.papers.len());
                        }
                        forbidden_names.push(name.clone());
                        name
                    }
                    Err(e) => {
                        if !quiet { elog!("    AI naming failed: {}", e); }
                        // Fallback: use keywords with level
                        let fallback = format!("{} (L{})", ai_client::extract_top_keywords(&all_titles, 4), level_idx);
                        forbidden_names.push(fallback.clone());
                        fallback
                    }
                }
            };

            // Create category node
            let category_id = format!("dendro-L{}-{}", level_idx, uuid::Uuid::new_v4());

            // Store merge_weight and level in tags JSON
            let tags_json = format!(
                r#"{{"merge_weight":{:.4},"size":{},"level":{}}}"#,
                threshold, component.papers.len(), level_idx
            );

            let category_node = Node {
                id: category_id.clone(),
                node_type: NodeType::Cluster,
                title: category_name.clone(),
                url: None,
                content: None,
                position: Position { x: 0.0, y: 0.0 },
                created_at: now,
                updated_at: now,
                cluster_id: None,
                cluster_label: Some(category_name.clone()),
                depth: depth as i32,
                is_item: false,
                is_universe: false,
                parent_id: Some(parent_id),
                child_count: component.papers.len() as i32, // Will include both sub-categories and papers
                ai_title: None,
                summary: None,
                tags: Some(tags_json),
                emoji: None,
                is_processed: true,
                conversation_id: None,
                sequence_index: None,
                is_pinned: false,
                last_accessed_at: None,
                latest_child_date: None,
                is_private: None,
                privacy_reason: None,
                source: None,
                pdf_available: None,
                content_type: None,
                associated_idea_id: None,
                privacy: None,
                human_edited: None,
                human_created: false,
                author: None,
                agent_id: None,
                node_class: None,
                meta_type: None,
            };

            db.insert_node(&category_node).map_err(|e| e.to_string())?;
            categories_created += 1;

            // Track this component's category ID and name
            component_to_category.insert((level_idx, *comp_idx), category_id.clone());
            category_to_name.insert(category_id.clone(), category_name.clone());

            // If this is the deepest level, track papers for assignment
            if level_idx == num_levels - 1 {
                for paper_id in &component.papers {
                    paper_to_leaf_category.insert(paper_id.clone(), category_id.clone());
                }
            }
        }
    }

    // Assign papers to their leaf categories (deepest level only)
    if !quiet { elog!("Assigning papers to leaf categories..."); }
    let final_depth = (num_levels + 1) as i32; // Papers are one level deeper than deepest category
    for (paper_id, category_id) in &paper_to_leaf_category {
        db.update_parent(paper_id, category_id).map_err(|e| e.to_string())?;
        db.update_node_hierarchy(paper_id, Some(category_id), final_depth).map_err(|e| e.to_string())?;
        papers_assigned += 1;
    }

    // Handle orphan papers (not in any nameable component)
    let orphan_count = papers.len() - papers_assigned;
    if orphan_count > 0 && !quiet {
        elog!("  {} papers remain as orphans (in clusters <{} papers)", orphan_count, min_component_size);
    }

    let total_time = start.elapsed().as_secs_f64();
    if !quiet {
        elog!("Dendrogram rebuild complete in {:.1}s", total_time);
        elog!("  {} categories created across {} levels", categories_created, num_levels);
        elog!("  {} papers assigned to leaf categories", papers_assigned);
        elog!("  {} AI naming calls", ai_calls);
    }

    Ok(DendrogramRebuildResult {
        categories: categories_created,
        papers_assigned,
        ai_calls,
    })
}

// ============================================================================
// Smart Add
// ============================================================================

async fn handle_smart_add(db: &Database, json: bool) -> Result<(), String> {
    use similarity::cosine_similarity;
    use std::time::Instant;

    let start = Instant::now();
    const SIMILARITY_THRESHOLD: f32 = 0.3;

    // Get all nodes
    let all_nodes = db.get_all_nodes(true).map_err(|e| e.to_string())?;

    // Get orphan items (is_item=true, parent_id=None)
    let orphans: Vec<_> = all_nodes.iter()
        .filter(|n| n.is_item && n.parent_id.is_none())
        .cloned()
        .collect();

    if orphans.is_empty() {
        if json {
            log!(r#"{{"orphans":0,"matched":0,"embedded":0,"ms":0}}"#);
        } else {
            log!("No orphan items found");
        }
        return Ok(());
    }

    log!("Found {} orphan items", orphans.len());

    // Get all items WITH parents (potential matches)
    let items_with_parents: Vec<_> = all_nodes.iter()
        .filter(|n| n.is_item && n.parent_id.is_some())
        .collect();

    log!("Found {} items with parents to match against", items_with_parents.len());

    // Pre-fetch all embeddings
    let all_embeddings = db.get_nodes_with_embeddings().map_err(|e| e.to_string())?;
    let mut emb_map: std::collections::HashMap<_, _> = all_embeddings.into_iter().collect();

    // Find orphans without embeddings and generate them
    let orphans_needing_embeddings: Vec<_> = orphans.iter()
        .filter(|o| !emb_map.contains_key(&o.id))
        .collect();

    let mut embedded = 0;
    if !orphans_needing_embeddings.is_empty() {
        log!("Generating embeddings for {} orphans...", orphans_needing_embeddings.len());

        // Prepare texts for batch embedding
        let texts: Vec<String> = orphans_needing_embeddings.iter()
            .map(|o| {
                let title = o.ai_title.as_ref().unwrap_or(&o.title);
                let content = o.content.as_deref().unwrap_or("");
                format!("{}\n{}", title, &content[..content.len().min(500)])
            })
            .collect();

        // Generate embeddings in batch
        if let Ok(embeddings) = mycelica_lib::local_embeddings::generate_batch(&texts.iter().map(|s| s.as_str()).collect::<Vec<_>>()) {
            for (orphan, embedding) in orphans_needing_embeddings.iter().zip(embeddings.into_iter()) {
                if db.update_node_embedding(&orphan.id, &embedding).is_ok() {
                    emb_map.insert(orphan.id.clone(), embedding);
                    embedded += 1;
                }
            }
            log!("Generated {} embeddings", embedded);
        }
    }

    let mut matched = 0;

    for orphan in &orphans {
        // Get orphan's embedding
        let orphan_emb = match emb_map.get(&orphan.id) {
            Some(e) => e,
            None => {
                log!("  [skip] '{}' - no embedding", orphan.title);
                continue;
            }
        };

        // Find most similar item that has a parent
        let mut best: Option<(&str, &str, i32, f32)> = None; // (item_id, parent_id, depth, score)
        for candidate in &items_with_parents {
            if let Some(cand_emb) = emb_map.get(&candidate.id) {
                let score = cosine_similarity(orphan_emb, cand_emb);
                if score > SIMILARITY_THRESHOLD {
                    if best.is_none() || score > best.unwrap().3 {
                        best = Some((
                            &candidate.id,
                            candidate.parent_id.as_ref().unwrap(),
                            candidate.depth,
                            score,
                        ));
                    }
                }
            }
        }

        // Place orphan as sibling of best match
        if let Some((similar_id, parent_id, depth, score)) = best {
            db.set_node_parent(&orphan.id, parent_id, depth).map_err(|e| e.to_string())?;
            db.increment_child_count(parent_id).map_err(|e| e.to_string())?;
            matched += 1;
            log!("  '{}' -> '{}' (sim: {:.2} with '{}')",
                orphan.ai_title.as_ref().unwrap_or(&orphan.title),
                parent_id,
                score,
                similar_id);
        } else {
            log!("  [skip] '{}' - no similar item found (threshold: {})", orphan.title, SIMILARITY_THRESHOLD);
        }
    }

    let elapsed = start.elapsed().as_millis() as u64;

    if json {
        log!(r#"{{"orphans":{},"matched":{},"embedded":{},"ms":{}}}"#, orphans.len(), matched, embedded, elapsed);
    } else {
        log!("Done: {}/{} orphans placed, {} embeddings generated in {}ms", matched, orphans.len(), embedded, elapsed);
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
                    log!(r#"{{"processed":0,"message":"No unprocessed nodes"}}"#);
                } else {
                    log!("No unprocessed nodes");
                }
                return Ok(());
            }

            if !quiet {
                log!("Processing {} nodes...", to_process.len());
            }

            // Process nodes with 10 concurrent API calls
            const CONCURRENT_REQUESTS: usize = 10;
            if !quiet {
                log!("[Processing {} nodes with {} concurrent API calls]",
                         to_process.len(), CONCURRENT_REQUESTS);
            }

            // Run AI analysis in parallel, collect results
            let results: Vec<_> = stream::iter(to_process.iter().cloned())
                .map(|node| async move {
                    let content = node.content.as_deref().unwrap_or(&node.title).to_string();
                    let result = mycelica_lib::ai_client::analyze_node(&node.title, &content).await;
                    (node, content, result)
                })
                .buffer_unordered(CONCURRENT_REQUESTS)
                .collect()
                .await;

            // Apply results to DB (sequential for thread safety)
            let mut processed_count = 0;
            for (i, (node, content, result)) in results.into_iter().enumerate() {
                if !quiet && i % 50 == 0 {
                    eprint!("\r[{}/{}] Saving...", i, to_process.len());
                    std::io::stderr().flush().ok();
                }

                match result {
                    Ok(ai_result) => {
                        // Preserve code_* content_types (from code import)
                        let final_content_type = if node.content_type
                            .as_ref()
                            .map(|ct| ct.starts_with("code_"))
                            .unwrap_or(false)
                        {
                            node.content_type.clone().unwrap_or_default()
                        } else {
                            ai_result.content_type.clone()
                        };

                        db.update_node_ai(
                            &node.id,
                            &ai_result.title,
                            &ai_result.summary,
                            &serde_json::to_string(&ai_result.tags).unwrap_or_default(),
                            &final_content_type,
                        ).map_err(|e| e.to_string())?;

                        // Generate embedding for the node
                        let embed_text = utils::safe_truncate(&content, 1000);
                        if let Ok(embedding) = ai_client::generate_embedding(embed_text).await {
                            db.update_node_embedding(&node.id, &embedding).ok();
                        }

                        processed_count += 1;
                    }
                    Err(e) => {
                        if !quiet {
                            elog!("\nError processing {}: {}", &node.id[..8], e);
                        }
                    }
                }
            }

            if !quiet { log!(""); }

            if json {
                log!(r#"{{"processed":{}}}"#, processed_count);
            } else {
                log!("Processed {} nodes", processed_count);
            }
        }
        ProcessCommands::Status => {
            let unprocessed = db.get_unprocessed_nodes().map_err(|e| e.to_string())?;
            let stats = db.get_stats().map_err(|e| e.to_string())?;
            let items = stats.1;

            if json {
                log!(r#"{{"unprocessed":{},"total":{}}}"#, unprocessed.len(), items);
            } else {
                log!("Unprocessed: {} / {} items", unprocessed.len(), items);
            }
        }
        ProcessCommands::Reset => {
            db.reset_ai_processing().map_err(|e| e.to_string())?;
            if json {
                log!(r#"{{"status":"ok"}}"#);
            } else {
                log!("AI processing reset");
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
        EmbeddingsCommands::BuildIndex => {
            use mycelica_lib::commands::{HnswIndex, hnsw_index_path};

            eprintln!("Building HNSW index for fast similarity search...");
            eprintln!("This may take several minutes for large databases.");

            // Get all embeddings
            let embeddings = db.get_nodes_with_embeddings().map_err(|e| e.to_string())?;
            let count = embeddings.len();

            if count == 0 {
                if json {
                    println!(r#"{{"status":"error","message":"No embeddings found"}}"#);
                } else {
                    eprintln!("No embeddings found. Run 'mycelica-cli process nodes' first.");
                }
                return Ok(());
            }

            eprintln!("Found {} embeddings", count);

            // Build index
            let mut index = HnswIndex::new();
            index.build(&embeddings);

            // Save to disk
            let db_path = PathBuf::from(db.get_path());
            let index_path = hnsw_index_path(&db_path);
            index.save(&index_path)?;

            if json {
                println!(r#"{{"status":"ok","embeddings":{},"path":"{}"}}"#,
                    count, index_path.display());
            } else {
                println!("Built HNSW index with {} embeddings", count);
                println!("Saved to: {:?}", index_path);
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

            let author = settings::get_author();
            let remote_url = settings::get_remote_url();

            if json {
                println!(r#"{{"anthropic_api_key":{},"openai_api_key":{},"openaire_api_key":{},"clustering_primary":{},"clustering_secondary":{},"privacy_threshold":{},"local_embeddings":{},"protect_recent_notes":{},"show_tips":{},"author":{},"remote_url":{}}}"#,
                    anthropic, openai, openaire,
                    cluster_p.map(|v| v.to_string()).unwrap_or("null".to_string()),
                    cluster_s.map(|v| v.to_string()).unwrap_or("null".to_string()),
                    privacy, local_emb, protect, tips,
                    author.as_ref().map(|a| format!("\"{}\"", a)).unwrap_or("null".to_string()),
                    remote_url.as_ref().map(|u| format!("\"{}\"", u)).unwrap_or("null".to_string()));
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
                println!("author:               {}", author.as_deref().unwrap_or("not set"));
                println!("remote-url:           {}", remote_url.as_deref().unwrap_or("not set"));
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
                "author" => settings::get_author().unwrap_or_else(|| "not set".to_string()),
                "remote-url" => settings::get_remote_url().unwrap_or_else(|| "not set".to_string()),
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
                "author" => {
                    settings::set_author(value.clone())?;
                }
                "remote-url" => {
                    settings::set_remote_url(value.clone())?;
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

async fn handle_setup(db: &Database, skip_pipeline: bool, include_code: bool, algorithm: &str, keywords_only: bool, yes: bool, quiet: bool) -> Result<(), String> {
    use std::io::{Write, BufRead};

    log!("=== Mycelica Setup ===\n");

    // Show current database
    log!("Database: {}\n", db.get_path());

    // Check and prompt for API keys
    let has_openai = settings::has_openai_api_key();
    let has_anthropic = settings::has_api_key();

    log!("API Keys:");
    log!("  OpenAI:    {}", if has_openai { "configured" } else { "not set" });
    log!("  Anthropic: {}", if has_anthropic { "configured" } else { "not set" });
    log!("");

    // Prompt for OpenAI key if not set (required for embeddings)
    // Skip prompt in non-interactive mode
    if !has_openai && !yes {
        print!("Enter OpenAI API key (for embeddings, or press Enter to skip): ");
        std::io::stdout().flush().ok();

        let mut key = String::new();
        std::io::stdin().lock().read_line(&mut key).ok();
        let key = key.trim().to_string();
        if !key.is_empty() {
            settings::set_openai_api_key(key)?;
            log!("  OpenAI key saved.\n");
        } else {
            log!("  Skipped.\n");
        }
    }

    // Prompt for Anthropic key if not set (required for AI processing)
    // Skip prompt in non-interactive mode
    if !has_anthropic && !yes {
        print!("Enter Anthropic API key (for AI processing, or press Enter to skip): ");
        std::io::stdout().flush().ok();

        let mut key = String::new();
        std::io::stdin().lock().read_line(&mut key).ok();
        let key = key.trim().to_string();
        if !key.is_empty() {
            settings::set_api_key(key)?;
            log!("  Anthropic key saved.\n");
        } else {
            log!("  Skipped.\n");
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

    log!("Database Stats:");
    log!("  Items:      {} ({} papers, {} other)", total, papers, non_papers);
    log!("  Processed:  {} / {} (papers skip AI processing)", non_paper_processed, non_papers);
    log!("  Embeddings: {} / {}", with_embeddings, total);
    log!("");

    if skip_pipeline {
        log!("Setup complete. (Pipeline skipped)");
        return Ok(());
    }

    // Ask to run pipeline if there's work to do
    // Papers don't need AI processing (they have metadata from OpenAIRE)
    let needs_processing = non_paper_processed < non_papers;
    let needs_embeddings = with_embeddings < total;

    // Check if hierarchy needs to be built
    // Hierarchy is needed if: no universe, universe has few children, or most items orphaned
    let needs_hierarchy = if let Ok(Some(universe)) = db.get_universe() {
        let universe_children = db.get_children(&universe.id).unwrap_or_default();
        let direct_items = universe_children.iter().filter(|n| n.is_item).count();
        let child_count = universe_children.len();

        // Needs hierarchy if:
        // 1. Universe has very few children (cleared hierarchy)
        // 2. >50% of items are directly under Universe (flat hierarchy)
        // 3. Most items have no parent (orphaned after clear)
        let orphaned_items = db.get_items().map(|items| {
            items.iter().filter(|n| n.parent_id.is_none()).count()
        }).unwrap_or(0);

        child_count < 3 || direct_items > total / 2 || orphaned_items > total / 2
    } else {
        true // No universe = definitely needs hierarchy
    };

    if !needs_processing && !needs_embeddings && !needs_hierarchy {
        log!("All items are processed, have embeddings, and hierarchy is built. Nothing to do!");
        return Ok(());
    }

    // Show what needs to be done
    if needs_hierarchy {
        log!("  Hierarchy: needs building");
    }

    // In non-interactive mode, auto-confirm; otherwise prompt
    let run_pipeline = if yes {
        true
    } else {
        print!("Run processing pipeline? [Y/n]: ");
        std::io::stdout().flush().ok();

        let mut input = String::new();
        std::io::stdin().lock().read_line(&mut input).ok();
        input.trim().is_empty() || input.trim().to_lowercase().starts_with('y')
    };

    if !run_pipeline {
        log!("Setup complete. Run 'mycelica-cli process run' later to process items.");
        return Ok(());
    }

    log!("");
    log!("═══════════════════════════════════════════════════════════");
    log!("Starting Mycelica Pipeline");
    log!("═══════════════════════════════════════════════════════════");

    // Step 0: Pattern Classification (FREE, identifies hidden items to skip)
    log!("");
    log!("▶ STEP 0/7: Pattern Classification");
    log!("───────────────────────────────────────────────────────────");
    log!("  Classifying items using pattern matching (FREE)...");

    let classified = classification::classify_all_items(db)?;

    // Re-fetch items with updated content_types
    let items = db.get_items().map_err(|e| e.to_string())?;

    // Count hidden items that will skip AI
    let hidden_count = items.iter()
        .filter(|n| n.content_type.as_ref()
            .and_then(|ct| classification::ContentType::from_str(ct))
            .map(|ct| ct.is_hidden())
            .unwrap_or(false))
        .filter(|n| !n.is_processed)
        .count();

    log!("  ✓ Classified {} items", classified);
    if hidden_count > 0 {
        log!("  → {} items classified as hidden (will skip AI processing)", hidden_count);
    }

    // Step 1: AI Processing + Embeddings
    log!("");
    log!("▶ STEP 1/7: AI Processing + Embeddings");
    log!("───────────────────────────────────────────────────────────");

    // 1a: AI process non-paper, non-hidden items
    // Papers have metadata from OpenAIRE
    // Code items included only if --include-code flag is set
    // Hidden items (debug, code, paste, trivial) skip AI processing
    let unprocessed_non_papers: Vec<_> = items.iter()
        .filter(|n| !n.is_processed)
        .filter(|n| n.content_type.as_deref() != Some("paper"))
        .filter(|n| n.content_type.as_deref() != Some("bookmark"))
        .filter(|n| {
            let is_code = n.content_type.as_ref().map(|ct| ct.starts_with("code_")).unwrap_or(false);
            // Include code items only if flag is set
            !is_code || include_code
        })
        // Skip hidden items (already classified by pattern matching in Step 0)
        .filter(|n| !n.content_type.as_ref()
            .and_then(|ct| classification::ContentType::from_str(ct))
            .map(|ct| ct.is_hidden())
            .unwrap_or(false))
        .collect();

    if !unprocessed_non_papers.is_empty() && settings::has_api_key() {
        log!("[1a] AI Processing {} items...", unprocessed_non_papers.len());

        let mut success_count = 0;
        let mut error_count = 0;
        let step_start = std::time::Instant::now();

        for (i, node) in unprocessed_non_papers.iter().enumerate() {
            if !quiet {
                let title_preview: String = node.title.chars().take(50).collect();
                log!("  [{}/{}] {}{}",
                    i + 1,
                    unprocessed_non_papers.len(),
                    title_preview,
                    if node.title.len() > 50 { "..." } else { "" }
                );
            }

            let content = node.content.as_deref().unwrap_or(&node.title);
            let is_code_item = node.content_type.as_ref().map(|ct| ct.starts_with("code_")).unwrap_or(false);

            match mycelica_lib::ai_client::analyze_node(&node.title, content).await {
                Ok(result) => {
                    // For code items, preserve the original code_* content_type
                    let final_content_type = if is_code_item {
                        node.content_type.as_deref().unwrap_or(&result.content_type)
                    } else {
                        &result.content_type
                    };

                    db.update_node_ai(
                        &node.id,
                        &result.title,
                        &result.summary,
                        &serde_json::to_string(&result.tags).unwrap_or_default(),
                        final_content_type,
                    ).ok();

                    // Generate embedding for the node
                    let embed_text = utils::safe_truncate(content, 1000);
                    if let Ok(embedding) = ai_client::generate_embedding(embed_text).await {
                        db.update_node_embedding(&node.id, &embedding).ok();
                    }

                    if !quiet {
                        log!("    → \"{}\" ({})", result.title, final_content_type);
                    }
                    success_count += 1;
                }
                Err(e) => {
                    if !quiet {
                        elog!("    ✗ Error: {}", e);
                    }
                    error_count += 1;
                }
            }
        }
        let elapsed = step_start.elapsed().as_secs_f64();
        log!("  ✓ Processed {} items in {:.1}s ({} errors)", success_count, elapsed, error_count);
    } else if !unprocessed_non_papers.is_empty() {
        log!("[1a] AI Processing... ⊘ SKIPPED (no Anthropic API key)");
    } else {
        log!("[1a] AI Processing... ✓ already complete");
    }

    // 1b: Generate embeddings for papers (they have metadata but need embeddings)
    let papers_needing_embeddings: Vec<_> = items.iter()
        .filter(|n| n.content_type.as_deref() == Some("paper"))
        .filter(|n| db.get_node_embedding(&n.id).ok().flatten().is_none())
        .collect();

    if !papers_needing_embeddings.is_empty() && settings::has_openai_api_key() {
        log!("[1b] Embedding {} papers...", papers_needing_embeddings.len());

        let mut success_count = 0;
        let step_start = std::time::Instant::now();

        for (i, node) in papers_needing_embeddings.iter().enumerate() {
            if !quiet && (i % 10 == 0 || i == papers_needing_embeddings.len() - 1) {
                log!("  [{}/{}] {}", i + 1, papers_needing_embeddings.len(),
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
        log!("  ✓ Embedded {} papers in {:.1}s", success_count, elapsed);
    } else if !papers_needing_embeddings.is_empty() {
        log!("[1b] Paper embeddings... ⊘ SKIPPED (no OpenAI API key)");
    } else if papers > 0 {
        log!("[1b] Paper embeddings... ✓ already complete");
    }

    // 1c: Generate embeddings for code items (skip AI, just embeddings)
    let code_needing_embeddings: Vec<_> = items.iter()
        .filter(|n| n.content_type.as_ref().map(|ct| ct.starts_with("code_")).unwrap_or(false))
        .filter(|n| db.get_node_embedding(&n.id).ok().flatten().is_none())
        .collect();

    if !code_needing_embeddings.is_empty() {
        log!("[1c] Embedding {} code items (skipping AI)...", code_needing_embeddings.len());

        let mut success_count = 0;
        let step_start = std::time::Instant::now();

        for (i, node) in code_needing_embeddings.iter().enumerate() {
            if !quiet && (i % 25 == 0 || i == code_needing_embeddings.len() - 1) {
                log!("  [{}/{}] {}", i + 1, code_needing_embeddings.len(),
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
        log!("  ✓ Embedded {} code items in {:.1}s", success_count, elapsed);
    } else {
        let code_count = items.iter()
            .filter(|n| n.content_type.as_ref().map(|ct| ct.starts_with("code_")).unwrap_or(false))
            .count();
        if code_count > 0 {
            log!("[1c] Code embeddings... ✓ already complete");
        }
    }

    // Step 2: Clustering & Hierarchy
    log!("");
    log!("▶ STEP 2/7: Clustering & Hierarchy");
    log!("───────────────────────────────────────────────────────────");

    if algorithm == "adaptive" {
        // Adaptive algorithm: uses edge-based clustering with auto-config
        log!("Running adaptive hierarchy algorithm...");
        log!("");

        // Step 2a: Generate semantic edges at 0.30 threshold for finer resolution
        log!("  [2a] Generating semantic edges (threshold=0.30)...");
        if let Ok(deleted) = db.delete_semantic_edges() {
            if deleted > 0 {
                log!("    Cleared {} old semantic edges", deleted);
            }
        }
        match db.create_semantic_edges(0.30, 10) {
            Ok(created) => log!("    ✓ Created {} semantic edges", created),
            Err(e) => {
                elog!("    ✗ Failed to create semantic edges: {}", e);
                return Err(format!("Semantic edge creation failed: {}", e));
            }
        }

        // Step 2b: Run adaptive hierarchy with auto-config
        log!("  [2b] Building hierarchy with adaptive algorithm (auto-config)...");
        // Use default values - auto_config will compute optimal params
        match rebuild_hierarchy_adaptive(
            db,
            5,      // min_size (will be overridden by auto)
            0.001,  // tight_threshold (will be overridden by auto)
            1.0,    // cohesion_threshold (will be overridden by auto)
            0.02,   // delta_min (will be overridden by auto)
            true,   // auto = true
            false,  // json
            quiet,
            keywords_only,
            false,  // fresh = false (setup preserves existing state)
        ).await {
            Ok(result) => {
                log!("");
                log!("  ✓ Hierarchy complete: {} categories, {} items assigned",
                    result.categories, result.papers_assigned);
                if result.sibling_edges > 0 {
                    log!("    {} sibling edges, {} bridges detected",
                        result.sibling_edges, result.bridges);
                }
            }
            Err(e) => {
                elog!("  ✗ Adaptive hierarchy failed: {}", e);
                return Err(format!("Adaptive hierarchy build failed: {}", e));
            }
        }
    } else {
        return Err(format!("Unknown algorithm '{}'. Valid options: adaptive, dendrogram", algorithm));
    }

    // Step 3: Code edges (only if code nodes exist)
    // Note: Category embeddings handled by Hierarchy Step 6/7
    log!("");
    log!("▶ STEP 3/7: Code Analysis");
    log!("───────────────────────────────────────────────────────────");

    let code_functions: Vec<_> = items.iter()
        .filter(|n| n.content_type.as_deref() == Some("code_function"))
        .collect();

    if !code_functions.is_empty() {
        log!("Analyzing {} code functions for call relationships...", code_functions.len());
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
                        updated_at: Some(chrono::Utc::now().timestamp_millis()),
                        author: None,
                        reason: None,
                        content: None,
                        agent_id: None,
                        superseded_by: None,
                        metadata: None,
                    };

                    if db.insert_edge(&edge).is_ok() {
                        edges_created += 1;
                    }
                }
            }
        }

        log!("  Indexed {} function names", name_to_id.len());
        log!("  Found {} existing call edges", existing_edges.len());

        if edges_created > 0 {
            log!("✓ Created {} new call edges", edges_created);
        } else {
            log!("✓ No new edges needed (already analyzed or no calls found)");
        }
    } else {
        log!("⊘ SKIPPED (no code_function nodes found)");
        log!("  Run 'mycelica-cli import code <path>' to import code first");
    }

    // Step 4: Flatten hierarchy (remove empty intermediate levels)
    log!("");
    log!("▶ STEP 4/7: Flatten Hierarchy");
    log!("───────────────────────────────────────────────────────────");
    match db.flatten_empty_levels() {
        Ok(removed) => {
            if removed > 0 {
                log!("✓ Flattened: removed {} empty intermediate levels", removed);
            } else {
                log!("✓ Hierarchy already flat (no empty levels)");
            }
        }
        Err(e) => {
            elog!("✗ Flatten failed: {}", e);
        }
    }

    // Step 5: Generate embeddings for categories (so similar nodes works for them)
    log!("");
    log!("▶ STEP 5/7: Category Embeddings");
    log!("───────────────────────────────────────────────────────────");
    {
        use mycelica_lib::local_embeddings;

        // Find categories without embeddings (get_nodes_needing_embeddings includes categories with titles)
        let nodes_needing_embeddings = db.get_nodes_needing_embeddings()
            .map_err(|e| e.to_string())?;
        let categories_without_embeddings: Vec<_> = nodes_needing_embeddings
            .into_iter()
            .filter(|n| !n.is_item && !n.title.is_empty())
            .collect();

        if categories_without_embeddings.is_empty() {
            log!("✓ All categories already have embeddings");
        } else {
            log!("Generating embeddings for {} categories...", categories_without_embeddings.len());
            let start = std::time::Instant::now();
            let mut generated = 0;

            for category in &categories_without_embeddings {
                // Use title + summary for embedding text
                let text = if let Some(summary) = &category.summary {
                    format!("{}: {}", category.title, summary)
                } else {
                    category.title.clone()
                };

                match local_embeddings::generate(&text) {
                    Ok(embedding) => {
                        if let Err(e) = db.update_node_embedding(&category.id, &embedding) {
                            elog!("  Failed to save embedding for {}: {}", category.id, e);
                        } else {
                            generated += 1;
                        }
                    }
                    Err(e) => {
                        elog!("  Failed to generate embedding for {}: {}", category.id, e);
                    }
                }
            }

            log!("✓ Generated {} category embeddings in {:.1}s",
                generated, start.elapsed().as_secs_f64());
        }
    }

    // Step 6: Build HNSW index for fast similarity search
    log!("");
    log!("▶ STEP 6/7: Build HNSW Index");
    log!("───────────────────────────────────────────────────────────");
    {
        use mycelica_lib::commands::{HnswIndex, hnsw_index_path};

        let embeddings = db.get_nodes_with_embeddings().map_err(|e| e.to_string())?;
        if embeddings.is_empty() {
            log!("⊘ SKIPPED (no embeddings found)");
        } else {
            log!("Building index for {} embeddings...", embeddings.len());
            let start = std::time::Instant::now();

            let mut index = HnswIndex::new();
            index.build(&embeddings);

            let db_path = PathBuf::from(db.get_path());
            let index_path = hnsw_index_path(&db_path);
            index.save(&index_path)?;

            log!("✓ HNSW index built in {:.1}s → {}",
                start.elapsed().as_secs_f64(),
                index_path.display());
        }
    }

    // Step 7: Index edges for fast view loading
    log!("");
    log!("▶ STEP 7/7: Index Edge Parents");
    log!("───────────────────────────────────────────────────────────");
    match db.update_edge_parents() {
        Ok(count) => log!("✓ Indexed {} edges with parent IDs", count),
        Err(e) => elog!("✗ Edge indexing failed: {}", e),
    }

    log!("");
    log!("═══════════════════════════════════════════════════════════");
    log!("✓ Setup Complete!");
    log!("═══════════════════════════════════════════════════════════");
    Ok(())
}

// ============================================================================
// Recent/Pinned Commands
// ============================================================================

fn handle_recent(cmd: RecentCommands, db: &Database, json: bool) -> Result<(), String> {
    match cmd {
        RecentCommands::List { limit, author } => {
            let recent = db.get_recent_nodes(limit as i32).map_err(|e| e.to_string())?;
            let filtered: Vec<_> = if let Some(ref author_filter) = author {
                recent.into_iter().filter(|n| {
                    n.author.as_ref().map_or(false, |a| a.to_lowercase().contains(&author_filter.to_lowercase()))
                }).collect()
            } else {
                recent
            };

            if json {
                let items: Vec<String> = filtered.iter().map(|n| {
                    format!(r#"{{"id":"{}","title":"{}","author":{}}}"#,
                        n.id, escape_json(&n.title),
                        n.author.as_ref().map(|a| format!("\"{}\"", escape_json(a))).unwrap_or("null".to_string()))
                }).collect();
                println!("[{}]", items.join(","));
            } else {
                for node in &filtered {
                    let auth = node.author.as_deref().unwrap_or("");
                    println!("{} {} {}", &node.id[..8], node.title, auth);
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
                            println!("{}  {} 📄 {} ({})", prefix, item_prefix, display_title, id);
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

async fn handle_search(query: &str, type_filter: &str, author_filter: Option<String>, recent_filter: Option<String>, limit: u32, db: &Database, json: bool) -> Result<(), String> {
    let results = db.search_nodes(query).map_err(|e| e.to_string())?;

    // Parse recency filter (e.g., "7d", "24h", "30d")
    let cutoff_ms = recent_filter.as_ref().map(|r| {
        let now = Utc::now().timestamp_millis();
        let duration_ms = if r.ends_with('d') {
            r.trim_end_matches('d').parse::<i64>().unwrap_or(7) * 86_400_000
        } else if r.ends_with('h') {
            r.trim_end_matches('h').parse::<i64>().unwrap_or(24) * 3_600_000
        } else {
            r.parse::<i64>().unwrap_or(7) * 86_400_000
        };
        now - duration_ms
    });

    let filtered: Vec<_> = results.into_iter()
        .filter(|node| {
            let type_ok = match type_filter {
                "item" => node.is_item,
                "category" => !node.is_item,
                "paper" => node.source.as_deref() == Some("openaire"),
                "concept" => node.content_type.as_deref() == Some("concept"),
                "question" => node.content_type.as_deref() == Some("question"),
                "decision" => node.content_type.as_deref() == Some("decision"),
                "reference" => node.content_type.as_deref() == Some("reference"),
                _ => true, // "all"
            };
            let author_ok = author_filter.as_ref().map_or(true, |a| {
                node.author.as_ref().map_or(false, |na| na.to_lowercase().contains(&a.to_lowercase()))
            });
            let recent_ok = cutoff_ms.map_or(true, |cutoff| node.created_at >= cutoff);
            type_ok && author_ok && recent_ok
        })
        .take(limit as usize)
        .collect();

    if json {
        let items: Vec<String> = filtered.iter().map(|node| {
            format!(
                r#"{{"id":"{}","title":"{}","type":"{}","content_type":{},"author":{},"depth":{}}}"#,
                node.id,
                escape_json(node.ai_title.as_ref().unwrap_or(&node.title)),
                if node.is_item { "item" } else { "category" },
                node.content_type.as_ref().map(|ct| format!("\"{}\"", ct)).unwrap_or("null".to_string()),
                node.author.as_ref().map(|a| format!("\"{}\"", escape_json(a))).unwrap_or("null".to_string()),
                node.depth
            )
        }).collect();
        println!("[{}]", items.join(","));
    } else {
        if filtered.is_empty() {
            log!("No results for '{}'", query);
        } else {
            log!("Found {} results for '{}':\n", filtered.len(), query);
            for node in &filtered {
                let emoji = if node.is_item { "[I]" } else { "[C]" };
                let title = node.ai_title.as_ref().unwrap_or(&node.title);
                let ct = node.content_type.as_deref().unwrap_or("?");
                let auth = node.author.as_deref().unwrap_or("");
                log!("{} {} [{}] {} {}", emoji, title, ct, auth, &node.id);
            }
        }
    }

    Ok(())
}

// ============================================================================
// Team Commands (Phase 2)
// ============================================================================

/// Create a node with proper sovereignty fields set. Returns the new node ID.
/// Delegates to team::create_human_node with CLI-specific defaults.
fn create_human_node(
    db: &Database,
    title: &str,
    content: Option<&str>,
    url: Option<&str>,
    content_type: &str,
    tags_json: Option<&str>,
) -> Result<String, String> {
    let author = settings::get_author_or_default();
    mycelica_lib::team::create_human_node(db, title, content, url, content_type, tags_json, &author, "cli", None)
}

/// Resolve a node reference (UUID, ID prefix, or title text).
/// Wraps team::resolve_node with CLI-friendly error formatting.
fn resolve_node(db: &Database, reference: &str) -> Result<Node, String> {
    use mycelica_lib::team::ResolveResult;
    match mycelica_lib::team::resolve_node(db, reference) {
        ResolveResult::Found(node) => Ok(node),
        ResolveResult::Ambiguous(candidates) => {
            let names: Vec<String> = candidates.iter().map(|c| {
                format!("  {} {}", &c.id[..8.min(c.id.len())], c.title)
            }).collect();
            Err(format!("Ambiguous reference '{}'. {} matches:\n{}\nUse a node ID instead.", reference, candidates.len(), names.join("\n")))
        }
        ResolveResult::NotFound(msg) => Err(msg),
    }
}

/// Process --connects-to terms: search for each, create Related edges.
/// Wraps team::create_connects_to_edges with CLI output formatting.
fn handle_connects_to(
    db: &Database,
    source_node_id: &str,
    terms: &[String],
    json: bool,
) -> Result<(), String> {
    let author = settings::get_author_or_default();
    let results = mycelica_lib::team::create_connects_to_edges(db, source_node_id, terms, &author);
    for result in results {
        match result {
            mycelica_lib::team::ConnectResult::Linked { target, .. } => {
                if !json {
                    println!("  Linked to: {} {}", &target.id[..8.min(target.id.len())], target.title);
                }
            }
            mycelica_lib::team::ConnectResult::Ambiguous { term, candidates } => {
                if !json {
                    let names: Vec<String> = candidates.iter().map(|c| {
                        format!("    {} {}", &c.id[..8.min(c.id.len())], c.title)
                    }).collect();
                    eprintln!("  Warning ({}): Ambiguous, {} matches:\n{}", term, candidates.len(), names.join("\n"));
                }
            }
            mycelica_lib::team::ConnectResult::NotFound { term } => {
                if !json {
                    eprintln!("  Warning: no node found for '{}'", term);
                }
            }
        }
    }
    Ok(())
}

async fn handle_add(
    url: &str,
    note: Option<String>,
    tag: Option<String>,
    connects_to: Option<Vec<String>>,
    db: &Database,
    json: bool,
) -> Result<(), String> {
    let title = note.as_deref().unwrap_or(url);
    let content = format!("{}\n\n{}", url, note.as_deref().unwrap_or(""));
    let tags_json = tag.map(|t| {
        let tags: Vec<&str> = t.split(',').map(|s| s.trim()).collect();
        serde_json::to_string(&tags).unwrap_or_default()
    });

    let id = create_human_node(db, title, Some(&content), Some(url), "reference", tags_json.as_deref())?;

    if json {
        println!(r#"{{"id":"{}","title":"{}","url":"{}"}}"#, id, escape_json(title), escape_json(url));
    } else {
        println!("Added: {} {}", &id[..8], title);
    }

    if let Some(terms) = connects_to {
        handle_connects_to(db, &id, &terms, json)?;
    }

    Ok(())
}

async fn handle_ask(
    question: &str,
    connects_to: Option<Vec<String>>,
    db: &Database,
    json: bool,
) -> Result<(), String> {
    let id = create_human_node(db, question, None, None, "question", None)?;

    if json {
        println!(r#"{{"id":"{}","question":"{}"}}"#, id, escape_json(question));
    } else {
        println!("Asked: {} {}", &id[..8], question);
    }

    if let Some(terms) = connects_to {
        handle_connects_to(db, &id, &terms, json)?;
    }

    Ok(())
}

async fn handle_concept(
    title: &str,
    note: Option<String>,
    connects_to: Option<Vec<String>>,
    db: &Database,
    json: bool,
) -> Result<(), String> {
    let id = create_human_node(db, title, note.as_deref(), None, "concept", None)?;

    if json {
        println!(r#"{{"id":"{}","title":"{}"}}"#, id, escape_json(title));
    } else {
        println!("Concept: {} {}", &id[..8], title);
    }

    if let Some(terms) = connects_to {
        handle_connects_to(db, &id, &terms, json)?;
    }

    Ok(())
}

async fn handle_decide(
    title: &str,
    reason: Option<String>,
    connects_to: Option<Vec<String>>,
    db: &Database,
    json: bool,
) -> Result<(), String> {
    let id = create_human_node(db, title, reason.as_deref(), None, "decision", None)?;

    if json {
        println!(r#"{{"id":"{}","title":"{}"}}"#, id, escape_json(title));
    } else {
        println!("Decided: {} {}", &id[..8], title);
        if let Some(ref r) = reason {
            println!("  Reason: {}", r);
        }
    }

    if let Some(terms) = connects_to {
        handle_connects_to(db, &id, &terms, json)?;
    }

    Ok(())
}

fn handle_migrate(cmd: MigrateCommands, db: &Database, _db_path: &Path, json: bool) -> Result<(), String> {
    match cmd {
        MigrateCommands::SporeSchema { .. } => {
            // Backup already created in run_cli() before Database::new() triggered auto-migration.
            // Just validate the columns exist.
            let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;

            let node_columns: Vec<String> = conn.prepare("SELECT name FROM pragma_table_info('nodes')")
                .map_err(|e| e.to_string())?
                .query_map([], |row| row.get(0))
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();

            let edge_columns: Vec<String> = conn.prepare("SELECT name FROM pragma_table_info('edges')")
                .map_err(|e| e.to_string())?
                .query_map([], |row| row.get(0))
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();

            let required_node_cols = ["agent_id", "node_class", "meta_type"];
            let required_edge_cols = ["content", "agent_id", "superseded_by", "metadata"];

            let mut missing = Vec::new();
            for col in &required_node_cols {
                if !node_columns.iter().any(|c| c == col) {
                    missing.push(format!("nodes.{}", col));
                }
            }
            for col in &required_edge_cols {
                if !edge_columns.iter().any(|c| c == col) {
                    missing.push(format!("edges.{}", col));
                }
            }

            if !missing.is_empty() {
                return Err(format!("Migration failed — missing columns: {}", missing.join(", ")));
            }

            // Step 3: Report stats
            let node_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM nodes WHERE agent_id IS NOT NULL", [], |row| row.get(0)
            ).unwrap_or(0);
            let edge_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM edges WHERE agent_id IS NOT NULL", [], |row| row.get(0)
            ).unwrap_or(0);

            if json {
                println!("{{\"status\":\"ok\",\"node_columns\":{:?},\"edge_columns\":{:?},\"nodes_with_agent\":{},\"edges_with_agent\":{}}}",
                    required_node_cols, required_edge_cols, node_count, edge_count);
            } else {
                eprintln!("Spore schema migration complete.");
                eprintln!("  Node columns added: {}", required_node_cols.join(", "));
                eprintln!("  Edge columns added: {}", required_edge_cols.join(", "));
                eprintln!("  Nodes with agent_id: {}", node_count);
                eprintln!("  Edges with agent_id: {}", edge_count);
            }

            Ok(())
        }
    }
}

// ============================================================================
// Spore Commands
// ============================================================================

async fn handle_spore(cmd: SporeCommands, db: &Database, json: bool) -> Result<(), String> {
    match cmd {
        SporeCommands::QueryEdges { edge_type, agent, target_agent, confidence_min, since, not_superseded, limit, compact } => {
            let since_millis = since.as_deref()
                .map(parse_since_to_millis)
                .transpose()?;

            let results = db.query_edges(
                edge_type.as_deref(),
                agent.as_deref(),
                target_agent.as_deref(),
                confidence_min,
                since_millis,
                not_superseded,
                limit,
            ).map_err(|e| e.to_string())?;

            if json {
                println!("{}", serde_json::to_string(&results).unwrap_or_default());
            } else if compact {
                for ewn in &results {
                    let e = &ewn.edge;
                    let src = ewn.source_title.as_deref().unwrap_or(&e.source[..8.min(e.source.len())]);
                    let tgt = ewn.target_title.as_deref().unwrap_or(&e.target[..8.min(e.target.len())]);
                    let conf = e.confidence.map(|c| format!("{:.2}", c)).unwrap_or_else(|| "?".to_string());
                    println!("{} {} {} -> {} [{}]",
                        &e.id[..8.min(e.id.len())],
                        e.edge_type.as_str(),
                        src, tgt, conf);
                }
            } else {
                if results.is_empty() {
                    println!("No matching edges found.");
                } else {
                    for ewn in &results {
                        let e = &ewn.edge;
                        let src = ewn.source_title.as_deref().unwrap_or(&e.source[..8.min(e.source.len())]);
                        let tgt = ewn.target_title.as_deref().unwrap_or(&e.target[..8.min(e.target.len())]);
                        let conf = e.confidence.map(|c| format!("{:.0}%", c * 100.0)).unwrap_or_else(|| "?".to_string());
                        let agent_str = e.agent_id.as_deref().unwrap_or("?");
                        let date = chrono::DateTime::from_timestamp_millis(e.created_at)
                            .map(|d| d.format("%Y-%m-%d").to_string())
                            .unwrap_or_else(|| "?".to_string());
                        println!("{} {} {} → {} [{}] agent:{} {}",
                            &e.id[..8.min(e.id.len())],
                            e.edge_type.as_str(),
                            src, tgt, conf, agent_str, date);
                    }
                    println!("\n{} edge(s)", results.len());
                }
            }
            Ok(())
        }

        SporeCommands::ExplainEdge { id, depth } => {
            let explanation = db.explain_edge(&id, depth).map_err(|e| e.to_string())?;
            match explanation {
                None => {
                    if json {
                        println!("null");
                    } else {
                        println!("Edge not found: {}", id);
                    }
                }
                Some(exp) => {
                    if json {
                        println!("{}", serde_json::to_string(&exp).unwrap_or_default());
                    } else {
                        let e = &exp.edge;
                        println!("Edge: {} [{}]", e.id, e.edge_type.as_str());
                        println!("  Confidence: {}", e.confidence.map(|c| format!("{:.0}%", c * 100.0)).unwrap_or_else(|| "?".to_string()));
                        println!("  Agent: {}", e.agent_id.as_deref().unwrap_or("?"));
                        if let Some(ref reason) = e.reason {
                            println!("  Reason: {}", reason);
                        }
                        if let Some(ref content) = e.content {
                            println!("  Content: {}", &content[..200.min(content.len())]);
                        }
                        if e.superseded_by.is_some() {
                            println!("  [SUPERSEDED by {}]", e.superseded_by.as_deref().unwrap_or("?"));
                        }
                        println!("\nSource: {} ({})", exp.source_node.ai_title.as_ref().unwrap_or(&exp.source_node.title), &exp.source_node.id[..8.min(exp.source_node.id.len())]);
                        if let Some(ref summary) = exp.source_node.summary {
                            println!("  {}", &summary[..200.min(summary.len())]);
                        }
                        println!("\nTarget: {} ({})", exp.target_node.ai_title.as_ref().unwrap_or(&exp.target_node.title), &exp.target_node.id[..8.min(exp.target_node.id.len())]);
                        if let Some(ref summary) = exp.target_node.summary {
                            println!("  {}", &summary[..200.min(summary.len())]);
                        }

                        if !exp.adjacent_edges.is_empty() {
                            println!("\nAdjacent edges ({}):", exp.adjacent_edges.len());
                            for ae in &exp.adjacent_edges {
                                println!("  {} {} {} → {}",
                                    &ae.id[..8.min(ae.id.len())],
                                    ae.edge_type.as_str(),
                                    &ae.source[..8.min(ae.source.len())],
                                    &ae.target[..8.min(ae.target.len())]);
                            }
                        }

                        if !exp.supersession_chain.is_empty() {
                            println!("\nSupersession chain ({}):", exp.supersession_chain.len());
                            for se in &exp.supersession_chain {
                                let status = if se.superseded_by.is_some() { "superseded" } else { "current" };
                                println!("  {} [{}] {}", &se.id[..8.min(se.id.len())], se.edge_type.as_str(), status);
                            }
                        }
                    }
                }
            }
            Ok(())
        }

        SporeCommands::PathBetween { from, to, max_hops, edge_types } => {
            let source = resolve_node(db, &from)?;
            let target = resolve_node(db, &to)?;

            let type_list: Option<Vec<String>> = edge_types.map(|s| s.split(',').map(|t| t.trim().to_string()).collect());
            let type_refs: Option<Vec<&str>> = type_list.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect());

            let paths = db.path_between(&source.id, &target.id, max_hops, type_refs.as_deref())
                .map_err(|e| e.to_string())?;

            if json {
                println!("{}", serde_json::to_string(&paths).unwrap_or_default());
            } else {
                if paths.is_empty() {
                    println!("No paths found between {} and {} (max {} hops)",
                        source.ai_title.as_ref().unwrap_or(&source.title),
                        target.ai_title.as_ref().unwrap_or(&target.title),
                        max_hops);
                } else {
                    let src_name = source.ai_title.as_ref().unwrap_or(&source.title);
                    for (i, path) in paths.iter().enumerate() {
                        let mut display = format!("{}", src_name);
                        for hop in path {
                            display.push_str(&format!(" →[{}]→ {}", hop.edge.edge_type.as_str(), hop.node_title));
                        }
                        println!("Path {}: {}", i + 1, display);
                    }
                    println!("\n{} path(s) found", paths.len());
                }
            }
            Ok(())
        }

        SporeCommands::EdgesForContext { id, top, not_superseded } => {
            let node = resolve_node(db, &id)?;

            let edges = db.edges_for_context(&node.id, top, not_superseded)
                .map_err(|e| e.to_string())?;

            if json {
                println!("{}", serde_json::to_string(&edges).unwrap_or_default());
            } else {
                if edges.is_empty() {
                    println!("No edges found for {}", node.ai_title.as_ref().unwrap_or(&node.title));
                } else {
                    println!("Top {} edges for: {}", edges.len(), node.ai_title.as_ref().unwrap_or(&node.title));
                    for (i, e) in edges.iter().enumerate() {
                        let other_id = if e.source == node.id { &e.target } else { &e.source };
                        let other_name = db.get_node(other_id).ok().flatten()
                            .map(|n| n.ai_title.unwrap_or(n.title))
                            .unwrap_or_else(|| other_id[..8.min(other_id.len())].to_string());
                        let conf = e.confidence.map(|c| format!(" {:.0}%", c * 100.0)).unwrap_or_default();
                        let dir = if e.source == node.id { "→" } else { "←" };
                        println!("  {}. {} {} {} [{}]{}",
                            i + 1, dir, e.edge_type.as_str(), other_name, &e.id[..8.min(e.id.len())], conf);
                    }
                }
            }
            Ok(())
        }

        SporeCommands::CreateMeta { meta_type, title, content, agent, connects_to, edge_type } => {
            // Validate meta_type
            let valid_types = ["summary", "contradiction", "status"];
            if !valid_types.contains(&meta_type.as_str()) {
                return Err(format!("Invalid meta type: '{}'. Must be one of: summary, contradiction, status", meta_type));
            }

            // Validate edge_type
            let et = EdgeType::from_str(&edge_type.to_lowercase())
                .ok_or_else(|| format!("Unknown edge type: '{}'", edge_type))?;

            // Find universe node for parent_id
            let universe_id = {
                let all_nodes = db.get_all_nodes(true).map_err(|e| e.to_string())?;
                all_nodes.iter().find(|n| n.is_universe).map(|n| n.id.clone())
                    .ok_or_else(|| "No universe node found. Run 'mycelica-cli hierarchy build' first.".to_string())?
            };

            let author = settings::get_author_or_default();
            let now = Utc::now().timestamp_millis();
            let node_id = uuid::Uuid::new_v4().to_string();

            let node = Node {
                id: node_id.clone(),
                node_type: NodeType::Thought,
                title: title.clone(),
                url: None,
                content,
                position: Position { x: 0.0, y: 0.0 },
                created_at: now,
                updated_at: now,
                cluster_id: None,
                cluster_label: None,
                depth: 1,
                is_item: false,
                is_universe: false,
                parent_id: Some(universe_id),
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
                source: Some("spore".to_string()),
                pdf_available: None,
                content_type: None,
                associated_idea_id: None,
                privacy: None,
                human_edited: None,
                human_created: true,
                author: Some(author.clone()),
                agent_id: Some(agent.clone()),
                node_class: Some("meta".to_string()),
                meta_type: Some(meta_type.clone()),
            };

            let mut edges = Vec::new();
            for target_id in &connects_to {
                edges.push(Edge {
                    id: uuid::Uuid::new_v4().to_string(),
                    source: node_id.clone(),
                    target: target_id.clone(),
                    edge_type: et.clone(),
                    label: None,
                    weight: Some(1.0),
                    edge_source: Some("user".to_string()),
                    evidence_id: None,
                    confidence: Some(1.0),
                    created_at: now,
                    updated_at: Some(now),
                    author: Some(author.clone()),
                    reason: None,
                    content: None,
                    agent_id: Some(agent.clone()),
                    superseded_by: None,
                    metadata: None,
                });
            }

            db.create_meta_node_with_edges(&node, &edges).map_err(|e| e.to_string())?;

            if json {
                println!(r#"{{"id":"{}","type":"{}","title":"{}","edges":{}}}"#,
                    node_id, meta_type, escape_json(&title), edges.len());
            } else {
                println!("Created meta node: {} [{}] \"{}\"", &node_id[..8], meta_type, title);
                if !connects_to.is_empty() {
                    println!("  {} {} edge(s) created", connects_to.len(), edge_type);
                }
            }
            Ok(())
        }

        SporeCommands::UpdateMeta { id, content, title, agent, add_connects, edge_type } => {
            let old_node = resolve_node(db, &id)?;

            // Verify it's a meta node
            if old_node.node_class.as_deref() != Some("meta") {
                return Err(format!("Node {} is not a meta node (class: {:?})", id, old_node.node_class));
            }

            let author = settings::get_author_or_default();
            let now = Utc::now().timestamp_millis();
            let new_id = uuid::Uuid::new_v4().to_string();

            // Create NEW meta node inheriting fields from old
            let new_node = Node {
                id: new_id.clone(),
                node_type: old_node.node_type.clone(),
                title: title.unwrap_or_else(|| old_node.title.clone()),
                url: old_node.url.clone(),
                content: content.or_else(|| old_node.content.clone()),
                position: Position { x: 0.0, y: 0.0 },
                created_at: now,
                updated_at: now,
                cluster_id: None,
                cluster_label: None,
                depth: old_node.depth,
                is_item: old_node.is_item,
                is_universe: false,
                parent_id: old_node.parent_id.clone(),
                child_count: 0,
                ai_title: old_node.ai_title.clone(),
                summary: old_node.summary.clone(),
                tags: old_node.tags.clone(),
                emoji: old_node.emoji.clone(),
                is_processed: old_node.is_processed,
                conversation_id: None,
                sequence_index: None,
                is_pinned: false,
                last_accessed_at: None,
                latest_child_date: None,
                is_private: old_node.is_private,
                privacy_reason: old_node.privacy_reason.clone(),
                source: Some("spore".to_string()),
                pdf_available: None,
                content_type: old_node.content_type.clone(),
                associated_idea_id: None,
                privacy: old_node.privacy,
                human_edited: None,
                human_created: true,
                author: Some(author.clone()),
                agent_id: Some(agent.clone()),
                node_class: old_node.node_class.clone(),
                meta_type: old_node.meta_type.clone(),
            };

            // Build edges for new node
            let mut edges = Vec::new();

            // 1. Supersedes edge: new -> old
            edges.push(Edge {
                id: uuid::Uuid::new_v4().to_string(),
                source: new_id.clone(),
                target: old_node.id.clone(),
                edge_type: EdgeType::Supersedes,
                label: None,
                weight: Some(1.0),
                edge_source: Some("spore".to_string()),
                evidence_id: None,
                confidence: Some(1.0),
                created_at: now,
                updated_at: Some(now),
                author: Some(author.clone()),
                reason: Some(format!("Supersedes {}", &old_node.id[..8.min(old_node.id.len())])),
                content: None,
                agent_id: Some(agent.clone()),
                superseded_by: None,
                metadata: None,
            });

            // 2. Copy old node's outgoing edges (excluding superseded and Supersedes-typed)
            let old_edges = db.get_edges_for_node(&old_node.id).map_err(|e| e.to_string())?;
            for old_edge in &old_edges {
                // Only copy outgoing edges from old node
                if old_edge.source != old_node.id {
                    continue;
                }
                // Skip edges that have been superseded
                if old_edge.superseded_by.is_some() {
                    continue;
                }
                // Skip Supersedes-typed edges (avoid false chains)
                if old_edge.edge_type == EdgeType::Supersedes {
                    continue;
                }
                edges.push(Edge {
                    id: uuid::Uuid::new_v4().to_string(),
                    source: new_id.clone(),
                    target: old_edge.target.clone(),
                    edge_type: old_edge.edge_type.clone(),
                    label: old_edge.label.clone(),
                    weight: old_edge.weight,
                    edge_source: Some("spore".to_string()),
                    evidence_id: old_edge.evidence_id.clone(),
                    confidence: old_edge.confidence,
                    created_at: now,
                    updated_at: Some(now),
                    author: Some(author.clone()),
                    reason: old_edge.reason.clone(),
                    content: old_edge.content.clone(),
                    agent_id: Some(agent.clone()),
                    superseded_by: None,
                    metadata: old_edge.metadata.clone(),
                });
            }

            // 3. New --add-connects edges
            if !add_connects.is_empty() {
                let et = EdgeType::from_str(&edge_type.to_lowercase())
                    .ok_or_else(|| format!("Unknown edge type: '{}'", edge_type))?;
                for target_id in &add_connects {
                    edges.push(Edge {
                        id: uuid::Uuid::new_v4().to_string(),
                        source: new_id.clone(),
                        target: target_id.clone(),
                        edge_type: et.clone(),
                        label: None,
                        weight: Some(1.0),
                        edge_source: Some("spore".to_string()),
                        evidence_id: None,
                        confidence: Some(1.0),
                        created_at: now,
                        updated_at: Some(now),
                        author: Some(author.clone()),
                        reason: None,
                        content: None,
                        agent_id: Some(agent.clone()),
                        superseded_by: None,
                        metadata: None,
                    });
                }
            }

            let copied_count = edges.len() - 1 - add_connects.len(); // total - supersedes - new
            db.create_meta_node_with_edges(&new_node, &edges).map_err(|e| e.to_string())?;

            if json {
                println!(r#"{{"newId":"{}","oldId":"{}","copiedEdges":{},"newEdges":{}}}"#,
                    new_id, old_node.id, copied_count, add_connects.len());
            } else {
                println!("Created superseding meta node: {} -> {}",
                    &new_id[..8.min(new_id.len())], &old_node.id[..8.min(old_node.id.len())]);
                println!("  Copied {} edge(s), added {} new edge(s)", copied_count, add_connects.len());
            }
            Ok(())
        }

        SporeCommands::Status { all, format } => {
            let full_mode = format == "full";
            // Meta nodes by type
            let meta_nodes = db.get_meta_nodes(None).map_err(|e| e.to_string())?;
            let summaries = meta_nodes.iter().filter(|n| n.meta_type.as_deref() == Some("summary")).count();
            let contradictions_meta = meta_nodes.iter().filter(|n| n.meta_type.as_deref() == Some("contradiction")).count();
            let statuses = meta_nodes.iter().filter(|n| n.meta_type.as_deref() == Some("status")).count();

            // Edge stats
            let all_edges = db.get_all_edges().map_err(|e| e.to_string())?;
            let now = Utc::now().timestamp_millis();
            let day_ago = now - 86_400_000;
            let week_ago = now - 7 * 86_400_000;

            let edges_24h = all_edges.iter().filter(|e| e.created_at >= day_ago).count();
            let edges_7d = all_edges.iter().filter(|e| e.created_at >= week_ago).count();

            // Unresolved contradictions (contradiction edges not superseded)
            let unresolved = all_edges.iter()
                .filter(|e| e.edge_type == EdgeType::Contradicts && e.superseded_by.is_none())
                .count();

            // Coverage: knowledge nodes referenced by summarizes edges
            let all_nodes = db.get_all_nodes(true).map_err(|e| e.to_string())?;
            let knowledge_nodes = all_nodes.iter().filter(|n| n.node_class.as_deref() != Some("meta") && n.is_item).count();
            let summarized_targets: std::collections::HashSet<&str> = all_edges.iter()
                .filter(|e| e.edge_type == EdgeType::Summarizes && e.superseded_by.is_none())
                .map(|e| e.target.as_str())
                .collect();
            let coverage = if knowledge_nodes > 0 {
                summarized_targets.len() as f64 / knowledge_nodes as f64
            } else {
                0.0
            };

            // Coherence
            let active_edges = all_edges.iter().filter(|e| e.superseded_by.is_none()).count();
            let coherence = if active_edges > 0 {
                1.0 - (unresolved as f64 / active_edges as f64)
            } else {
                1.0
            };

            if json {
                println!(r#"{{"metaNodes":{{"summary":{},"contradiction":{},"status":{}}},"edges":{{"last24h":{},"last7d":{},"total":{}}},"unresolvedContradictions":{},"coverage":{:.4},"coherence":{:.6}}}"#,
                    summaries, contradictions_meta, statuses,
                    edges_24h, edges_7d, all_edges.len(),
                    unresolved, coverage, coherence);
            } else {
                println!("=== Spore Status ===\n");
                println!("Meta nodes: {} total", meta_nodes.len());
                println!("  Summaries:      {}", summaries);
                println!("  Contradictions: {}", contradictions_meta);
                println!("  Status nodes:   {}", statuses);

                println!("\nEdge activity:");
                println!("  Last 24h: {}", edges_24h);
                println!("  Last 7d:  {}", edges_7d);
                println!("  Total:    {}", all_edges.len());

                println!("\nUnresolved contradictions: {}", unresolved);
                println!("Coverage: {:.1}% ({} / {} knowledge nodes summarized)", coverage * 100.0, summarized_targets.len(), knowledge_nodes);
                println!("Coherence: {:.4}", coherence);

                if all {
                    // Agent breakdown
                    let mut agent_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
                    for e in all_edges.iter().filter(|e| e.created_at >= week_ago) {
                        let agent_key = e.agent_id.as_deref().unwrap_or("unknown").to_string();
                        *agent_counts.entry(agent_key).or_insert(0) += 1;
                    }
                    if !agent_counts.is_empty() {
                        println!("\nEdges by agent (last 7d):");
                        let mut sorted: Vec<_> = agent_counts.into_iter().collect();
                        sorted.sort_by(|a, b| b.1.cmp(&a.1));
                        for (agent_name, count) in sorted {
                            println!("  {}: {}", agent_name, count);
                        }
                    }

                    // List unresolved contradictions
                    if unresolved > 0 {
                        println!("\nUnresolved contradiction edges:");
                        for e in all_edges.iter()
                            .filter(|e| e.edge_type == EdgeType::Contradicts && e.superseded_by.is_none())
                            .take(10)
                        {
                            let src_name = db.get_node(&e.source).ok().flatten()
                                .map(|n| n.ai_title.unwrap_or(n.title))
                                .unwrap_or_else(|| e.source[..8.min(e.source.len())].to_string());
                            let tgt_name = db.get_node(&e.target).ok().flatten()
                                .map(|n| n.ai_title.unwrap_or(n.title))
                                .unwrap_or_else(|| e.target[..8.min(e.target.len())].to_string());
                            println!("  {} contradicts {}", src_name, tgt_name);
                        }
                        if unresolved > 10 {
                            println!("  ... and {} more", unresolved - 10);
                        }
                    }

                    // List meta nodes
                    if !meta_nodes.is_empty() {
                        println!("\nMeta nodes:");
                        for mn in &meta_nodes {
                            let mt = mn.meta_type.as_deref().unwrap_or("?");
                            let agent_name = mn.agent_id.as_deref().unwrap_or("?");
                            println!("  [{}] {} (agent: {}, {})",
                                mt, mn.title, agent_name, &mn.id[..8.min(mn.id.len())]);
                        }
                    }
                }

                if full_mode {
                    // (1) Top 5 most-connected nodes by edge count
                    println!("\n--- Full Mode ---");
                    println!("\nTop 5 most-connected nodes:");
                    // Collect IDs first, then drop the lock before calling get_node
                    let top_nodes: Vec<(String, i64)> = (|| -> Result<Vec<_>, String> {
                        let conn = db.raw_conn().lock().unwrap();
                        let mut stmt = conn.prepare(
                            "SELECT node_id, COUNT(*) as edge_count FROM (
                                SELECT source_id as node_id FROM edges
                                UNION ALL
                                SELECT target_id as node_id FROM edges
                            ) GROUP BY node_id ORDER BY edge_count DESC LIMIT 5"
                        ).map_err(|e| e.to_string())?;
                        let rows: Vec<_> = stmt.query_map([], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                        }).map_err(|e| e.to_string())?
                        .filter_map(|r| r.ok())
                        .collect();
                        Ok(rows)
                    })().unwrap_or_default();
                    for (node_id, count) in &top_nodes {
                        let name = db.get_node(node_id).ok().flatten()
                            .map(|n| n.ai_title.unwrap_or(n.title))
                            .unwrap_or_else(|| node_id[..8.min(node_id.len())].to_string());
                        println!("  {} edges  {}  ({})", count, name, &node_id[..8.min(node_id.len())]);
                    }

                    // (2) Edge type distribution
                    println!("\nEdge type distribution:");
                    {
                        let mut type_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
                        for e in &all_edges {
                            *type_counts.entry(e.edge_type.as_str().to_string()).or_insert(0) += 1;
                        }
                        let mut sorted: Vec<_> = type_counts.into_iter().collect();
                        sorted.sort_by(|a, b| b.1.cmp(&a.1));
                        for (etype, count) in sorted {
                            println!("  {:20} {}", etype, count);
                        }
                    }

                    // (3) Recent operational nodes (last 24h)
                    println!("\nRecent operational nodes (last 24h):");
                    {
                        let ops: Vec<_> = all_nodes.iter()
                            .filter(|n| n.node_class.as_deref() == Some("operational") && n.created_at >= day_ago)
                            .collect();
                        if ops.is_empty() {
                            println!("  (none)");
                        } else {
                            for n in &ops {
                                let agent_name = n.agent_id.as_deref().unwrap_or("?");
                                let ts = chrono::DateTime::from_timestamp_millis(n.created_at)
                                    .map(|dt| dt.format("%H:%M:%S").to_string())
                                    .unwrap_or_else(|| "?".to_string());
                                println!("  [{}] {} (agent: {}, {})",
                                    ts, n.title, agent_name, &n.id[..8.min(n.id.len())]);
                            }
                        }
                    }
                }
            }
            Ok(())
        }

        // Gap 3a: Create edge between existing nodes (delegates to handle_link)
        SporeCommands::CreateEdge { from, to, edge_type, content, reason, agent, confidence, supersedes } => {
            handle_link(&from, &to, &edge_type, reason, content, &agent, confidence, supersedes, "spore", db, json).await
        }

        // Gap 3b: Read full content of a node (no metadata noise)
        SporeCommands::ReadContent { id } => {
            let node = resolve_node(db, &id)?;
            if json {
                println!(r#"{{"id":"{}","title":"{}","content":{},"tags":{},"content_type":{},"node_class":{},"meta_type":{}}}"#,
                    node.id,
                    escape_json(&node.title),
                    node.content.as_ref().map(|c| format!("\"{}\"", escape_json(c))).unwrap_or("null".to_string()),
                    node.tags.as_ref().map(|t| format!("\"{}\"", escape_json(t))).unwrap_or("null".to_string()),
                    node.content_type.as_ref().map(|c| format!("\"{}\"", c)).unwrap_or("null".to_string()),
                    node.node_class.as_ref().map(|c| format!("\"{}\"", c)).unwrap_or("null".to_string()),
                    node.meta_type.as_ref().map(|m| format!("\"{}\"", m)).unwrap_or("null".to_string()),
                );
            } else {
                if let Some(ref content) = node.content {
                    println!("{}", content);
                } else {
                    println!("(no content)");
                }
            }
            Ok(())
        }

        // Gap 3c: List descendants of a category
        SporeCommands::ListRegion { id, class, items_only, limit } => {
            let parent = resolve_node(db, &id)?;
            let descendants = db.get_descendants(&parent.id, class.as_deref(), items_only, limit)
                .map_err(|e| e.to_string())?;

            if json {
                let items: Vec<String> = descendants.iter().map(|n| {
                    format!(r#"{{"id":"{}","title":"{}","depth":{},"is_item":{},"node_class":{},"content_type":{}}}"#,
                        n.id,
                        escape_json(&n.title),
                        n.depth,
                        n.is_item,
                        n.node_class.as_ref().map(|c| format!("\"{}\"", c)).unwrap_or("null".to_string()),
                        n.content_type.as_ref().map(|c| format!("\"{}\"", c)).unwrap_or("null".to_string()),
                    )
                }).collect();
                println!("[{}]", items.join(","));
            } else {
                if descendants.is_empty() {
                    println!("No descendants found for: {} ({})", parent.title, &parent.id[..8.min(parent.id.len())]);
                } else {
                    let parent_depth = parent.depth;
                    for n in &descendants {
                        let indent = "  ".repeat((n.depth - parent_depth).max(0) as usize);
                        let marker = if n.is_item { "[I]" } else { "[C]" };
                        let class_label = n.node_class.as_deref().unwrap_or("");
                        let ct_label = n.content_type.as_deref().map(|c| format!(" ({})", c)).unwrap_or_default();
                        println!("{}{} {} {}{}", indent, marker, &n.id[..8.min(n.id.len())], n.title, if class_label.is_empty() { ct_label } else { format!(" [{}]{}", class_label, ct_label) });
                    }
                    println!("\n{} descendant(s)", descendants.len());
                }
            }
            Ok(())
        }

        // Gap 3f: Check freshness of summary meta-nodes
        SporeCommands::CheckFreshness { id } => {
            let node = resolve_node(db, &id)?;

            // Find all summarizes edges where this node is the TARGET
            let edges = db.get_edges_for_node(&node.id).map_err(|e| e.to_string())?;
            let summary_edges: Vec<&Edge> = edges.iter()
                .filter(|e| e.edge_type == EdgeType::Summarizes && e.target == node.id && e.superseded_by.is_none())
                .collect();

            if summary_edges.is_empty() {
                // Check if this node is itself a summary — look at outgoing summarizes edges
                let outgoing: Vec<&Edge> = edges.iter()
                    .filter(|e| e.edge_type == EdgeType::Summarizes && e.source == node.id && e.superseded_by.is_none())
                    .collect();

                if outgoing.is_empty() {
                    if json {
                        println!(r#"{{"id":"{}","summaries":[],"message":"No summarizes edges found"}}"#, node.id);
                    } else {
                        println!("No summarizes edges found for: {}", node.title);
                    }
                } else {
                    // This node IS a summary — check if its targets have been updated since
                    let summary_updated = node.updated_at;
                    if json {
                        let items: Vec<String> = outgoing.iter().map(|e| {
                            let target_node = db.get_node(&e.target).ok().flatten();
                            let target_updated = target_node.as_ref().map(|n| n.updated_at).unwrap_or(0);
                            let stale = target_updated > summary_updated;
                            let target_title = target_node.map(|n| n.title).unwrap_or_else(|| e.target.clone());
                            format!(r#"{{"targetId":"{}","targetTitle":"{}","stale":{},"targetUpdated":{},"summaryUpdated":{}}}"#,
                                e.target, escape_json(&target_title), stale, target_updated, summary_updated)
                        }).collect();
                        println!(r#"{{"id":"{}","title":"{}","targets":[{}]}}"#, node.id, escape_json(&node.title), items.join(","));
                    } else {
                        println!("Summary: {} (updated {})", node.title,
                            chrono::DateTime::from_timestamp_millis(summary_updated)
                                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                                .unwrap_or_else(|| "?".to_string()));
                        for e in &outgoing {
                            let target_node = db.get_node(&e.target).ok().flatten();
                            let target_updated = target_node.as_ref().map(|n| n.updated_at).unwrap_or(0);
                            let stale = target_updated > summary_updated;
                            let target_title = target_node.map(|n| n.title).unwrap_or_else(|| e.target.clone());
                            let status = if stale { "STALE" } else { "fresh" };
                            println!("  {} {} (updated {})", status, target_title,
                                chrono::DateTime::from_timestamp_millis(target_updated)
                                    .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                                    .unwrap_or_else(|| "?".to_string()));
                        }
                    }
                }
            } else {
                // Node is summarized BY other nodes — show their freshness
                if json {
                    let items: Vec<String> = summary_edges.iter().map(|e| {
                        let summary_node = db.get_node(&e.source).ok().flatten();
                        let summary_updated = summary_node.as_ref().map(|n| n.updated_at).unwrap_or(0);
                        let stale = node.updated_at > summary_updated;
                        let summary_title = summary_node.map(|n| n.title).unwrap_or_else(|| e.source.clone());
                        format!(r#"{{"summaryId":"{}","summaryTitle":"{}","stale":{},"summaryUpdated":{},"nodeUpdated":{}}}"#,
                            e.source, escape_json(&summary_title), stale, summary_updated, node.updated_at)
                    }).collect();
                    println!(r#"{{"id":"{}","title":"{}","summaries":[{}]}}"#, node.id, escape_json(&node.title), items.join(","));
                } else {
                    println!("Node: {} (updated {})", node.title,
                        chrono::DateTime::from_timestamp_millis(node.updated_at)
                            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "?".to_string()));
                    for e in &summary_edges {
                        let summary_node = db.get_node(&e.source).ok().flatten();
                        let summary_updated = summary_node.as_ref().map(|n| n.updated_at).unwrap_or(0);
                        let stale = node.updated_at > summary_updated;
                        let summary_title = summary_node.map(|n| n.title).unwrap_or_else(|| e.source.clone());
                        let status = if stale { "STALE" } else { "fresh" };
                        println!("  {} {} (updated {})", status, summary_title,
                            chrono::DateTime::from_timestamp_millis(summary_updated)
                                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                                .unwrap_or_else(|| "?".to_string()));
                    }
                }
            }
            Ok(())
        }

        SporeCommands::Runs { cmd } => {
            handle_runs(cmd, db, json)
        }

        SporeCommands::Orchestrate { task, max_bounces, max_turns, coder_prompt, verifier_prompt, dry_run, verbose } => {
            handle_orchestrate(db, &task, max_bounces, max_turns, &coder_prompt, &verifier_prompt, dry_run, verbose).await
        }

        SporeCommands::ContextForTask { id, budget, max_hops, max_cost, edge_types, not_superseded, items_only } => {
            let source = resolve_node(db, &id)?;

            let type_list: Option<Vec<String>> = edge_types.map(|s| s.split(',').map(|t| t.trim().to_string()).collect());
            let type_refs: Option<Vec<&str>> = type_list.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect());

            let results = db.context_for_task(
                &source.id, budget, Some(max_hops), Some(max_cost),
                type_refs.as_deref(), not_superseded, items_only,
            ).map_err(|e| e.to_string())?;

            if json {
                let output = serde_json::json!({
                    "source": {
                        "id": source.id,
                        "title": source.ai_title.as_ref().unwrap_or(&source.title),
                    },
                    "budget": budget,
                    "results": results,
                    "count": results.len(),
                });
                println!("{}", serde_json::to_string(&output).unwrap_or_default());
            } else {
                let src_name = source.ai_title.as_ref().unwrap_or(&source.title);
                if results.is_empty() {
                    println!("No context nodes found for: {}", src_name);
                } else {
                    println!("Context for: {} ({})  budget={}\n",
                        src_name, &source.id[..8.min(source.id.len())], budget);
                    for r in &results {
                        let class_label = r.node_class.as_deref().map(|c| format!(" [{}]", c)).unwrap_or_default();
                        let marker = if r.is_item { "[I]" } else { "[C]" };
                        println!("  {:>2}. {} {}{} — dist={:.3} rel={:.0}% hops={}",
                            r.rank, marker, r.node_title, class_label,
                            r.distance, r.relevance * 100.0, r.hops);
                        if !r.path.is_empty() {
                            let path_str: Vec<String> = r.path.iter()
                                .map(|hop| format!("→[{}]→ {}",
                                    hop.edge.edge_type.as_str(),
                                    &hop.node_title[..hop.node_title.len().min(40)]))
                                .collect();
                            println!("      {}", path_str.join(" "));
                        }
                    }
                    println!("\n{} node(s) within budget", results.len());
                }
            }
            Ok(())
        }
    }
}

/// Parse a --since value as either an ISO date (YYYY-MM-DD) or a relative duration (e.g. 30m, 1h, 2d, 1w).
/// Returns epoch milliseconds.
fn parse_since_to_millis(s: &str) -> Result<i64, String> {
    // Try relative duration first: number + unit suffix
    let s_trimmed = s.trim();
    if let Some((num_str, unit)) = s_trimmed
        .strip_suffix('m')
        .map(|n| (n, 'm'))
        .or_else(|| s_trimmed.strip_suffix('h').map(|n| (n, 'h')))
        .or_else(|| s_trimmed.strip_suffix('d').map(|n| (n, 'd')))
        .or_else(|| s_trimmed.strip_suffix('w').map(|n| (n, 'w')))
    {
        if let Ok(num) = num_str.parse::<u64>() {
            let seconds = match unit {
                'm' => num * 60,
                'h' => num * 3600,
                'd' => num * 86400,
                'w' => num * 604800,
                _ => unreachable!(),
            };
            let now = chrono::Utc::now().timestamp_millis();
            return Ok(now - (seconds as i64 * 1000));
        }
    }
    // Fall back to ISO date
    let date = chrono::NaiveDate::parse_from_str(s_trimmed, "%Y-%m-%d")
        .map_err(|e| format!("Invalid --since '{}': {}. Use YYYY-MM-DD or relative (1h, 2d, 1w).", s, e))?;
    let dt = date.and_hms_opt(0, 0, 0).unwrap();
    Ok(dt.and_utc().timestamp_millis())
}

fn handle_runs(cmd: RunCommands, db: &Database, json: bool) -> Result<(), String> {
    match cmd {
        RunCommands::List { agent, since, limit } => {
            let since_millis = since.as_deref()
                .map(parse_since_to_millis)
                .transpose()?;
            let runs = db.list_runs(agent.as_deref(), since_millis, limit)
                .map_err(|e| format!("Failed to list runs: {}", e))?;

            if json {
                println!("{}", serde_json::to_string_pretty(&runs).unwrap_or_default());
            } else if runs.is_empty() {
                println!("No runs found.");
            } else {
                println!("{:<38} {:>5}  {:<20}  {}", "RUN ID", "EDGES", "STARTED", "AGENTS");
                println!("{}", "-".repeat(90));
                for r in &runs {
                    let started = chrono::DateTime::from_timestamp_millis(r.started_at)
                        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "?".to_string());
                    println!("{:<38} {:>5}  {:<20}  {}", r.run_id, r.edge_count, started, r.agents);
                }
                println!("\n{} run(s)", runs.len());
            }
            Ok(())
        }

        RunCommands::Get { run_id, json: local_json } => {
            let edges = db.get_run_edges(&run_id)
                .map_err(|e| format!("Failed to get run edges: {}", e))?;

            if json || local_json {
                println!("{}", serde_json::to_string_pretty(&edges).unwrap_or_default());
            } else if edges.is_empty() {
                println!("No edges found for run: {}", run_id);
            } else {
                println!("Run: {}", run_id);
                println!("{} edge(s):\n", edges.len());
                for e in &edges {
                    let created = chrono::DateTime::from_timestamp_millis(e.created_at)
                        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "?".to_string());
                    println!("  {} {:?} {} -> {} [{}]",
                        &e.id[..8.min(e.id.len())],
                        e.edge_type,
                        &e.source[..8.min(e.source.len())],
                        &e.target[..8.min(e.target.len())],
                        created,
                    );
                    if let Some(ref reason) = e.reason {
                        println!("    reason: {}", reason);
                    }
                }
            }
            Ok(())
        }

        RunCommands::Rollback { run_id, delete_nodes, force, dry_run } => {
            if dry_run {
                let (edges, nodes) = db.preview_rollback_run(&run_id, delete_nodes)
                    .map_err(|e| format!("Failed to preview rollback: {}", e))?;

                if json {
                    let obj = serde_json::json!({
                        "runId": run_id,
                        "dryRun": true,
                        "edgesCount": edges.len(),
                        "nodesCount": nodes.len(),
                        "edges": edges,
                        "nodes": nodes.iter().map(|n| serde_json::json!({
                            "id": n.id,
                            "title": n.title,
                            "nodeClass": n.node_class,
                            "agentId": n.agent_id,
                        })).collect::<Vec<_>>(),
                    });
                    println!("{}", serde_json::to_string_pretty(&obj).unwrap_or_default());
                } else if edges.is_empty() {
                    println!("No edges found for run: {}", run_id);
                } else {
                    println!("[dry-run] Run: {}", run_id);
                    println!("\nEdges that would be deleted ({}):", edges.len());
                    for e in &edges {
                        let created = chrono::DateTime::from_timestamp_millis(e.created_at)
                            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or_else(|| "?".to_string());
                        println!("  {} {:?} {} -> {} [{}]",
                            &e.id[..8.min(e.id.len())],
                            e.edge_type,
                            &e.source[..8.min(e.source.len())],
                            &e.target[..8.min(e.target.len())],
                            created,
                        );
                        if let Some(ref reason) = e.reason {
                            println!("    reason: {}", reason);
                        }
                    }
                    if delete_nodes {
                        if nodes.is_empty() {
                            println!("\nNo operational nodes would be deleted.");
                        } else {
                            println!("\nNodes that would be deleted ({}):", nodes.len());
                            for n in &nodes {
                                println!("  {} {}",
                                    &n.id[..8.min(n.id.len())],
                                    n.title,
                                );
                                if let Some(ref agent) = n.agent_id {
                                    println!("    agent: {}", agent);
                                }
                            }
                        }
                    }
                    println!("\nNo changes made. Remove --dry-run and use --force to execute.");
                }
                return Ok(());
            }

            // Show what will be deleted first
            let edges = db.get_run_edges(&run_id)
                .map_err(|e| format!("Failed to get run edges: {}", e))?;
            if edges.is_empty() {
                println!("No edges found for run: {}", run_id);
                return Ok(());
            }

            if !force {
                println!("Will delete {} edge(s) from run {}", edges.len(), run_id);
                if delete_nodes {
                    println!("Will also delete operational nodes created during this run.");
                }
                println!("Use --force to confirm, or --dry-run to preview details.");
                return Ok(());
            }

            let (edges_deleted, nodes_deleted) = db.rollback_run(&run_id, delete_nodes)
                .map_err(|e| format!("Rollback failed: {}", e))?;

            if json {
                println!(r#"{{"runId":"{}","edgesDeleted":{},"nodesDeleted":{}}}"#,
                    run_id, edges_deleted, nodes_deleted);
            } else {
                println!("Rolled back run {}: {} edge(s) deleted, {} node(s) deleted",
                    run_id, edges_deleted, nodes_deleted);
            }
            Ok(())
        }
    }
}

// ============================================================================
// Orchestrator
// ============================================================================

fn make_orchestrator_node(
    id: String,
    title: String,
    content: String,
    node_class: &str,
    meta_type: Option<&str>,
) -> Node {
    let now = chrono::Utc::now().timestamp_millis();
    Node {
        id,
        node_type: NodeType::Thought,
        title,
        url: None,
        content: Some(content),
        position: Position { x: 0.0, y: 0.0 },
        created_at: now,
        updated_at: now,
        cluster_id: None,
        cluster_label: None,
        ai_title: None,
        summary: None,
        tags: None,
        emoji: None,
        is_processed: true,
        depth: 0,
        is_item: true,
        is_universe: false,
        parent_id: None,
        child_count: 0,
        conversation_id: None,
        sequence_index: None,
        is_pinned: false,
        last_accessed_at: None,
        latest_child_date: None,
        is_private: None,
        privacy_reason: None,
        source: Some("orchestrator".to_string()),
        pdf_available: None,
        content_type: None,
        associated_idea_id: None,
        privacy: None,
        human_edited: None,
        human_created: false,
        author: Some("orchestrator".to_string()),
        agent_id: Some("spore:orchestrator".to_string()),
        node_class: Some(node_class.to_string()),
        meta_type: meta_type.map(|s| s.to_string()),
    }
}

/// Resolve the full path to the mycelica-cli binary (for MCP configs).
fn resolve_cli_binary() -> Result<PathBuf, String> {
    // We ARE mycelica-cli, so current_exe gives the full path
    std::env::current_exe()
        .map_err(|e| format!("Failed to resolve CLI binary path: {}", e))
}

struct ClaudeResult {
    success: bool,
    exit_code: i32,
    session_id: Option<String>,
    result_text: Option<String>,
    total_cost_usd: Option<f64>,
    num_turns: Option<u32>,
    duration_ms: Option<u64>,
    stdout_raw: String,
    stderr_raw: String,
}

/// Spawn Claude Code as a subprocess with streaming output.
/// Reads stdout line-by-line (stream-json format) and prints real-time progress.
fn spawn_claude(
    prompt: &str,
    mcp_config: &Path,
    max_turns: usize,
    verbose: bool,
    role: &str,
    allowed_tools: Option<&str>,
    disallowed_tools: Option<&str>,
) -> Result<ClaudeResult, String> {
    use std::process::{Command, Stdio};

    let mut cmd = Command::new("claude");
    cmd.arg("-p")
        .arg(prompt)
        .arg("--mcp-config")
        .arg(mcp_config)
        .arg("--strict-mcp-config")
        .arg("--dangerously-skip-permissions")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--max-turns")
        .arg(max_turns.to_string());

    if let Some(tools) = allowed_tools {
        cmd.arg("--allowedTools").arg(tools);
    }
    if let Some(tools) = disallowed_tools {
        cmd.arg("--disallowedTools").arg(tools);
    }

    // Clear CLAUDECODE env var so the child process doesn't refuse to start
    // when the orchestrator itself is running inside a Claude Code session.
    cmd.env_remove("CLAUDECODE");

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn claude: {}", e))?;

    let stdout = child.stdout.take()
        .ok_or_else(|| "Failed to capture stdout".to_string())?;
    let reader = std::io::BufReader::new(stdout);

    let mut session_id: Option<String> = None;
    let mut result_text: Option<String> = None;
    let mut total_cost_usd: Option<f64> = None;
    let mut num_turns: Option<u32> = None;
    let mut duration_ms: Option<u64> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }

        let json: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match json.get("type").and_then(|t| t.as_str()) {
            Some("system") => {
                eprintln!("[{}] Connected", role);
                // Check MCP server status
                if let Some(servers) = json.get("mcp_servers").and_then(|s| s.as_array()) {
                    for srv in servers {
                        let name = srv.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                        let status = srv.get("status").and_then(|s| s.as_str()).unwrap_or("?");
                        eprintln!("[{}] MCP: {} ({})", role, name, status);
                    }
                }
            }
            Some("assistant") => {
                if let Some(content) = json.pointer("/message/content").and_then(|c| c.as_array()) {
                    for block in content {
                        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match block_type {
                            "text" => {
                                if verbose {
                                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                        let text = text.trim();
                                        if !text.is_empty() {
                                            let truncated = if text.len() > 120 {
                                                format!("{}...", &text[..text.floor_char_boundary(120)])
                                            } else {
                                                text.to_string()
                                            };
                                            eprintln!("[{}] {}", role, truncated);
                                        }
                                    }
                                }
                            }
                            "tool_use" => {
                                let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                                let input = block.get("input");
                                let summary = match name {
                                    "Bash" => {
                                        let cmd = input
                                            .and_then(|i| i.get("command"))
                                            .and_then(|c| c.as_str())
                                            .unwrap_or("?");
                                        let cmd = if cmd.len() > 120 {
                                            format!("{}...", &cmd[..cmd.floor_char_boundary(120)])
                                        } else {
                                            cmd.to_string()
                                        };
                                        format!("$ {}", cmd)
                                    }
                                    n if n.starts_with("mcp__") => {
                                        let tool_name = n.rsplit("__").next().unwrap_or(n);
                                        format!("mcp: {}", tool_name)
                                    }
                                    "Read" | "Edit" | "Write" => {
                                        let path = input
                                            .and_then(|i| i.get("file_path"))
                                            .and_then(|p| p.as_str())
                                            .unwrap_or("?");
                                        format!("{}: {}", name, path)
                                    }
                                    _ => format!("tool: {}", name),
                                };
                                eprintln!("[{}] {}", role, summary);
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some("result") => {
                session_id = json.get("session_id").and_then(|v| v.as_str()).map(|s| s.to_string());
                result_text = json.get("result").and_then(|v| v.as_str()).map(|s| s.to_string());
                total_cost_usd = json.get("total_cost_usd").and_then(|v| v.as_f64());
                num_turns = json.get("num_turns").and_then(|v| v.as_u64()).map(|n| n as u32);
                duration_ms = json.get("duration_ms").and_then(|v| v.as_u64());
            }
            _ => {}
        }
    }

    // Read stderr after stdout is drained
    let mut stderr_buf = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut stderr_buf);
    }

    let status = child.wait()
        .map_err(|e| format!("Failed to wait on claude: {}", e))?;
    let exit_code = status.code().unwrap_or(-1);

    Ok(ClaudeResult {
        success: status.success(),
        exit_code,
        session_id,
        result_text,
        total_cost_usd,
        num_turns,
        duration_ms,
        stdout_raw: String::new(),
        stderr_raw: stderr_buf,
    })
}

/// Write a temporary MCP config file for an agent run.
fn write_temp_mcp_config(
    cli_binary: &Path,
    role: &str,
    agent_id: &str,
    run_id: &str,
    db_path: &str,
) -> Result<PathBuf, String> {
    let dir = PathBuf::from("/tmp/mycelica-orchestrator");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;

    let filename = format!("mcp-{}-{}.json", role, &run_id[..8.min(run_id.len())]);
    let path = dir.join(filename);

    let binary_str = cli_binary.to_string_lossy();
    let args = vec![
        "mcp-server".to_string(),
        "--stdio".to_string(),
        "--agent-role".to_string(),
        role.to_string(),
        "--agent-id".to_string(),
        agent_id.to_string(),
        "--run-id".to_string(),
        run_id.to_string(),
        "--db".to_string(),
        db_path.to_string(),
    ];

    let config = serde_json::json!({
        "mcpServers": {
            "mycelica": {
                "command": binary_str,
                "args": args,
            }
        }
    });

    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap())
        .map_err(|e| format!("Failed to write MCP config: {}", e))?;
    Ok(path)
}

#[derive(Debug, PartialEq)]
enum Verdict {
    Supports,
    Contradicts,
    Unknown,
}

/// Check whether the verifier supports or contradicts the implementation node.
fn check_verdict(db: &Database, impl_node_id: &str) -> Verdict {
    let edges = match db.get_edges_for_node(impl_node_id) {
        Ok(e) => e,
        Err(_) => return Verdict::Unknown,
    };

    for edge in &edges {
        // Only incoming edges targeting the impl node
        if edge.target != impl_node_id {
            continue;
        }
        // Only from verifier
        if edge.agent_id.as_deref() != Some("spore:verifier") {
            continue;
        }
        // Only non-superseded
        if edge.superseded_by.is_some() {
            continue;
        }
        match edge.edge_type {
            EdgeType::Supports => return Verdict::Supports,
            EdgeType::Contradicts => return Verdict::Contradicts,
            _ => {}
        }
    }
    Verdict::Unknown
}

/// Post-coder cleanup: re-index changed files, reinstall CLI if needed, create related edges.
/// Failures warn but do NOT abort orchestration.
fn post_coder_cleanup(
    db: &Database,
    impl_node_id: &str,
    before_dirty: &HashSet<String>,
    before_untracked: &HashSet<String>,
    cli_binary: &Path,
    verbose: bool,
) {
    // 1. Get files dirty/untracked NOW, subtract pre-existing ones
    let after_dirty: HashSet<String> = std::process::Command::new("git")
        .args(["diff", "--name-only"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect())
        .unwrap_or_default();

    let after_untracked: HashSet<String> = std::process::Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect())
        .unwrap_or_default();

    let changed_files: Vec<String> = after_dirty.difference(before_dirty)
        .chain(after_untracked.difference(before_untracked))
        .cloned()
        .collect();

    if changed_files.is_empty() {
        if verbose {
            eprintln!("[orchestrator] No files changed by coder");
        }
        return;
    }

    println!("[orchestrator] {} file(s) changed by coder", changed_files.len());

    // 2. Re-index changed .rs files
    let rs_files: Vec<&str> = changed_files.iter()
        .filter(|f| f.ends_with(".rs"))
        .map(|f| f.as_str())
        .collect();

    for file in &rs_files {
        println!("[orchestrator] Indexing: {}", file);
        let result = std::process::Command::new(cli_binary)
            .args(["import", "code", file, "--update"])
            .output();
        match result {
            Ok(o) if !o.status.success() => {
                eprintln!("[orchestrator] WARNING: Failed to index {}", file);
            }
            Err(e) => {
                eprintln!("[orchestrator] WARNING: Failed to run import for {}: {}", file, e);
            }
            _ => {}
        }
    }

    // 3. Reinstall CLI if src-tauri/ files changed
    let needs_reinstall = changed_files.iter().any(|f| f.starts_with("src-tauri/"));
    if needs_reinstall {
        println!("[orchestrator] Reinstalling CLI (source files changed)");
        let install_result = std::process::Command::new("cargo")
            .args(["+nightly", "install", "--path", "src-tauri", "--bin", "mycelica-cli", "--features", "mcp", "--force"])
            .output();
        match install_result {
            Ok(o) if o.status.success() => {
                // Copy sidecar
                let home = std::env::var("HOME").unwrap_or_else(|_| "/home/ekats".to_string());
                let src = PathBuf::from(&home).join(".cargo/bin/mycelica-cli");
                let dst = PathBuf::from("binaries/mycelica-cli-x86_64-unknown-linux-gnu");
                if let Err(e) = std::fs::copy(&src, &dst) {
                    eprintln!("[orchestrator] WARNING: Failed to copy sidecar: {}", e);
                } else {
                    println!("[orchestrator] CLI reinstalled and sidecar updated");
                }
            }
            Ok(o) => {
                eprintln!("[orchestrator] WARNING: cargo install failed (exit {})", o.status.code().unwrap_or(-1));
            }
            Err(e) => {
                eprintln!("[orchestrator] WARNING: Failed to run cargo install: {}", e);
            }
        }
    }

    // 4. Create related edges from impl node to code nodes matching changed files
    let mut linked = 0usize;
    for file in &changed_files {
        // Find code nodes with this file_path in tags JSON
        let node_ids: Vec<String> = (|| -> Option<Vec<String>> {
            let conn = db.raw_conn().lock().ok()?;
            let mut stmt = conn.prepare(
                "SELECT id FROM nodes WHERE JSON_EXTRACT(tags, '$.file_path') = ?1 LIMIT 10"
            ).ok()?;
            let rows = stmt.query_map([file.as_str()], |row| row.get(0)).ok()?;
            Some(rows.filter_map(|r| r.ok()).collect())
        })().unwrap_or_default();

        let now = chrono::Utc::now().timestamp_millis();
        for node_id in &node_ids {
            let edge = Edge {
                id: uuid::Uuid::new_v4().to_string(),
                source: impl_node_id.to_string(),
                target: node_id.clone(),
                edge_type: EdgeType::Related,
                label: None,
                weight: None,
                edge_source: Some("orchestrator".to_string()),
                evidence_id: None,
                confidence: Some(0.85),
                created_at: now,
                updated_at: Some(now),
                author: None,
                reason: None,
                content: Some("Implementation modifies this code".to_string()),
                agent_id: Some("spore:orchestrator".to_string()),
                superseded_by: None,
                metadata: None,
            };
            if db.insert_edge(&edge).is_ok() {
                linked += 1;
            }
        }
    }

    if linked > 0 {
        println!("[orchestrator] Linked impl node to {} code node(s)", linked);
    }
}

/// Generate a task file with graph context for an agent before spawning.
///
/// Produces a markdown file at `docs/spore/tasks/task-<run_id>.md` that serves as
/// both bootstrap context for the spawned agent and an audit trail for the run.
///
/// # Generated Sections
///
/// - **Header** — Run metadata: truncated task title, run ID, agent role, bounce
///   number (current/max), and UTC timestamp.
/// - **Task** — The full, untruncated task description as provided by the caller.
/// - **Previous Bounce** (conditional) — Present only when `last_impl_id` is set,
///   meaning the verifier rejected a prior implementation. Points the agent at the
///   failed node so it can read the incoming `contradicts` edges and fix the issues.
/// - **Graph Context** — A relevance-ranked table of knowledge-graph nodes related
///   to the task. Each row shows the node title, short ID, relevance score, the
///   anchor it was reached from, and the edge-type path taken to reach it.
/// - **Checklist** — Static reminders for the agent: read context first, create an
///   operational node when done, and link it to modified code nodes.
///
/// # Context Gathering: Semantic Search + Dijkstra
///
/// 1. **Semantic anchor search** — The task description is embedded using the local
///    all-MiniLM-L6-v2 model and compared against all stored node embeddings via
///    cosine similarity. This captures meaning ("format flag" ≈ SporeCommands) rather
///    than requiring keyword overlap. Falls back to FTS5 with OR-joined tokens if
///    embedding generation fails (model not downloaded, etc.).
///    The top 3 non-operational matches become anchor nodes.
/// 2. **Dijkstra expansion** — For each anchor, [`Database::context_for_task`] runs
///    a weighted shortest-path traversal (max 4 hops, cost ceiling 2.0) that
///    follows semantic edges (supports, contradicts, derives_from, etc.) while
///    skipping structural edges (defined_in, belongs_to, sibling). Edge confidence
///    and type priority determine traversal weights, so high-confidence semantic
///    edges are explored first.
/// 3. **Dedup and rank** — Nodes discovered from multiple anchors are deduplicated,
///    keeping the highest relevance score. The final list is sorted by descending
///    relevance and rendered into the Graph Context table.
fn generate_task_file(
    db: &Database,
    task: &str,
    role: &str,
    run_id: &str,
    _task_node_id: &str,
    bounce: usize,
    max_bounces: usize,
    last_impl_id: Option<&str>,
) -> Result<PathBuf, String> {
    use std::collections::HashMap;

    // 1. Find anchor nodes via semantic search (embedding similarity).
    //    Natural language task descriptions like "Add a format flag to spore status"
    //    work poorly with FTS5 (common words match everything or nothing useful).
    //    Embedding similarity captures meaning: "format flag" is semantically close
    //    to the SporeCommands enum and handle_spore function even without keyword overlap.
    //    Falls back to FTS5 if embedding generation fails (model not downloaded, etc.).
    let anchors: Vec<Node> = {
        let semantic_anchors = (|| -> Result<Vec<Node>, String> {
            let query_embedding = local_embeddings::generate(task)?;
            let all_embeddings = db.get_nodes_with_embeddings()
                .map_err(|e| e.to_string())?;
            let similar = similarity::find_similar(
                &query_embedding, &all_embeddings, _task_node_id, 10, 0.3,
            );
            // Resolve node IDs to full nodes, filtering operational
            let mut result = Vec::new();
            for (node_id, _score) in similar {
                if let Ok(Some(node)) = db.get_node(&node_id) {
                    if node.node_class.as_deref() != Some("operational") {
                        result.push(node);
                        if result.len() >= 3 {
                            break;
                        }
                    }
                }
            }
            Ok(result)
        })();

        match semantic_anchors {
            Ok(anchors) if !anchors.is_empty() => {
                println!("[task-file] Semantic search found {} anchor(s)", anchors.len());
                anchors
            }
            _ => {
                // Fallback: FTS5 with OR logic
                println!("[task-file] Falling back to FTS search");
                let stopwords = ["the", "a", "an", "in", "on", "at", "to", "for", "of", "is", "it",
                                 "and", "or", "with", "from", "by", "this", "that", "as", "be"];
                let fts_query: String = task.split_whitespace()
                    .filter(|w| w.len() > 2 && !stopwords.contains(&w.to_lowercase().as_str()))
                    .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
                    .filter(|w| !w.is_empty())
                    .collect::<Vec<_>>()
                    .join(" OR ");
                if fts_query.is_empty() {
                    Vec::new()
                } else {
                    db.search_nodes(&fts_query)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|n| n.id != _task_node_id)
                        .filter(|n| n.node_class.as_deref() != Some("operational"))
                        .take(3)
                        .collect()
                }
            }
        }
    };

    // 2. Gather context via Dijkstra from each anchor
    let mut seen: HashMap<String, (f64, String, String, String)> = HashMap::new(); // id -> (relevance, title, anchor_title, via)

    for anchor in &anchors {
        let anchor_title = anchor.ai_title.as_deref().unwrap_or(&anchor.title);
        let context = db.context_for_task(
            &anchor.id, 7, Some(4), Some(2.0), None, true, true,
        ).unwrap_or_default();

        for node in &context {
            // Skip operational nodes (other orchestrator artifacts)
            if node.node_class.as_deref() == Some("operational") {
                continue;
            }
            let via = if node.path.is_empty() {
                "direct".to_string()
            } else {
                node.path.iter()
                    .map(|hop| hop.edge.edge_type.as_str())
                    .collect::<Vec<_>>()
                    .join(" → ")
            };
            let key = node.node_id.clone();
            if !seen.contains_key(&key) || node.relevance > seen[&key].0 {
                seen.insert(key, (
                    node.relevance,
                    node.node_title.clone(),
                    anchor_title.to_string(),
                    via,
                ));
            }
        }

        // Also include the anchor itself
        if !seen.contains_key(&anchor.id) {
            seen.insert(anchor.id.clone(), (
                1.0,
                anchor_title.to_string(),
                "search".to_string(),
                "FTS match".to_string(),
            ));
        }
    }

    // 3. Sort by relevance, filter out the task node itself
    let mut context_rows: Vec<_> = seen.into_iter()
        .filter(|(id, _)| id != _task_node_id)
        .collect();
    context_rows.sort_by(|a, b| b.1.0.partial_cmp(&a.1.0).unwrap_or(std::cmp::Ordering::Equal));

    // 4. Format markdown
    let now = chrono::Utc::now();
    let task_short = if task.len() > 60 { &task[..60] } else { task };

    let mut md = String::new();
    md.push_str(&format!("# Task: {}\n\n", task_short));
    md.push_str(&format!("- **Run:** {}\n", &run_id[..8.min(run_id.len())]));
    md.push_str(&format!("- **Agent:** {}\n", role));
    md.push_str(&format!("- **Bounce:** {}/{}\n", bounce + 1, max_bounces));
    md.push_str(&format!("- **Generated:** {}\n\n", now.format("%Y-%m-%d %H:%M:%S UTC")));

    md.push_str("## Task\n\n");
    md.push_str(task);
    md.push_str("\n\n");

    if let Some(impl_id) = last_impl_id {
        md.push_str("## Previous Bounce\n\n");
        md.push_str(&format!(
            "Verifier found issues with node `{}`. Check its incoming `contradicts` edges and fix the code.\n\n",
            impl_id
        ));
    }

    md.push_str("## Graph Context\n\n");
    md.push_str("Relevant nodes found by search + Dijkstra traversal from the task description.\n");
    md.push_str("Use `mycelica_node_get` or `mycelica_read_content` to read full content of any node.\n\n");

    if context_rows.is_empty() {
        md.push_str("_No relevant nodes found in the graph._\n\n");
    } else {
        md.push_str("| # | Node | ID | Relevance | Via |\n");
        md.push_str("|---|------|----|-----------|-----|\n");
        for (i, (id, (rel, title, anchor, via))) in context_rows.iter().enumerate() {
            let title_short = if title.len() > 50 { &title[..50] } else { title.as_str() };
            let id_short = &id[..12.min(id.len())];
            md.push_str(&format!(
                "| {} | {} | `{}` | {:.0}% | {} → {} |\n",
                i + 1, title_short, id_short, rel * 100.0, anchor, via
            ));
        }
        md.push_str("\n");
    }

    md.push_str("## Checklist\n\n");
    md.push_str("- [ ] Read relevant context nodes above before starting\n");
    md.push_str("- [ ] Record implementation as operational node when done\n");
    md.push_str("- [ ] Link implementation to modified code nodes with edges\n");

    // 5. Write to disk
    let tasks_dir = PathBuf::from("docs/spore/tasks");
    std::fs::create_dir_all(&tasks_dir)
        .map_err(|e| format!("Failed to create tasks dir: {}", e))?;

    let filename = format!("task-{}.md", &run_id[..8.min(run_id.len())]);
    let path = tasks_dir.join(&filename);
    std::fs::write(&path, &md)
        .map_err(|e| format!("Failed to write task file: {}", e))?;

    Ok(path)
}

async fn handle_orchestrate(
    db: &Database,
    task: &str,
    max_bounces: usize,
    max_turns: usize,
    coder_prompt_path: &Path,
    verifier_prompt_path: &Path,
    dry_run: bool,
    verbose: bool,
) -> Result<(), String> {
    // Resolve CLI binary path
    let cli_binary = resolve_cli_binary()?;
    if verbose {
        eprintln!("[orchestrator] CLI binary: {}", cli_binary.display());
    }

    // Fail fast if claude is not available
    let which_result = std::process::Command::new("which")
        .arg("claude")
        .output()
        .map_err(|e| format!("Failed to check for claude: {}", e))?;
    if !which_result.status.success() {
        return Err("'claude' not found in PATH. Install Claude Code first.".to_string());
    }

    // Resolve DB path early — its parent is the repo root, used for prompt path fallback
    let db_path_str = db.get_path();
    let repo_root = Path::new(&db_path_str).parent();

    // Resolve prompt paths: try CWD first, then relative to repo root (DB parent dir)
    let resolve_prompt = |p: &Path| -> PathBuf {
        if p.exists() {
            return p.to_path_buf();
        }
        if let Some(root) = repo_root {
            let resolved = root.join(p);
            if resolved.exists() {
                return resolved;
            }
        }
        p.to_path_buf()
    };
    let coder_path = resolve_prompt(coder_prompt_path);
    let verifier_path = resolve_prompt(verifier_prompt_path);

    // Read agent prompts
    let coder_prompt_template = std::fs::read_to_string(&coder_path)
        .map_err(|e| format!("Failed to read coder prompt at {}: {}", coder_path.display(), e))?;
    let verifier_prompt_template = std::fs::read_to_string(&verifier_path)
        .map_err(|e| format!("Failed to read verifier prompt at {}: {}", verifier_path.display(), e))?;

    // Clean up old temp MCP configs from previous runs
    let _ = std::fs::remove_dir_all("/tmp/mycelica-orchestrator");
    let db_path = std::fs::canonicalize(&db_path_str)
        .map_err(|e| format!("Failed to resolve absolute DB path '{}': {}", db_path_str, e))?
        .to_string_lossy()
        .to_string();

    if dry_run {
        println!("=== DRY RUN ===");
        println!("Task: {}", task);
        println!("Max bounces: {}", max_bounces);
        println!("Max turns per agent: {}", max_turns);
        println!("CLI binary: {}", cli_binary.display());
        println!("Coder prompt: {} ({} bytes)", coder_prompt_path.display(), coder_prompt_template.len());
        println!("Verifier prompt: {} ({} bytes)", verifier_prompt_path.display(), verifier_prompt_template.len());
        println!("DB path: {}", db_path);
        println!("\nWould run {} bounce(s) of: Coder -> Verifier", max_bounces);

        // Generate a preview task file to inspect context quality
        let preview_run_id = "dry-run-preview-00000000";
        let preview_task_id = "00000000-0000-0000-0000-000000000000";
        match generate_task_file(db, task, "coder", preview_run_id, preview_task_id, 0, max_bounces, None) {
            Ok(path) => println!("\nPreview task file: {}", path.display()),
            Err(e) => println!("\nTask file preview failed: {}", e),
        }
        return Ok(());
    }

    // Create task node in graph
    let task_node_id = uuid::Uuid::new_v4().to_string();
    let task_node = make_orchestrator_node(
        task_node_id.clone(),
        format!("Orchestration: {}", if task.len() > 60 { &task[..60] } else { task }),
        format!("## Task\n{}\n\n## Config\n- max_bounces: {}\n- max_turns: {}", task, max_bounces, max_turns),
        "operational",
        None,
    );
    db.insert_node(&task_node).map_err(|e| format!("Failed to create task node: {}", e))?;
    println!("Created task node: {}", &task_node_id[..8]);

    let mut last_impl_id: Option<String> = None;

    for bounce in 0..max_bounces {
        println!("\n--- Bounce {}/{} ---", bounce + 1, max_bounces);

        // === CODER PHASE ===
        let coder_run_id = uuid::Uuid::new_v4().to_string();

        // Capture dirty + untracked files before coder runs (to diff against after)
        let before_dirty: HashSet<String> = std::process::Command::new("git")
            .args(["diff", "--name-only"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.to_string())
                .collect())
            .unwrap_or_default();

        let before_untracked: HashSet<String> = std::process::Command::new("git")
            .args(["ls-files", "--others", "--exclude-standard"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.to_string())
                .collect())
            .unwrap_or_default();

        let coder_start = chrono::Utc::now().timestamp_millis();

        let coder_mcp = write_temp_mcp_config(
            &cli_binary, "coder", "spore:coder", &coder_run_id, &db_path,
        )?;

        // Generate task file with graph context
        let task_file = generate_task_file(
            db, task, "coder", &coder_run_id, &task_node_id,
            bounce, max_bounces, last_impl_id.as_deref(),
        )?;
        println!("[coder] Task file: {}", task_file.display());

        let coder_prompt = if let Some(ref impl_id) = last_impl_id {
            format!(
                "{}\n\nRead the task file at {} for full context and graph-gathered information.\n\nThe Verifier found issues with node {}. Check its incoming contradicts edges and fix the code.\n\nYour task: {}",
                coder_prompt_template, task_file.display(), impl_id, task
            )
        } else {
            format!(
                "{}\n\nRead the task file at {} for full context and graph-gathered information.\n\nYour task: {}",
                coder_prompt_template, task_file.display(), task
            )
        };

        println!("[coder] Starting (run: {})", &coder_run_id[..8]);
        let coder_result = spawn_claude(
            &coder_prompt, &coder_mcp, max_turns, verbose, "coder",
            Some("Read,Write,Edit,Bash(*),mcp__mycelica__*"),
            Some("Grep,Glob"),
        )?;

        if !coder_result.success {
            eprintln!("[coder] FAILED (exit code {})", coder_result.exit_code);
            if !coder_result.stderr_raw.is_empty() {
                eprintln!("[coder stderr] {}", &coder_result.stderr_raw[..2000.min(coder_result.stderr_raw.len())]);
            }
            // Record failure
            record_run_status(db, &task_node_id, &coder_run_id, "spore:coder", "failed", coder_result.exit_code)?;
            return Err(format!("Coder failed on bounce {} with exit code {}", bounce + 1, coder_result.exit_code));
        }
        println!("[coder] Done ({} turns, ${:.2}, {:.0}s)",
            coder_result.num_turns.unwrap_or(0),
            coder_result.total_cost_usd.unwrap_or(0.0),
            coder_result.duration_ms.unwrap_or(0) as f64 / 1000.0,
        );
        if let Some(ref sid) = coder_result.session_id {
            println!("[coder] Session: {}", sid);
        }

        // Find coder's output node
        let coder_nodes = db.find_nodes_by_agent_and_time("spore:coder", coder_start)
            .map_err(|e| format!("Failed to query coder nodes: {}", e))?;

        if coder_nodes.is_empty() {
            eprintln!("[coder] WARNING: Coder completed (exit 0) but created no operational nodes.");
            if let Some(ref text) = coder_result.result_text {
                eprintln!("[coder] Agent result: {}", text);
            }
            eprintln!("[coder] This may mean the feature already exists or the agent skipped graph recording.");
            record_run_status(db, &task_node_id, &coder_run_id, "spore:coder", "incomplete", 0)?;
            return Err("Coder produced no operational nodes — see agent result above for details.".to_string());
        }

        let impl_node = &coder_nodes[0]; // Most recent
        println!("[coder] Implementation node: {} ({})", &impl_node.id[..8], impl_node.title);
        record_run_status(db, &task_node_id, &coder_run_id, "spore:coder", "completed", 0)?;

        // === POST-CODER CLEANUP ===
        post_coder_cleanup(db, &impl_node.id, &before_dirty, &before_untracked, &cli_binary, verbose);

        // === VERIFIER PHASE ===
        let verifier_run_id = uuid::Uuid::new_v4().to_string();

        let verifier_mcp = write_temp_mcp_config(
            &cli_binary, "verifier", "spore:verifier", &verifier_run_id, &db_path,
        )?;

        // Generate verifier task file with graph context
        let verifier_task_file = generate_task_file(
            db, task, "verifier", &verifier_run_id, &task_node_id,
            bounce, max_bounces, Some(&impl_node.id),
        )?;
        println!("[verifier] Task file: {}", verifier_task_file.display());

        let verifier_prompt = format!(
            "{}\n\nRead the task file at {} for full context.\n\nCheck implementation node {}",
            verifier_prompt_template, verifier_task_file.display(), impl_node.id
        );

        println!("[verifier] Starting (run: {})", &verifier_run_id[..8]);
        let verifier_result = spawn_claude(
            &verifier_prompt, &verifier_mcp, max_turns, verbose, "verifier",
            Some("Read,Grep,Glob,Bash(cargo:*),Bash(cd:*),Bash(mycelica-cli:*),mcp__mycelica__*"),
            None,
        )?;

        if !verifier_result.success {
            eprintln!("[verifier] FAILED (exit code {})", verifier_result.exit_code);
            if !verifier_result.stderr_raw.is_empty() {
                eprintln!("[verifier stderr] {}", &verifier_result.stderr_raw[..2000.min(verifier_result.stderr_raw.len())]);
            }
            record_run_status(db, &task_node_id, &verifier_run_id, "spore:verifier", "failed", verifier_result.exit_code)?;
            return Err(format!("Verifier failed on bounce {} with exit code {}", bounce + 1, verifier_result.exit_code));
        }
        println!("[verifier] Done ({} turns, ${:.2}, {:.0}s)",
            verifier_result.num_turns.unwrap_or(0),
            verifier_result.total_cost_usd.unwrap_or(0.0),
            verifier_result.duration_ms.unwrap_or(0) as f64 / 1000.0,
        );
        record_run_status(db, &task_node_id, &verifier_run_id, "spore:verifier", "completed", 0)?;

        // Check verdict
        let verdict = check_verdict(db, &impl_node.id);
        match verdict {
            Verdict::Supports => {
                println!("\n=== TASK COMPLETE ===");
                println!("Verifier supports the implementation after {} bounce(s).", bounce + 1);
                println!("Implementation: {} ({})", &impl_node.id[..8], impl_node.title);
                println!("Task node: {}", &task_node_id[..8]);
                return Ok(());
            }
            Verdict::Contradicts => {
                println!("[verifier] Contradicts implementation — will bounce to coder");
                last_impl_id = Some(impl_node.id.clone());
            }
            Verdict::Unknown => {
                eprintln!("[verifier] WARNING: No supports/contradicts edge found. Verifier may not have recorded a verdict.");
                last_impl_id = Some(impl_node.id.clone());
            }
        }
    }

    // Max bounces reached — escalate
    println!("\n=== MAX BOUNCES REACHED ({}) ===", max_bounces);
    if let Some(ref impl_id) = last_impl_id {
        create_escalation(db, &task_node_id, impl_id, max_bounces, task)?;
        println!("Escalation node created. Human review required.");
    }
    Err(format!("Task not resolved after {} bounce(s). Escalation created.", max_bounces))
}

/// Record a run status edge from task node to itself.
fn record_run_status(
    db: &Database,
    task_node_id: &str,
    run_id: &str,
    agent: &str,
    status: &str,
    exit_code: i32,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    let metadata = serde_json::json!({
        "run_id": run_id,
        "status": status,
        "exit_code": exit_code,
        "agent": agent,
    });
    let edge = Edge {
        id: uuid::Uuid::new_v4().to_string(),
        source: task_node_id.to_string(),
        target: task_node_id.to_string(),
        edge_type: EdgeType::Tracks,
        label: None,
        weight: None,
        edge_source: Some("orchestrator".to_string()),
        evidence_id: None,
        confidence: Some(1.0),
        created_at: now,
        updated_at: Some(now),
        author: Some("orchestrator".to_string()),
        reason: Some(format!("{} run {}", agent, status)),
        content: None,
        agent_id: Some("spore:orchestrator".to_string()),
        superseded_by: None,
        metadata: Some(metadata.to_string()),
    };
    db.insert_edge(&edge).map_err(|e| format!("Failed to record run status: {}", e))?;
    Ok(())
}

/// Create an escalation meta node after max bounces.
fn create_escalation(
    db: &Database,
    task_node_id: &str,
    last_impl_id: &str,
    bounce_count: usize,
    task: &str,
) -> Result<(), String> {
    let esc_id = uuid::Uuid::new_v4().to_string();
    let esc_node = make_orchestrator_node(
        esc_id.clone(),
        format!("ESCALATION: {} (after {} bounces)", if task.len() > 40 { &task[..40] } else { task }, bounce_count),
        format!(
            "## Escalation\n\nTask did not converge after {} Coder-Verifier bounces.\n\n\
             ### Task\n{}\n\n\
             ### Last Implementation\nNode: {}\n\n\
             ### Action Required\nHuman review needed. Check the contradicts edges on the last implementation node \
             to understand what the Verifier flagged.",
            bounce_count, task, last_impl_id
        ),
        "meta",
        Some("escalation"),
    );
    db.insert_node(&esc_node).map_err(|e| format!("Failed to create escalation node: {}", e))?;
    let now = esc_node.created_at;

    // Edge: escalation flags last implementation
    let flags_edge = Edge {
        id: uuid::Uuid::new_v4().to_string(),
        source: esc_id.clone(),
        target: last_impl_id.to_string(),
        edge_type: EdgeType::Flags,
        label: None,
        weight: None,
        edge_source: Some("orchestrator".to_string()),
        evidence_id: None,
        confidence: Some(1.0),
        created_at: now,
        updated_at: Some(now),
        author: Some("orchestrator".to_string()),
        reason: Some(format!("Escalation after {} bounces", bounce_count)),
        content: None,
        agent_id: Some("spore:orchestrator".to_string()),
        superseded_by: None,
        metadata: None,
    };
    db.insert_edge(&flags_edge).map_err(|e| format!("Failed to create flags edge: {}", e))?;

    // Edge: escalation tracks task
    let tracks_edge = Edge {
        id: uuid::Uuid::new_v4().to_string(),
        source: esc_id,
        target: task_node_id.to_string(),
        edge_type: EdgeType::Tracks,
        label: None,
        weight: None,
        edge_source: Some("orchestrator".to_string()),
        evidence_id: None,
        confidence: Some(1.0),
        created_at: now,
        updated_at: Some(now),
        author: Some("orchestrator".to_string()),
        reason: Some("Tracks orchestration task".to_string()),
        content: None,
        agent_id: Some("spore:orchestrator".to_string()),
        superseded_by: None,
        metadata: None,
    };
    db.insert_edge(&tracks_edge).map_err(|e| format!("Failed to create tracks edge: {}", e))?;

    println!("Escalation: {} ({})", &esc_node.id[..8], esc_node.title);
    Ok(())
}

async fn handle_link(
    source_ref: &str,
    target_ref: &str,
    edge_type_str: &str,
    reason: Option<String>,
    content: Option<String>,
    agent: &str,
    confidence: Option<f64>,
    supersedes: Option<String>,
    edge_source: &str,
    db: &Database,
    json: bool,
) -> Result<(), String> {
    // Parse edge type (case-insensitive)
    let edge_type = EdgeType::from_str(&edge_type_str.to_lowercase())
        .ok_or_else(|| format!(
            "Unknown edge type: '{}'. Valid types include: related, reference, because, contains, \
             belongs_to, calls, uses_type, implements, defined_in, imports, tests, documents, \
             prerequisite, contradicts, supports, evolved_from, questions, \
             summarizes, tracks, flags, resolves, derives_from, supersedes",
            edge_type_str
        ))?;

    let source = resolve_node(db, source_ref)?;
    let target = resolve_node(db, target_ref)?;

    let author = settings::get_author_or_default();
    let now = Utc::now().timestamp_millis();
    let edge_id = uuid::Uuid::new_v4().to_string();

    let edge = Edge {
        id: edge_id.clone(),
        source: source.id.clone(),
        target: target.id.clone(),
        edge_type,
        label: None,
        weight: Some(1.0),
        edge_source: Some(edge_source.to_string()),
        evidence_id: None,
        confidence: Some(confidence.unwrap_or(1.0)),
        created_at: now,
        updated_at: Some(now),
        author: Some(author),
        reason,
        content,
        agent_id: Some(agent.to_string()),
        superseded_by: None,
        metadata: None,
    };

    db.insert_edge(&edge).map_err(|e| e.to_string())?;

    // If superseding another edge, mark the old one
    if let Some(ref old_edge_id) = supersedes {
        db.supersede_edge(old_edge_id, &edge_id).map_err(|e| e.to_string())?;
    }

    if json {
        println!(r#"{{"id":"{}","source":"{}","target":"{}","type":"{}","agent":"{}","confidence":{}}}"#,
            edge_id, source.id, target.id, edge_type_str.to_lowercase(), agent, confidence.unwrap_or(1.0));
    } else {
        let conf_str = confidence.map(|c| format!(" ({:.0}%)", c * 100.0)).unwrap_or_default();
        println!("Linked: {} -> {} [{}]{} (agent: {})",
            source.ai_title.as_ref().unwrap_or(&source.title),
            target.ai_title.as_ref().unwrap_or(&target.title),
            edge_type_str.to_lowercase(),
            conf_str,
            agent);
        if let Some(ref old_id) = supersedes {
            println!("  Superseded edge: {}", &old_id[..8.min(old_id.len())]);
        }
    }

    Ok(())
}

async fn handle_orphans(limit: u32, db: &Database, json: bool) -> Result<(), String> {
    let orphans = db.get_orphan_nodes(limit as i32).map_err(|e| e.to_string())?;

    if json {
        let items: Vec<String> = orphans.iter().map(|n| {
            format!(r#"{{"id":"{}","title":"{}","content_type":{}}}"#,
                n.id,
                escape_json(n.ai_title.as_ref().unwrap_or(&n.title)),
                n.content_type.as_ref().map(|ct| format!("\"{}\"", ct)).unwrap_or("null".to_string()),
            )
        }).collect();
        println!("[{}]", items.join(","));
    } else {
        if orphans.is_empty() {
            println!("No orphaned nodes found.");
        } else {
            println!("Orphaned nodes ({}):\n", orphans.len());
            for node in &orphans {
                let ct = node.content_type.as_deref().unwrap_or("?");
                let title = node.ai_title.as_ref().unwrap_or(&node.title);
                println!("  {} [{}] {}", &node.id[..8], ct, title);
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
                log!("\nWARNING: This will permanently delete {} nodes!", total);
                log!("This action CANNOT be undone.\n");
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
            log!("Deleted {} nodes and all edges", total);
        }

        MaintenanceCommands::ResetAi { force } => {
            if !force && !confirm_action("reset AI processing (titles, summaries, tags)")? {
                return Err("Operation cancelled".into());
            }
            let count = db.reset_ai_processing().map_err(|e| e.to_string())?;
            log!("Reset AI processing for {} nodes", count);
        }

        MaintenanceCommands::ResetClusters { force } => {
            if !force && !confirm_action("reset clustering data")? {
                return Err("Operation cancelled".into());
            }
            // Reset cluster assignments for all items
            db.clear_item_parents().map_err(|e| e.to_string())?;
            // Reset needs_clustering flag so items get re-clustered
            let count = db.mark_all_items_need_clustering().map_err(|e| e.to_string())?;
            log!("Reset clustering for {} items (cleared parents, cluster_id, cluster_label, needs_clustering=1)", count);
        }

        MaintenanceCommands::ResetPrivacy { force } => {
            if !force && !confirm_action("reset privacy scores")? {
                return Err("Operation cancelled".into());
            }
            let count = db.reset_all_privacy_flags().map_err(|e| e.to_string())?;
            log!("Reset privacy scores for {} nodes", count);
        }

        MaintenanceCommands::ClearEmbeddings { force } => {
            if !force && !confirm_action("clear all embeddings")? {
                return Err("Operation cancelled".into());
            }
            let count = db.clear_all_embeddings().map_err(|e| e.to_string())?;
            log!("Cleared {} embeddings", count);
        }

        MaintenanceCommands::ClearHierarchy { force } => {
            if !force && !confirm_action("clear hierarchy (delete intermediate nodes, keep items)")? {
                return Err("Operation cancelled".into());
            }
            // Clear parent_id on items
            db.clear_item_parents().map_err(|e| e.to_string())?;
            // Delete intermediate hierarchy nodes (clusters, categories)
            let deleted = db.delete_hierarchy_nodes().map_err(|e| e.to_string())?;
            log!("Cleared hierarchy: {} intermediate nodes deleted", deleted);
        }

        MaintenanceCommands::ClearTags { force } => {
            if !force && !confirm_action("clear all tags")? {
                return Err("Operation cancelled".into());
            }
            db.delete_all_tags().map_err(|e| e.to_string())?;
            log!("Cleared all tags");
        }

        MaintenanceCommands::DeleteEmpty { force } => {
            // Use delete_empty_items which returns count
            if !force && !confirm_action("delete nodes with empty content")? {
                return Err("Operation cancelled".into());
            }

            let deleted = db.delete_empty_items().map_err(|e| e.to_string())?;
            log!("Deleted {} empty items", deleted);
        }

        MaintenanceCommands::Vacuum => {
            // Fix counts, depths, and prune edges
            log!("Tidying database...");
            db.fix_all_child_counts().map_err(|e| e.to_string())?;
            db.fix_all_depths().map_err(|e| e.to_string())?;
            db.prune_dead_edges().map_err(|e| e.to_string())?;
            log!("Database tidied (for VACUUM, use: sqlite3 <db> 'VACUUM')");
        }

        MaintenanceCommands::FixCounts { verbose } => {
            let fixed = db.fix_all_child_counts().map_err(|e| e.to_string())?;
            if verbose {
                log!("Fixed {} node child counts", fixed);
            } else {
                log!("Fixed child counts");
            }
        }

        MaintenanceCommands::FixDepths { verbose } => {
            let fixed = db.fix_all_depths().map_err(|e| e.to_string())?;
            if verbose {
                log!("Fixed {} node depths", fixed);
            } else {
                log!("Fixed depths");
            }
        }

        MaintenanceCommands::PruneEdges { verbose } => {
            let pruned = db.prune_dead_edges().map_err(|e| e.to_string())?;
            if verbose {
                log!("Pruned {} dead edges", pruned);
            } else {
                log!("Pruned dead edges");
            }
        }

        MaintenanceCommands::IndexEdges => {
            log!("Indexing edges by parent for fast per-view loading...");
            let count = db.update_edge_parents().map_err(|e| e.to_string())?;
            log!("Indexed {} edges", count);
        }

        MaintenanceCommands::MergeSmallCategories { threshold, max_size } => {
            log!("Merging small sibling categories (threshold={:.2}, max_size={})...", threshold, max_size);
            let result = hierarchy::merge_small_categories(db, None, threshold, max_size).await
                .map_err(|e| e.to_string())?;
            log!("Merge complete:");
            log!("  Categories merged: {}", result.categories_merged);
            log!("  Children reparented: {}", result.children_reparented);
            log!("  Categories renamed: {}", result.categories_renamed);
            log!("  Levels processed: {}", result.levels_processed);
        }

        MaintenanceCommands::RepairCodeTags { path, dry_run } => {
            repair_code_tags(db, &path, dry_run)?;
        }

        MaintenanceCommands::RefineGraph { merge_threshold, min_component, dry_run } => {
            log!("Refining hierarchy by graph (merge={:.2}, min_component={}, dry_run={})...",
                merge_threshold, min_component, dry_run);

            let config = hierarchy::RefineGraphConfig {
                merge_threshold,
                min_component_size: min_component,
                dry_run,
            };

            let result = hierarchy::refine_hierarchy_by_graph(db, None, config).await
                .map_err(|e| format!("Refinement failed: {}", e))?;

            log!("Refinement complete:");
            log!("  {} categories analyzed", result.categories_analyzed);
            log!("  {} papers moved", result.papers_moved);
            log!("  {} subcategories created", result.subcategories_created);
            log!("  {} categories merged", result.categories_merged);
            log!("  {} iterations", result.iterations);
        }

        MaintenanceCommands::DiagnoseCoherence { max_depth, sample_size } => {
            diagnose_coherence(db, max_depth, sample_size)?;
        }

        MaintenanceCommands::RegenerateEdges { threshold, max_edges, force } => {
            // Count existing edges
            let existing = db.count_semantic_edges().map_err(|e| e.to_string())?;

            if !force {
                log!("\nThis will delete {} existing semantic edges and regenerate at threshold {:.2}", existing, threshold);
                log!("This may take several minutes for large databases.\n");
                print!("Type 'yes' to confirm: ");
                std::io::stdout().flush().ok();

                let mut input = String::new();
                std::io::stdin().read_line(&mut input).map_err(|e| e.to_string())?;

                if input.trim() != "yes" {
                    return Err("Operation cancelled".into());
                }
            }

            log!("Deleting existing semantic edges...");
            let deleted = db.delete_semantic_edges().map_err(|e| e.to_string())?;
            log!("  Deleted {} edges", deleted);

            log!("Regenerating edges at threshold {:.2} (max {} per node)...", threshold, max_edges);
            let created = db.create_semantic_edges(threshold, max_edges).map_err(|e| e.to_string())?;
            log!("  Created {} edges", created);

            // Index edges for view lookups
            log!("Indexing edges for view lookups...");
            let indexed = db.update_edge_parents().map_err(|e| e.to_string())?;
            log!("  Indexed {} edges", indexed);

            log!("\nEdge regeneration complete. Run 'hierarchy rebuild --algorithm adaptive' to use new edges.");
        }

        MaintenanceCommands::CleanDuplicates { dry_run } => {
            log!("Scanning for garbage duplicate papers...");

            // Helper: check if content is garbage (purely numeric, empty, or very short)
            fn is_garbage(content: Option<&str>) -> bool {
                match content {
                    None => true,
                    Some(s) => {
                        let trimmed = s.trim();
                        if trimmed.is_empty() || trimmed.len() < 50 {
                            return true;
                        }
                        // Purely numeric (like "40016569269")
                        trimmed.chars().all(|c| c.is_ascii_digit() || c.is_whitespace() || c == '.' || c == '-')
                    }
                }
            }

            let duplicates = db.find_duplicate_papers_by_title().map_err(|e| e.to_string())?;
            if duplicates.is_empty() {
                log!("No duplicate papers found.");
                return Ok(());
            }

            // Group by title: (node_id, doi, abstract, node_content)
            let mut by_title: std::collections::HashMap<String, Vec<(String, Option<String>, Option<String>, Option<String>)>> =
                std::collections::HashMap::new();
            for (node_id, title, doi, abstract_text, node_content) in duplicates {
                by_title.entry(title).or_default().push((node_id, doi, abstract_text, node_content));
            }

            let mut to_delete = Vec::new();
            let mut skipped_different_dois = 0;
            let mut groups_cleaned = 0;

            for (title, papers) in &by_title {
                if papers.len() <= 1 {
                    continue;
                }

                // Check if papers have different DOIs - if so, they're legitimately different
                let dois: std::collections::HashSet<_> = papers.iter()
                    .filter_map(|(_, doi, _, _)| doi.as_ref().map(|d| d.to_lowercase()))
                    .collect();

                if dois.len() > 1 {
                    // Different DOIs = different papers, skip this group
                    skipped_different_dois += 1;
                    continue;
                }

                // All papers have same DOI (or no DOI) - find garbage ones to delete
                let mut garbage_papers = Vec::new();
                let mut good_papers = Vec::new();

                for (node_id, doi, abstract_text, node_content) in papers {
                    // Paper is garbage if BOTH abstract and node_content are garbage
                    let abs_garbage = is_garbage(abstract_text.as_deref());
                    let content_garbage = is_garbage(node_content.as_deref());

                    if abs_garbage && content_garbage && doi.is_none() {
                        garbage_papers.push((node_id.clone(), title.clone()));
                    } else {
                        good_papers.push(node_id.clone());
                    }
                }

                // Only delete garbage papers if we have at least one good one to keep
                // (or if all are garbage, keep one arbitrarily)
                if !garbage_papers.is_empty() {
                    if good_papers.is_empty() {
                        // All garbage - keep one, delete rest
                        garbage_papers.pop(); // Remove last one to keep
                    }
                    groups_cleaned += 1;
                    to_delete.extend(garbage_papers);
                }
            }

            log!("Analysis:");
            log!("  {} title groups with duplicates", by_title.len());
            log!("  {} groups skipped (different DOIs = different papers)", skipped_different_dois);
            log!("  {} groups with garbage duplicates to clean", groups_cleaned);
            log!("  {} garbage papers to delete", to_delete.len());

            if to_delete.is_empty() {
                log!("\nNo garbage duplicates found.");
                return Ok(());
            }

            if dry_run {
                log!("\n[DRY RUN] Would delete:");
                for (node_id, title) in &to_delete {
                    log!("  {} - {}", node_id, title);
                }
                log!("\nRun without --dry-run to delete.");
                return Ok(());
            }

            let mut deleted = 0;
            for (node_id, title) in &to_delete {
                match db.delete_paper_and_node(node_id) {
                    Ok(_) => deleted += 1,
                    Err(e) => log!("  Failed to delete '{}': {}", title, e),
                }
            }

            log!("\nDeleted {} garbage duplicate papers.", deleted);
        }

        MaintenanceCommands::BackfillHashes => {
            use sha2::{Sha256, Digest};

            log!("Backfilling content hashes for existing papers...");

            let papers = db.get_papers_needing_content_hash().map_err(|e| e.to_string())?;

            let total = papers.len();
            if total == 0 {
                log!("All papers already have content hashes.");
                return Ok(());
            }

            log!("Backfilling {} papers...", total);

            let mut updated = 0;
            for (node_id, title, abstract_text) in papers {
                // Compute SHA-256 hash (stable across Rust versions)
                let normalized_title = title.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
                let normalized_abstract = abstract_text
                    .map(|a| a.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" "))
                    .unwrap_or_default();

                let mut hasher = Sha256::new();
                hasher.update(normalized_title.as_bytes());
                hasher.update(b"|");
                hasher.update(normalized_abstract.as_bytes());
                let result = hasher.finalize();
                let content_hash = format!("{:032x}", u128::from_be_bytes(result[..16].try_into().unwrap()));

                if db.update_paper_content_hash(&node_id, &content_hash).is_ok() {
                    updated += 1;
                    if updated % 1000 == 0 {
                        log!("  Progress: {}/{}", updated, total);
                    }
                }
            }

            log!("Backfilled content hashes for {} papers.", updated);
        }
    }
    Ok(())
}

/// Compute percentiles from a sorted slice
fn percentiles(sorted: &[f32]) -> (f32, f32, f32, f32, f32, f32) {
    if sorted.is_empty() {
        return (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    }
    let n = sorted.len();
    let min = sorted[0];
    let max = sorted[n - 1];
    let p25 = sorted[n / 4];
    let p50 = sorted[n / 2];
    let p75 = sorted[3 * n / 4];
    let p90 = sorted[9 * n / 10];
    (min, p25, p50, p75, p90, max)
}

/// Diagnose coherence and sibling similarity distributions by depth
fn diagnose_coherence(db: &Database, max_depth: i32, sample_size: usize) -> Result<(), String> {
    use similarity::cosine_similarity;
    use rand::seq::SliceRandom;

    log!("\n=== COHERENCE & SIMILARITY DIAGNOSTIC ===\n");

    let mut coherence_stats: Vec<(i32, usize, f32, f32, f32, f32, f32, f32)> = Vec::new();
    let mut sibling_stats: Vec<(i32, usize, f32, f32, f32, f32, f32, f32)> = Vec::new();

    for depth in 0..=max_depth {
        let nodes = db.get_nodes_at_depth(depth).map_err(|e| e.to_string())?;
        let categories: Vec<_> = nodes.iter().filter(|n| !n.is_item).collect();

        if categories.is_empty() {
            continue;
        }

        // === COHERENCE ANALYSIS ===
        // Sample categories at this depth
        let mut rng = rand::thread_rng();
        let mut sampled: Vec<_> = categories.clone();
        sampled.shuffle(&mut rng);
        sampled.truncate(sample_size);

        let mut coherence_scores: Vec<f32> = Vec::new();
        for cat in &sampled {
            let children = db.get_children(&cat.id).map_err(|e| e.to_string())?;
            if children.len() < 2 {
                continue;
            }

            // Get centroid
            let centroid = match db.get_node_embedding(&cat.id).map_err(|e| e.to_string())? {
                Some(c) => c,
                None => continue,
            };

            // Compute mean similarity to centroid
            let mut total_sim = 0.0f32;
            let mut count = 0usize;
            for child in &children {
                if let Some(emb) = db.get_node_embedding(&child.id).map_err(|e| e.to_string())? {
                    total_sim += cosine_similarity(&centroid, &emb);
                    count += 1;
                }
            }

            if count >= 2 {
                coherence_scores.push(total_sim / count as f32);
            }
        }

        coherence_scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let (min, p25, p50, p75, p90, max) = percentiles(&coherence_scores);
        coherence_stats.push((depth, coherence_scores.len(), min, p25, p50, p75, p90, max));

        // === SIBLING SIMILARITY ANALYSIS ===
        // Group categories by parent
        let mut by_parent: std::collections::HashMap<String, Vec<&Node>> = std::collections::HashMap::new();
        for cat in &categories {
            if let Some(ref pid) = cat.parent_id {
                by_parent.entry(pid.clone()).or_default().push(*cat);
            }
        }

        let mut sibling_sims: Vec<f32> = Vec::new();
        for siblings in by_parent.values() {
            if siblings.len() < 2 {
                continue;
            }

            // Get embeddings
            let embeddings: Vec<(String, Vec<f32>)> = siblings.iter()
                .filter_map(|s| db.get_node_embedding(&s.id).ok().flatten()
                    .map(|emb| (s.id.clone(), emb)))
                .collect();

            // Compute pairwise similarities
            for i in 0..embeddings.len() {
                for j in (i + 1)..embeddings.len() {
                    let sim = cosine_similarity(&embeddings[i].1, &embeddings[j].1);
                    sibling_sims.push(sim);
                }
            }
        }

        sibling_sims.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let (min, p25, p50, p75, p90, max) = percentiles(&sibling_sims);
        sibling_stats.push((depth, sibling_sims.len(), min, p25, p50, p75, p90, max));
    }

    // Print coherence table
    log!("COHERENCE BY DEPTH (mean child-to-centroid similarity)");
    log!("Higher = tighter cluster, Lower = scattered children\n");
    log!("{:>5} {:>8} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6}",
         "Depth", "Samples", "Min", "P25", "P50", "P75", "P90", "Max");
    log!("{}", "-".repeat(60));
    for (depth, n, min, p25, p50, p75, p90, max) in &coherence_stats {
        log!("{:>5} {:>8} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3}",
             depth, n, min, p25, p50, p75, p90, max);
    }

    log!("\n\nSIBLING SIMILARITY BY DEPTH (pairwise centroid similarity)");
    log!("Higher = siblings are similar (merge candidates), Lower = distinct\n");
    log!("{:>5} {:>8} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6}",
         "Depth", "Pairs", "Min", "P25", "P50", "P75", "P90", "Max");
    log!("{}", "-".repeat(60));
    for (depth, n, min, p25, p50, p75, p90, max) in &sibling_stats {
        log!("{:>5} {:>8} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3}",
             depth, n, min, p25, p50, p75, p90, max);
    }

    log!("\n\nSUGGESTED THRESHOLDS:");
    // Find natural boundaries
    if let Some((_, _, _, _, p50_coh, _, _, _)) = coherence_stats.iter().find(|(d, _, _, _, _, _, _, _)| *d == 2) {
        log!("  Coherence threshold: {:.2} (based on depth-2 median)", p50_coh);
    }
    if let Some((_, _, _, _, _, _, p90_sib, _)) = sibling_stats.iter().find(|(d, _, _, _, _, _, _, _)| *d == 2) {
        log!("  Merge threshold: {:.2} (based on depth-2 P90 sibling sim)", p90_sib);
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
        AnalyzeCommands::DocEdges => {
            analyze_doc_edges(db, json, quiet)?;
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
                        updated_at: Some(chrono::Utc::now().timestamp_millis()),
                        author: None,
                        reason: None,
                        content: None,
                        agent_id: None,
                        superseded_by: None,
                        metadata: None,
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

/// Repair code node tags by restoring file_path metadata from source files.
/// This fixes nodes where AI processing overwrote the file location metadata.
fn repair_code_tags(db: &Database, path: &str, dry_run: bool) -> Result<(), String> {
    use std::collections::HashMap;
    use std::path::Path;
    use mycelica_lib::code::{self, Language, CodeItem};

    log!("Scanning source files in: {}", path);
    if dry_run {
        log!("(dry-run mode - no changes will be made)");
    }

    let path = Path::new(path);

    // Collect all code files
    let files = code::collect_code_files(path, None)
        .map_err(|e| format!("Failed to collect files: {}", e))?;

    log!("Found {} source files to scan", files.len());

    // Build a map of node_id -> correct metadata JSON
    let mut id_to_metadata: HashMap<String, String> = HashMap::new();

    for file_path in &files {
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let language = match Language::from_extension(ext) {
            Some(lang) => lang,
            None => continue,
        };

        // Skip doc files - they don't have the broken tags issue
        if language == Language::Markdown || language == Language::Rst {
            continue;
        }

        // Parse the file to get code items
        let items: Vec<CodeItem> = match language {
            Language::Rust => code::rust_parser::parse_rust_file(file_path)
                .unwrap_or_default(),
            Language::TypeScript | Language::JavaScript => code::ts_parser::parse_ts_file(file_path)
                .unwrap_or_default(),
            Language::Python => code::python_parser::parse_py_file(file_path)
                .unwrap_or_default(),
            Language::C => code::c_parser::parse_c_file(file_path)
                .unwrap_or_default(),
            _ => continue,
        };

        for item in items {
            let node_id = item.generate_id();
            let metadata = item.metadata_json();
            id_to_metadata.insert(node_id, metadata);
        }
    }

    log!("Parsed {} code items from source files", id_to_metadata.len());

    // Get all code nodes from database
    let all_nodes = db.get_items().map_err(|e| e.to_string())?;
    let code_nodes: Vec<_> = all_nodes
        .into_iter()
        .filter(|n| n.content_type.as_deref().map(|ct| ct.starts_with("code_")).unwrap_or(false))
        .filter(|n| n.id.starts_with("code-"))
        .collect();

    log!("Found {} code nodes in database", code_nodes.len());

    let mut repaired = 0;
    let mut already_ok = 0;
    let mut not_found = 0;

    for node in code_nodes {
        // Check if we have metadata for this node
        let correct_metadata = match id_to_metadata.get(&node.id) {
            Some(m) => m,
            None => {
                not_found += 1;
                continue;
            }
        };

        // Check if current tags are broken (array format instead of object)
        let current_tags = node.tags.as_deref().unwrap_or("[]");
        let is_broken = current_tags.trim_start().starts_with('[');

        if !is_broken {
            already_ok += 1;
            continue;
        }

        // Repair the tags
        if dry_run {
            log!("Would repair: {} -> {}", node.id, &correct_metadata[..correct_metadata.len().min(60)]);
        } else {
            db.update_node_tags(&node.id, correct_metadata)
                .map_err(|e| format!("Failed to update {}: {}", node.id, e))?;
        }
        repaired += 1;
    }

    log!("");
    log!("Repair complete:");
    log!("  Repaired: {}", repaired);
    log!("  Already OK: {}", already_ok);
    log!("  Not found in source: {}", not_found);

    if dry_run && repaired > 0 {
        log!("");
        log!("Run without --dry-run to apply changes");
    }

    Ok(())
}

/// Analyze docs and create "Documents" edges to code they reference.
/// Scans doc content for backtick references and function call patterns.
fn analyze_doc_edges(db: &Database, json: bool, quiet: bool) -> Result<(), String> {
    use std::collections::{HashMap, HashSet};
    use mycelica_lib::db::{Edge, EdgeType};
    use regex::Regex;

    let all_nodes = db.get_items().map_err(|e| e.to_string())?;

    // Build name→id map for code items (functions, structs, classes, macros)
    let mut name_to_id: HashMap<String, String> = HashMap::new();
    for item in &all_nodes {
        let content_type = item.content_type.as_deref().unwrap_or("");
        if let Some(name) = extract_code_name_for_docs(&item.title, content_type) {
            name_to_id.insert(name, item.id.clone());
        }
    }

    if !quiet {
        eprintln!("[DocEdges] Built index of {} code names", name_to_id.len());
    }

    // Get doc nodes
    let doc_nodes: Vec<_> = all_nodes
        .iter()
        .filter(|n| n.content_type.as_deref() == Some("code_doc"))
        .collect();

    if !quiet {
        eprintln!("[DocEdges] Found {} docs to analyze", doc_nodes.len());
    }

    // Get existing Documents edges
    let existing_edges: HashSet<(String, String)> = db
        .get_all_edges()
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.edge_type == EdgeType::Documents)
        .map(|e| (e.source, e.target))
        .collect();

    let backtick_re = Regex::new(r"`([^`]+)`").unwrap();
    let fn_call_re = Regex::new(r"\b([a-z_][a-z0-9_]*)\s*\(").unwrap();
    let type_re = Regex::new(r"\b([A-Z][a-zA-Z0-9]+)\b").unwrap();

    let skip_keywords: HashSet<&str> = ["if", "for", "while", "match", "return", "let", "const",
        "fn", "def", "class", "struct", "enum", "type", "impl", "trait", "pub", "async",
        "e", "g", "i", "s", "t", "a", "eg", "ie", "etc", "vs", "or", "and"].into_iter().collect();

    let skip_types: HashSet<&str> = ["The", "This", "That", "These", "Those", "When", "What",
        "How", "Why", "Where", "Which", "See", "For", "From", "With", "Into", "After", "Before",
        "JSON", "API", "SQL", "PDF", "URL", "CSS", "HTML", "HTTP", "HTTPS", "UTF", "ASCII",
        "CLI", "TUI", "GUI", "README", "TODO", "FIXME", "NOTE", "OK", "NULL", "TRUE", "FALSE"].into_iter().collect();

    let mut edges_created = 0;

    for doc in &doc_nodes {
        let content = match &doc.content {
            Some(c) => c,
            None => continue,
        };

        let mut seen_in_doc: HashSet<String> = HashSet::new();

        let mut try_create_edge = |name: String| {
            if seen_in_doc.contains(&name) { return; }
            if let Some(code_id) = name_to_id.get(&name) {
                if existing_edges.contains(&(doc.id.clone(), code_id.clone())) {
                    seen_in_doc.insert(name);
                    return;
                }
                let edge = Edge {
                    id: format!("edge-doc-{}-{}", &doc.id[..8.min(doc.id.len())], &code_id[..8.min(code_id.len())]),
                    source: doc.id.clone(),
                    target: code_id.clone(),
                    edge_type: EdgeType::Documents,
                    label: None,
                    weight: Some(1.0),
                    edge_source: Some("doc-analysis".to_string()),
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
                if db.insert_edge(&edge).is_ok() {
                    edges_created += 1;
                }
                seen_in_doc.insert(name);
            }
        };

        // Backtick references
        for cap in backtick_re.captures_iter(content) {
            for name in extract_backtick_names(&cap[1]) {
                try_create_edge(name);
            }
        }

        // Function call patterns
        for cap in fn_call_re.captures_iter(content) {
            let name = cap[1].to_string();
            if !skip_keywords.contains(name.as_str()) && (name.len() > 3 || name.contains('_')) {
                try_create_edge(name);
            }
        }

        // Type names
        for cap in type_re.captures_iter(content) {
            let name = cap[1].to_string();
            if !skip_types.contains(name.as_str()) {
                try_create_edge(name);
            }
        }
    }

    if json {
        println!(r#"{{"docs_analyzed":{},"edges_created":{}}}"#, doc_nodes.len(), edges_created);
    } else {
        println!("Analyzed {} docs", doc_nodes.len());
        println!("Created {} new Documents edges", edges_created);
    }

    Ok(())
}

/// Extract code name from title for doc→code edge matching.
/// Filters out single-character names to avoid false positives.
fn extract_code_name_for_docs(title: &str, content_type: &str) -> Option<String> {
    match content_type {
        "code_function" => {
            // Rust: fn name
            if let Some(fn_idx) = title.find("fn ") {
                let after = &title[fn_idx + 3..];
                let end = after.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(after.len());
                let name = &after[..end];
                if name.len() >= 2 { return Some(name.to_string()); }
            }
            // Python: def name
            if let Some(def_idx) = title.find("def ") {
                let after = &title[def_idx + 4..];
                let end = after.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(after.len());
                let name = &after[..end];
                if name.len() >= 2 { return Some(name.to_string()); }
            }
            // C: last identifier before (
            if let Some(paren_idx) = title.find('(') {
                let before = title[..paren_idx].trim_end();
                let start = before.rfind(|c: char| !c.is_alphanumeric() && c != '_').map(|i| i + 1).unwrap_or(0);
                let name = &before[start..];
                let skip = ["void", "int", "char", "float", "double", "long", "short", "unsigned", "signed", "const", "static", "inline"];
                if name.len() >= 2 && !skip.contains(&name) { return Some(name.to_string()); }
            }
            None
        }
        "code_struct" => {
            if let Some(caps) = regex::Regex::new(r"(?:pub\s+)?(?:typedef\s+)?struct\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            None
        }
        "code_enum" => {
            if let Some(caps) = regex::Regex::new(r"(?:pub\s+)?(?:typedef\s+)?enum\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            None
        }
        "code_class" => {
            if let Some(caps) = regex::Regex::new(r"class\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            None
        }
        "code_macro" => {
            if let Some(caps) = regex::Regex::new(r"#define\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            None
        }
        "code_trait" => {
            if let Some(caps) = regex::Regex::new(r"(?:pub\s+)?trait\s+([a-zA-Z_][a-zA-Z0-9_]*)").ok()?.captures(title) {
                return Some(caps[1].to_string());
            }
            None
        }
        _ => None,
    }
}

/// Extract identifiers from backtick content.
/// Filters out single-character names to avoid false positives.
fn extract_backtick_names(content: &str) -> Vec<String> {
    let content = content.split('(').next().unwrap_or(content);
    content
        .split(|c| c == ':' || c == '.')
        .filter(|s| s.len() >= 2) // Skip single-char names
        .filter(|s| s.chars().next().map(|c| c.is_alphabetic() || c == '_').unwrap_or(false))
        .filter(|s| s.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .map(|s| s.to_string())
        .collect()
}

/// Extract function name from a title. Supports Rust, Python, and C patterns.
fn extract_function_name(title: &str) -> Option<String> {
    // Rust: "fn name" or "pub fn name" or "async fn name"
    if let Some(fn_idx) = title.find("fn ") {
        let after_fn = &title[fn_idx + 3..];
        let name_end = after_fn
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(after_fn.len());
        let name = &after_fn[..name_end];
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }

    // Python: "def name(" or "async def name("
    if let Some(def_idx) = title.find("def ") {
        let after_def = &title[def_idx + 4..];
        let name_end = after_def
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(after_def.len());
        let name = &after_def[..name_end];
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }

    // C: "type name(" - look for identifier followed by (
    // Common C patterns: "void foo(", "int bar(", "PyObject *func(", "static void baz("
    // Strategy: find last identifier before first '('
    if let Some(paren_idx) = title.find('(') {
        let before_paren = title[..paren_idx].trim_end();
        // Find the function name (last word before paren)
        let name_start = before_paren
            .rfind(|c: char| !c.is_alphanumeric() && c != '_')
            .map(|i| i + 1)
            .unwrap_or(0);
        let name = &before_paren[name_start..];
        // Skip if it looks like a keyword or type
        let skip_words = ["if", "for", "while", "switch", "return", "sizeof", "typeof",
                          "void", "int", "char", "float", "double", "long", "short",
                          "unsigned", "signed", "const", "static", "inline", "struct", "enum"];
        if !name.is_empty() && !skip_words.contains(&name) {
            return Some(name.to_string());
        }
    }

    None
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

            // Check if this is a code node
            let is_code_node = node.content_type.as_ref()
                .map(|ct| ct.starts_with("code_"))
                .unwrap_or(false);

            if !is_code_node {
                return Err(format!(
                    "Node '{}' is not a code node (content_type: {:?}). Use 'node get {}' instead.",
                    node.title,
                    node.content_type,
                    id
                ));
            }

            // Parse tags JSON to get file_path, line_start, line_end
            let tags = node.tags.as_ref()
                .ok_or("Code node has no tags metadata (missing file_path/line info)")?;

            #[derive(serde::Deserialize)]
            struct CodeMetadata {
                file_path: String,
                #[serde(default)]
                line_start: Option<usize>,
                #[serde(default)]
                line_end: Option<usize>,
                #[serde(default)]
                language: Option<String>,
            }

            // Check if tags looks like an array (regular tags) vs object (code metadata)
            let trimmed = tags.trim();
            if trimmed.starts_with('[') {
                return Err(format!(
                    "Code node '{}' has regular tags instead of source location metadata.\n\
                     This node was likely imported without --update or from a different source.\n\
                     Re-import with: mycelica-cli import code <file-or-directory> --update",
                    node.title
                ));
            }

            let metadata: CodeMetadata = serde_json::from_str(tags)
                .map_err(|e| format!("Failed to parse code metadata: {}.\nTags: {}", e, tags))?;

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

            // Determine line range (whole file if not specified)
            let line_start = metadata.line_start.unwrap_or(1);
            let line_end = metadata.line_end.unwrap_or(lines.len());

            // Validate line range
            if line_start == 0 || line_start > lines.len() {
                return Err(format!("Invalid line_start: {} (file has {} lines)", line_start, lines.len()));
            }
            let line_end = line_end.min(lines.len());

            // Extract the code range (1-indexed to 0-indexed)
            let code_lines = &lines[line_start - 1..line_end];

            // Print header
            println!("=== {} ===", node.title);
            println!("File: {}", metadata.file_path);
            if let Some(ref lang) = metadata.language {
                println!("Language: {}", lang);
            }
            println!("Lines: {}-{}", line_start, line_end);
            println!();

            // Print code with line numbers
            for (i, line) in code_lines.iter().enumerate() {
                let line_num = line_start + i;
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

    // Calculate available width for titles (area width - borders - emoji - percentage - spaces)
    let available_width = area.width.saturating_sub(2 + 2 + 5) as usize; // borders + emoji + " XX%"
    let title_max_len = available_width.saturating_sub(2).max(10); // at least 10 chars

    let similar_items: Vec<ListItem> = app.similar_nodes.iter().enumerate().map(|(i, sim)| {
        let emoji = sim.emoji.as_deref().unwrap_or("📄");
        let similarity_pct = (sim.similarity * 100.0) as i32;
        // Normalized gradient: spreads colors across visible range (red→yellow | blue→cyan)
        let color = similarity_color_normalized(sim.similarity as f64, min_sim, max_sim);

        let truncated_title = utils::safe_truncate(&sim.title, title_max_len);
        let content = format!("{} {} {}%", emoji, truncated_title, similarity_pct);

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

        // Calculate available width for call titles (area width - borders - arrow - space)
        let calls_title_max = area.width.saturating_sub(2 + 2 + 1) as usize; // borders + "→ "

        let call_items: Vec<ListItem> = app.calls_for_node.iter().enumerate().map(|(i, (_, title, is_outgoing))| {
            // Direction indicator: → for outgoing (calls), ← for incoming (called by)
            let arrow = if *is_outgoing { "→" } else { "←" };
            let truncated_title = utils::safe_truncate(title, calls_title_max.max(10));
            let content = format!("{} {}", arrow, truncated_title);

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
