//! Mycelica Team Server — HTTP API for shared knowledge graph.
//!
//! Thin axum server wrapping the shared mycelica_lib database layer.
//! Sovereignty enforcement point: all write endpoints set human_created,
//! human_edited, author, and edge_source correctly.
//!
//! Authentication: API key-based. GET endpoints are public (no auth required).
//! POST/PATCH/DELETE require a valid `Authorization: Bearer <key>` header.
//! Use `--no-auth` to disable authentication (for trusted networks like Tailscale).
//!
//! Usage:
//!   MYCELICA_DB=/path/to/team.db mycelica-server
//!   mycelica-server --db /path/to/team.db --bind 0.0.0.0:3741  # all interfaces
//!   mycelica-server --no-auth  # disable auth for trusted networks
//!
//! Admin commands:
//!   mycelica-server admin create-key <name> [--role admin|editor]
//!   mycelica-server admin list-keys
//!   mycelica-server admin revoke-key <id>

use axum::{
    extract::{ConnectInfo, Extension, Path, Query, Request, State},
    http::{Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response, Json},
    routing::{get, post, patch},
    Router,
};
use governor::{Quota, RateLimiter, clock::Clock};
use governor::clock::DefaultClock;
use governor::state::keyed::DashMapStateStore;
use mycelica_lib::db::{Database, Node, Edge, EdgeType, ApiKey};
use mycelica_lib::{settings, local_embeddings, team};
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tower_http::cors::{CorsLayer, Any};
use tower_http::limit::RequestBodyLimitLayer;

// ============================================================================
// AppState
// ============================================================================

type WriteRateLimiter = RateLimiter<std::net::IpAddr, DashMapStateStore<std::net::IpAddr>, DefaultClock>;

#[derive(Clone)]
struct AppState {
    db: Arc<Database>,
    start_time: Instant,
    no_auth: bool,
    rate_limiter: Arc<WriteRateLimiter>,
}

// ============================================================================
// Auth
// ============================================================================

/// Injected into request extensions by auth middleware.
#[derive(Clone, Debug)]
struct AuthContext {
    user_name: String,
    role: String,  // "admin" or "editor"
}

fn hash_api_key(raw_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_key.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Auth middleware: GET/HEAD/OPTIONS pass through. Writes require Bearer token.
async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    // Skip auth for read-only methods
    if matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(next.run(request).await);
    }

    // Skip auth if --no-auth mode
    if state.no_auth {
        return Ok(next.run(request).await);
    }

    // Extract bearer token
    let auth_header = request.headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string());

    let token = match auth_header {
        Some(t) if !t.is_empty() => t,
        _ => return Err(AppError(StatusCode::UNAUTHORIZED,
            "Missing or invalid Authorization header. Use: Authorization: Bearer <api-key>".to_string())),
    };

    // Hash and look up
    let key_hash = hash_api_key(&token);
    let api_key = state.db.get_api_key_by_hash(&key_hash)
        .map_err(|e| AppError::from(e.to_string()))?
        .ok_or_else(|| AppError(StatusCode::UNAUTHORIZED, "Invalid API key".to_string()))?;

    // Inject auth context into request extensions
    request.extensions_mut().insert(AuthContext {
        user_name: api_key.user_name,
        role: api_key.role,
    });

    Ok(next.run(request).await)
}

// ============================================================================
// Rate limiting
// ============================================================================

/// Rate limit middleware: only applies to write methods (POST/PATCH/DELETE).
/// GET/HEAD/OPTIONS pass through unthrottled.
async fn rate_limit_middleware(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    // Read methods pass through — public reads are never rate limited
    if matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(request).await;
    }

    match state.rate_limiter.check_key(&addr.ip()) {
        Ok(_) => next.run(request).await,
        Err(not_until) => {
            let wait = not_until.wait_time_from(DefaultClock::default().now());
            let retry_after = wait.as_secs().max(1);
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("Retry-After", retry_after.to_string())],
                Json(serde_json::json!({
                    "error": "rate_limited",
                    "retry_after_seconds": retry_after
                })),
            ).into_response()
        }
    }
}

// ============================================================================
// Error type
// ============================================================================

struct AppError(StatusCode, String);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({"error": self.1}))).into_response()
    }
}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError(StatusCode::INTERNAL_SERVER_ERROR, s)
    }
}

fn not_found(msg: impl Into<String>) -> AppError {
    AppError(StatusCode::NOT_FOUND, msg.into())
}

fn bad_request(msg: impl Into<String>) -> AppError {
    AppError(StatusCode::BAD_REQUEST, msg.into())
}

fn forbidden(msg: impl Into<String>) -> AppError {
    AppError(StatusCode::FORBIDDEN, msg.into())
}

