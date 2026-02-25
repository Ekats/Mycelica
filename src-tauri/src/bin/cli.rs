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

pub(crate) static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

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
pub(crate) fn log_both(msg: &str) {
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
pub(crate) fn elog_both(msg: &str) {
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
pub(crate) fn is_predominantly_english(texts: &[String]) -> bool {
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

#[path = "cli/spore.rs"]
mod spore;

#[path = "cli/spore_runs.rs"]
mod spore_runs;

#[path = "cli/tui.rs"]
mod tui;

#[path = "cli/spore_analyzer.rs"]
mod spore_analyzer;

#[path = "cli/graph_ops.rs"]
mod graph_ops;

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
    /// Graph operations (edges, nodes, meta)
    Graph {
        #[command(subcommand)]
        cmd: GraphCommands,
    },
    #[cfg(feature = "mcp")]
    /// Start MCP server for agent coordination
    McpServer {
        /// Agent role: human, ingestor, coder, verifier, planner, architect, synthesizer, summarizer, docwriter, researcher
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
pub(crate) enum SporeCommands {
    /// Run tracking: list, inspect, or rollback orchestrator runs
    Runs {
        #[command(subcommand)]
        cmd: RunCommands,
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
    /// List all Lesson: nodes from the graph
    Lessons {
        /// Show one lesson per line (title only, no content preview)
        #[arg(long)]
        compact: bool,
    },
    /// Show a combined dashboard: recent runs, lessons, costs, and graph health
    Dashboard {
        /// Number of recent runs to show
        #[arg(long, default_value = "5")]
        limit: usize,
        /// Output format: text (default), json, or csv
        #[arg(long, default_value = "text", value_enum)]
        format: DashboardFormat,
        /// Show only summary counts (runs, verified, cost) without recent runs table or lessons
        #[arg(long)]
        count: bool,
        /// Show today's cost (runs created today UTC)
        #[arg(long)]
        cost: bool,
        /// Show count of stale code nodes (files no longer on disk)
        #[arg(long)]
        stale: bool,
    },
    /// Distill an orchestrator run into a summary node with lessons learned
    Distill {
        /// Run ID prefix (from task node ID) or "latest" for most recent run
        #[arg(default_value = "latest")]
        run: String,
        /// Print only one-line summary (outcome, duration, bounces) without the full trail
        #[arg(long)]
        compact: bool,
    },
    /// Check system health: database, CLI binary, agent prompts, MCP sidecar
    Health,
    /// Show line counts for all agent prompt files
    PromptStats,
    /// Analyze graph structure: topology, staleness, bridges, health score
    Analyze {
        /// Scope analysis to descendants of this node ID
        #[arg(long)]
        region: Option<String>,
        /// Number of top items to show per section
        #[arg(long, default_value = "10")]
        top_n: usize,
        /// Days since update to consider a node stale
        #[arg(long, default_value = "60")]
        stale_days: i64,
        /// Minimum degree to consider a node a hub
        #[arg(long, default_value = "15")]
        hub_threshold: usize,
    },

    // ---- Deprecation aliases: old `spore <graph-cmd>` paths ----
    // Hidden from help; print a deprecation warning then delegate to graph_ops.

    #[command(hide = true)]
    QueryEdges {
        #[arg(long = "type", short = 't')]
        edge_type: Option<String>,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        target_agent: Option<String>,
        #[arg(long)]
        confidence_min: Option<f64>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        not_superseded: bool,
        #[arg(long, default_value = "20")]
        limit: usize,
        #[arg(long)]
        compact: bool,
    },
    #[command(hide = true)]
    ExplainEdge {
        id: String,
        #[arg(long, default_value = "1")]
        depth: usize,
    },
    #[command(hide = true)]
    PathBetween {
        from: String,
        to: String,
        #[arg(long, default_value = "5")]
        max_hops: usize,
        #[arg(long = "edge-types")]
        edge_types: Option<String>,
    },
    #[command(hide = true)]
    EdgesForContext {
        id: String,
        #[arg(long, default_value = "10")]
        top: usize,
        #[arg(long)]
        not_superseded: bool,
    },
    #[command(hide = true)]
    CreateMeta {
        #[arg(long = "type", short = 't')]
        meta_type: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        content: Option<String>,
        #[arg(long, default_value = "human")]
        agent: String,
        #[arg(long = "connects-to", num_args = 1..)]
        connects_to: Vec<String>,
        #[arg(long = "edge-type", default_value = "summarizes")]
        edge_type: String,
    },
    #[command(hide = true)]
    UpdateMeta {
        id: String,
        #[arg(long)]
        content: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, default_value = "human")]
        agent: String,
        #[arg(long = "add-connects", num_args = 1..)]
        add_connects: Vec<String>,
        #[arg(long = "edge-type", default_value = "summarizes")]
        edge_type: String,
    },
    #[command(hide = true)]
    Status {
        #[arg(long)]
        all: bool,
        #[arg(long, default_value = "compact")]
        format: String,
    },
    #[command(hide = true)]
    CreateEdge {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long = "type", short = 't')]
        edge_type: String,
        #[arg(long)]
        content: Option<String>,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long, default_value = "spore")]
        agent: String,
        #[arg(long)]
        confidence: Option<f64>,
        #[arg(long)]
        supersedes: Option<String>,
        #[arg(long)]
        metadata: Option<String>,
    },
    #[command(hide = true)]
    ReadContent {
        id: String,
    },
    #[command(hide = true)]
    ListRegion {
        id: String,
        #[arg(long)]
        class: Option<String>,
        #[arg(long)]
        items_only: bool,
        #[arg(long, default_value = "50")]
        limit: usize,
    },
    #[command(hide = true)]
    CheckFreshness {
        id: String,
    },
    #[command(hide = true)]
    Gc {
        #[arg(long, default_value = "7")]
        days: u32,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum GraphCommands {
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
    /// Graph status dashboard (meta nodes, edge activity, coverage, coherence)
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
        /// JSON metadata string (e.g. run tracking data)
        #[arg(long)]
        metadata: Option<String>,
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
    /// Find stale operational nodes with no incoming edges (GC candidates)
    Gc {
        /// Age threshold in days (default: 7)
        #[arg(long, default_value = "7")]
        days: u32,
        /// Dry run -- only print candidates without deleting
        #[arg(long)]
        dry_run: bool,
        /// Include Lesson: and Summary: nodes in GC candidates (normally excluded)
        #[arg(long)]
        force: bool,
    },
}

#[derive(Clone, clap::ValueEnum)]
pub(crate) enum DashboardFormat {
    Text,
    Json,
    Csv,
    Compact,
}

#[derive(Subcommand)]
pub(crate) enum RunCommands {
    /// List all orchestrator runs with status
    List {
        /// Also show non-Orchestration operational nodes that have tracks edges
        #[arg(long)]
        all: bool,
        /// Sort by cost (most expensive first) and show total cost for each run
        #[arg(long)]
        cost: bool,
        /// Show only runs that have an associated ESCALATION node
        #[arg(long)]
        escalated: bool,
        /// Filter by run status (comma-separated: verified,implemented,escalated,cancelled,pending)
        #[arg(long)]
        status: Option<String>,
        /// Only show runs created after this date (YYYY-MM-DD or relative: 1h, 2d, 1w)
        #[arg(long)]
        since: Option<String>,
        /// Maximum number of runs to show (0 = no limit)
        #[arg(long, default_value = "0")]
        limit: usize,
        /// Show full task text instead of truncating
        #[arg(long, short)]
        verbose: bool,
        /// Output format: text (default), compact, json, or csv
        #[arg(long, default_value = "text", value_enum)]
        format: DashboardFormat,
        /// Only show runs with total duration >= this many seconds
        #[arg(long)]
        duration: Option<u64>,
        /// Filter by agent name (e.g. "researcher", "coder", or "spore:coder")
        #[arg(long)]
        agent: Option<String>,
    },
    /// Show all edges in a run
    Get {
        /// Run ID (UUID)
        run_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Compare two runs side-by-side
    Compare {
        /// First run ID (prefix or UUID)
        run_a: String,
        /// Second run ID (prefix or UUID)
        run_b: String,
    },
    /// Compare two experiment batches side-by-side
    CompareExperiments {
        /// Experiment labels to compare (exactly 2)
        #[arg(long, required = true, num_args = 2)]
        experiment: Vec<String>,
    },
    /// Show complete timeline of a run: agents, edges, outcomes
    History {
        /// Run ID (prefix match)
        run_id: String,
    },
    /// Alias for 'history'
    Show {
        /// Run ID (prefix match)
        run_id: String,
    },
    /// Show source code files changed by a run
    Diff {
        /// Run ID (prefix match)
        run_id: String,
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
    /// Cancel a pending run (marks it as cancelled)
    Cancel {
        /// Run ID (prefix match)
        run_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show cost breakdown: total, average, by status, and today's cost
    Cost {
        /// Only include runs created after this date (YYYY-MM-DD or relative: 1h, 2d, 1w)
        #[arg(long)]
        since: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show the top N most expensive runs by total cost (default: 5)
    Top {
        /// Number of top results to show
        #[arg(long, default_value = "5")]
        limit: usize,
    },
    /// Show aggregate statistics across all runs
    Stats {
        /// Filter stats to only runs tagged with this experiment label
        #[arg(long)]
        experiment: Option<String>,
    },
    /// One-paragraph natural language summary of recent orchestrator activity
    Summary,
    /// Show a vertical timeline of agent phases for a run
    Timeline {
        /// Run ID (prefix match)
        run_id: String,
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
        /// Agent ID attribution (e.g. spore:coder)
        #[arg(long)]
        agent_id: Option<String>,
        /// Node class: knowledge, meta, operational
        #[arg(long)]
        node_class: Option<String>,
        /// Meta type (e.g. task, implementation, summary, escalation)
        #[arg(long)]
        meta_type: Option<String>,
        /// Source attribution
        #[arg(long)]
        source: Option<String>,
        /// Author attribution
        #[arg(long)]
        author: Option<String>,
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
    /// Collapse binary cascade routing nodes (post-processing)
    CollapseBinary,
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
    /// Detect stale code nodes whose source files no longer exist on disk
    Stale {
        /// Delete stale nodes and their edges (default: dry-run report only)
        #[arg(long)]
        fix: bool,
        /// Only check nodes matching this path prefix
        #[arg(long)]
        path: Option<String>,
    },
}

// ============================================================================
// Main Entry Point
// ============================================================================

#[tokio::main]
async fn main() {
    // Ignore SIGPIPE so piping through head/tail doesn't kill the process.
    // Without this, `mycelica-cli spore orchestrate ... | head -30` sends SIGPIPE
    // when head closes its stdin, terminating the entire orchestrator.
    #[cfg(unix)]
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN); }

    // Exit cleanly on broken pipe instead of panicking.
    // println! internally unwraps write results, so even with SIGPIPE ignored,
    // it panics when the pipe is closed. This hook catches that and exits quietly.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info.to_string();
        if msg.contains("Broken pipe") {
            std::process::exit(0);
        }
        default_hook(info);
    }));

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
        Commands::Link { source, target, edge_type, reason, content, agent, confidence, supersedes } => graph_ops::handle_link(&source, &target, &edge_type, reason, content, &agent, confidence, supersedes, None, "user", &db, cli.json).await,
        Commands::Orphans { limit } => handle_orphans(limit, &db, cli.json).await,
        Commands::Migrate { cmd } => handle_migrate(cmd, &db, &db_path, cli.json),
        Commands::Spore { cmd } => spore::handle_spore(cmd, &db, cli.json).await,
        Commands::Graph { cmd } => graph_ops::handle_graph(cmd, &db, cli.json).await,
        #[cfg(feature = "mcp")]
        Commands::McpServer { agent_role, agent_id, stdio, run_id } => {
            if !stdio {
                return Err("Only --stdio transport is currently supported".to_string());
            }
            let role = mycelica_lib::mcp::AgentRole::from_str(&agent_role)
                .ok_or_else(|| format!("Unknown role: '{}'. Valid: human, ingestor, coder, tester, verifier, planner, architect, synthesizer, summarizer, docwriter, researcher, operator", agent_role))?;
            mycelica_lib::mcp::run_mcp_server(db.clone(), agent_id, role, run_id).await
        },
        Commands::Tui => tui::run_tui(&db).await,
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
// Code Update (reusable for import --update and watch)
// ============================================================================

/// Summary of a code update operation (delete + reimport + embeddings + call edges).
pub(crate) struct CodeUpdateSummary {
    pub deleted: usize,
    pub imported: usize,
    pub embeddings: usize,
    pub calls_edges: usize,
    pub node_ids: Vec<String>,
    /// Stale nodes cleaned (from files that no longer exist on disk)
    pub stale_cleaned: usize,
    /// The raw import result (for detailed per-type counts in output)
    pub import_result: mycelica_lib::code::CodeImportResult,
}

/// Run a full code update: delete existing nodes for the given path, reimport,
/// generate embeddings, and refresh Calls edges. This is the core logic extracted
/// from `ImportCommands::Code` with `--update`.
pub(crate) async fn run_code_update(
    db: &Database,
    path: &str,
    language: Option<&str>,
    quiet: bool,
    _json: bool,
) -> Result<CodeUpdateSummary, String> {
    use mycelica_lib::code;
    use mycelica_lib::ai_client;

    // Collect files and delete existing nodes
    let file_path = std::path::Path::new(path);
    let is_directory = file_path.is_dir();
    let files_to_process: Vec<String> = if file_path.is_file() {
        vec![path.to_string()]
    } else {
        code::collect_code_files(file_path, language)
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect()
    };

    // When updating a directory, detect and clean stale nodes from deleted/moved files.
    // Compare DB file paths against files found on disk — any DB path under this directory
    // that no longer exists on disk is stale.
    let mut stale_cleaned = 0;
    if is_directory {
        let disk_files: std::collections::HashSet<String> = files_to_process.iter().cloned().collect();
        if let Ok(db_paths) = db.get_all_code_file_paths() {
            for (db_file, _count) in &db_paths {
                // Only check paths that would be under the update directory
                if db_file.contains(path) && !disk_files.contains(db_file) {
                    // Verify the file truly doesn't exist (resolve relative paths)
                    let resolved = if std::path::Path::new(db_file).is_relative() {
                        std::path::Path::new(".").join(db_file)
                    } else {
                        std::path::PathBuf::from(db_file)
                    };
                    if !resolved.exists() {
                        match db.delete_nodes_by_file_path(db_file) {
                            Ok(deleted) if deleted > 0 => {
                                if !quiet {
                                    log!("  Cleaned {} stale nodes: {} (file no longer exists)", deleted, db_file);
                                }
                                stale_cleaned += deleted;
                            }
                            _ => {}
                        }
                    }
                }
            }
            if stale_cleaned > 0 {
                // Prune dangling edges from deleted stale nodes
                let _ = db.prune_dead_edges();
                if !quiet {
                    log!("[Code] Cleaned {} stale nodes from deleted/moved files", stale_cleaned);
                }
            }
        }
    }

    let mut total_deleted = 0;
    for file in &files_to_process {
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

    // Reimport
    let result = code::import_code(db, path, language)?;

    // Generate embeddings and refresh edges
    let mut embeddings_generated = 0;
    let mut calls_edges_created = 0;
    let mut new_node_ids: Vec<String> = Vec::new();

    if result.total_items() > 0 {
        // Collect newly imported node IDs
        for file in &files_to_process {
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
                        let file_path = node.tags.as_ref()
                            .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
                            .and_then(|v| v.get("file_path").and_then(|s| s.as_str()).map(|s| s.to_string()));
                        let text = if let Some(fp) = file_path {
                            format!("[{}] {}\n{}", fp, node.title, node.content.as_deref().unwrap_or(""))
                        } else {
                            format!("{}\n{}", node.title, node.content.as_deref().unwrap_or(""))
                        };
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

            let all_functions: Vec<_> = db.get_items()
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|n| n.content_type.as_deref() == Some("code_function"))
                .collect();

            let fn_name_to_id: std::collections::HashMap<String, String> = all_functions.iter()
                .filter_map(|f| {
                    let title = &f.title;
                    let name = title.split('(').next()
                        .and_then(|s| s.split_whitespace().last())
                        .map(|s| s.to_string());
                    name.map(|n| (n, f.id.clone()))
                })
                .collect();

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64;

            for func in &functions {
                if let Some(content) = &func.content {
                    for (fn_name, target_id) in &fn_name_to_id {
                        if target_id != &func.id && content.contains(&format!("{}(", fn_name)) {
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

    Ok(CodeUpdateSummary {
        deleted: total_deleted,
        imported: result.total_items(),
        embeddings: embeddings_generated,
        calls_edges: calls_edges_created,
        node_ids: new_node_ids,
        stale_cleaned,
        import_result: result,
    })
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

            if update {
                // Delegate to reusable run_code_update()
                let summary = run_code_update(db, &path, language.as_deref(), quiet, json).await?;
                let result = &summary.import_result;

                if json {
                    println!("{}", serde_json::to_string(result).map_err(|e| e.to_string())?);
                } else {
                    if summary.deleted > 0 {
                        log!("Updated {} files (replaced {} existing nodes):", result.files_processed, summary.deleted);
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
                    if result.classes > 0 { log!("  Classes: {}", result.classes); }
                    if result.interfaces > 0 { log!("  Interfaces: {}", result.interfaces); }
                    if result.types > 0 { log!("  Types: {}", result.types); }
                    if result.consts > 0 { log!("  Consts: {}", result.consts); }
                    log!("  Edges created: {}", result.edges_created);
                    if result.doc_edges > 0 { log!("  Doc→code edges: {}", result.doc_edges); }
                    if summary.embeddings > 0 { log!("  Embeddings generated: {}", summary.embeddings); }
                    if summary.calls_edges > 0 { log!("  Calls edges refreshed: {}", summary.calls_edges); }
                    if summary.stale_cleaned > 0 { log!("  Stale nodes cleaned: {}", summary.stale_cleaned); }
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
            } else {
                // Non-update: plain import (no delete, no embeddings, no edge refresh)
                let result = code::import_code(db, &path, language.as_deref())?;

                if json {
                    println!("{}", serde_json::to_string(&result).map_err(|e| e.to_string())?);
                } else {
                    log!("Imported {} items from {} files:", result.total_items(), result.files_processed);
                    if result.functions > 0 { log!("  Functions: {}", result.functions); }
                    if result.structs > 0 { log!("  Structs: {}", result.structs); }
                    if result.enums > 0 { log!("  Enums: {}", result.enums); }
                    if result.traits > 0 { log!("  Traits: {}", result.traits); }
                    if result.impls > 0 { log!("  Impl blocks: {}", result.impls); }
                    if result.modules > 0 { log!("  Modules: {}", result.modules); }
                    if result.macros > 0 { log!("  Macros: {}", result.macros); }
                    if result.docs > 0 { log!("  Docs: {}", result.docs); }
                    if result.classes > 0 { log!("  Classes: {}", result.classes); }
                    if result.interfaces > 0 { log!("  Interfaces: {}", result.interfaces); }
                    if result.types > 0 { log!("  Types: {}", result.types); }
                    if result.consts > 0 { log!("  Consts: {}", result.consts); }
                    log!("  Edges created: {}", result.edges_created);
                    if result.doc_edges > 0 { log!("  Doc→code edges: {}", result.doc_edges); }
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
        NodeCommands::Create { title, content, node_type, agent_id, node_class, meta_type, source, author } => {
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
                source: source.or_else(|| Some("cli".to_string())),
                pdf_available: None,
                content_type: None,
                associated_idea_id: None,
                human_edited: None,
                human_created: false,
                author,
                agent_id,
                node_class,
                meta_type,
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
        HierarchyCommands::CollapseBinary => {
            let collapsed = mycelica_lib::dendrogram::collapse_binary_cascades(&db)
                .map_err(|e| e.to_string())?;
            if json {
                log!(r#"{{"collapsed":{}}}"#, collapsed);
            } else {
                log!("Collapsed {} binary routing nodes", collapsed);
            }
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
            max_depth: 7,
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

    // Step 7.6: Collapse binary cascade routing nodes
    if !quiet { elog!("Step 7.6/7: Collapsing binary cascades..."); }
    let collapsed = mycelica_lib::dendrogram::collapse_binary_cascades(db)
        .map_err(|e| e.to_string())?;
    if collapsed > 0 {
        if !quiet { elog!("  Collapsed {} binary routing nodes", collapsed); }
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

                let file_path = node.tags.as_ref()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
                    .and_then(|v| v.get("file_path").and_then(|s| s.as_str()).map(|s| s.to_string()));
                let text = if let Some(fp) = file_path {
                    format!("[{}] {}\n{}", fp, node.title, node.content.as_deref().unwrap_or(""))
                } else {
                    format!("{}\n{}", node.title, node.content.as_deref().unwrap_or(""))
                };
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

            // Embed file path + signature + content for semantic search
            let file_path = node.tags.as_ref()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
                .and_then(|v| v.get("file_path").and_then(|s| s.as_str()).map(|s| s.to_string()))
                .unwrap_or_default();
            let text = if file_path.is_empty() {
                format!("{}\n{}", node.title, node.content.as_deref().unwrap_or(""))
            } else {
                format!("[{}] {}\n{}", file_path, node.title, node.content.as_deref().unwrap_or(""))
            };
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
pub(crate) fn create_human_node(
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
pub(crate) fn resolve_node(db: &Database, reference: &str) -> Result<Node, String> {
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
pub(crate) fn handle_connects_to(
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

pub(crate) fn confirm_action(action: &str) -> Result<bool, String> {
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
        CodeCommands::Stale { fix, path } => {
            let db_dir = db_path.parent().unwrap_or(Path::new("."));

            // Get all file paths from code nodes
            let all_paths = db.get_all_code_file_paths().map_err(|e| e.to_string())?;

            let mut stale_files: Vec<(String, usize)> = Vec::new();
            let mut ok_files = 0usize;

            for (file_path, node_count) in &all_paths {
                // Apply path filter if provided
                if let Some(ref prefix) = path {
                    if !file_path.contains(prefix.as_str()) {
                        continue;
                    }
                }

                // Resolve relative paths from the DB directory
                let resolved = if PathBuf::from(file_path).is_relative() {
                    db_dir.join(file_path)
                } else {
                    PathBuf::from(file_path)
                };

                if resolved.exists() {
                    ok_files += 1;
                } else {
                    stale_files.push((file_path.clone(), *node_count));
                }
            }

            if stale_files.is_empty() {
                println!("No stale code nodes found ({} files checked, all exist on disk).", ok_files);
                return Ok(());
            }

            let total_stale_nodes: usize = stale_files.iter().map(|(_, c)| c).sum();

            if fix {
                println!("Cleaning {} stale file(s) ({} nodes)...", stale_files.len(), total_stale_nodes);
                let mut total_deleted = 0;
                for (file_path, _) in &stale_files {
                    match db.delete_nodes_by_file_path(file_path) {
                        Ok(deleted) => {
                            println!("  Deleted {} nodes: {}", deleted, file_path);
                            total_deleted += deleted;
                        }
                        Err(e) => {
                            eprintln!("  Error deleting nodes for {}: {}", file_path, e);
                        }
                    }
                }
                // Prune any dangling edges left behind
                if let Ok(pruned) = db.prune_dead_edges() {
                    if pruned > 0 {
                        println!("  Pruned {} dangling edges", pruned);
                    }
                }
                println!("Done: deleted {} stale nodes from {} missing files ({} files OK).",
                    total_deleted, stale_files.len(), ok_files);
            } else {
                println!("Found {} stale file(s) ({} nodes) — files no longer exist on disk:\n",
                    stale_files.len(), total_stale_nodes);
                for (file_path, node_count) in &stale_files {
                    println!("  {:>4} nodes  {}", node_count, file_path);
                }
                println!("\n{} files OK.", ok_files);
                println!("\nRun with --fix to delete these stale nodes and their edges.");
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
// Utility Functions
// ============================================================================

/// Auto-discover project-specific database by walking up from current directory.
/// Looks for .mycelica.db in each directory up to root.
pub(crate) fn find_project_db() -> Option<PathBuf> {
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

pub(crate) fn find_database() -> PathBuf {
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

pub(crate) fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_runs_list_limit_default_is_zero() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list",
        ]).expect("should parse without --limit");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { limit, .. } } } => {
                assert_eq!(limit, 0, "default limit should be 0 (no limit)");
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_limit_explicit_value() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--limit", "5",
        ]).expect("should parse --limit 5");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { limit, .. } } } => {
                assert_eq!(limit, 5);
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_limit_one() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--limit", "1",
        ]).expect("should parse --limit 1");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { limit, .. } } } => {
                assert_eq!(limit, 1);
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_format_compact() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--format", "compact",
        ]).expect("should parse --format compact");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { format, .. } } } => {
                assert!(matches!(format, DashboardFormat::Compact));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_format_default_is_text() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list",
        ]).expect("should parse without --format");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { format, .. } } } => {
                assert!(matches!(format, DashboardFormat::Text), "default format should be text");
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_format_text_explicit() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--format", "text",
        ]).expect("should parse --format text");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { format, .. } } } => {
                assert!(matches!(format, DashboardFormat::Text));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_format_json() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--format", "json",
        ]).expect("should parse --format json");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { format, .. } } } => {
                assert!(matches!(format, DashboardFormat::Json));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_format_csv() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--format", "csv",
        ]).expect("should parse --format csv");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { format, .. } } } => {
                assert!(matches!(format, DashboardFormat::Csv));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_format_invalid_value() {
        let result = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--format", "xml",
        ]);
        assert!(result.is_err(), "invalid format value should fail");
    }

    #[test]
    fn test_runs_list_format_compact_with_verbose() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--format", "compact", "--verbose",
        ]).expect("should parse --format compact with --verbose");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { format, verbose, .. } } } => {
                assert!(matches!(format, DashboardFormat::Compact));
                assert!(verbose);
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_format_compact_combined_with_all_flags() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list",
            "--format", "compact", "--all", "--cost", "--limit", "5",
        ]).expect("should parse compact with all flags");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { format, all, cost, limit, .. } } } => {
                assert!(matches!(format, DashboardFormat::Compact));
                assert!(all);
                assert!(cost);
                assert_eq!(limit, 5);
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_dashboard_format_compact() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard", "--format", "compact",
        ]).expect("should parse --format compact on dashboard");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { format, .. } } => {
                assert!(matches!(format, DashboardFormat::Compact));
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    #[test]
    fn test_runs_list_limit_invalid_not_a_number() {
        let result = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--limit", "abc",
        ]);
        assert!(result.is_err(), "non-numeric --limit should fail");
    }

    #[test]
    fn test_runs_list_limit_combined_with_other_flags() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--all", "--limit", "10", "--cost",
        ]).expect("should parse --limit with --all and --cost");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { all, cost, limit, .. } } } => {
                assert!(all);
                assert!(cost);
                assert_eq!(limit, 10);
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_limit_zero_preserves_all_rows() {
        let rows: Vec<i32> = vec![1, 2, 3, 4, 5];
        let limit: usize = 0;
        let result: Vec<i32> = if limit > 0 {
            rows.into_iter().take(limit).collect()
        } else {
            rows
        };
        assert_eq!(result, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_limit_truncates_to_n() {
        let rows: Vec<i32> = vec![1, 2, 3, 4, 5];
        let limit: usize = 3;
        let result: Vec<i32> = if limit > 0 {
            rows.into_iter().take(limit).collect()
        } else {
            rows
        };
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn test_limit_larger_than_rows_returns_all() {
        let rows: Vec<i32> = vec![1, 2];
        let limit: usize = 100;
        let result: Vec<i32> = if limit > 0 {
            rows.into_iter().take(limit).collect()
        } else {
            rows
        };
        assert_eq!(result, vec![1, 2]);
    }

    #[test]
    fn test_limit_on_empty_vec() {
        let rows: Vec<i32> = vec![];
        let limit: usize = 5;
        let result: Vec<i32> = if limit > 0 {
            rows.into_iter().take(limit).collect()
        } else {
            rows
        };
        assert!(result.is_empty());
    }

    // --- Tests for --cost flag on 'spore dashboard' ---

    #[test]
    fn test_dashboard_cost_flag_default_false() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard",
        ]).expect("should parse without --cost");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { cost, .. } } => {
                assert!(!cost, "--cost should default to false");
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    #[test]
    fn test_dashboard_cost_flag_true() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard", "--cost",
        ]).expect("should parse --cost");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { cost, .. } } => {
                assert!(cost, "--cost should be true when provided");
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    #[test]
    fn test_dashboard_cost_with_count_flag() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard", "--cost", "--count",
        ]).expect("should parse --cost --count");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { cost, count, .. } } => {
                assert!(cost);
                assert!(count);
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    #[test]
    fn test_dashboard_cost_with_format_json() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard", "--cost", "--format", "json",
        ]).expect("should parse --cost --format json");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { cost, format, .. } } => {
                assert!(cost);
                assert!(matches!(format, DashboardFormat::Json));
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    #[test]
    fn test_dashboard_cost_with_format_csv() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard", "--cost", "--format", "csv",
        ]).expect("should parse --cost --format csv");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { cost, format, .. } } => {
                assert!(cost);
                assert!(matches!(format, DashboardFormat::Csv));
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    #[test]
    fn test_dashboard_cost_with_limit() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard", "--cost", "--limit", "10",
        ]).expect("should parse --cost --limit 10");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { cost, limit, .. } } => {
                assert!(cost);
                assert_eq!(limit, 10);
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    #[test]
    fn test_dashboard_all_flags_combined() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard", "--cost", "--count", "--format", "json", "--limit", "3",
        ]).expect("should parse all dashboard flags together");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { cost, count, format, limit, .. } } => {
                assert!(cost);
                assert!(count);
                assert!(matches!(format, DashboardFormat::Json));
                assert_eq!(limit, 3);
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    // --- Tests for --stale flag on 'spore dashboard' ---

    #[test]
    fn test_dashboard_stale_flag_default_false() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard",
        ]).expect("should parse without --stale");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { stale, .. } } => {
                assert!(!stale, "--stale should default to false");
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    #[test]
    fn test_dashboard_stale_flag_true() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard", "--stale",
        ]).expect("should parse --stale");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { stale, .. } } => {
                assert!(stale, "--stale should be true when provided");
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    #[test]
    fn test_dashboard_stale_with_cost_flag() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard", "--stale", "--cost",
        ]).expect("should parse --stale --cost");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { stale, cost, .. } } => {
                assert!(stale);
                assert!(cost);
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    #[test]
    fn test_dashboard_stale_with_count_flag() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard", "--stale", "--count",
        ]).expect("should parse --stale --count");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { stale, count, .. } } => {
                assert!(stale);
                assert!(count);
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    #[test]
    fn test_dashboard_stale_with_format_json() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard", "--stale", "--format", "json",
        ]).expect("should parse --stale --format json");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { stale, format, .. } } => {
                assert!(stale);
                assert!(matches!(format, DashboardFormat::Json));
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    #[test]
    fn test_dashboard_stale_with_format_csv() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard", "--stale", "--format", "csv",
        ]).expect("should parse --stale --format csv");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { stale, format, .. } } => {
                assert!(stale);
                assert!(matches!(format, DashboardFormat::Csv));
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    #[test]
    fn test_runs_list_status_filter_parses() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--status", "verified,escalated",
        ]).expect("should parse --status verified,escalated");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { status, .. } } } => {
                assert_eq!(status, Some("verified,escalated".to_string()));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_status_filter_default_none() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list",
        ]).expect("should parse without --status");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { status, .. } } } => {
                assert_eq!(status, None);
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    // --- Tests for `spore runs list --duration` flag ---

    #[test]
    fn test_runs_list_duration_default_none() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list",
        ]).expect("should parse without --duration");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { duration, .. } } } => {
                assert_eq!(duration, None, "default duration should be None");
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_duration_explicit_value() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--duration", "120",
        ]).expect("should parse --duration 120");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { duration, .. } } } => {
                assert_eq!(duration, Some(120));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_duration_invalid_not_a_number() {
        let result = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--duration", "abc",
        ]);
        assert!(result.is_err(), "--duration abc should fail to parse");
    }

    #[test]
    fn test_runs_list_duration_zero() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--duration", "0",
        ]).expect("should parse --duration 0");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { duration, .. } } } => {
                assert_eq!(duration, Some(0), "--duration 0 should be Some(0)");
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_duration_equals_syntax() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--duration=300",
        ]).expect("should parse --duration=300 (equals syntax)");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { duration, .. } } } => {
                assert_eq!(duration, Some(300));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_duration_negative_rejected() {
        let result = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--duration", "-10",
        ]);
        assert!(result.is_err(), "negative --duration should be rejected (u64 type)");
    }

    #[test]
    fn test_runs_list_duration_combined_with_other_flags() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list",
            "--duration", "120",
            "--status", "verified",
            "--since", "7d",
            "--limit", "10",
            "--format", "json",
        ]).expect("should parse --duration combined with --status, --since, --limit, --format");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { duration, status, since, limit, format, .. } } } => {
                assert_eq!(duration, Some(120));
                assert_eq!(status, Some("verified".to_string()));
                assert_eq!(since, Some("7d".to_string()));
                assert_eq!(limit, 10);
                assert!(matches!(format, DashboardFormat::Json));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_duration_large_value() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--duration", "86400",
        ]).expect("should parse --duration 86400 (24 hours)");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { duration, .. } } } => {
                assert_eq!(duration, Some(86400));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    /// Tests the duration filtering logic: seconds → milliseconds conversion
    /// and comparison with `unwrap_or(0)` for missing duration values.
    #[test]
    fn test_duration_filter_logic() {
        // Simulate the filtering logic from handle_runs (spore.rs ~line 2681)
        struct FakeRun { total_duration_ms: Option<u64> }

        let runs = vec![
            FakeRun { total_duration_ms: Some(180_000) }, // 3 minutes
            FakeRun { total_duration_ms: Some(60_000) },  // 1 minute
            FakeRun { total_duration_ms: Some(300_000) }, // 5 minutes
            FakeRun { total_duration_ms: None },          // unknown duration
            FakeRun { total_duration_ms: Some(0) },       // zero duration
        ];

        // Filter: --duration 120 (2 minutes = 120_000ms)
        let min_secs: u64 = 120;
        let min_ms = min_secs * 1000;
        let filtered: Vec<_> = runs.iter()
            .filter(|r| r.total_duration_ms.unwrap_or(0) >= min_ms)
            .collect();
        assert_eq!(filtered.len(), 2, "only 3min and 5min runs should pass");
        assert_eq!(filtered[0].total_duration_ms, Some(180_000));
        assert_eq!(filtered[1].total_duration_ms, Some(300_000));

        // Filter: --duration 0 (should pass everything since all >= 0)
        let min_secs: u64 = 0;
        let min_ms = min_secs * 1000;
        let filtered: Vec<_> = runs.iter()
            .filter(|r| r.total_duration_ms.unwrap_or(0) >= min_ms)
            .collect();
        assert_eq!(filtered.len(), 5, "--duration 0 should pass all runs");
    }

    /// Tests that runs with None duration are excluded by any non-zero filter,
    /// since unwrap_or(0) < any positive threshold.
    #[test]
    fn test_duration_filter_excludes_none_duration() {
        struct FakeRun { total_duration_ms: Option<u64> }

        let runs = vec![
            FakeRun { total_duration_ms: None },
            FakeRun { total_duration_ms: None },
            FakeRun { total_duration_ms: Some(1000) },
        ];

        let min_secs: u64 = 1; // 1 second = 1000ms
        let min_ms = min_secs * 1000;
        let filtered: Vec<_> = runs.iter()
            .filter(|r| r.total_duration_ms.unwrap_or(0) >= min_ms)
            .collect();
        assert_eq!(filtered.len(), 1, "only the 1000ms run should pass; None durations should be excluded");
        assert_eq!(filtered[0].total_duration_ms, Some(1000));
    }

    // --- Tests for `spore runs list --agent` flag ---

    #[test]
    fn test_runs_list_agent_full_name() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--agent", "spore:coder",
        ]).expect("should parse --agent spore:coder");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { agent, .. } } } => {
                assert_eq!(agent, Some("spore:coder".to_string()));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    /// Tests the agent filtering logic: short name normalization and matching.
    #[test]
    fn test_agent_filter_logic() {
        let agents_a = vec!["spore:coder".to_string(), "spore:verifier".to_string()];
        let agents_b = vec!["spore:researcher".to_string()];
        let agents_c: Vec<String> = vec![];

        // Short name "coder" → matches "spore:coder"
        let needle = format!("spore:{}", "coder");
        assert!(agents_a.iter().any(|a| a.to_lowercase() == needle));
        assert!(!agents_b.iter().any(|a| a.to_lowercase() == needle));
        assert!(!agents_c.iter().any(|a| a.to_lowercase() == needle));

        // Full name "spore:researcher" → matches directly
        let needle = "spore:researcher".to_lowercase();
        assert!(!agents_a.iter().any(|a| a.to_lowercase() == needle));
        assert!(agents_b.iter().any(|a| a.to_lowercase() == needle));

        // Case insensitive
        let needle = format!("spore:{}", "Coder".to_lowercase());
        assert!(agents_a.iter().any(|a| a.to_lowercase() == needle));
    }

    // --- Tests for `spore runs top --limit` flag ---

    #[test]
    fn test_runs_top_limit_default_is_five() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "top",
        ]).expect("should parse without --limit");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::Top { limit } } } => {
                assert_eq!(limit, 5, "default limit should be 5");
            }
            _ => panic!("expected Spore > Runs > Top"),
        }
    }

    #[test]
    fn test_runs_top_limit_explicit_value() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "top", "--limit", "10",
        ]).expect("should parse --limit 10");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::Top { limit } } } => {
                assert_eq!(limit, 10);
            }
            _ => panic!("expected Spore > Runs > Top"),
        }
    }

    #[test]
    fn test_runs_top_limit_one() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "top", "--limit", "1",
        ]).expect("should parse --limit 1");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::Top { limit } } } => {
                assert_eq!(limit, 1);
            }
            _ => panic!("expected Spore > Runs > Top"),
        }
    }

    #[test]
    fn test_runs_top_limit_invalid_not_a_number() {
        let result = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "top", "--limit", "abc",
        ]);
        assert!(result.is_err(), "non-numeric --limit should fail");
    }

    #[test]
    fn test_runs_top_limit_large_value() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "top", "--limit", "100",
        ]).expect("should parse --limit 100");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::Top { limit } } } => {
                assert_eq!(limit, 100);
            }
            _ => panic!("expected Spore > Runs > Top"),
        }
    }

    #[test]
    fn test_runs_top_limit_zero() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "top", "--limit", "0",
        ]).expect("should parse --limit 0");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::Top { limit } } } => {
                assert_eq!(limit, 0);
            }
            _ => panic!("expected Spore > Runs > Top"),
        }
    }

    #[test]
    fn test_runs_top_truncate_with_limit() {
        // Simulates the truncate(limit) behavior in handle_runs Top handler
        let mut entries = vec![5.0, 4.0, 3.0, 2.0, 1.0];
        let limit: usize = 3;
        entries.truncate(limit);
        assert_eq!(entries, vec![5.0, 4.0, 3.0]);
    }

    #[test]
    fn test_runs_top_truncate_with_default_limit() {
        let mut entries = vec![10.0, 9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0];
        let limit: usize = 5; // default
        entries.truncate(limit);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries, vec![10.0, 9.0, 8.0, 7.0, 6.0]);
    }

    #[test]
    fn test_runs_top_truncate_limit_larger_than_entries() {
        let mut entries = vec![3.0, 2.0, 1.0];
        let limit: usize = 10;
        entries.truncate(limit);
        assert_eq!(entries, vec![3.0, 2.0, 1.0], "should keep all when limit > count");
    }

    #[test]
    fn test_runs_top_truncate_limit_zero_clears() {
        let mut entries = vec![3.0, 2.0, 1.0];
        let limit: usize = 0;
        entries.truncate(limit);
        assert!(entries.is_empty(), "truncate(0) should clear all entries");
    }

    #[test]
    fn test_dashboard_all_flags_with_stale() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "dashboard", "--cost", "--count", "--stale", "--format", "json", "--limit", "3",
        ]).expect("should parse all dashboard flags including --stale");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Dashboard { cost, count, stale, format, limit } } => {
                assert!(cost);
                assert!(count);
                assert!(stale);
                assert!(matches!(format, DashboardFormat::Json));
                assert_eq!(limit, 3);
            }
            _ => panic!("expected Spore > Dashboard"),
        }
    }

    // --- Tests for 'spore runs stats' subcommand ---

    #[test]
    fn test_runs_stats_parses() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "stats",
        ]).expect("should parse 'spore runs stats'");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::Stats { .. } } } => {}
            _ => panic!("expected Spore > Runs > Stats"),
        }
    }

    #[test]
    fn test_runs_stats_rejects_positional_args() {
        let result = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "stats", "extra",
        ]);
        assert!(result.is_err(), "stats should not accept positional args");
    }

    // --- Tests for --agent flag on 'spore runs list' ---

    #[test]
    fn test_runs_list_agent_default_none() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list",
        ]).expect("should parse without --agent");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { agent, .. } } } => {
                assert!(agent.is_none(), "--agent should default to None");
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_agent_short_name() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--agent", "researcher",
        ]).expect("should parse --agent researcher");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { agent, .. } } } => {
                assert_eq!(agent, Some("researcher".to_string()));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_agent_qualified_name() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--agent", "spore:coder",
        ]).expect("should parse --agent spore:coder");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { agent, .. } } } => {
                assert_eq!(agent, Some("spore:coder".to_string()));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_agent_combined_with_other_flags() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list",
            "--agent", "coder", "--limit", "5", "--cost", "--status", "verified",
        ]).expect("should parse --agent with other flags");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { agent, limit, cost, status, .. } } } => {
                assert_eq!(agent, Some("coder".to_string()));
                assert_eq!(limit, 5);
                assert!(cost);
                assert_eq!(status, Some("verified".to_string()));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_agent_requires_value() {
        let result = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--agent",
        ]);
        assert!(result.is_err(), "--agent without a value should fail");
    }

    // --- Tests for agent filter normalization logic ---

    #[test]
    fn test_agent_filter_normalizes_short_name() {
        // Replicates the normalization logic from handle_runs
        let agent_name = "researcher";
        let needle = if agent_name.contains(':') {
            agent_name.to_lowercase()
        } else {
            format!("spore:{}", agent_name.to_lowercase())
        };
        assert_eq!(needle, "spore:researcher");
    }

    #[test]
    fn test_agent_filter_preserves_qualified_name() {
        let agent_name = "spore:coder";
        let needle = if agent_name.contains(':') {
            agent_name.to_lowercase()
        } else {
            format!("spore:{}", agent_name.to_lowercase())
        };
        assert_eq!(needle, "spore:coder");
    }

    #[test]
    fn test_agent_filter_case_insensitive() {
        let agent_name = "Researcher";
        let needle = if agent_name.contains(':') {
            agent_name.to_lowercase()
        } else {
            format!("spore:{}", agent_name.to_lowercase())
        };
        assert_eq!(needle, "spore:researcher");

        // Simulates matching against agents list
        let agents = vec!["spore:Researcher".to_string()];
        let matches = agents.iter().any(|a| a.to_lowercase() == needle);
        assert!(matches, "should match case-insensitively");
    }

    #[test]
    fn test_agent_filter_qualified_case_insensitive() {
        let agent_name = "Spore:Verifier";
        let needle = if agent_name.contains(':') {
            agent_name.to_lowercase()
        } else {
            format!("spore:{}", agent_name.to_lowercase())
        };
        assert_eq!(needle, "spore:verifier");
    }

    #[test]
    fn test_agent_filter_matches_in_agents_list() {
        let agent_filter = "coder";
        let needle = if agent_filter.contains(':') {
            agent_filter.to_lowercase()
        } else {
            format!("spore:{}", agent_filter.to_lowercase())
        };

        struct FakeRun { agents: Vec<String> }
        let runs = vec![
            FakeRun { agents: vec!["spore:coder".to_string(), "spore:verifier".to_string()] },
            FakeRun { agents: vec!["spore:researcher".to_string()] },
            FakeRun { agents: vec!["spore:coder".to_string()] },
            FakeRun { agents: vec![] },
        ];

        let filtered: Vec<&FakeRun> = runs.iter().filter(|r| {
            r.agents.iter().any(|a| a.to_lowercase() == needle)
        }).collect();

        assert_eq!(filtered.len(), 2, "should keep only runs with a matching agent");
        assert!(filtered[0].agents.contains(&"spore:coder".to_string()));
        assert!(filtered[1].agents.contains(&"spore:coder".to_string()));
    }

    #[test]
    fn test_agent_filter_no_match_returns_empty() {
        let agent_filter = "planner";
        let needle = if agent_filter.contains(':') {
            agent_filter.to_lowercase()
        } else {
            format!("spore:{}", agent_filter.to_lowercase())
        };

        struct FakeRun { agents: Vec<String> }
        let runs = vec![
            FakeRun { agents: vec!["spore:coder".to_string()] },
            FakeRun { agents: vec!["spore:researcher".to_string()] },
        ];

        let filtered: Vec<&FakeRun> = runs.iter().filter(|r| {
            r.agents.iter().any(|a| a.to_lowercase() == needle)
        }).collect();

        assert!(filtered.is_empty(), "no runs should match 'planner'");
    }

    #[test]
    fn test_agent_filter_empty_agents_list() {
        let agent_filter = "coder";
        let needle = format!("spore:{}", agent_filter.to_lowercase());

        struct FakeRun { agents: Vec<String> }
        let runs = vec![
            FakeRun { agents: vec![] },
        ];

        let filtered: Vec<&FakeRun> = runs.iter().filter(|r| {
            r.agents.iter().any(|a| a.to_lowercase() == needle)
        }).collect();

        assert!(filtered.is_empty(), "run with no agents should not match");
    }

    #[test]
    fn test_agent_filter_none_skips_filtering() {
        // When --agent is not provided, all runs are kept
        let agent_filter: Option<String> = None;

        let runs = vec![1, 2, 3, 4, 5];
        let result: Vec<i32> = if agent_filter.is_some() {
            runs.into_iter().filter(|_| false).collect() // would filter
        } else {
            runs
        };
        assert_eq!(result.len(), 5, "None filter should keep all runs");
    }

    // --- Tests for agent filter edge cases (tester agent) ---

    #[test]
    fn test_agent_filter_partial_name_does_not_match() {
        // "code" should NOT match "spore:coder" — exact match required
        let agent_filter = "code";
        let needle = format!("spore:{}", agent_filter.to_lowercase());
        assert_eq!(needle, "spore:code");

        struct FakeRun { agents: Vec<String> }
        let runs = vec![
            FakeRun { agents: vec!["spore:coder".to_string()] },
        ];
        let filtered: Vec<&FakeRun> = runs.iter().filter(|r| {
            r.agents.iter().any(|a| a.to_lowercase() == needle)
        }).collect();
        assert!(filtered.is_empty(), "partial name 'code' must not match 'spore:coder'");
    }

    #[test]
    fn test_agent_filter_non_spore_namespace() {
        // "other:coder" should be preserved as-is (contains ':')
        let agent_name = "other:coder";
        let needle = if agent_name.contains(':') {
            agent_name.to_lowercase()
        } else {
            format!("spore:{}", agent_name.to_lowercase())
        };
        assert_eq!(needle, "other:coder", "non-spore namespace should be preserved");

        // It should not match spore:coder
        let agents = vec!["spore:coder".to_string()];
        assert!(!agents.iter().any(|a| a.to_lowercase() == needle));
    }

    #[test]
    fn test_runs_list_agent_equals_syntax() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--agent=verifier",
        ]).expect("should parse --agent=verifier");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { agent, .. } } } => {
                assert_eq!(agent, Some("verifier".to_string()));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_agent_with_all_flag() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--all", "--agent", "coder",
        ]).expect("should parse --all --agent coder");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { all, agent, .. } } } => {
                assert!(all);
                assert_eq!(agent, Some("coder".to_string()));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_agent_and_duration_filters_combined() {
        // Both filters should apply: agent narrows by agent, duration narrows by time
        struct FakeRun { agents: Vec<String>, total_duration_ms: Option<u64> }

        let runs = vec![
            FakeRun { agents: vec!["spore:coder".to_string()], total_duration_ms: Some(300_000) },
            FakeRun { agents: vec!["spore:coder".to_string()], total_duration_ms: Some(30_000) },
            FakeRun { agents: vec!["spore:researcher".to_string()], total_duration_ms: Some(300_000) },
            FakeRun { agents: vec!["spore:researcher".to_string()], total_duration_ms: Some(30_000) },
        ];

        let needle = "spore:coder".to_string();
        let min_ms: u64 = 120 * 1000;

        // Apply duration filter first, then agent filter (mirrors handle_runs order)
        let after_duration: Vec<&FakeRun> = runs.iter()
            .filter(|r| r.total_duration_ms.unwrap_or(0) >= min_ms)
            .collect();
        assert_eq!(after_duration.len(), 2, "duration filter should keep 2 runs");

        let after_agent: Vec<&&FakeRun> = after_duration.iter()
            .filter(|r| r.agents.iter().any(|a| a.to_lowercase() == needle))
            .collect();
        assert_eq!(after_agent.len(), 1, "combined filters should keep only coder run with long duration");
        assert!(after_agent[0].agents.contains(&"spore:coder".to_string()));
        assert_eq!(after_agent[0].total_duration_ms, Some(300_000));
    }

    #[test]
    fn test_agent_filter_multiple_agents_per_run() {
        // A run with multiple agents should match if ANY agent matches
        let needle = "spore:verifier".to_string();

        struct FakeRun { agents: Vec<String> }
        let runs = vec![
            FakeRun { agents: vec![
                "spore:coder".to_string(),
                "spore:verifier".to_string(),
                "spore:tester".to_string(),
            ]},
        ];

        let filtered: Vec<&FakeRun> = runs.iter().filter(|r| {
            r.agents.iter().any(|a| a.to_lowercase() == needle)
        }).collect();
        assert_eq!(filtered.len(), 1, "should match when verifier is one of multiple agents");
    }

    #[test]
    fn test_runs_list_agent_with_duration_flag() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list",
            "--agent", "tester", "--duration", "60",
        ]).expect("should parse --agent with --duration");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { agent, duration, .. } } } => {
                assert_eq!(agent, Some("tester".to_string()));
                assert_eq!(duration, Some(60));
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    // --- Tests for 'spore runs timeline' subcommand ---

    #[test]
    fn test_runs_timeline_parses_run_id() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "timeline", "abc12345",
        ]).expect("should parse timeline with run_id");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::Timeline { run_id } } } => {
                assert_eq!(run_id, "abc12345");
            }
            _ => panic!("expected Spore > Runs > Timeline"),
        }
    }

    #[test]
    fn test_runs_timeline_parses_full_uuid() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "timeline",
            "deb1da1e-391e-4216-9690-0ad0f8c88070",
        ]).expect("should parse timeline with full UUID");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::Timeline { run_id } } } => {
                assert_eq!(run_id, "deb1da1e-391e-4216-9690-0ad0f8c88070");
            }
            _ => panic!("expected Spore > Runs > Timeline"),
        }
    }

    #[test]
    fn test_runs_timeline_missing_run_id_fails() {
        let result = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "timeline",
        ]);
        assert!(result.is_err(), "timeline without run_id should fail");
    }

    #[test]
    fn test_runs_timeline_wall_duration_single_phase() {
        // When there's only one phase, wall duration = that phase's duration
        let total_duration_ms: u64 = 45_000;
        let phases_len = 1;
        let first_ts = 1000_i64;
        let last_ts = 1000_i64;
        let last_phase_duration_ms: u64 = 45_000;

        let wall_duration_secs = if phases_len >= 2 {
            let last_dur = last_phase_duration_ms;
            ((last_ts - first_ts) as u64 + last_dur) / 1000
        } else {
            total_duration_ms / 1000
        };
        assert_eq!(wall_duration_secs, 45);
    }

    #[test]
    fn test_runs_timeline_wall_duration_multiple_phases() {
        // Two phases: first at t=0, second at t=120_000ms with 30s duration
        // Wall = (120_000 - 0 + 30_000) / 1000 = 150s
        let phases_len = 2;
        let first_ts: i64 = 0;
        let last_ts: i64 = 120_000;
        let last_phase_duration_ms: u64 = 30_000;

        let wall_duration_secs = if phases_len >= 2 {
            ((last_ts - first_ts) as u64 + last_phase_duration_ms) / 1000
        } else {
            unreachable!();
        };
        assert_eq!(wall_duration_secs, 150);
        assert_eq!(wall_duration_secs / 60, 2);
        assert_eq!(wall_duration_secs % 60, 30);
    }

    #[test]
    fn test_runs_timeline_wall_duration_last_phase_no_duration() {
        // When last phase has no duration_ms, last_dur defaults to 0
        let phases_len = 3;
        let first_ts: i64 = 1_000_000;
        let last_ts: i64 = 1_300_000;
        let last_phase_duration_ms: u64 = 0; // no duration recorded

        let wall_duration_secs = if phases_len >= 2 {
            ((last_ts - first_ts) as u64 + last_phase_duration_ms) / 1000
        } else {
            unreachable!();
        };
        assert_eq!(wall_duration_secs, 300); // 5 minutes
    }

    #[test]
    fn test_runs_timeline_phase_duration_seconds_format() {
        // Duration < 60s should display as "{secs}s"
        let duration_ms: u64 = 45_000;
        let secs = duration_ms / 1000;
        assert!(secs < 60);
        let display = format!("Duration: {}s", secs);
        assert_eq!(display, "Duration: 45s");
    }

    #[test]
    fn test_runs_timeline_phase_duration_minutes_format() {
        // Duration >= 60s should display as "{min}m {sec}s"
        let duration_ms: u64 = 135_000;
        let secs = duration_ms / 1000;
        assert!(secs >= 60);
        let display = format!("Duration: {}m {}s", secs / 60, secs % 60);
        assert_eq!(display, "Duration: 2m 15s");
    }

    #[test]
    fn test_runs_timeline_phase_duration_exact_minute() {
        let duration_ms: u64 = 60_000;
        let secs = duration_ms / 1000;
        assert!(secs >= 60);
        let display = format!("Duration: {}m {}s", secs / 60, secs % 60);
        assert_eq!(display, "Duration: 1m 0s");
    }

    #[test]
    fn test_runs_timeline_cost_accumulation() {
        // Test that cost accumulates correctly across phases
        let costs: Vec<Option<f64>> = vec![Some(0.15), None, Some(0.25), Some(0.10)];
        let mut total_cost = 0.0_f64;
        for c in &costs {
            if let Some(v) = c { total_cost += v; }
        }
        assert!((total_cost - 0.50).abs() < 1e-10);
    }

    #[test]
    fn test_runs_timeline_turns_accumulation() {
        // Test turns from either "turns" or "num_turns" keys
        let metas: Vec<serde_json::Value> = vec![
            serde_json::json!({"turns": 5}),
            serde_json::json!({"num_turns": 8}),
            serde_json::json!({"other": "no turns"}),
            serde_json::json!({"turns": 3}),
        ];
        let mut total_turns = 0_u64;
        for meta in &metas {
            let turns = meta["turns"].as_u64().or_else(|| meta["num_turns"].as_u64());
            if let Some(t) = turns { total_turns += t; }
        }
        assert_eq!(total_turns, 16);
    }

    #[test]
    fn test_runs_timeline_agent_strip_prefix() {
        // Agent names with "spore:" prefix should have it stripped for display
        let agent = "spore:coder";
        let short = agent.strip_prefix("spore:").unwrap_or(agent);
        assert_eq!(short, "coder");

        let agent2 = "external-agent";
        let short2 = agent2.strip_prefix("spore:").unwrap_or(agent2);
        assert_eq!(short2, "external-agent");
    }

    #[test]
    fn test_runs_timeline_task_desc_truncation_short() {
        // Descriptions <= 60 chars should not be truncated
        let task_desc = "Short task description";
        let short_desc = if task_desc.chars().count() > 60 {
            format!("{}...", &task_desc[..57])
        } else {
            task_desc.to_string()
        };
        assert_eq!(short_desc, "Short task description");
    }

    #[test]
    fn test_runs_timeline_task_desc_truncation_long() {
        // Descriptions > 60 chars should be truncated to 57 + "..."
        let task_desc = "This is a very long task description that exceeds sixty characters limit and should be truncated";
        assert!(task_desc.chars().count() > 60);
        let short_desc = if task_desc.chars().count() > 60 {
            format!("{}...", &task_desc[..57])
        } else {
            task_desc.to_string()
        };
        assert_eq!(short_desc.len(), 60);
        assert!(short_desc.ends_with("..."));
    }

    #[test]
    fn test_runs_timeline_orchestration_prefix_strip() {
        // Task titles with "Orchestration:" prefix should have it stripped
        let title = "Orchestration: Build the widget";
        let desc = title.strip_prefix("Orchestration:").unwrap_or(title).trim();
        assert_eq!(desc, "Build the widget");

        // Titles without the prefix should be unchanged
        let title2 = "Regular task title";
        let desc2 = title2.strip_prefix("Orchestration:").unwrap_or(title2).trim();
        assert_eq!(desc2, "Regular task title");
    }

    #[test]
    fn test_runs_timeline_timestamp_formatting() {
        // Verify timestamp millis -> HH:MM:SS formatting
        let ts: i64 = 1708300800000; // 2024-02-19 00:00:00 UTC
        let time_str = chrono::DateTime::from_timestamp_millis(ts)
            .map(|d| d.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| "??:??:??".to_string());
        assert_eq!(time_str, "00:00:00");
    }

    #[test]
    fn test_runs_timeline_timestamp_invalid() {
        // Invalid timestamp should fall back to "??:??:??"
        // Note: from_timestamp_millis returns None for out-of-range values
        let time_str = chrono::DateTime::from_timestamp_millis(i64::MAX)
            .map(|d| d.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| "??:??:??".to_string());
        assert_eq!(time_str, "??:??:??");
    }

    #[test]
    fn test_runs_timeline_connector_chars_last_vs_middle() {
        // Last phase uses "\-" prefix, middle phases use "+-"
        let phase_count = 3;
        for i in 0..phase_count {
            let is_last = i == phase_count - 1;
            let connector = if is_last { "\\-" } else { "+-" };
            let pipe = if is_last { " " } else { "|" };
            if i < phase_count - 1 {
                assert_eq!(connector, "+-");
                assert_eq!(pipe, "|");
            } else {
                assert_eq!(connector, "\\-");
                assert_eq!(pipe, " ");
            }
        }
    }

    #[test]
    fn test_runs_timeline_bounce_display() {
        // Bounce number should format as " (bounce N)" or empty
        let bounce: Option<u64> = Some(2);
        let bounce_str = bounce.map(|b| format!(" (bounce {})", b)).unwrap_or_default();
        assert_eq!(bounce_str, " (bounce 2)");

        let no_bounce: Option<u64> = None;
        let no_bounce_str = no_bounce.map(|b| format!(" (bounce {})", b)).unwrap_or_default();
        assert_eq!(no_bounce_str, "");
    }

    #[test]
    fn test_runs_timeline_details_join() {
        // Multiple details should be joined with " | "
        let mut details = Vec::new();
        details.push(format!("Turns: {}", 5));
        details.push(format!("Duration: {}s", 45));
        details.push(format!("Cost: ${:.2}", 0.15));
        let joined = details.join(" | ");
        assert_eq!(joined, "Turns: 5 | Duration: 45s | Cost: $0.15");
    }

    #[test]
    fn test_runs_timeline_footer_format() {
        // Test the footer total line format
        let phase_count = 4;
        let total_turns = 18_u64;
        let wall_duration_secs = 310_u64; // 5m 10s
        let total_cost = 1.23_f64;
        let footer = format!("Total: {} phases | {} turns | {}m {}s | ${:.2}",
            phase_count, total_turns,
            wall_duration_secs / 60, wall_duration_secs % 60,
            total_cost);
        assert_eq!(footer, "Total: 4 phases | 18 turns | 5m 10s | $1.23");
    }

    // --- Tests for --cost sort on 'spore runs list' ---

    #[test]
    fn test_runs_list_cost_flag_alone() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list", "--cost",
        ]).expect("should parse --cost alone");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { cost, .. } } } => {
                assert!(cost, "--cost should be true");
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_runs_list_cost_flag_default_false() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "list",
        ]).expect("should parse without --cost");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::List { cost, .. } } } => {
                assert!(!cost, "--cost should default to false");
            }
            _ => panic!("expected Spore > Runs > List"),
        }
    }

    #[test]
    fn test_cost_sort_descending_order() {
        // Mirrors the sorting logic in handle_runs when show_cost=true
        struct FakeRun { id: &'static str, total_cost: Option<f64> }

        let runs = vec![
            FakeRun { id: "cheap",   total_cost: Some(0.05) },
            FakeRun { id: "mid",     total_cost: Some(0.50) },
            FakeRun { id: "costly",  total_cost: Some(2.10) },
        ];

        let mut sorted = runs;
        sorted.sort_by(|a, b| {
            let cost_a = a.total_cost.unwrap_or(0.0);
            let cost_b = b.total_cost.unwrap_or(0.0);
            cost_b.partial_cmp(&cost_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        assert_eq!(sorted[0].id, "costly");
        assert_eq!(sorted[1].id, "mid");
        assert_eq!(sorted[2].id, "cheap");
    }

    #[test]
    fn test_cost_sort_none_costs_sort_to_bottom() {
        struct FakeRun { id: &'static str, total_cost: Option<f64> }

        let runs = vec![
            FakeRun { id: "no_cost",  total_cost: None },
            FakeRun { id: "has_cost", total_cost: Some(0.10) },
            FakeRun { id: "no_cost2", total_cost: None },
        ];

        let mut sorted = runs;
        sorted.sort_by(|a, b| {
            let cost_a = a.total_cost.unwrap_or(0.0);
            let cost_b = b.total_cost.unwrap_or(0.0);
            cost_b.partial_cmp(&cost_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        assert_eq!(sorted[0].id, "has_cost");
        // None costs (treated as 0.0) should be after any run with positive cost
        assert!(sorted[0].total_cost.is_some());
        assert!(sorted[1].total_cost.is_none() || sorted[1].total_cost == Some(0.0));
        assert!(sorted[2].total_cost.is_none() || sorted[2].total_cost == Some(0.0));
    }

    #[test]
    fn test_cost_sort_all_none_costs() {
        // When all costs are None, sort should not panic and order is stable
        struct FakeRun { id: &'static str, total_cost: Option<f64> }

        let runs = vec![
            FakeRun { id: "a", total_cost: None },
            FakeRun { id: "b", total_cost: None },
            FakeRun { id: "c", total_cost: None },
        ];

        let mut sorted = runs;
        sorted.sort_by(|a, b| {
            let cost_a = a.total_cost.unwrap_or(0.0);
            let cost_b = b.total_cost.unwrap_or(0.0);
            cost_b.partial_cmp(&cost_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Stable sort: order should be preserved when all equal
        assert_eq!(sorted[0].id, "a");
        assert_eq!(sorted[1].id, "b");
        assert_eq!(sorted[2].id, "c");
    }

    #[test]
    fn test_cost_sort_no_sort_when_flag_false() {
        // Without --cost, order should be preserved (chronological)
        struct FakeRun { id: &'static str, total_cost: Option<f64> }

        let runs = vec![
            FakeRun { id: "oldest", total_cost: Some(5.00) },
            FakeRun { id: "middle", total_cost: Some(0.10) },
            FakeRun { id: "newest", total_cost: Some(1.00) },
        ];

        let show_cost = false;
        let result: Vec<&str> = if show_cost {
            let mut sorted = runs;
            sorted.sort_by(|a, b| {
                let cost_a = a.total_cost.unwrap_or(0.0);
                let cost_b = b.total_cost.unwrap_or(0.0);
                cost_b.partial_cmp(&cost_a).unwrap_or(std::cmp::Ordering::Equal)
            });
            sorted.iter().map(|r| r.id).collect()
        } else {
            runs.iter().map(|r| r.id).collect()
        };

        // Original order should be preserved
        assert_eq!(result, vec!["oldest", "middle", "newest"]);
    }

    #[test]
    fn test_cost_sort_after_filters_applied() {
        // --cost sort should happen AFTER agent and duration filters
        struct FakeRun { id: &'static str, total_cost: Option<f64>, agents: Vec<String>, total_duration_ms: Option<u64> }

        let runs = vec![
            FakeRun { id: "expensive_coder",  total_cost: Some(3.00), agents: vec!["spore:coder".into()],      total_duration_ms: Some(120_000) },
            FakeRun { id: "cheap_researcher",  total_cost: Some(0.10), agents: vec!["spore:researcher".into()], total_duration_ms: Some(60_000) },
            FakeRun { id: "mid_coder",         total_cost: Some(1.50), agents: vec!["spore:coder".into()],      total_duration_ms: Some(300_000) },
            FakeRun { id: "cheap_coder_short", total_cost: Some(0.05), agents: vec!["spore:coder".into()],      total_duration_ms: Some(30_000) },
        ];

        // Apply duration filter (>= 60s)
        let min_ms: u64 = 60 * 1000;
        let filtered: Vec<&FakeRun> = runs.iter()
            .filter(|r| r.total_duration_ms.unwrap_or(0) >= min_ms)
            .collect();

        // Apply agent filter
        let needle = "spore:coder".to_string();
        let mut filtered: Vec<&FakeRun> = filtered.into_iter()
            .filter(|r| r.agents.iter().any(|a| a.to_lowercase() == needle))
            .collect();

        // Apply cost sort
        filtered.sort_by(|a, b| {
            let cost_a = a.total_cost.unwrap_or(0.0);
            let cost_b = b.total_cost.unwrap_or(0.0);
            cost_b.partial_cmp(&cost_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Should only have coder runs with duration >= 60s, sorted by cost desc
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].id, "expensive_coder");
        assert_eq!(filtered[1].id, "mid_coder");
    }

    #[test]
    fn test_cost_sort_equal_costs_stable() {
        // Runs with equal costs should maintain their relative order (stable sort)
        struct FakeRun { id: &'static str, total_cost: Option<f64> }

        let runs = vec![
            FakeRun { id: "first",  total_cost: Some(1.00) },
            FakeRun { id: "second", total_cost: Some(1.00) },
            FakeRun { id: "third",  total_cost: Some(1.00) },
        ];

        let mut sorted = runs;
        sorted.sort_by(|a, b| {
            let cost_a = a.total_cost.unwrap_or(0.0);
            let cost_b = b.total_cost.unwrap_or(0.0);
            cost_b.partial_cmp(&cost_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        assert_eq!(sorted[0].id, "first");
        assert_eq!(sorted[1].id, "second");
        assert_eq!(sorted[2].id, "third");
    }

    #[test]
    fn test_runs_summary_parses() {
        let cli = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "summary",
        ]).expect("should parse 'spore runs summary'");
        match cli.command {
            Commands::Spore { cmd: SporeCommands::Runs { cmd: RunCommands::Summary } } => {}
            _ => panic!("expected Spore > Runs > Summary"),
        }
    }

    #[test]
    fn test_runs_summary_rejects_positional_args() {
        let result = Cli::try_parse_from([
            "mycelica-cli", "spore", "runs", "summary", "extra",
        ]);
        assert!(result.is_err(), "summary should not accept positional args");
    }

    // --- Tests for 'spore runs summary' prose-building logic ---

    #[test]
    fn test_summary_status_parts_ordering() {
        // The summary iterates statuses in a fixed order: verified, implemented, pending, escalated, cancelled
        let mut status_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        status_counts.insert("cancelled".to_string(), 1);
        status_counts.insert("verified".to_string(), 5);
        status_counts.insert("pending".to_string(), 2);
        status_counts.insert("escalated".to_string(), 1);
        status_counts.insert("implemented".to_string(), 3);

        let mut status_parts: Vec<String> = Vec::new();
        for s in &["verified", "implemented", "pending", "escalated", "cancelled"] {
            if let Some(c) = status_counts.get(*s) {
                status_parts.push(format!("{} {}", c, s));
            }
        }

        assert_eq!(status_parts, vec![
            "5 verified", "3 implemented", "2 pending", "1 escalated", "1 cancelled"
        ]);
    }

    #[test]
    fn test_summary_status_parts_skips_missing_statuses() {
        // Only present statuses should appear in the output
        let mut status_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        status_counts.insert("verified".to_string(), 3);
        status_counts.insert("pending".to_string(), 1);

        let mut status_parts: Vec<String> = Vec::new();
        for s in &["verified", "implemented", "pending", "escalated", "cancelled"] {
            if let Some(c) = status_counts.get(*s) {
                status_parts.push(format!("{} {}", c, s));
            }
        }

        assert_eq!(status_parts, vec!["3 verified", "1 pending"]);
    }

    #[test]
    fn test_summary_success_rate_percentage() {
        // Mirrors the percentage calculation in the summary prose builder
        let total = 10;
        let verified = 7_usize;
        let pct = if total > 0 { (verified as f64 / total as f64) * 100.0 } else { 0.0 };
        assert_eq!(format!("{:.0}% success rate", pct), "70% success rate");
    }

    #[test]
    fn test_summary_success_rate_zero_total() {
        let total = 0;
        let verified = 0_usize;
        let pct = if total > 0 { (verified as f64 / total as f64) * 100.0 } else { 0.0 };
        assert_eq!(format!("{:.0}% success rate", pct), "0% success rate");
    }

    #[test]
    fn test_summary_success_rate_all_verified() {
        let total = 5;
        let verified = 5_usize;
        let pct = (verified as f64 / total as f64) * 100.0;
        assert_eq!(format!("{:.0}% success rate", pct), "100% success rate");
    }

    #[test]
    fn test_summary_success_rate_none_verified() {
        let total = 8;
        let verified = 0_usize;
        let pct = if total > 0 { (verified as f64 / total as f64) * 100.0 } else { 0.0 };
        assert_eq!(format!("{:.0}% success rate", pct), "0% success rate");
    }

    #[test]
    fn test_summary_verified_titles_single() {
        // A single verified title should be quoted without joining
        let verified_titles = vec!["Add dark mode".to_string()];
        let titles_str = if verified_titles.len() == 1 {
            format!("\"{}\"", verified_titles[0])
        } else {
            let all: Vec<String> = verified_titles.iter().map(|t| {
                let display = if t.chars().count() > 50 {
                    format!("{}...", t.chars().take(47).collect::<String>())
                } else {
                    t.clone()
                };
                format!("\"{}\"", display)
            }).collect();
            all.join(", ")
        };
        assert_eq!(titles_str, "\"Add dark mode\"");
    }

    #[test]
    fn test_summary_verified_titles_multiple() {
        let verified_titles = vec![
            "Add dark mode".to_string(),
            "Fix auth bug".to_string(),
            "Refactor DB layer".to_string(),
        ];
        let titles_str = if verified_titles.len() == 1 {
            format!("\"{}\"", verified_titles[0])
        } else {
            let all: Vec<String> = verified_titles.iter().map(|t| {
                let display = if t.chars().count() > 50 {
                    format!("{}...", t.chars().take(47).collect::<String>())
                } else {
                    t.clone()
                };
                format!("\"{}\"", display)
            }).collect();
            all.join(", ")
        };
        assert_eq!(titles_str, "\"Add dark mode\", \"Fix auth bug\", \"Refactor DB layer\"");
    }

    #[test]
    fn test_summary_verified_title_truncation_at_50_chars() {
        // Titles > 50 chars should be truncated to 47 chars + "..."
        let long_title = "This is a very long feature title that exceeds fifty characters easily".to_string();
        assert!(long_title.chars().count() > 50);

        let display = if long_title.chars().count() > 50 {
            format!("{}...", long_title.chars().take(47).collect::<String>())
        } else {
            long_title.clone()
        };
        assert_eq!(display.chars().count(), 50);
        assert!(display.ends_with("..."));
        assert_eq!(display, "This is a very long feature title that exceeds ...");
    }

    #[test]
    fn test_summary_verified_title_no_truncation_at_50() {
        // Title of exactly 50 chars should NOT be truncated
        let title = "A".repeat(50);
        assert_eq!(title.chars().count(), 50);
        let display = if title.chars().count() > 50 {
            format!("{}...", title.chars().take(47).collect::<String>())
        } else {
            title.clone()
        };
        assert_eq!(display, title);
    }

    #[test]
    fn test_summary_escalated_plural_single() {
        // 1 escalated task: "was" (singular)
        let escalated_titles = vec!["broken feature".to_string()];
        let plural = if escalated_titles.len() == 1 { " was" } else { "s were" };
        let msg = format!("{} task{} escalated", escalated_titles.len(), plural);
        assert_eq!(msg, "1 task was escalated");
    }

    #[test]
    fn test_summary_escalated_plural_multiple() {
        // 2+ escalated tasks: "were" (plural)
        let escalated_titles = vec!["broken feature".to_string(), "another issue".to_string()];
        let plural = if escalated_titles.len() == 1 { " was" } else { "s were" };
        let msg = format!("{} task{} escalated", escalated_titles.len(), plural);
        assert_eq!(msg, "2 tasks were escalated");
    }

    #[test]
    fn test_summary_today_spend_format() {
        let today_cost = 1.5_f64;
        assert_eq!(format!("Today's spend is ${:.2}", today_cost), "Today's spend is $1.50");
    }

    #[test]
    fn test_summary_today_spend_zero() {
        let today_cost = 0.0_f64;
        assert_eq!(format!("Today's spend is ${:.2}", today_cost), "Today's spend is $0.00");
    }

    #[test]
    fn test_summary_prose_joins_with_periods() {
        // The final output joins parts with ". " and appends a trailing "."
        let parts = vec![
            "Across 10 runs, 7 verified, 3 pending (70% success rate)".to_string(),
            "Today's spend is $1.50".to_string(),
        ];
        let output = parts.join(". ") + ".";
        assert!(output.ends_with("."));
        assert!(output.contains(". Today's spend"));
    }

    #[test]
    fn test_summary_full_prose_all_statuses() {
        // Simulate full prose building with all status types
        let total = 12;
        let mut status_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        status_counts.insert("verified".to_string(), 5);
        status_counts.insert("implemented".to_string(), 3);
        status_counts.insert("pending".to_string(), 2);
        status_counts.insert("escalated".to_string(), 1);
        status_counts.insert("cancelled".to_string(), 1);

        let verified_titles = vec!["dark mode".to_string(), "auth fix".to_string()];
        let escalated_titles = vec!["data migration".to_string()];
        let today_cost = 2.75_f64;

        let mut parts: Vec<String> = Vec::new();

        // Opening
        let mut status_parts: Vec<String> = Vec::new();
        for s in &["verified", "implemented", "pending", "escalated", "cancelled"] {
            if let Some(c) = status_counts.get(*s) {
                status_parts.push(format!("{} {}", c, s));
            }
        }
        let verified = *status_counts.get("verified").unwrap_or(&0);
        let pct = if total > 0 { (verified as f64 / total as f64) * 100.0 } else { 0.0 };
        parts.push(format!("Across {} orchestrator runs, {} ({})",
            total, status_parts.join(", "), format!("{:.0}% success rate", pct)));

        // Verified
        if !verified_titles.is_empty() {
            let titles_str = if verified_titles.len() == 1 {
                format!("\"{}\"", verified_titles[0])
            } else {
                let all: Vec<String> = verified_titles.iter().map(|t| {
                    let display = if t.chars().count() > 50 {
                        format!("{}...", t.chars().take(47).collect::<String>())
                    } else { t.clone() };
                    format!("\"{}\"", display)
                }).collect();
                all.join(", ")
            };
            parts.push(format!("The most recently verified features are: {}", titles_str));
        }

        // Escalated
        if !escalated_titles.is_empty() {
            let titles_str: Vec<String> = escalated_titles.iter().map(|t| {
                let display = if t.chars().count() > 50 {
                    format!("{}...", t.chars().take(47).collect::<String>())
                } else { t.clone() };
                format!("\"{}\"", display)
            }).collect();
            parts.push(format!("{} task{} escalated: {}",
                escalated_titles.len(),
                if escalated_titles.len() == 1 { " was" } else { "s were" },
                titles_str.join(", ")));
        }

        // Today's spend
        parts.push(format!("Today's spend is ${:.2}", today_cost));

        let output = parts.join(". ") + ".";

        assert!(output.starts_with("Across 12 orchestrator runs"));
        assert!(output.contains("5 verified, 3 implemented, 2 pending, 1 escalated, 1 cancelled"));
        assert!(output.contains("42% success rate"));
        assert!(output.contains("The most recently verified features are: \"dark mode\", \"auth fix\""));
        assert!(output.contains("1 task was escalated: \"data migration\""));
        assert!(output.contains("Today's spend is $2.75."));
    }

    #[test]
    fn test_summary_prose_no_verified_no_escalated() {
        // When there are no verified/escalated, those sentences are omitted
        let total = 3;
        let mut status_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        status_counts.insert("pending".to_string(), 3);

        let verified_titles: Vec<String> = Vec::new();
        let escalated_titles: Vec<String> = Vec::new();
        let today_cost = 0.0_f64;

        let mut parts: Vec<String> = Vec::new();

        let mut status_parts: Vec<String> = Vec::new();
        for s in &["verified", "implemented", "pending", "escalated", "cancelled"] {
            if let Some(c) = status_counts.get(*s) {
                status_parts.push(format!("{} {}", c, s));
            }
        }
        let verified = *status_counts.get("verified").unwrap_or(&0);
        let pct = if total > 0 { (verified as f64 / total as f64) * 100.0 } else { 0.0 };
        parts.push(format!("Across {} orchestrator runs, {} ({})",
            total, status_parts.join(", "), format!("{:.0}% success rate", pct)));

        if !verified_titles.is_empty() {
            parts.push("verified features sentence".to_string());
        }
        if !escalated_titles.is_empty() {
            parts.push("escalated tasks sentence".to_string());
        }
        parts.push(format!("Today's spend is ${:.2}", today_cost));

        let output = parts.join(". ") + ".";

        // Should have exactly 2 sentences: opening + today's spend
        assert_eq!(parts.len(), 2);
        assert!(output.contains("3 pending"));
        assert!(output.contains("0% success rate"));
        assert!(!output.contains("verified features"));
        assert!(!output.contains("escalated"));
        assert!(output.contains("Today's spend is $0.00."));
    }

    #[test]
    fn test_summary_orchestration_prefix_strip() {
        // Title "Orchestration: foo" → "foo", used for short_title in summary
        let title = "Orchestration: Add dark mode toggle";
        let short_title = title.strip_prefix("Orchestration:").unwrap_or(title).trim();
        assert_eq!(short_title, "Add dark mode toggle");
    }

    #[test]
    fn test_summary_orchestration_prefix_strip_no_prefix() {
        let title = "Just a regular title";
        let short_title = title.strip_prefix("Orchestration:").unwrap_or(title).trim();
        assert_eq!(short_title, "Just a regular title");
    }

    #[test]
    fn test_summary_verified_titles_capped_at_five() {
        // The implementation caps verified_titles at 5
        let mut verified_titles: Vec<String> = Vec::new();
        let all_titles = vec![
            "feat A", "feat B", "feat C", "feat D", "feat E", "feat F", "feat G",
        ];
        for title in all_titles {
            if verified_titles.len() < 5 {
                verified_titles.push(title.to_string());
            }
        }
        assert_eq!(verified_titles.len(), 5);
        assert_eq!(verified_titles.last().unwrap(), "feat E");
    }

    #[test]
    fn test_summary_escalated_titles_capped_at_three() {
        // The implementation caps escalated_titles at 3
        let mut escalated_titles: Vec<String> = Vec::new();
        let all_titles = vec!["esc A", "esc B", "esc C", "esc D", "esc E"];
        for title in all_titles {
            if escalated_titles.len() < 3 {
                escalated_titles.push(title.to_string());
            }
        }
        assert_eq!(escalated_titles.len(), 3);
        assert_eq!(escalated_titles.last().unwrap(), "esc C");
    }

    #[test]
    fn test_summary_cost_from_tracks_metadata() {
        // Mirrors how the summary extracts cost_usd from tracks edge metadata
        let metas: Vec<serde_json::Value> = vec![
            serde_json::json!({"cost_usd": 0.50, "run_id": "abc"}),
            serde_json::json!({"cost_usd": 0.25}),
            serde_json::json!({"status": "done"}),  // no cost_usd
        ];
        let run_cost: f64 = metas.iter()
            .filter_map(|v| v["cost_usd"].as_f64())
            .sum();
        assert!((run_cost - 0.75).abs() < 1e-10);
    }

    #[test]
    fn test_summary_cost_no_metadata() {
        // When there are no tracks edges with cost_usd, cost should be 0
        let metas: Vec<serde_json::Value> = vec![
            serde_json::json!({"status": "done"}),
            serde_json::json!({"note": "no cost here"}),
        ];
        let run_cost: f64 = metas.iter()
            .filter_map(|v| v["cost_usd"].as_f64())
            .sum();
        assert_eq!(run_cost, 0.0);
    }

    // Helpers mirroring the inline logic from handle_setup step 1c (code embeddings).
    // Uses unwrap_or_default() + is_empty() pattern matching the setup path (~line 6089).
    // Tests verify the [file_path] bracket-prefix format added to embedding text construction.
    fn extract_file_path_from_tags(tags: Option<&str>) -> String {
        tags.and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
            .and_then(|v| v.get("file_path").and_then(|s| s.as_str()).map(|s| s.to_string()))
            .unwrap_or_default()
    }

    fn build_code_embed_text(file_path: &str, title: &str, content: Option<&str>) -> String {
        if file_path.is_empty() {
            format!("{}\n{}", title, content.unwrap_or(""))
        } else {
            format!("[{}] {}\n{}", file_path, title, content.unwrap_or(""))
        }
    }

    // Helper mirroring the import code / embeddings regenerate paths (~lines 2411, 5206).
    // These use Option<String> instead of unwrap_or_default(), and the bracket format when Some.
    fn build_embed_text_option(title: &str, content: Option<&str>, tags: Option<&str>) -> String {
        let file_path = tags
            .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
            .and_then(|v| v.get("file_path").and_then(|s| s.as_str()).map(|s| s.to_string()));
        if let Some(fp) = file_path {
            format!("[{}] {}\n{}", fp, title, content.unwrap_or(""))
        } else {
            format!("{}\n{}", title, content.unwrap_or(""))
        }
    }

    #[test]
    fn test_code_embed_text_with_file_path() {
        let tags = r#"{"file_path": "src-tauri/src/bin/cli.rs", "line_start": 100}"#;
        let file_path = extract_file_path_from_tags(Some(tags));
        let text = build_code_embed_text(&file_path, "fn handle_setup()", Some("sets up the database"));
        assert!(text.starts_with("[src-tauri/src/bin/cli.rs]"));
        assert!(text.contains("fn handle_setup()"));
        assert!(text.contains("sets up the database"));
    }

    #[test]
    fn test_code_embed_text_ordering_file_path_first() {
        // file_path must appear first as [file_path] title on one line, then content on next
        let tags = r#"{"file_path": "src/commands/graph.rs"}"#;
        let file_path = extract_file_path_from_tags(Some(tags));
        let text = build_code_embed_text(&file_path, "pub fn get_node()", Some("returns a node"));
        let parts: Vec<&str> = text.splitn(2, '\n').collect();
        assert_eq!(parts[0], "[src/commands/graph.rs] pub fn get_node()");
        assert_eq!(parts[1], "returns a node");
    }

    #[test]
    fn test_code_embed_text_tags_none_omits_file_path() {
        let file_path = extract_file_path_from_tags(None);
        assert!(file_path.is_empty());
        let text = build_code_embed_text(&file_path, "fn bar()", Some("content"));
        assert_eq!(text, "fn bar()\ncontent");
    }

    #[test]
    fn test_code_embed_text_tags_missing_file_path_key() {
        // Tags JSON present but no "file_path" key — falls back to title+content only
        let tags = r#"{"line_start": 100, "line_end": 200}"#;
        let file_path = extract_file_path_from_tags(Some(tags));
        assert!(file_path.is_empty());
        let text = build_code_embed_text(&file_path, "fn foo()", Some("body"));
        assert_eq!(text, "fn foo()\nbody");
    }

    #[test]
    fn test_code_embed_text_tags_malformed_json_falls_back() {
        let file_path = extract_file_path_from_tags(Some("not valid json"));
        assert!(file_path.is_empty());
        let text = build_code_embed_text(&file_path, "fn baz()", None);
        assert_eq!(text, "fn baz()\n");
    }

    #[test]
    fn test_code_embed_text_no_content_uses_empty_string() {
        let tags = r#"{"file_path": "src/lib.rs"}"#;
        let file_path = extract_file_path_from_tags(Some(tags));
        let text = build_code_embed_text(&file_path, "fn qux()", None);
        assert_eq!(text, "[src/lib.rs] fn qux()\n");
    }

    #[test]
    fn test_code_embed_text_truncated_at_1000_bytes() {
        let tags = r#"{"file_path": "src/lib.rs"}"#;
        let file_path = extract_file_path_from_tags(Some(tags));
        let long_content = "x".repeat(2000);
        let text = build_code_embed_text(&file_path, "fn huge()", Some(&long_content));
        let truncated = utils::safe_truncate(&text, 1000);
        assert!(truncated.len() <= 1000);
        assert!(truncated.starts_with("[src/lib.rs]"));
    }

    // --- Tests for the import-code / embeddings-regenerate path (Option<String> variant) ---

    #[test]
    fn test_embed_text_option_path_with_file_path() {
        let tags = r#"{"file_path": "src-tauri/src/similarity.rs"}"#;
        let text = build_embed_text_option("fn cosine_sim()", Some("computes similarity"), Some(tags));
        assert_eq!(text, "[src-tauri/src/similarity.rs] fn cosine_sim()\ncomputes similarity");
    }

    #[test]
    fn test_embed_text_option_path_tags_none_fallback() {
        let text = build_embed_text_option("fn my_fn()", Some("body text"), None);
        assert_eq!(text, "fn my_fn()\nbody text");
    }

    #[test]
    fn test_embed_text_option_path_no_file_path_key_fallback() {
        let tags = r#"{"line_start": 42}"#;
        let text = build_embed_text_option("fn bar()", Some("impl"), Some(tags));
        assert_eq!(text, "fn bar()\nimpl");
    }

    #[test]
    fn test_embed_text_option_path_invalid_json_fallback() {
        let text = build_embed_text_option("fn baz()", None, Some("{broken"));
        assert_eq!(text, "fn baz()\n");
    }

    #[test]
    fn test_embed_text_option_path_file_path_not_string_fallback() {
        // file_path value is a JSON number — as_str() returns None, so falls back
        let tags = r#"{"file_path": 123}"#;
        let text = build_embed_text_option("fn num()", Some("content"), Some(tags));
        assert_eq!(text, "fn num()\ncontent");
    }

    #[test]
    fn test_embed_text_option_path_content_none_empty_string() {
        let tags = r#"{"file_path": "src/main.rs"}"#;
        let text = build_embed_text_option("fn main()", None, Some(tags));
        assert_eq!(text, "[src/main.rs] fn main()\n");
    }

    #[test]
    fn test_code_embed_text_empty_file_path_string_treated_as_no_path() {
        // Setup path uses is_empty() — explicit empty string in tags → fallback format
        let tags = r#"{"file_path": ""}"#;
        let file_path = extract_file_path_from_tags(Some(tags));
        assert!(file_path.is_empty(), "empty string file_path should be treated as absent");
        let text = build_code_embed_text(&file_path, "fn empty()", Some("body"));
        assert_eq!(text, "fn empty()\nbody");
    }

    #[test]
    fn test_embed_text_bracket_format_exact() {
        // Explicit check that brackets and space are used, not just newlines
        let tags = r#"{"file_path": "src/db/schema.rs"}"#;
        let file_path = extract_file_path_from_tags(Some(tags));
        let text = build_code_embed_text(&file_path, "struct Node", Some("graph node"));
        assert!(text.starts_with('['), "must start with opening bracket");
        assert!(text.contains("] struct Node\n"), "title follows bracket-wrapped path with space");
    }

    #[test]
    fn test_prefix_extraction_logic() {
        // 1. tags with file_path → "[path] title\ncontent"
        let tags_with_path = r#"{"file_path": "src/lib.rs"}"#;
        let text = build_embed_text_option("fn foo()", Some("body"), Some(tags_with_path));
        assert_eq!(text, "[src/lib.rs] fn foo()\nbody");

        // 2. tags without file_path → "title\ncontent"
        let tags_no_path = r#"{"line_start": 10}"#;
        let text = build_embed_text_option("fn foo()", Some("body"), Some(tags_no_path));
        assert_eq!(text, "fn foo()\nbody");

        // 3. no tags → "title\ncontent"
        let text = build_embed_text_option("fn foo()", Some("body"), None);
        assert_eq!(text, "fn foo()\nbody");
    }

    #[test]
    fn test_parse_spore_analyze_default() {
        let args = Cli::try_parse_from(&["mycelica-cli", "--db", "test.db", "spore", "analyze"]).unwrap();
        if let Commands::Spore { cmd, .. } = args.command {
            if let SporeCommands::Analyze { region, top_n, stale_days, hub_threshold } = cmd {
                assert!(region.is_none());
                assert_eq!(top_n, 10);
                assert_eq!(stale_days, 60);
                assert_eq!(hub_threshold, 15);
            } else {
                panic!("Expected Analyze command");
            }
        } else {
            panic!("Expected Spore command");
        }
    }

    #[test]
    fn test_parse_spore_analyze_all_flags() {
        let args = Cli::try_parse_from(&[
            "mycelica-cli", "--db", "test.db", "spore", "analyze",
            "--region", "abc123", "--top-n", "5",
            "--stale-days", "90", "--hub-threshold", "20"
        ]).unwrap();
        if let Commands::Spore { cmd, .. } = args.command {
            if let SporeCommands::Analyze { region, top_n, stale_days, hub_threshold } = cmd {
                assert_eq!(region.as_deref(), Some("abc123"));
                assert_eq!(top_n, 5);
                assert_eq!(stale_days, 90);
                assert_eq!(hub_threshold, 20);
            } else {
                panic!("Expected Analyze command");
            }
        } else {
            panic!("Expected Spore command");
        }
    }

}
