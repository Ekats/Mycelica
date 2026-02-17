//! MCP (Model Context Protocol) server for Mycelica knowledge graph.
//! Provides 16 tools for agent coordination: 12 read + 4 write.
//! Launch: `mycelica-cli mcp-server --stdio --agent-role <role> --agent-id <id>`
//!
//! ## rmcp v0.15 Parameters<T> gotcha
//!
//! rmcp uses an Axum-style extractor pattern for tool parameters. Plain structs
//! (even with Deserialize + JsonSchema) don't satisfy `IntoToolRoute` — the compiler
//! error is opaque ("the trait bound ... IntoToolRoute<Tools, _> is not satisfied").
//!
//! The fix: wrap every tool parameter in `Parameters<T>` from
//! `rmcp::handler::server::wrapper::Parameters`. This wrapper implements
//! `FromContextPart<ToolCallContext>` for any `T: DeserializeOwned`, which is what
//! the `#[tool_router]` macro actually requires. Use destructuring in the signature:
//!
//! ```ignore
//! #[tool(description = "...")]
//! async fn my_tool(&self, Parameters(p): Parameters<MyParams>) -> Result<CallToolResult, McpError> {
//!     // access p.field directly
//! }
//! ```
//!
//! Neither the rmcp README nor `#[tool(aggr)]` (which doesn't exist in v0.15)
//! documents this. The clue is in rmcp's test files and the `wrapper/parameters.rs`
//! source.

use std::collections::HashSet;
use std::sync::Arc;

use rmcp::{
    RoleServer, ServerHandler, ServiceExt,
    handler::server::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::*,
    service::RequestContext,
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError,
};
use serde::Serialize;

use crate::db::{Database, Edge, EdgeType, Node, NodeType, Position};
use crate::team::{resolve_node as team_resolve_node, ResolveResult};

// ─── AgentRole ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum AgentRole {
    Human,
    Ingestor,
    Coder,
    Verifier,
    Planner,
    Synthesizer,
    Summarizer,
    DocWriter,
}

impl AgentRole {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "human" => Some(Self::Human),
            "ingestor" => Some(Self::Ingestor),
            "coder" => Some(Self::Coder),
            "verifier" => Some(Self::Verifier),
            "planner" => Some(Self::Planner),
            "synthesizer" => Some(Self::Synthesizer),
            "summarizer" => Some(Self::Summarizer),
            "docwriter" => Some(Self::DocWriter),
            _ => None,
        }
    }

    pub fn allowed_tools(&self) -> HashSet<&'static str> {
        let mut tools: HashSet<&str> = READ_TOOLS.iter().copied().collect();
        let write: &[&str] = match self {
            Self::Human => &[
                "mycelica_create_edge",
                "mycelica_create_meta",
                "mycelica_update_meta",
                "mycelica_create_node",
            ],
            Self::Ingestor => &["mycelica_create_edge", "mycelica_create_node"],
            Self::Coder => &["mycelica_create_edge", "mycelica_create_node"],
            Self::Verifier => &["mycelica_create_edge", "mycelica_create_node"],
            Self::Planner => &[
                "mycelica_create_edge",
                "mycelica_create_meta",
                "mycelica_create_node",
            ],
            Self::Synthesizer => &["mycelica_create_edge"],
            Self::Summarizer => &[
                "mycelica_create_edge",
                "mycelica_create_meta",
                "mycelica_update_meta",
            ],
            Self::DocWriter => &["mycelica_create_edge"],
        };
        tools.extend(write);
        tools
    }
}

const READ_TOOLS: [&str; 12] = [
    "mycelica_search",
    "mycelica_node_get",
    "mycelica_read_content",
    "mycelica_nav_edges",
    "mycelica_query_edges",
    "mycelica_explain_edge",
    "mycelica_path_between",
    "mycelica_edges_for_context",
    "mycelica_list_region",
    "mycelica_check_freshness",
    "mycelica_status",
    "mycelica_db_stats",
];