// ============================================================================
// Input validation
// ============================================================================

fn validate_node_input(title: Option<&str>, content: Option<&str>, tags: Option<&str>, author: Option<&str>) -> Result<(), AppError> {
    if let Some(t) = title {
        if t.len() > 2000 { return Err(bad_request("title exceeds maximum length of 2000 characters")); }
        if t.trim().is_empty() { return Err(bad_request("title cannot be empty")); }
    }
    if let Some(c) = content {
        if c.len() > 1_048_576 { return Err(bad_request("content exceeds maximum size of 1MB")); }
    }
    if let Some(t) = tags {
        if t.len() > 10_000 { return Err(bad_request("tags exceeds maximum length of 10000 characters")); }
    }
    if let Some(a) = author {
        if a.len() > 100 { return Err(bad_request("author exceeds maximum length of 100 characters")); }
    }
    Ok(())
}

fn validate_edge_input(reason: Option<&str>, author: Option<&str>) -> Result<(), AppError> {
    if let Some(r) = reason {
        if r.len() > 2000 { return Err(bad_request("reason exceeds maximum length of 2000 characters")); }
    }
    if let Some(a) = author {
        if a.len() > 100 { return Err(bad_request("author exceeds maximum length of 100 characters")); }
    }
    Ok(())
}

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Deserialize)]
struct CreateNodeRequest {
    title: String,
    content: Option<String>,
    url: Option<String>,
    content_type: Option<String>,
    tags: Option<String>,
    author: Option<String>,
    connects_to: Option<Vec<String>>,
    is_item: Option<bool>,
}

#[derive(Serialize)]
struct CreateNodeResponse {
    node: Node,
    edges_created: Vec<EdgeSummary>,
    ambiguous: Vec<AmbiguousResult>,
}

#[derive(Serialize)]
struct EdgeSummary {
    edge_id: String,
    target_id: String,
    target_title: String,
}

#[derive(Serialize)]
struct AmbiguousResult {
    term: String,
    candidates: Vec<team::NodeSummary>,
}

#[derive(Deserialize)]
struct PatchNodeRequest {
    title: Option<String>,
    content: Option<String>,
    tags: Option<String>,
    content_type: Option<String>,
    parent_id: Option<String>,
    author: Option<String>,
}

#[derive(Deserialize)]
struct CreateEdgeRequest {
    #[serde(alias = "source_id")]
    source: String,       // UUID, ID prefix, or title text
    #[serde(alias = "target_id")]
    target: String,       // UUID, ID prefix, or title text
    edge_type: Option<String>,
    reason: Option<String>,
    author: Option<String>,
}

#[derive(Serialize)]
struct CreateEdgeResponse {
    edge: Edge,
    source_resolved: team::NodeSummary,
    target_resolved: team::NodeSummary,
}

#[derive(Deserialize)]
struct PatchEdgeRequest {
    reason: Option<String>,
    edge_type: Option<String>,
    author: Option<String>,
}

#[derive(Deserialize)]
struct NodesQuery {
    search: Option<String>,
    since: Option<String>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct EdgesQuery {
    since: String,
}

#[derive(Deserialize)]
struct LimitQuery {
    n: Option<u32>,
    limit: Option<u32>,
}

#[derive(Serialize)]
struct NodeWithEdges {
    node: Node,
    edges: Vec<Edge>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
    nodes: usize,
    edges: usize,
    uptime_secs: u64,
    auth_enabled: bool,
}

#[derive(Serialize)]
struct RebuildResponse {
    categories: usize,
    papers_assigned: usize,
    sibling_edges: usize,
    bridges: usize,
    duration_secs: f64,
    log: Vec<String>,
}

#[derive(Serialize)]
struct DeletedItem {
    original_id: String,
    deleted_at: i64,
    deleted_by: Option<String>,
}

// ============================================================================
// Helpers
// ============================================================================

/// Resolve author: prefer auth context (server-enforced), fall back to request body (--no-auth mode).
fn resolve_author(auth: Option<&AuthContext>, req_author: Option<String>) -> String {
    if let Some(ctx) = auth {
        return ctx.user_name.clone();
    }
    req_author.unwrap_or_else(|| settings::get_author_or_default())
}

fn parse_since(s: &str) -> Result<i64, AppError> {
    // Try epoch millis first
    if let Ok(ms) = s.parse::<i64>() {
        return Ok(ms);
    }
    // Try ISO 8601
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.timestamp_millis())
        .map_err(|e| bad_request(format!("Invalid timestamp '{}': {}", s, e)))
}

// ============================================================================
// Handlers
// ============================================================================

