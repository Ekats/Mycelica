//! Mycelica Team Server — HTTP API for shared knowledge graph.
//!
//! Thin axum server wrapping the shared mycelica_lib database layer.
//! Sovereignty enforcement point: all write endpoints set human_created,
//! human_edited, author, and edge_source correctly.
//!
//! Usage:
//!   MYCELICA_DB=/path/to/team.db MYCELICA_BIND=100.x.x.x:3741 mycelica-server
//!
//! Or with args:
//!   mycelica-server --db /path/to/team.db --bind 0.0.0.0:3741

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response, Json},
    routing::{get, post, patch, delete},
    Router,
};
use mycelica_lib::db::{Database, Node, Edge, EdgeType};
use mycelica_lib::{settings, local_embeddings, team};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::path::PathBuf;
use std::time::{Duration, Instant};

// ============================================================================
// AppState
// ============================================================================

#[derive(Clone)]
struct AppState {
    db: Arc<Database>,
    start_time: Instant,
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

fn resolve_author(req_author: Option<String>) -> String {
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
    Json(req): Json<CreateNodeRequest>,
) -> Result<(StatusCode, Json<CreateNodeResponse>), AppError> {
    let author = resolve_author(req.author);
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
    Json(req): Json<PatchNodeRequest>,
) -> Result<Json<Node>, AppError> {
    // Verify node exists
    state.db.get_node(&id)
        .map_err(|e| AppError::from(e.to_string()))?
        .ok_or_else(|| not_found(format!("Node '{}' not found", id)))?;

    let author = resolve_author(req.author.clone());

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
) -> Result<StatusCode, AppError> {
    state.db.get_node(&id)
        .map_err(|e| AppError::from(e.to_string()))?
        .ok_or_else(|| not_found(format!("Node '{}' not found", id)))?;

    let deleted_by = settings::get_author_or_default();
    state.db.delete_node_tracked(&id, &deleted_by)
        .map_err(|e| AppError::from(e.to_string()))?;

    println!("[DELETE /nodes/{}] Deleted by {}", &id[..8], deleted_by);

    Ok(StatusCode::NO_CONTENT)
}

// POST /edges
async fn create_edge_handler(
    State(state): State<AppState>,
    Json(req): Json<CreateEdgeRequest>,
) -> Result<(StatusCode, Json<CreateEdgeResponse>), AppError> {
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

    let author = resolve_author(req.author);
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
    Json(req): Json<PatchEdgeRequest>,
) -> Result<Json<Edge>, AppError> {
    state.db.get_edge(&id)
        .map_err(|e| AppError::from(e.to_string()))?
        .ok_or_else(|| not_found(format!("Edge '{}' not found", id)))?;

    let author = req.author.as_deref();
    let edge_type = req.edge_type.as_deref();

    state.db.update_edge_fields(&id, req.reason.as_deref(), edge_type, author)
        .map_err(|e| AppError::from(e.to_string()))?;

    println!("[PATCH /edges/{}] Updated by {}",
        &id[..8], author.unwrap_or("?"));

    let edge = state.db.get_edge(&id)
        .map_err(|e| AppError::from(e.to_string()))?
        .ok_or_else(|| not_found(format!("Edge '{}' not found after update", id)))?;

    Ok(Json(edge))
}

// DELETE /edges/:id
async fn delete_edge_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    state.db.get_edge(&id)
        .map_err(|e| AppError::from(e.to_string()))?
        .ok_or_else(|| not_found(format!("Edge '{}' not found", id)))?;

    let deleted_by = settings::get_author_or_default();
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
// Main
// ============================================================================

#[tokio::main]
async fn main() {
    // Parse simple args (no clap to keep binary small)
    let args: Vec<String> = std::env::args().collect();
    let mut db_arg: Option<&str> = None;
    let mut bind_arg: Option<&str> = None;

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
            "--help" | "-h" => {
                println!("mycelica-server — Team knowledge graph HTTP API");
                println!();
                println!("Usage: mycelica-server [--db PATH] [--bind ADDR:PORT]");
                println!();
                println!("Environment variables:");
                println!("  MYCELICA_DB    Database path");
                println!("  MYCELICA_BIND  Bind address (default: 0.0.0.0:3741)");
                std::process::exit(0);
            }
            _ => { i += 1; }
        }
    }

    let bind_addr = bind_arg
        .map(|s| s.to_string())
        .or_else(|| std::env::var("MYCELICA_BIND").ok())
        .unwrap_or_else(|| "0.0.0.0:3741".to_string());

    let db_path = find_database(db_arg);
    println!("[Server] Database: {}", db_path.display());
    println!("[Server] Binding to: {}", bind_addr);

    // Initialize settings
    let app_data_dir = dirs::data_dir()
        .map(|p| p.join("com.mycelica.app"))
        .unwrap_or_else(|| PathBuf::from("."));
    settings::init(app_data_dir);

    // Open database
    let db = match Database::new(&db_path) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            eprintln!("[Server] Failed to open database: {}", e);
            std::process::exit(1);
        }
    };

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
    let state = AppState {
        db,
        start_time: Instant::now(),
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
    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("[Server] Server error: {}", e);
        std::process::exit(1);
    }
}