// ─── Parameter Structs ───────────────────────────────────────────────────────

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct SearchParams {
    /// Text query to search for in the knowledge graph
    query: String,
    /// Maximum number of results to return (default: 20)
    limit: Option<u32>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct NodeGetParams {
    /// Node ID, ID prefix (6+ hex chars), or title text to search
    id: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct ReadContentParams {
    /// Node ID, ID prefix, or title text
    id: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct NavEdgesParams {
    /// Node ID, ID prefix, or title text
    id: String,
    /// Filter by edge type (e.g. "supports", "contradicts", "summarizes")
    edge_type: Option<String>,
    /// Filter by direction: "incoming", "outgoing", or omit for both
    direction: Option<String>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct QueryEdgesParams {
    /// Filter by edge type
    edge_type: Option<String>,
    /// Filter by agent_id (edge creator)
    agent: Option<String>,
    /// Filter by target node's agent_id (whose work is being targeted)
    target_agent: Option<String>,
    /// Minimum confidence threshold (0.0-1.0)
    confidence_min: Option<f64>,
    /// Only edges created after this ISO date (YYYY-MM-DD)
    since: Option<String>,
    /// If true, exclude superseded edges
    not_superseded: Option<bool>,
    /// Maximum number of results (default: 50)
    limit: Option<u32>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct ExplainEdgeParams {
    /// Edge ID to explain
    id: String,
    /// Depth of supersession chain to follow (default: 3)
    depth: Option<usize>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct PathBetweenParams {
    /// Source node ID, prefix, or title
    from: String,
    /// Target node ID, prefix, or title
    to: String,
    /// Maximum hops (default: 4)
    max_hops: Option<usize>,
    /// Comma-separated edge types to traverse (e.g. "supports,contradicts")
    edge_types: Option<String>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct EdgesForContextParams {
    /// Node ID, prefix, or title
    id: String,
    /// Number of top edges to return (default: 10)
    top: Option<usize>,
    /// If true, exclude superseded edges
    not_superseded: Option<bool>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct ListRegionParams {
    /// Parent node ID, prefix, or title
    id: String,
    /// Filter by node_class (e.g. "knowledge", "meta", "operational")
    class: Option<String>,
    /// If true, only return items (leaf nodes)
    items_only: Option<bool>,
    /// Maximum number of results (default: 100)
    limit: Option<u32>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct CheckFreshnessParams {
    /// Node ID, prefix, or title
    id: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct StatusParams {}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct DbStatsParams {}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct CreateEdgeParams {
    /// Source node ID, prefix, or title
    from: String,
    /// Target node ID, prefix, or title
    to: String,
    /// Edge type (e.g. "supports", "contradicts", "summarizes", "derives_from")
    edge_type: String,
    /// Reasoning or explanation for this edge
    content: Option<String>,
    /// Short provenance note
    reason: Option<String>,
    /// Confidence score 0.0-1.0 (default: 1.0)
    confidence: Option<f64>,
    /// Edge ID this supersedes (marks old edge as superseded)
    supersedes: Option<String>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct CreateMetaParams {
    /// Meta node type: "summary", "contradiction", "todo", "status", "decision"
    meta_type: String,
    /// Title for the meta node
    title: String,
    /// Content/body of the meta node
    content: String,
    /// List of node IDs to connect this meta node to
    connects_to: Vec<String>,
    /// Edge type for connections (default: "summarizes")
    edge_type: Option<String>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct UpdateMetaParams {
    /// Existing meta node ID, prefix, or title to supersede
    id: String,
    /// New content for the meta node
    content: String,
    /// New title (optional, inherits from old if omitted)
    title: Option<String>,
    /// Additional node IDs to connect to
    add_connects: Option<Vec<String>>,
    /// Edge type for new connections (default: "summarizes")
    edge_type: Option<String>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct CreateNodeParams {
    /// Title for the new node
    title: String,
    /// Content/body of the node
    content: Option<String>,
    /// Content type (e.g. "text", "code", "markdown")
    content_type: Option<String>,
    /// Node class: "knowledge" (default), "operational"
    node_class: Option<String>,
    /// List of node IDs to create Related edges to
    connects_to: Option<Vec<String>>,
}

// ─── Output Structs ──────────────────────────────────────────────────────────

#[derive(Serialize)]
struct SlimNode {
    id: String,
    title: String,
    node_class: Option<String>,
    is_item: bool,
    depth: i32,
    meta_type: Option<String>,
}

impl From<&Node> for SlimNode {
    fn from(n: &Node) -> Self {
        Self {
            id: n.id.clone(),
            title: n.ai_title.clone().unwrap_or_else(|| n.title.clone()),
            node_class: n.node_class.clone(),
            is_item: n.is_item,
            depth: n.depth,
            meta_type: n.meta_type.clone(),
        }
    }
}

#[derive(Serialize)]
struct SlimEdge {
    id: String,
    source: String,
    target: String,
    edge_type: String,
    confidence: Option<f64>,
    agent_id: Option<String>,
    reason: Option<String>,
    created_at: i64,
    superseded_by: Option<String>,
}

impl From<&Edge> for SlimEdge {
    fn from(e: &Edge) -> Self {
        Self {
            id: e.id.clone(),
            source: e.source.clone(),
            target: e.target.clone(),
            edge_type: e.edge_type.as_str().to_string(),
            confidence: e.confidence,
            agent_id: e.agent_id.clone(),
            reason: e.reason.clone(),
            created_at: e.created_at,
            superseded_by: e.superseded_by.clone(),
        }
    }
}

// ─── Tools ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Tools {
    tool_router: ToolRouter<Self>,
    db: Arc<Database>,
    agent_id: String,
    role: AgentRole,
    run_id: Option<String>,
}

impl Tools {
    fn make_metadata(&self) -> Option<String> {
        self.run_id.as_ref().map(|rid| serde_json::json!({"run_id": rid}).to_string())
    }

    fn merge_metadata(&self, existing: Option<&str>) -> Option<String> {
        match (&self.run_id, existing) {
            (Some(rid), Some(json_str)) => {
                let mut obj: serde_json::Value = serde_json::from_str(json_str)
                    .unwrap_or(serde_json::json!({}));
                if let Some(map) = obj.as_object_mut() {
                    map.insert("run_id".to_string(), serde_json::json!(rid));
                }
                Some(obj.to_string())
            }
            (Some(rid), None) => Some(serde_json::json!({"run_id": rid}).to_string()),
            (None, existing) => existing.map(|s| s.to_string()),
        }
    }

    fn resolve(&self, reference: &str) -> Result<Node, String> {
        match team_resolve_node(&self.db, reference) {
            ResolveResult::Found(node) => Ok(node),
            ResolveResult::Ambiguous(candidates) => {
                let list: Vec<String> = candidates
                    .iter()
                    .map(|c| format!("{} {}", &c.id[..8.min(c.id.len())], c.title))
                    .collect();
                Err(format!(
                    "Ambiguous: {} matches. Use a full ID:\n{}",
                    candidates.len(),
                    list.join("\n")
                ))
            }
            ResolveResult::NotFound(msg) => Err(msg),
        }
    }

    fn filter_nodes(&self, nodes: &mut Vec<SlimNode>) {
        match self.role {
            AgentRole::Summarizer => nodes.retain(|n| {
                n.node_class.as_deref() != Some("meta")
                    || n.meta_type.as_deref() == Some("contradiction")
            }),
            AgentRole::Synthesizer => nodes.retain(|n| {
                matches!(
                    n.node_class.as_deref(),
                    Some("knowledge") | Some("operational")
                ) && n.is_item
            }),
            _ => {}
        }
    }

    fn should_filter_node(&self, node: &Node) -> bool {
        match self.role {
            AgentRole::Summarizer => {
                node.node_class.as_deref() == Some("meta")
                    && node.meta_type.as_deref() != Some("contradiction")
            }
            AgentRole::Synthesizer => {
                !matches!(
                    node.node_class.as_deref(),
                    Some("knowledge") | Some("operational")
                ) || !node.is_item
            }
            _ => false,
        }
    }

    fn tool_error(msg: impl Into<String>) -> CallToolResult {
        let mut result = CallToolResult::success(vec![Content::text(msg.into())]);
        result.is_error = Some(true);
        result
    }

    fn tool_ok(value: &impl Serialize) -> CallToolResult {
        let json = serde_json::to_string_pretty(value).unwrap_or_default();
        CallToolResult::success(vec![Content::text(json)])
    }

    fn now_ms() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}

// ─── Tool Implementations ────────────────────────────────────────────────────

#[tool_router]
impl Tools {
    fn new(db: Arc<Database>, agent_id: String, role: AgentRole, run_id: Option<String>) -> Self {
        Self {
            tool_router: Self::tool_router(),
            db,
            agent_id,
            role,
            run_id,
        }
    }

    // ── Read Tools ───────────────────────────────────────────────────────

    #[tool(description = "Search the knowledge graph by text query. Returns matching nodes.")]
    async fn mycelica_search(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let limit = p.limit.unwrap_or(20) as usize;
        let nodes = match self.db.search_nodes(&p.query) {
            Ok(n) => n,
            Err(e) => return Ok(Self::tool_error(format!("Search failed: {}", e))),
        };
        let total = nodes.len();
        let mut slim: Vec<SlimNode> = nodes.iter().take(limit).map(SlimNode::from).collect();
        self.filter_nodes(&mut slim);
        Ok(Self::tool_ok(&serde_json::json!({
            "results": slim,
            "total": total,
            "shown": slim.len(),
        })))
    }

    #[tool(description = "Get full details of a node by ID, prefix, or title.")]
    async fn mycelica_node_get(
        &self,
        Parameters(p): Parameters<NodeGetParams>,
    ) -> Result<CallToolResult, McpError> {
        let node = match self.resolve(&p.id) {
            Ok(n) => n,
            Err(msg) => return Ok(Self::tool_error(msg)),
        };
        if self.should_filter_node(&node) {
            return Ok(Self::tool_error("Node filtered by role permissions"));
        }
        Ok(Self::tool_ok(&serde_json::json!({
            "id": node.id,
            "title": node.ai_title.as_ref().unwrap_or(&node.title),
            "content": node.content,
            "summary": node.summary,
            "tags": node.tags,
            "node_class": node.node_class,
            "meta_type": node.meta_type,
            "is_item": node.is_item,
            "depth": node.depth,
            "parent_id": node.parent_id,
            "agent_id": node.agent_id,
            "author": node.author,
            "created_at": node.created_at,
            "updated_at": node.updated_at,
            "source": node.source,
            "content_type": node.content_type,
        })))
    }

    #[tool(description = "Read the full content of a node. Returns content, summary, and tags.")]
    async fn mycelica_read_content(
        &self,
        Parameters(p): Parameters<ReadContentParams>,
    ) -> Result<CallToolResult, McpError> {
        let node = match self.resolve(&p.id) {
            Ok(n) => n,
            Err(msg) => return Ok(Self::tool_error(msg)),
        };
        if self.should_filter_node(&node) {
            return Ok(Self::tool_error("Node filtered by role permissions"));
        }
        Ok(Self::tool_ok(&serde_json::json!({
            "id": node.id,
            "title": node.ai_title.as_ref().unwrap_or(&node.title),
            "content": node.content,
            "summary": node.summary,
            "tags": node.tags,
        })))
    }

    #[tool(description = "List edges connected to a node, optionally filtered by type and direction.")]
    async fn mycelica_nav_edges(
        &self,
        Parameters(p): Parameters<NavEdgesParams>,
    ) -> Result<CallToolResult, McpError> {
        let node = match self.resolve(&p.id) {
            Ok(n) => n,
            Err(msg) => return Ok(Self::tool_error(msg)),
        };
        let edges = match self.db.get_edges_for_node(&node.id) {
            Ok(e) => e,
            Err(e) => return Ok(Self::tool_error(format!("Failed to get edges: {}", e))),
        };
        let filtered: Vec<SlimEdge> = edges
            .iter()
            .filter(|e| {
                if let Some(ref et) = p.edge_type {
                    if e.edge_type.as_str() != et.as_str() {
                        return false;
                    }
                }
                if let Some(ref dir) = p.direction {
                    match dir.as_str() {
                        "incoming" => return e.target == node.id,
                        "outgoing" => return e.source == node.id,
                        _ => {}
                    }
                }
                true
            })
            .map(SlimEdge::from)
            .collect();
        Ok(Self::tool_ok(&serde_json::json!({
            "node": node.id,
            "edges": filtered,
            "count": filtered.len(),
        })))
    }

    #[tool(description = "Query edges across the graph with filters (type, agent, confidence, date, supersession).")]
    async fn mycelica_query_edges(
        &self,
        Parameters(p): Parameters<QueryEdgesParams>,
    ) -> Result<CallToolResult, McpError> {
        let since_ms = if let Some(ref since_str) = p.since {
            match chrono::NaiveDate::parse_from_str(since_str, "%Y-%m-%d") {
                Ok(d) => {
                    Some(d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis())
                }
                Err(_) => return Ok(Self::tool_error(format!("Invalid date: {}", since_str))),
            }
        } else {
            None
        };
        let limit = p.limit.unwrap_or(50) as usize;
        let edges = match self.db.query_edges(
            p.edge_type.as_deref(),
            p.agent.as_deref(),
            p.target_agent.as_deref(),
            p.confidence_min,
            since_ms,
            p.not_superseded.unwrap_or(false),
            limit,
        ) {
            Ok(e) => e,
            Err(e) => return Ok(Self::tool_error(format!("Query failed: {}", e))),
        };
        let slim: Vec<serde_json::Value> = edges
            .iter()
            .map(|ew| {
                serde_json::json!({
                    "id": ew.edge.id,
                    "source": ew.edge.source,
                    "target": ew.edge.target,
                    "edge_type": ew.edge.edge_type.as_str(),
                    "confidence": ew.edge.confidence,
                    "agent_id": ew.edge.agent_id,
                    "reason": ew.edge.reason,
                    "source_title": ew.source_title,
                    "target_title": ew.target_title,
                    "created_at": ew.edge.created_at,
                })
            })
            .collect();
        Ok(Self::tool_ok(&serde_json::json!({
            "edges": slim,
            "count": slim.len(),
        })))
    }

    #[tool(description = "Explain an edge: show connected nodes, adjacent edges, and supersession chain.")]
    async fn mycelica_explain_edge(
        &self,
        Parameters(p): Parameters<ExplainEdgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let depth = p.depth.unwrap_or(3);
        let explanation = match self.db.explain_edge(&p.id, depth) {
            Ok(Some(ex)) => ex,
            Ok(None) => return Ok(Self::tool_error(format!("Edge not found: {}", p.id))),
            Err(e) => return Ok(Self::tool_error(format!("Explain failed: {}", e))),
        };
        Ok(Self::tool_ok(&serde_json::json!({
            "edge": SlimEdge::from(&explanation.edge),
            "source_node": {
                "id": explanation.source_node.id,
                "title": explanation.source_node.ai_title.as_ref().unwrap_or(&explanation.source_node.title),
            },
            "target_node": {
                "id": explanation.target_node.id,
                "title": explanation.target_node.ai_title.as_ref().unwrap_or(&explanation.target_node.title),
            },
            "adjacent_edges": explanation.adjacent_edges.iter().map(SlimEdge::from).collect::<Vec<_>>(),
            "supersession_chain": explanation.supersession_chain.iter().map(SlimEdge::from).collect::<Vec<_>>(),
        })))
    }

    #[tool(description = "Find paths between two nodes through the edge graph.")]
    async fn mycelica_path_between(
        &self,
        Parameters(p): Parameters<PathBetweenParams>,
    ) -> Result<CallToolResult, McpError> {
        let from_node = match self.resolve(&p.from) {
            Ok(n) => n,
            Err(msg) => return Ok(Self::tool_error(msg)),
        };
        let to_node = match self.resolve(&p.to) {
            Ok(n) => n,
            Err(msg) => return Ok(Self::tool_error(msg)),
        };
        let max_hops = p.max_hops.unwrap_or(4);
        let edge_type_strs: Option<Vec<String>> = p.edge_types.as_ref().map(|s| {
            s.split(',').map(|t| t.trim().to_string()).collect()
        });
        let edge_type_refs: Option<Vec<&str>> = edge_type_strs
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_str()).collect());
        let paths = match self.db.path_between(
            &from_node.id,
            &to_node.id,
            max_hops,
            edge_type_refs.as_deref(),
        ) {
            Ok(p) => p,
            Err(e) => return Ok(Self::tool_error(format!("Path search failed: {}", e))),
        };
        let path_json: Vec<Vec<serde_json::Value>> = paths
            .iter()
            .map(|path| {
                path.iter()
                    .map(|hop| {
                        serde_json::json!({
                            "node_id": hop.node_id,
                            "node_title": hop.node_title,
                            "edge": SlimEdge::from(&hop.edge),
                        })
                    })
                    .collect()
            })
            .collect();
        Ok(Self::tool_ok(&serde_json::json!({
            "from": from_node.id,
            "to": to_node.id,
            "paths": path_json,
            "count": path_json.len(),
        })))
    }

    #[tool(description = "Get the most relevant edges for a node, ranked by type and recency.")]
    async fn mycelica_edges_for_context(
        &self,
        Parameters(p): Parameters<EdgesForContextParams>,
    ) -> Result<CallToolResult, McpError> {
        let node = match self.resolve(&p.id) {
            Ok(n) => n,
            Err(msg) => return Ok(Self::tool_error(msg)),
        };
        let top = p.top.unwrap_or(10);
        let not_superseded = p.not_superseded.unwrap_or(false);
        let edges = match self.db.edges_for_context(&node.id, top, not_superseded) {
            Ok(e) => e,
            Err(e) => return Ok(Self::tool_error(format!("Context query failed: {}", e))),
        };
        let slim: Vec<SlimEdge> = edges.iter().map(SlimEdge::from).collect();
        Ok(Self::tool_ok(&serde_json::json!({
            "node": node.id,
            "edges": slim,
            "count": slim.len(),
        })))
    }

    #[tool(description = "List descendants of a node (subtree). Filter by class and item status.")]
    async fn mycelica_list_region(
        &self,
        Parameters(p): Parameters<ListRegionParams>,
    ) -> Result<CallToolResult, McpError> {
        let node = match self.resolve(&p.id) {
            Ok(n) => n,
            Err(msg) => return Ok(Self::tool_error(msg)),
        };
        let limit = p.limit.unwrap_or(100) as usize;
        let items_only = p.items_only.unwrap_or(false);
        let descendants = match self.db.get_descendants(
            &node.id,
            p.class.as_deref(),
            items_only,
            limit,
        ) {
            Ok(d) => d,
            Err(e) => return Ok(Self::tool_error(format!("List region failed: {}", e))),
        };
        let mut slim: Vec<SlimNode> = descendants.iter().map(SlimNode::from).collect();
        self.filter_nodes(&mut slim);
        Ok(Self::tool_ok(&serde_json::json!({
            "parent": node.id,
            "descendants": slim,
            "count": slim.len(),
        })))
    }

    #[tool(description = "Check freshness of a node: compare its timestamp to connected edge timestamps.")]
    async fn mycelica_check_freshness(
        &self,
        Parameters(p): Parameters<CheckFreshnessParams>,
    ) -> Result<CallToolResult, McpError> {
        let node = match self.resolve(&p.id) {
            Ok(n) => n,
            Err(msg) => return Ok(Self::tool_error(msg)),
        };
        let edges = match self.db.get_edges_for_node(&node.id) {
            Ok(e) => e,
            Err(e) => return Ok(Self::tool_error(format!("Failed to get edges: {}", e))),
        };
        let newest_edge = edges.iter().map(|e| e.created_at).max().unwrap_or(0);
        let status = if edges.is_empty() {
            "no_edges"
        } else if newest_edge > node.updated_at {
            "stale"
        } else {
            "fresh"
        };
        Ok(Self::tool_ok(&serde_json::json!({
            "node_id": node.id,
            "title": node.ai_title.as_ref().unwrap_or(&node.title),
            "node_updated_at": node.updated_at,
            "newest_edge_at": newest_edge,
            "edge_count": edges.len(),
            "status": status,
        })))
    }

    #[tool(description = "Get spore status: meta node summary, contradiction count, edge statistics.")]
    async fn mycelica_status(
        &self,
        Parameters(_p): Parameters<StatusParams>,
    ) -> Result<CallToolResult, McpError> {
        let meta_nodes = match self.db.get_meta_nodes(None) {
            Ok(n) => n,
            Err(e) => return Ok(Self::tool_error(format!("Failed to get meta nodes: {}", e))),
        };

        // Group meta nodes by type (small set, need actual data)
        let mut by_type: std::collections::HashMap<String, Vec<serde_json::Value>> =
            std::collections::HashMap::new();
        for n in &meta_nodes {
            let mt = n.meta_type.clone().unwrap_or_else(|| "unknown".to_string());
            by_type.entry(mt).or_default().push(serde_json::json!({
                "id": n.id,
                "title": n.ai_title.as_ref().unwrap_or(&n.title),
                "agent_id": n.agent_id,
                "created_at": n.created_at,
            }));
        }

        // SQL aggregates — no full table loads
        let edges_by_agent: std::collections::HashMap<String, i64> =
            self.db.count_edges_by_agent().unwrap_or_default().into_iter().collect();
        let nodes_by_class: std::collections::HashMap<String, i64> =
            self.db.count_nodes_by_class().unwrap_or_default().into_iter().collect();
        let total_edges: i64 = edges_by_agent.values().sum();
        let total_nodes: i64 = nodes_by_class.values().sum();
        let unresolved = self.db.count_unresolved_contradictions().unwrap_or(0);

        Ok(Self::tool_ok(&serde_json::json!({
            "meta_nodes_by_type": by_type,
            "total_meta_nodes": meta_nodes.len(),
            "unresolved_contradictions": unresolved,
            "total_nodes": total_nodes,
            "nodes_by_class": nodes_by_class,
            "total_edges": total_edges,
            "edges_by_agent": edges_by_agent,
        })))
    }

    #[tool(description = "Get database statistics: total nodes, edges, items count.")]
    async fn mycelica_db_stats(
        &self,
        Parameters(_p): Parameters<DbStatsParams>,
    ) -> Result<CallToolResult, McpError> {
        let (total_nodes, total_edges, items, categories) = match self.db.count_db_stats() {
            Ok(s) => s,
            Err(e) => return Ok(Self::tool_error(format!("Stats query failed: {}", e))),
        };
        Ok(Self::tool_ok(&serde_json::json!({
            "total_nodes": total_nodes,
            "items": items,
            "categories": categories,
            "total_edges": total_edges,
        })))
    }

    // ── Write Tools ──────────────────────────────────────────────────────

    #[tool(description = "Create a typed edge between two existing nodes. Agent ID is auto-injected.")]
    async fn mycelica_create_edge(
        &self,
        Parameters(p): Parameters<CreateEdgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let from_node = match self.resolve(&p.from) {
            Ok(n) => n,
            Err(msg) => return Ok(Self::tool_error(msg)),
        };
        let to_node = match self.resolve(&p.to) {
            Ok(n) => n,
            Err(msg) => return Ok(Self::tool_error(msg)),
        };
        let edge_type = match EdgeType::from_str(&p.edge_type) {
            Some(et) => et,
            None => {
                return Ok(Self::tool_error(format!(
                    "Unknown edge type: '{}'. Valid types: supports, contradicts, summarizes, \
                     derives_from, flags, resolves, supersedes, related, reference, because, \
                     contains, questions, evolved_from, prerequisite, tracks",
                    p.edge_type
                )))
            }
        };
        let now = Self::now_ms();
        let edge_id = uuid::Uuid::new_v4().to_string();
        let author = crate::settings::get_author_or_default();
        let edge = Edge {
            id: edge_id.clone(),
            source: from_node.id.clone(),
            target: to_node.id.clone(),
            edge_type,
            label: None,
            weight: None,
            edge_source: Some("spore".to_string()),
            evidence_id: None,
            confidence: Some(p.confidence.unwrap_or(1.0)),
            created_at: now,
            updated_at: Some(now),
            author: Some(author),
            reason: p.reason,
            content: p.content,
            agent_id: Some(self.agent_id.clone()),
            superseded_by: None,
            metadata: self.make_metadata(),
        };
        if let Err(e) = self.db.insert_edge(&edge) {
            return Ok(Self::tool_error(format!("Failed to insert edge: {}", e)));
        }
        if let Some(ref old_id) = p.supersedes {
            if let Err(e) = self.db.supersede_edge(old_id, &edge_id) {
                return Ok(Self::tool_error(format!(
                    "Edge created but supersession failed: {}",
                    e
                )));
            }
        }
        Ok(Self::tool_ok(&serde_json::json!({
            "id": edge_id,
            "source": from_node.id,
            "target": to_node.id,
            "type": p.edge_type,
            "agent_id": self.agent_id,
            "confidence": p.confidence.unwrap_or(1.0),
        })))
    }

    #[tool(description = "Create a meta node (summary, contradiction, todo, etc.) with edges to existing nodes.")]
    async fn mycelica_create_meta(
        &self,
        Parameters(p): Parameters<CreateMetaParams>,
    ) -> Result<CallToolResult, McpError> {
        let valid_types = [
            "summary",
            "contradiction",
            "todo",
            "status",
            "decision",
        ];
        if !valid_types.contains(&p.meta_type.as_str()) {
            return Ok(Self::tool_error(format!(
                "Invalid meta_type: '{}'. Valid: {}",
                p.meta_type,
                valid_types.join(", ")
            )));
        }

        // Resolve all connect targets first
        let mut connect_nodes = Vec::new();
        for ref_str in &p.connects_to {
            match self.resolve(ref_str) {
                Ok(n) => connect_nodes.push(n),
                Err(msg) => return Ok(Self::tool_error(msg)),
            }
        }

        // Find universe node for parent
        let parent_id = match self.db.get_universe() {
            Ok(Some(u)) => Some(u.id),
            _ => None,
        };

        let now = Self::now_ms();
        let node_id = uuid::Uuid::new_v4().to_string();
        let author = crate::settings::get_author_or_default();
        let edge_type_str = p.edge_type.as_deref().unwrap_or("summarizes");
        let edge_type = EdgeType::from_str(edge_type_str).unwrap_or(EdgeType::Summarizes);

        let node = Node {
            id: node_id.clone(),
            node_type: NodeType::Thought,
            title: p.title.clone(),
            url: None,
            content: Some(p.content),
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: None,
            depth: 1,
            is_item: true,
            is_universe: false,
            parent_id,
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
            content_type: Some("text".to_string()),
            associated_idea_id: None,
            privacy: None,
            human_edited: None,
            human_created: true,
            author: Some(author.clone()),
            agent_id: Some(self.agent_id.clone()),
            node_class: Some("meta".to_string()),
            meta_type: Some(p.meta_type.clone()),
        };

        let mut edges = Vec::new();
        for target in &connect_nodes {
            edges.push(Edge {
                id: uuid::Uuid::new_v4().to_string(),
                source: node_id.clone(),
                target: target.id.clone(),
                edge_type: edge_type.clone(),
                label: None,
                weight: None,
                edge_source: Some("spore".to_string()),
                evidence_id: None,
                confidence: Some(1.0),
                created_at: now,
                updated_at: Some(now),
                author: Some(author.clone()),
                reason: Some(format!("Meta node: {}", p.meta_type)),
                content: None,
                agent_id: Some(self.agent_id.clone()),
                superseded_by: None,
                metadata: self.make_metadata(),
            });
        }

        if let Err(e) = self.db.create_meta_node_with_edges(&node, &edges) {
            return Ok(Self::tool_error(format!(
                "Failed to create meta node: {}",
                e
            )));
        }
        Ok(Self::tool_ok(&serde_json::json!({
            "id": node_id,
            "type": p.meta_type,
            "title": p.title,
            "edges": edges.len(),
            "agent_id": self.agent_id,
        })))
    }

    #[tool(description = "Update a meta node via supersession: creates a new node linked to the old via Supersedes edge. Old outgoing edges are copied (excluding superseded ones).")]
    async fn mycelica_update_meta(
        &self,
        Parameters(p): Parameters<UpdateMetaParams>,
    ) -> Result<CallToolResult, McpError> {
        let old_node = match self.resolve(&p.id) {
            Ok(n) => n,
            Err(msg) => return Ok(Self::tool_error(msg)),
        };
        if old_node.node_class.as_deref() != Some("meta") {
            return Ok(Self::tool_error(format!(
                "Node {} is not a meta node (class: {:?})",
                old_node.id,
                old_node.node_class
            )));
        }

        let now = Self::now_ms();
        let new_id = uuid::Uuid::new_v4().to_string();
        let author = crate::settings::get_author_or_default();
        let new_title = p.title.unwrap_or_else(|| old_node.title.clone());

        // Create new meta node inheriting fields from old
        let new_node = Node {
            id: new_id.clone(),
            node_type: old_node.node_type.clone(),
            title: new_title.clone(),
            url: old_node.url.clone(),
            content: Some(p.content),
            position: old_node.position.clone(),
            created_at: now,
            updated_at: now,
            cluster_id: old_node.cluster_id,
            cluster_label: old_node.cluster_label.clone(),
            depth: old_node.depth,
            is_item: old_node.is_item,
            is_universe: false,
            parent_id: old_node.parent_id.clone(),
            child_count: 0,
            ai_title: old_node.ai_title.clone(),
            summary: old_node.summary.clone(),
            tags: old_node.tags.clone(),
            emoji: old_node.emoji.clone(),
            is_processed: false,
            conversation_id: old_node.conversation_id.clone(),
            sequence_index: old_node.sequence_index,
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
            agent_id: Some(self.agent_id.clone()),
            node_class: Some("meta".to_string()),
            meta_type: old_node.meta_type.clone(),
        };

        let mut edges = Vec::new();

        // 1. Supersedes edge: new → old
        edges.push(Edge {
            id: uuid::Uuid::new_v4().to_string(),
            source: new_id.clone(),
            target: old_node.id.clone(),
            edge_type: EdgeType::Supersedes,
            label: None,
            weight: None,
            edge_source: Some("spore".to_string()),
            evidence_id: None,
            confidence: Some(1.0),
            created_at: now,
            updated_at: Some(now),
            author: Some(author.clone()),
            reason: Some(format!(
                "Supersedes {}",
                &old_node.id[..8.min(old_node.id.len())]
            )),
            content: None,
            agent_id: Some(self.agent_id.clone()),
            superseded_by: None,
            metadata: self.make_metadata(),
        });

        // 2. Copy old node's outgoing edges (3-part filter)
        let old_edges = self
            .db
            .get_edges_for_node(&old_node.id)
            .unwrap_or_default();
        let mut copied_count = 0;
        for old_edge in &old_edges {
            // Only outgoing
            if old_edge.source != old_node.id {
                continue;
            }
            // Skip superseded edges
            if old_edge.superseded_by.is_some() {
                continue;
            }
            // Skip Supersedes-typed edges
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
                agent_id: Some(self.agent_id.clone()),
                superseded_by: None,
                metadata: self.merge_metadata(old_edge.metadata.as_deref()),
            });
            copied_count += 1;
        }

        // 3. Add new connections
        let mut new_edge_count = 0;
        if let Some(ref connects) = p.add_connects {
            let edge_type_str = p.edge_type.as_deref().unwrap_or("summarizes");
            let edge_type =
                EdgeType::from_str(edge_type_str).unwrap_or(EdgeType::Summarizes);
            for ref_str in connects {
                let target = match self.resolve(ref_str) {
                    Ok(n) => n,
                    Err(msg) => return Ok(Self::tool_error(msg)),
                };
                edges.push(Edge {
                    id: uuid::Uuid::new_v4().to_string(),
                    source: new_id.clone(),
                    target: target.id.clone(),
                    edge_type: edge_type.clone(),
                    label: None,
                    weight: None,
                    edge_source: Some("spore".to_string()),
                    evidence_id: None,
                    confidence: Some(1.0),
                    created_at: now,
                    updated_at: Some(now),
                    author: Some(author.clone()),
                    reason: Some("update-meta connection".to_string()),
                    content: None,
                    agent_id: Some(self.agent_id.clone()),
                    superseded_by: None,
                    metadata: self.make_metadata(),
                });
                new_edge_count += 1;
            }
        }

        if let Err(e) = self.db.create_meta_node_with_edges(&new_node, &edges) {
            return Ok(Self::tool_error(format!(
                "Failed to create updated meta node: {}",
                e
            )));
        }
        Ok(Self::tool_ok(&serde_json::json!({
            "new_id": new_id,
            "old_id": old_node.id,
            "title": new_title,
            "copied_edges": copied_count,
            "new_edges": new_edge_count,
            "agent_id": self.agent_id,
        })))
    }

    #[tool(description = "Create a knowledge or operational node with optional connections.")]
    async fn mycelica_create_node(
        &self,
        Parameters(p): Parameters<CreateNodeParams>,
    ) -> Result<CallToolResult, McpError> {
        let node_class = p.node_class.as_deref().unwrap_or("knowledge");
        if !["knowledge", "operational"].contains(&node_class) {
            return Ok(Self::tool_error(format!(
                "Invalid node_class: '{}'. Valid: knowledge, operational",
                node_class
            )));
        }

        let parent_id = match self.db.get_universe() {
            Ok(Some(u)) => Some(u.id),
            _ => None,
        };

        let now = Self::now_ms();
        let node_id = uuid::Uuid::new_v4().to_string();
        let author = crate::settings::get_author_or_default();

        let node = Node {
            id: node_id.clone(),
            node_type: NodeType::Thought,
            title: p.title.clone(),
            url: None,
            content: p.content,
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: None,
            depth: 1,
            is_item: true,
            is_universe: false,
            parent_id,
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
            content_type: p.content_type.or(Some("text".to_string())),
            associated_idea_id: None,
            privacy: None,
            human_edited: None,
            human_created: true,
            author: Some(author.clone()),
            agent_id: Some(self.agent_id.clone()),
            node_class: Some(node_class.to_string()),
            meta_type: None,
        };

        // Build connection edges
        let mut edges = Vec::new();
        if let Some(ref connects) = p.connects_to {
            for ref_str in connects {
                let target = match self.resolve(ref_str) {
                    Ok(n) => n,
                    Err(msg) => return Ok(Self::tool_error(msg)),
                };
                edges.push(Edge {
                    id: uuid::Uuid::new_v4().to_string(),
                    source: node_id.clone(),
                    target: target.id.clone(),
                    edge_type: EdgeType::Related,
                    label: None,
                    weight: None,
                    edge_source: Some("spore".to_string()),
                    evidence_id: None,
                    confidence: Some(1.0),
                    created_at: now,
                    updated_at: Some(now),
                    author: Some(author.clone()),
                    reason: Some("create-node connection".to_string()),
                    content: None,
                    agent_id: Some(self.agent_id.clone()),
                    superseded_by: None,
                    metadata: self.make_metadata(),
                });
            }
        }

        if let Err(e) = self.db.create_meta_node_with_edges(&node, &edges) {
            return Ok(Self::tool_error(format!(
                "Failed to create node: {}",
                e
            )));
        }
        Ok(Self::tool_ok(&serde_json::json!({
            "id": node_id,
            "title": p.title,
            "node_class": node_class,
            "edges": edges.len(),
            "agent_id": self.agent_id,
        })))
    }
}

// ─── ServerHandler ───────────────────────────────────────────────────────────

#[tool_handler]
impl ServerHandler for Tools {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(format!(
                "Mycelica knowledge graph (inner). Role: {:?}, Agent: {}",
                self.role, self.agent_id
            )),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

/// Wrapper that filters tools by agent role permissions.
pub struct McpServer {
    inner: Tools,
    allowed_tools: HashSet<String>,
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(format!(
                "Mycelica knowledge graph. Role: {:?}, Agent: {}. \
                 Use these tools to read and write to the knowledge graph.",
                self.inner.role, self.inner.agent_id
            )),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut result = self.inner.list_tools(request, context).await?;
        result
            .tools
            .retain(|t| self.allowed_tools.contains(t.name.as_ref()));
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        if !self.allowed_tools.contains(request.name.as_ref()) {
            return Ok(Tools::tool_error(format!(
                "Tool '{}' not permitted for role {:?}",
                request.name, self.inner.role
            )));
        }
        self.inner.call_tool(request, context).await
    }
}

// ─── Entry Point ─────────────────────────────────────────────────────────────

pub async fn run_mcp_server(
    db: Arc<Database>,
    agent_id: String,
    role: AgentRole,
    run_id: Option<String>,
) -> Result<(), String> {
    let tools = Tools::new(db, agent_id, role.clone(), run_id);
    let allowed: HashSet<String> = role
        .allowed_tools()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let server = McpServer {
        inner: tools,
        allowed_tools: allowed,
    };

    let service = server
        .serve(stdio())
        .await
        .map_err(|e| format!("MCP server error: {}", e))?;
    service
        .waiting()
        .await
        .map_err(|e| format!("MCP server terminated: {}", e))?;
    Ok(())
}