// POST /nodes
async fn create_node_handler(
    State(state): State<AppState>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<CreateNodeRequest>,
) -> Result<(StatusCode, Json<CreateNodeResponse>), AppError> {
    validate_node_input(Some(&req.title), req.content.as_deref(), req.tags.as_deref(), req.author.as_deref())?;
    let auth = auth.map(|Extension(ctx)| ctx);
    let author = resolve_author(auth.as_ref(), req.author);
    let content_type = req.content_type.as_deref().unwrap_or("concept");

    let node_id = team::create_human_node(
        &state.db, &req.title, req.content.as_deref(), req.url.as_deref(),
        content_type, req.tags.as_deref(), &author, "server", req.is_item,
    ).map_err(AppError::from)?;

    let mut edges_created = Vec::new();
    let mut ambiguous = Vec::new();

    if let Some(terms) = req.connects_to {
        let results = team::create_connects_to_edges(&state.db, &node_id, &terms, &author);
        for result in results {
            match result {
                team::ConnectResult::Linked { edge_id, target } => {
                    edges_created.push(EdgeSummary {
                        edge_id,
                        target_id: target.id,
                        target_title: target.title,
                    });
                }
                team::ConnectResult::Ambiguous { term, candidates } => {
                    ambiguous.push(AmbiguousResult { term, candidates });
                }
                team::ConnectResult::NotFound { .. } => {}
            }
        }
    }

    let node = state.db.get_node(&node_id)
        .map_err(|e| AppError::from(e.to_string()))?
        .ok_or_else(|| AppError(StatusCode::INTERNAL_SERVER_ERROR, "Created node not found".to_string()))?;

    println!("[POST /nodes] Created '{}' by {} (id: {}, edges: {})",
        node.title, author, &node.id[..8], edges_created.len());

    Ok((StatusCode::CREATED, Json(CreateNodeResponse { node, edges_created, ambiguous })))
}

// PATCH /nodes/:id
async fn patch_node_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<PatchNodeRequest>,
) -> Result<Json<Node>, AppError> {
    validate_node_input(req.title.as_deref(), req.content.as_deref(), req.tags.as_deref(), req.author.as_deref())?;
    let auth = auth.map(|Extension(ctx)| ctx);

    // Verify node exists
    let existing = state.db.get_node(&id)
        .map_err(|e| AppError::from(e.to_string()))?
        .ok_or_else(|| not_found(format!("Node '{}' not found", id)))?;

    // Editor ownership check: editors can only modify their own nodes
    if let Some(ref ctx) = auth {
        if ctx.role == "editor" {
            if existing.author.as_deref() != Some(&ctx.user_name) {
                return Err(forbidden(format!(
                    "Editor '{}' cannot modify node owned by '{}'",
                    ctx.user_name, existing.author.as_deref().unwrap_or("unknown")
                )));
            }
        }
    }

    let author = resolve_author(auth.as_ref(), req.author.clone());

    state.db.patch_node_fields(
        &id,
        req.title.as_deref(),
        req.content.as_deref(),
        req.tags.as_deref(),
        req.content_type.as_deref(),
        req.parent_id.as_deref(),
        Some(&author),
    ).map_err(|e| AppError::from(e.to_string()))?;

    // Regenerate embedding if title or content changed
    if req.title.is_some() || req.content.is_some() {
        if let Ok(Some(updated)) = state.db.get_node(&id) {
            let embed_text = format!("{}\n{}", updated.title, updated.content.as_deref().unwrap_or(""));
            let embed_text = &embed_text[..embed_text.len().min(2000)];
            if let Ok(embedding) = local_embeddings::generate(embed_text) {
                state.db.update_node_embedding(&id, &embedding).ok();
            }
        }
    }

    let node = state.db.get_node(&id)
        .map_err(|e| AppError::from(e.to_string()))?
        .ok_or_else(|| not_found(format!("Node '{}' not found after update", id)))?;

    println!("[PATCH /nodes/{}] Updated by {} (fields: {})",
        &id[..8], author,
        [req.title.as_ref().map(|_| "title"), req.content.as_ref().map(|_| "content"),
         req.tags.as_ref().map(|_| "tags"), req.content_type.as_ref().map(|_| "content_type"),
         req.parent_id.as_ref().map(|_| "parent_id")]
            .iter().flatten().copied().collect::<Vec<_>>().join(", "));

    Ok(Json(node))
}

// DELETE /nodes/:id
async fn delete_node_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    auth: Option<Extension<AuthContext>>,
) -> Result<StatusCode, AppError> {
    let auth = auth.map(|Extension(ctx)| ctx);

    let existing = state.db.get_node(&id)
        .map_err(|e| AppError::from(e.to_string()))?
        .ok_or_else(|| not_found(format!("Node '{}' not found", id)))?;

    let deleted_by = resolve_author(auth.as_ref(), None);

    // Editor ownership check: editors can only delete their own nodes
    if let Some(ref ctx) = auth {
        if ctx.role == "editor" {
            if existing.author.as_deref() != Some(&ctx.user_name) {
                return Err(forbidden(format!(
                    "Editor '{}' cannot delete node owned by '{}'",
                    ctx.user_name, existing.author.as_deref().unwrap_or("unknown")
                )));
            }
        }
    }

    state.db.delete_node_tracked(&id, &deleted_by)
        .map_err(|e| AppError::from(e.to_string()))?;

    println!("[DELETE /nodes/{}] Deleted by {}", &id[..8], deleted_by);

    Ok(StatusCode::NO_CONTENT)
}

// POST /edges
async fn create_edge_handler(
    State(state): State<AppState>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<CreateEdgeRequest>,
) -> Result<(StatusCode, Json<CreateEdgeResponse>), AppError> {
    validate_edge_input(req.reason.as_deref(), req.author.as_deref())?;
    let auth = auth.map(|Extension(ctx)| ctx);

    // Resolve source — supports UUID, ID prefix, and title text
    let (source_node, source_summary) = match team::resolve_node(&state.db, &req.source) {
        team::ResolveResult::Found(node) => {
            let summary = team::NodeSummary {
                id: node.id.clone(),
                title: node.ai_title.clone().unwrap_or_else(|| node.title.clone()),
            };
            (node, summary)
        }
        team::ResolveResult::Ambiguous(candidates) => {
            let names: Vec<_> = candidates.iter()
                .map(|c| format!("  {} — {}", &c.id[..8], c.title))
                .collect();
            return Err(bad_request(format!(
                "Source '{}' is ambiguous. Candidates:\n{}", req.source, names.join("\n")
            )));
        }
        team::ResolveResult::NotFound(msg) => {
            return Err(bad_request(format!("Source: {}", msg)));
        }
    };

    // Resolve target — same cascade
    let (target_node, target_summary) = match team::resolve_node(&state.db, &req.target) {
        team::ResolveResult::Found(node) => {
            let summary = team::NodeSummary {
                id: node.id.clone(),
                title: node.ai_title.clone().unwrap_or_else(|| node.title.clone()),
            };
            (node, summary)
        }
        team::ResolveResult::Ambiguous(candidates) => {
            let names: Vec<_> = candidates.iter()
                .map(|c| format!("  {} — {}", &c.id[..8], c.title))
                .collect();
            return Err(bad_request(format!(
                "Target '{}' is ambiguous. Candidates:\n{}", req.target, names.join("\n")
            )));
        }
        team::ResolveResult::NotFound(msg) => {
            return Err(bad_request(format!("Target: {}", msg)));
        }
    };

    let author = resolve_author(auth.as_ref(), req.author);
    let edge_type_str = req.edge_type.as_deref().unwrap_or("related");
    let edge_type = EdgeType::from_str(&edge_type_str.to_lowercase())
        .ok_or_else(|| bad_request(format!("Unknown edge type '{}'", edge_type_str)))?;

    let now = chrono::Utc::now().timestamp_millis();
    let edge = Edge {
        id: uuid::Uuid::new_v4().to_string(),
        source: source_node.id,
        target: target_node.id,
        edge_type,
        label: None,
        weight: Some(1.0),
        edge_source: Some("user".to_string()),
        evidence_id: None,
        confidence: Some(1.0),
        created_at: now,
        updated_at: Some(now),
        author: Some(author),
        reason: req.reason,
        content: None,
        agent_id: Some("human".to_string()),
        superseded_by: None,
        metadata: None,
    };

    state.db.insert_edge(&edge).map_err(|e| AppError::from(e.to_string()))?;

    println!("[POST /edges] {} --{}--> {} by {}",
        &source_summary.title, edge_type_str, &target_summary.title,
        edge.author.as_deref().unwrap_or("?"));

    Ok((StatusCode::CREATED, Json(CreateEdgeResponse {
        edge,
        source_resolved: source_summary,
        target_resolved: target_summary,
    })))
}

// PATCH /edges/:id
async fn patch_edge_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<PatchEdgeRequest>,
) -> Result<Json<Edge>, AppError> {
    validate_edge_input(req.reason.as_deref(), req.author.as_deref())?;
    let auth = auth.map(|Extension(ctx)| ctx);

    let existing = state.db.get_edge(&id)
        .map_err(|e| AppError::from(e.to_string()))?
        .ok_or_else(|| not_found(format!("Edge '{}' not found", id)))?;

    // Editor ownership check
    if let Some(ref ctx) = auth {
        if ctx.role == "editor" {
            if existing.author.as_deref() != Some(&ctx.user_name) {
                return Err(forbidden(format!(
                    "Editor '{}' cannot modify edge owned by '{}'",
                    ctx.user_name, existing.author.as_deref().unwrap_or("unknown")
                )));
            }
        }
    }

    let author = resolve_author(auth.as_ref(), req.author);
    let edge_type = req.edge_type.as_deref();

    state.db.update_edge_fields(&id, req.reason.as_deref(), edge_type, Some(&author))
        .map_err(|e| AppError::from(e.to_string()))?;

    println!("[PATCH /edges/{}] Updated by {}", &id[..8], author);

    let edge = state.db.get_edge(&id)
        .map_err(|e| AppError::from(e.to_string()))?
        .ok_or_else(|| not_found(format!("Edge '{}' not found after update", id)))?;

    Ok(Json(edge))
}

// DELETE /edges/:id
async fn delete_edge_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    auth: Option<Extension<AuthContext>>,
) -> Result<StatusCode, AppError> {
    let auth = auth.map(|Extension(ctx)| ctx);

    let existing = state.db.get_edge(&id)
        .map_err(|e| AppError::from(e.to_string()))?
        .ok_or_else(|| not_found(format!("Edge '{}' not found", id)))?;

    let deleted_by = resolve_author(auth.as_ref(), None);

    // Editor ownership check
    if let Some(ref ctx) = auth {
        if ctx.role == "editor" {
            if existing.author.as_deref() != Some(&ctx.user_name) {
                return Err(forbidden(format!(
                    "Editor '{}' cannot delete edge owned by '{}'",
                    ctx.user_name, existing.author.as_deref().unwrap_or("unknown")
                )));
            }
        }
    }

    state.db.delete_edge_tracked(&id, &deleted_by)
        .map_err(|e| AppError::from(e.to_string()))?;

    println!("[DELETE /edges/{}] Deleted by {}", &id[..8], deleted_by);

    Ok(StatusCode::NO_CONTENT)
}

// GET /nodes/:id
async fn get_node_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<NodeWithEdges>, AppError> {
    let node = state.db.get_node(&id)
        .map_err(|e| AppError::from(e.to_string()))?
        .ok_or_else(|| not_found(format!("Node '{}' not found", id)))?;

    let edges = state.db.get_edges_for_node(&id)
        .map_err(|e| AppError::from(e.to_string()))?;

    Ok(Json(NodeWithEdges { node, edges }))
}

// GET /nodes?search=X&since=ISO&limit=N
async fn list_nodes_handler(
    State(state): State<AppState>,
    Query(params): Query<NodesQuery>,
) -> Result<Json<Vec<Node>>, AppError> {
    let limit = params.limit.unwrap_or(100) as usize;

    match (params.search.as_deref(), params.since.as_deref()) {
        // Both search and since — search then filter by recency
        (Some(query), Some(since_str)) => {
            let since_ms = parse_since(since_str)?;
            let results = state.db.search_nodes(query)
                .map_err(|e| AppError::from(e.to_string()))?;
            let filtered: Vec<Node> = results.into_iter()
                .filter(|n| n.updated_at > since_ms)
                .take(limit)
                .collect();
            Ok(Json(filtered))
        }
        // Search only
        (Some(query), None) => {
            let mut results = state.db.search_nodes(query)
                .map_err(|e| AppError::from(e.to_string()))?;
            results.truncate(limit);
            Ok(Json(results))
        }
        // Since only — delta sync
        (None, Some(since_str)) => {
            let since_ms = parse_since(since_str)?;
            let results = state.db.get_nodes_updated_since(since_ms)
                .map_err(|e| AppError::from(e.to_string()))?;
            Ok(Json(results))
        }
        // No params — return recent
        (None, None) => {
            let results = state.db.get_recent_nodes(limit as i32)
                .map_err(|e| AppError::from(e.to_string()))?;
            Ok(Json(results))
        }
    }
}

// GET /edges?since=ISO
async fn list_edges_handler(
    State(state): State<AppState>,
    Query(params): Query<EdgesQuery>,
) -> Result<Json<Vec<Edge>>, AppError> {
    let since_ms = parse_since(&params.since)?;
    let edges = state.db.get_edges_updated_since(since_ms)
        .map_err(|e| AppError::from(e.to_string()))?;
    Ok(Json(edges))
}

// GET /snapshot
async fn snapshot_handler(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let temp_path = std::env::temp_dir().join(format!("mycelica-snapshot-{}.db", uuid::Uuid::new_v4()));
    let temp_str = temp_path.to_string_lossy().to_string();

    state.db.backup_to(&temp_str)
        .map_err(|e| AppError::from(format!("Backup failed: {}", e)))?;

    let bytes = tokio::fs::read(&temp_path).await
        .map_err(|e| AppError::from(format!("Failed to read backup: {}", e)))?;

    // Clean up temp file
    tokio::fs::remove_file(&temp_path).await.ok();

    Ok((
        StatusCode::OK,
        [
            ("content-type", "application/octet-stream"),
            ("content-disposition", "attachment; filename=\"mycelica-snapshot.db\""),
        ],
        bytes,
    ))
}

// GET /recent?n=20
async fn recent_handler(
    State(state): State<AppState>,
    Query(params): Query<LimitQuery>,
) -> Result<Json<Vec<Node>>, AppError> {
    let limit = params.n.or(params.limit).unwrap_or(20) as i32;
    let nodes = state.db.get_recent_nodes(limit)
        .map_err(|e| AppError::from(e.to_string()))?;
    Ok(Json(nodes))
}

// GET /orphans
async fn orphans_handler(
    State(state): State<AppState>,
    Query(params): Query<LimitQuery>,
) -> Result<Json<Vec<Node>>, AppError> {
    let limit = params.n.or(params.limit).unwrap_or(50) as i32;
    let nodes = state.db.get_orphan_nodes(limit)
        .map_err(|e| AppError::from(e.to_string()))?;
    Ok(Json(nodes))
}

// GET /health
async fn health_handler(
    State(state): State<AppState>,
) -> Result<Json<HealthResponse>, AppError> {
    let stats = state.db.get_stats()
        .map_err(|e| AppError::from(e.to_string()))?;
    let edge_count = state.db.get_edge_count()
        .map_err(|e| AppError::from(e.to_string()))?;
    let uptime = state.start_time.elapsed().as_secs();

    Ok(Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        nodes: stats.0,
        edges: edge_count,
        uptime_secs: uptime,
        auth_enabled: !state.no_auth,
    }))
}

// POST /admin/rebuild
async fn rebuild_handler(
    State(state): State<AppState>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<RebuildResponse>, AppError> {
    // Admin-only: require admin role when auth is enabled
    if !state.no_auth {
        let ctx = auth.as_ref()
            .map(|Extension(ctx)| ctx)
            .ok_or_else(|| AppError(StatusCode::UNAUTHORIZED,
                "Authentication required for admin endpoints".to_string()))?;
        if ctx.role != "admin" {
            return Err(forbidden("Only admin users can trigger a rebuild"));
        }
    }

    println!("[POST /admin/rebuild] Starting adaptive hierarchy rebuild...");
    let start = std::time::Instant::now();

    let log: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log_ref = log.clone();

    let config = mycelica_lib::rebuild::RebuildConfig::default();
    let result = mycelica_lib::rebuild::rebuild_adaptive(&state.db, config, &move |msg| {
        println!("[rebuild] {}", msg);
        if let Ok(mut v) = log_ref.lock() {
            v.push(msg.to_string());
        }
    }).await.map_err(AppError::from)?;

    let duration = start.elapsed().as_secs_f64();
    let log_lines = log.lock().map(|v| v.clone()).unwrap_or_default();

    println!("[POST /admin/rebuild] Done in {:.1}s: {} categories, {} papers",
        duration, result.categories, result.papers_assigned);

    Ok(Json(RebuildResponse {
        categories: result.categories,
        papers_assigned: result.papers_assigned,
        sibling_edges: result.sibling_edges,
        bridges: result.bridges,
        duration_secs: duration,
        log: log_lines,
    }))
}

// ============================================================================
// Auto-backup system
// ============================================================================

fn backup_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mycelica-team/backups")
}

fn run_backup(db: &Database, label: &str) {
    let dir = backup_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        eprintln!("[Backup] Failed to create backup directory: {}", dir.display());
        return;
    }
    let now = chrono::Utc::now();
    let filename = format!("{}-{}.db", label, now.format("%Y%m%d-%H%M%S"));
    let path = dir.join(&filename);
    match db.backup_to(&path.to_string_lossy()) {
        Ok(_) => println!("[Backup] {}: {}", label, path.display()),
        Err(e) => eprintln!("[Backup] Failed ({}): {}", label, e),
    }
}

fn prune_backups() {
    let dir = backup_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else { return };

    let mut hourly_files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("hourly-") && name.ends_with(".db") {
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    hourly_files.push((path, modified));
                }
            }
        }
    }

    // Sort newest first
    hourly_files.sort_by(|a, b| b.1.cmp(&a.1));

    // Keep last 24 hourly, delete the rest
    for (path, _) in hourly_files.iter().skip(24) {
        std::fs::remove_file(path).ok();
    }
}

async fn backup_loop(db: Arc<Database>) {
    let mut interval = tokio::time::interval(Duration::from_secs(3600));
    loop {
        interval.tick().await;
        run_backup(&db, "hourly");
        prune_backups();
    }
}

// ============================================================================
// Database path resolution (matches CLI pattern)
// ============================================================================

fn find_database(db_arg: Option<&str>) -> PathBuf {
    // 1. CLI argument
    if let Some(path) = db_arg {
        return PathBuf::from(path);
    }

    // 2. Environment variable
    if let Ok(path) = std::env::var("MYCELICA_DB") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }

    // 3. Walk up directory tree for .mycelica.db
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = cwd.as_path();
        loop {
            let candidate = dir.join(".mycelica.db");
            if candidate.exists() {
                return candidate;
            }
            match dir.parent() {
                Some(p) => dir = p,
                None => break,
            }
        }
    }

    // 4. Default app data directory
    dirs::data_dir()
        .map(|p| p.join("com.mycelica.app/mycelica.db"))
        .unwrap_or_else(|| PathBuf::from("mycelica.db"))
}

// ============================================================================
// Admin commands (run before server starts)
// ============================================================================

fn handle_admin(args: &[String], db: &Database) {
    if args.is_empty() {
        eprintln!("Usage: mycelica-server admin <command>");
        eprintln!("Commands:");
        eprintln!("  create-key <name> [--role admin|editor]  Create an API key");
        eprintln!("  list-keys                                List all API keys");
        eprintln!("  revoke-key <id>                          Revoke an API key");
        std::process::exit(1);
    }

    match args[0].as_str() {
        "create-key" => {
            if args.len() < 2 {
                eprintln!("Usage: mycelica-server admin create-key <name> [--role admin|editor]");
                std::process::exit(1);
            }
            let name = &args[1];
            let mut role = "editor".to_string();
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--role" && i + 1 < args.len() {
                    role = args[i + 1].to_lowercase();
                    if role != "admin" && role != "editor" {
                        eprintln!("Invalid role '{}'. Must be 'admin' or 'editor'.", role);
                        std::process::exit(1);
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }

            // Generate random key (32 bytes = 64 hex chars)
            let raw_key: String = {
                use std::io::Read;
                let mut bytes = [0u8; 32];
                std::fs::File::open("/dev/urandom")
                    .and_then(|mut f| f.read_exact(&mut bytes))
                    .unwrap_or_else(|_| {
                        // Fallback: use timestamp + uuid
                        let fallback = format!("{}{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0), uuid::Uuid::new_v4());
                        bytes.copy_from_slice(&sha2::Sha256::digest(fallback.as_bytes())[..32]);
                    });
                hex::encode(bytes)
            };

            let key_hash = hash_api_key(&raw_key);
            let api_key = ApiKey {
                id: uuid::Uuid::new_v4().to_string(),
                key_hash,
                user_name: name.to_string(),
                role: role.clone(),
                created_at: chrono::Utc::now().timestamp_millis(),
            };

            match db.insert_api_key(&api_key) {
                Ok(_) => {
                    println!("API key created successfully.");
                    println!();
                    println!("  Name: {}", name);
                    println!("  Role: {}", role);
                    println!("  Key:  {}", raw_key);
                    println!();
                    println!("Save this key now — it cannot be retrieved later.");
                    println!("Use it with: Authorization: Bearer {}", raw_key);
                }
                Err(e) => {
                    eprintln!("Failed to create API key: {}", e);
                    std::process::exit(1);
                }
            }
        }
        "list-keys" => {
            match db.list_api_keys() {
                Ok(keys) => {
                    if keys.is_empty() {
                        println!("No API keys configured.");
                        return;
                    }
                    println!("{:<36}  {:<16}  {:<8}  {}", "ID", "Name", "Role", "Created");
                    println!("{}", "-".repeat(80));
                    for key in keys {
                        let created = chrono::DateTime::from_timestamp_millis(key.created_at)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        println!("{:<36}  {:<16}  {:<8}  {}", key.id, key.user_name, key.role, created);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to list API keys: {}", e);
                    std::process::exit(1);
                }
            }
        }
        "revoke-key" => {
            if args.len() < 2 {
                eprintln!("Usage: mycelica-server admin revoke-key <id>");
                std::process::exit(1);
            }
            let id = &args[1];
            match db.delete_api_key(id) {
                Ok(count) if count > 0 => println!("API key {} revoked.", id),
                Ok(_) => {
                    eprintln!("No API key found with ID: {}", id);
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("Failed to revoke API key: {}", e);
                    std::process::exit(1);
                }
            }
        }
        cmd => {
            eprintln!("Unknown admin command: {}", cmd);
            std::process::exit(1);
        }
    }
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() {
    // Parse simple args (no clap to keep binary small)
    let args: Vec<String> = std::env::args().collect();
    let mut db_arg: Option<&str> = None;
    let mut bind_arg: Option<&str> = None;
    let mut no_auth = false;
    let mut admin_args: Option<Vec<String>> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--db" if i + 1 < args.len() => {
                db_arg = Some(&args[i + 1]);
                i += 2;
            }
            "--bind" if i + 1 < args.len() => {
                bind_arg = Some(&args[i + 1]);
                i += 2;
            }
            "--no-auth" => {
                no_auth = true;
                i += 1;
            }
            "admin" => {
                admin_args = Some(args[i + 1..].to_vec());
                break;
            }
            "--help" | "-h" => {
                println!("mycelica-server — Team knowledge graph HTTP API");
                println!();
                println!("Usage: mycelica-server [OPTIONS]");
                println!("       mycelica-server admin <command>");
                println!();
                println!("Options:");
                println!("  --db PATH         Database path");
                println!("  --bind ADDR:PORT  Bind address (default: 127.0.0.1:3741)");
                println!("  --no-auth         Disable authentication (for trusted networks)");
                println!();
                println!("Admin commands:");
                println!("  admin create-key <name> [--role admin|editor]");
                println!("  admin list-keys");
                println!("  admin revoke-key <id>");
                println!();
                println!("Environment variables:");
                println!("  MYCELICA_DB    Database path");
                println!("  MYCELICA_BIND  Bind address");
                std::process::exit(0);
            }
            _ => { i += 1; }
        }
    }

    let db_path = find_database(db_arg);

    // Initialize settings
    let app_data_dir = dirs::data_dir()
        .map(|p| p.join("com.mycelica.app"))
        .unwrap_or_else(|| PathBuf::from("."));
    settings::init(app_data_dir);

    // Open database
    let db = match Database::new(&db_path) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            eprintln!("[Server] Failed to open database {}: {}", db_path.display(), e);
            std::process::exit(1);
        }
    };

    // Handle admin commands (exit after)
    if let Some(admin) = admin_args {
        handle_admin(&admin, &db);
        return;
    }

    let bind_addr = bind_arg
        .map(|s| s.to_string())
        .or_else(|| std::env::var("MYCELICA_BIND").ok())
        .unwrap_or_else(|| "127.0.0.1:3741".to_string());

    println!("[Server] Database: {}", db_path.display());
    println!("[Server] Binding to: {}", bind_addr);
    if no_auth {
        println!("[Server] Authentication: DISABLED (--no-auth)");
    } else {
        let key_count = db.list_api_keys().map(|k| k.len()).unwrap_or(0);
        println!("[Server] Authentication: ENABLED ({} API key(s) configured)", key_count);
        if key_count == 0 {
            eprintln!("[Server] WARNING: Auth enabled but no API keys exist. All writes will be rejected.");
            eprintln!("[Server] Create a key: mycelica-server --db {} admin create-key <name>", db_path.display());
        }
    }

    // Warm up embedding model
    println!("[Server] Warming up embedding model...");
    match local_embeddings::generate("warmup") {
        Ok(_) => println!("[Server] Embedding model ready"),
        Err(e) => eprintln!("[Server] Warning: embedding warmup failed: {}", e),
    }

    // Initial backup
    run_backup(&db, "startup");

    // Start backup loop
    let backup_db = db.clone();
    tokio::spawn(backup_loop(backup_db));

    // Build router
    let rate_limiter = Arc::new(
        RateLimiter::dashmap(Quota::per_minute(NonZeroU32::new(60).unwrap()))
    );

    let state = AppState {
        db,
        start_time: Instant::now(),
        no_auth,
        rate_limiter,
    };

    let app = Router::new()
        .route("/nodes", post(create_node_handler).get(list_nodes_handler))
        .route("/nodes/{id}", get(get_node_handler).patch(patch_node_handler).delete(delete_node_handler))
        .route("/edges", post(create_edge_handler).get(list_edges_handler))
        .route("/edges/{id}", patch(patch_edge_handler).delete(delete_edge_handler))
        .route("/snapshot", get(snapshot_handler))
        .route("/recent", get(recent_handler))
        .route("/orphans", get(orphans_handler))
        .route("/health", get(health_handler))
        .route("/admin/rebuild", post(rebuild_handler))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024)) // 2MB body limit
        .layer(
            CorsLayer::new()
                .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE, Method::OPTIONS])
                .allow_headers(Any)
                .allow_origin(Any) // Auth middleware handles write protection; CORS allows Tauri webview + local dev
        )
        .layer(middleware::from_fn_with_state(state.clone(), rate_limit_middleware))
        .with_state(state);

    // Bind and serve
    let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[Server] Failed to bind to {}: {}", bind_addr, e);
            std::process::exit(1);
        }
    };

    println!("[Server] Listening on {}", bind_addr);
    if let Err(e) = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await {
        eprintln!("[Server] Server error: {}", e);
        std::process::exit(1);
    }
}
