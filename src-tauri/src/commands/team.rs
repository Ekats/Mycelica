//! Tauri commands for the team client GUI.
//!
//! Gated behind `#[cfg(feature = "team")]` — not compiled for the personal app.
//! Thin wrappers: writes go through RemoteClient (HTTP), reads come from snapshot DB.

use crate::remote_client::{
    CreateEdgeRequest, CreateEdgeResponse, CreateNodeRequest, CreateNodeResponse, NodeWithEdges,
    PatchNodeRequest, RemoteClient,
};
use crate::db::{Database, Edge, Node};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

// ============================================================================
// Config
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    pub server_url: String,
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

impl TeamConfig {
    fn config_dir() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".local/share/com.mycelica.app/team")
    }

    fn config_path() -> PathBuf {
        Self::config_dir().join("config.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            if let Ok(data) = std::fs::read_to_string(&path) {
                if let Ok(config) = serde_json::from_str(&data) {
                    return config;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = Self::config_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create config dir: {}", e))?;
        let json =
            serde_json::to_string_pretty(self).map_err(|e| format!("Serialize error: {}", e))?;
        std::fs::write(Self::config_path(), json)
            .map_err(|e| format!("Write config error: {}", e))?;
        Ok(())
    }
}

impl Default for TeamConfig {
    fn default() -> Self {
        let author = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "anonymous".to_string());
        Self {
            server_url: "http://localhost:3741".to_string(),
            author,
            snapshot_path: None,
            api_key: None,
        }
    }
}

// ============================================================================
// State
// ============================================================================

pub struct TeamState {
    client: Mutex<RemoteClient>,
    snapshot_db: Mutex<Option<Database>>,
    snapshot_path: PathBuf,
    local_db: Database,
    pub config: Mutex<TeamConfig>,
}

impl TeamState {
    pub fn new(config: TeamConfig) -> Self {
        let dir = TeamConfig::config_dir();
        std::fs::create_dir_all(&dir).ok();

        let snapshot_path = config
            .snapshot_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| dir.join("snapshot.db"));

        let local_db_path = dir.join("local.db");
        let local_db = Database::new(&local_db_path).expect("Failed to open local.db");
        init_local_schema(&local_db);

        let client = RemoteClient::with_api_key(&config.server_url, config.api_key.clone());

        Self {
            client: Mutex::new(client),
            snapshot_db: Mutex::new(None),
            snapshot_path,
            local_db,
            config: Mutex::new(config),
        }
    }

    fn make_client(&self) -> Result<RemoteClient, String> {
        let c = self.client.lock().map_err(|e| e.to_string())?;
        Ok(RemoteClient::with_api_key(c.base_url(), c.api_key().map(|s| s.to_string())))
    }
}

fn init_local_schema(db: &Database) {
    let conn = db.raw_conn();
    let conn = conn.lock().unwrap();
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS personal_nodes (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            content TEXT,
            content_type TEXT DEFAULT 'concept',
            tags TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS personal_edges (
            id TEXT PRIMARY KEY,
            source_id TEXT NOT NULL,
            target_id TEXT NOT NULL,
            edge_type TEXT DEFAULT 'related',
            reason TEXT,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS node_positions (
            node_id TEXT PRIMARY KEY,
            x REAL NOT NULL,
            y REAL NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS fetched_content (
            node_id TEXT PRIMARY KEY,
            url TEXT NOT NULL,
            html TEXT NOT NULL,
            text_content TEXT NOT NULL,
            title TEXT,
            fetched_at INTEGER NOT NULL
        );
        ",
    )
    .expect("Failed to init local schema");
}

// ============================================================================
// Snapshot types
// ============================================================================

#[derive(Serialize)]
pub struct TeamSnapshot {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

// ============================================================================
// Personal data types
// ============================================================================

#[derive(Serialize, Deserialize, Clone)]
pub struct PersonalNode {
    pub id: String,
    pub title: String,
    pub content: Option<String>,
    #[serde(rename = "contentType")]
    pub content_type: Option<String>,
    pub tags: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PersonalEdge {
    pub id: String,
    #[serde(rename = "sourceId")]
    pub source_id: String,
    #[serde(rename = "targetId")]
    pub target_id: String,
    #[serde(rename = "edgeType")]
    pub edge_type: String,
    pub reason: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
}

#[derive(Serialize)]
pub struct PersonalData {
    pub nodes: Vec<PersonalNode>,
    pub edges: Vec<PersonalEdge>,
}

// ============================================================================
// Refresh — the core operation
// ============================================================================

#[tauri::command]
pub async fn team_refresh(state: State<'_, TeamState>) -> Result<TeamSnapshot, String> {
    let (base_url, api_key) = {
        let c = state.client.lock().map_err(|e| e.to_string())?;
        (c.base_url().to_string(), c.api_key().map(|s| s.to_string()))
    };
    let client = RemoteClient::with_api_key(&base_url, api_key);

    // Download snapshot to temp file
    let tmp_path = state.snapshot_path.with_extension("db.tmp");
    client
        .snapshot(&tmp_path.to_string_lossy())
        .await?;

    // Close existing snapshot connection
    {
        let mut db = state.snapshot_db.lock().map_err(|e| e.to_string())?;
        *db = None;
    }

    // Atomic rename
    std::fs::rename(&tmp_path, &state.snapshot_path)
        .map_err(|e| format!("Rename error: {}", e))?;

    // Reopen read-only
    let db = Database::open_readonly(&state.snapshot_path)
        .map_err(|e| format!("Open error: {}", e))?;

    let nodes = db.get_all_nodes(true).map_err(|e| e.to_string())?;
    let edges = db.get_all_edges().map_err(|e| e.to_string())?;

    {
        let mut snapshot = state.snapshot_db.lock().map_err(|e| e.to_string())?;
        *snapshot = Some(db);
    }

    Ok(TeamSnapshot { nodes, edges })
}

// ============================================================================
// Write commands (thin wrappers around RemoteClient)
// ============================================================================

#[tauri::command]
pub async fn team_create_node(
    state: State<'_, TeamState>,
    req: CreateNodeRequest,
) -> Result<CreateNodeResponse, String> {
    let client = state.make_client()?;
    client.create_node(&req).await
}

#[tauri::command]
pub async fn team_update_node(
    state: State<'_, TeamState>,
    id: String,
    req: PatchNodeRequest,
) -> Result<Node, String> {
    let client = state.make_client()?;
    client.patch_node(&id, &req).await
}

#[tauri::command]
pub async fn team_delete_node(
    state: State<'_, TeamState>,
    id: String,
) -> Result<(), String> {
    let client = state.make_client()?;
    client.delete_node(&id).await
}

#[tauri::command]
pub async fn team_create_edge(
    state: State<'_, TeamState>,
    req: CreateEdgeRequest,
) -> Result<CreateEdgeResponse, String> {
    let client = state.make_client()?;
    client.create_edge(&req).await
}

#[tauri::command]
pub async fn team_delete_edge(
    state: State<'_, TeamState>,
    id: String,
) -> Result<(), String> {
    let client = state.make_client()?;
    client.delete_edge(&id).await
}

// ============================================================================
// Read commands
// ============================================================================

#[tauri::command]
pub async fn team_search(
    state: State<'_, TeamState>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<Node>, String> {
    let limit = limit.unwrap_or(20) as usize;
    let mut results: Vec<Node> = Vec::new();

    // Search local snapshot (team nodes)
    {
        let db_guard = state.snapshot_db.lock().map_err(|e| e.to_string())?;
        if let Some(ref db) = *db_guard {
            if let Ok(nodes) = db.search_nodes(&query) {
                if !nodes.is_empty() {
                    results.extend(nodes);
                } else if let Ok(nodes) = db.search_nodes_by_title_substring(&query, limit as i32) {
                    results.extend(nodes);
                }
            }
        }
    }

    // Search personal nodes (local.db)
    {
        let pattern = format!("%{}%", query);
        let conn = state.local_db.raw_conn().lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id, title, content, content_type, tags, created_at, updated_at
             FROM personal_nodes
             WHERE title LIKE ?1 COLLATE NOCASE
             ORDER BY updated_at DESC LIMIT ?2"
        ).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![pattern, limit as i32], |row| {
            Ok(Node {
                id: row.get(0)?,
                node_type: crate::db::NodeType::Thought,
                title: row.get(1)?,
                url: None,
                content: row.get(2)?,
                position: crate::db::Position { x: 0.0, y: 0.0 },
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
                cluster_id: None,
                cluster_label: None,
                depth: 0,
                is_item: true,
                is_universe: false,
                parent_id: None,
                child_count: 0,
                ai_title: None,
                summary: None,
                tags: row.get(4)?,
                emoji: None,
                is_processed: false,
                conversation_id: None,
                sequence_index: None,
                is_pinned: false,
                last_accessed_at: None,
                latest_child_date: None,
                is_private: None,
                privacy_reason: None,
                source: Some("personal".to_string()),
                pdf_available: None,
                content_type: row.get::<_, Option<String>>(3)?,
                associated_idea_id: None,
                privacy: None,
                human_edited: None,
                human_created: true,
                author: None,
                agent_id: None,
                meta_type: None,
                node_class: Some("knowledge".to_string()),
            })
        }).map_err(|e| e.to_string())?;
        for row in rows {
            if let Ok(node) = row {
                results.push(node);
            }
        }
    }

    // Fall back to server if no snapshot and no personal matches
    if results.is_empty() {
        let client = state.make_client()?;
        if let Ok(server_results) = client.search(&query, limit as u32).await {
            results = server_results;
        }
    }

    results.truncate(limit);
    Ok(results)
}

#[tauri::command]
pub async fn team_get_node(
    state: State<'_, TeamState>,
    id: String,
) -> Result<NodeWithEdges, String> {
    let client = state.make_client()?;
    client.get_node(&id).await
}

#[tauri::command]
pub async fn team_get_orphans(
    state: State<'_, TeamState>,
    limit: Option<u32>,
) -> Result<Vec<Node>, String> {
    let client = state.make_client()?;
    client.get_orphans(limit.unwrap_or(50)).await
}

#[tauri::command]
pub async fn team_get_recent(
    state: State<'_, TeamState>,
    limit: Option<u32>,
) -> Result<Vec<Node>, String> {
    let client = state.make_client()?;
    client.get_recent(limit.unwrap_or(20)).await
}

// ============================================================================
// Local graph data (signal imports etc. — reads from local.db's nodes table)
// ============================================================================

#[tauri::command]
pub fn team_get_local_data(
    state: State<'_, TeamState>,
    source: Option<String>,
) -> Result<TeamSnapshot, String> {
    let src = source.as_deref().unwrap_or("signal");
    let nodes = state.local_db.get_nodes_by_source(src).map_err(|e| e.to_string())?;
    let edges = state.local_db.get_edges_for_source_nodes(src).map_err(|e| e.to_string())?;
    Ok(TeamSnapshot { nodes, edges })
}

// ============================================================================
// Personal data commands (local.db — never touches server)
// ============================================================================

#[tauri::command]
pub fn team_create_personal_node(
    state: State<'_, TeamState>,
    title: String,
    content: Option<String>,
    content_type: Option<String>,
    tags: Option<String>,
) -> Result<PersonalNode, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = state.local_db.raw_conn();
    let conn = conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO personal_nodes (id, title, content, content_type, tags, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, title, content, content_type, tags, now, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(PersonalNode {
        id,
        title,
        content,
        content_type,
        tags,
        created_at: now,
        updated_at: now,
    })
}

#[tauri::command]
pub fn team_create_personal_edge(
    state: State<'_, TeamState>,
    source_id: String,
    target_id: String,
    edge_type: Option<String>,
    reason: Option<String>,
) -> Result<PersonalEdge, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    let et = edge_type.unwrap_or_else(|| "related".to_string());
    let conn = state.local_db.raw_conn();
    let conn = conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO personal_edges (id, source_id, target_id, edge_type, reason, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, source_id, target_id, et, reason, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(PersonalEdge {
        id,
        source_id,
        target_id,
        edge_type: et,
        reason,
        created_at: now,
    })
}

#[tauri::command]
pub fn team_delete_personal_node(
    state: State<'_, TeamState>,
    id: String,
) -> Result<(), String> {
    let conn = state.local_db.raw_conn();
    let conn = conn.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM personal_edges WHERE source_id = ?1 OR target_id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM personal_nodes WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn team_update_personal_node(
    state: State<'_, TeamState>,
    id: String,
    title: Option<String>,
    content: Option<String>,
    content_type: Option<String>,
    tags: Option<String>,
) -> Result<PersonalNode, String> {
    let conn = state.local_db.raw_conn();
    let conn = conn.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp_millis();

    // Build dynamic UPDATE
    let mut sets = Vec::new();
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;

    if let Some(ref t) = title {
        idx += 1; sets.push(format!("title = ?{}", idx));
        values.push(Box::new(t.clone()));
    }
    if let Some(ref c) = content {
        idx += 1; sets.push(format!("content = ?{}", idx));
        values.push(Box::new(c.clone()));
    }
    if let Some(ref ct) = content_type {
        idx += 1; sets.push(format!("content_type = ?{}", idx));
        values.push(Box::new(ct.clone()));
    }
    if let Some(ref tg) = tags {
        idx += 1; sets.push(format!("tags = ?{}", idx));
        values.push(Box::new(tg.clone()));
    }

    if !sets.is_empty() {
        idx += 1;
        sets.push(format!("updated_at = ?{}", idx));
        values.push(Box::new(now));

        let sql = format!("UPDATE personal_nodes SET {} WHERE id = ?1", sets.join(", "));
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        all_params.push(Box::new(id.clone()));
        all_params.extend(values);
        conn.execute(&sql, rusqlite::params_from_iter(all_params.iter().map(|v| v.as_ref())))
            .map_err(|e| e.to_string())?;
    }

    // Return updated node
    conn.query_row(
        "SELECT id, title, content, content_type, tags, created_at, updated_at FROM personal_nodes WHERE id = ?1",
        params![id],
        |row| Ok(PersonalNode {
            id: row.get(0)?,
            title: row.get(1)?,
            content: row.get(2)?,
            content_type: row.get(3)?,
            tags: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        }),
    ).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn team_get_personal_data(state: State<'_, TeamState>) -> Result<PersonalData, String> {
    let conn = state.local_db.raw_conn();
    let conn = conn.lock().map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT id, title, content, content_type, tags, created_at, updated_at
             FROM personal_nodes ORDER BY created_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let nodes = stmt
        .query_map([], |row| {
            Ok(PersonalNode {
                id: row.get(0)?,
                title: row.get(1)?,
                content: row.get(2)?,
                content_type: row.get(3)?,
                tags: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT id, source_id, target_id, edge_type, reason, created_at
             FROM personal_edges ORDER BY created_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let edges = stmt
        .query_map([], |row| {
            Ok(PersonalEdge {
                id: row.get(0)?,
                source_id: row.get(1)?,
                target_id: row.get(2)?,
                edge_type: row.get(3)?,
                reason: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;

    Ok(PersonalData { nodes, edges })
}

// ============================================================================
// Position persistence
// ============================================================================

#[derive(Deserialize)]
pub struct NodePosition {
    pub node_id: String,
    pub x: f64,
    pub y: f64,
}

#[tauri::command]
pub fn team_save_positions(
    state: State<'_, TeamState>,
    positions: Vec<NodePosition>,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    let conn = state.local_db.raw_conn();
    let conn = conn.lock().map_err(|e| e.to_string())?;
    for pos in positions {
        conn.execute(
            "INSERT OR REPLACE INTO node_positions (node_id, x, y, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![pos.node_id, pos.x, pos.y, now],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Serialize)]
pub struct SavedPosition {
    pub node_id: String,
    pub x: f64,
    pub y: f64,
}

#[tauri::command]
pub fn team_get_positions(state: State<'_, TeamState>) -> Result<Vec<SavedPosition>, String> {
    let conn = state.local_db.raw_conn();
    let conn = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT node_id, x, y FROM node_positions")
        .map_err(|e| e.to_string())?;
    let positions = stmt
        .query_map([], |row| {
            Ok(SavedPosition {
                node_id: row.get(0)?,
                x: row.get(1)?,
                y: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(positions)
}

// ============================================================================
// Settings commands
// ============================================================================

#[tauri::command]
pub fn team_get_settings(state: State<'_, TeamState>) -> Result<TeamConfig, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    Ok(config.clone())
}

#[tauri::command]
pub fn team_save_settings(
    state: State<'_, TeamState>,
    new_config: TeamConfig,
) -> Result<(), String> {
    new_config.save()?;

    {
        let mut client = state.client.lock().map_err(|e| e.to_string())?;
        *client = RemoteClient::with_api_key(&new_config.server_url, new_config.api_key.clone());
    }
    {
        let mut config = state.config.lock().map_err(|e| e.to_string())?;
        *config = new_config;
    }

    Ok(())
}

// ============================================================================
// URL content fetching (local.db cache)
// ============================================================================

#[derive(Serialize, Deserialize, Clone)]
pub struct FetchedContent {
    #[serde(rename = "nodeId")]
    pub node_id: String,
    pub url: String,
    pub html: String,
    #[serde(rename = "textContent")]
    pub text_content: String,
    pub title: Option<String>,
    #[serde(rename = "fetchedAt")]
    pub fetched_at: i64,
}

#[tauri::command]
pub async fn team_fetch_url(
    state: State<'_, TeamState>,
    node_id: String,
    url: String,
) -> Result<FetchedContent, String> {
    // Fetch the URL
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("Mycelica/0.9")
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Fetch error: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status.as_u16(), status.canonical_reason().unwrap_or("error")));
    }

    // Check content type
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    if !content_type.contains("text/html") && !content_type.contains("text/xhtml") && !content_type.contains("application/xhtml") {
        return Err(format!("Not HTML content (got: {})", content_type));
    }

    // Read body (truncate at 1MB)
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Read body error: {}", e))?;
    let body = if body.len() > 1_048_576 {
        body[..1_048_576].to_string()
    } else {
        body
    };

    // Extract readable content and sanitize
    let (html, text_content, title) = extract_and_sanitize(&body, &url)?;

    let now = chrono::Utc::now().timestamp_millis();

    // Store in local.db
    {
        let conn = state.local_db.raw_conn();
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO fetched_content (node_id, url, html, text_content, title, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![node_id, url, html, text_content, title, now],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(FetchedContent {
        node_id,
        url,
        html,
        text_content,
        title,
        fetched_at: now,
    })
}

#[cfg(feature = "team")]
fn extract_and_sanitize(raw_html: &str, base_url: &str) -> Result<(String, String, Option<String>), String> {
    use scraper::{Html, Selector};

    let document = Html::parse_document(raw_html);

    // Extract title
    let title = Selector::parse("title")
        .ok()
        .and_then(|sel| document.select(&sel).next())
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|t| !t.is_empty());

    // Try readability selectors in order
    let selectors = [
        "article",
        "[role=main]",
        "main",
        "#content",
        ".post-content",
        ".article-body",
        ".entry-content",
    ];

    let mut inner_html = None;
    for sel_str in &selectors {
        if let Ok(sel) = Selector::parse(sel_str) {
            if let Some(el) = document.select(&sel).next() {
                inner_html = Some(el.inner_html());
                break;
            }
        }
    }

    // Fall back to body
    if inner_html.is_none() {
        if let Ok(sel) = Selector::parse("body") {
            if let Some(el) = document.select(&sel).next() {
                inner_html = Some(el.inner_html());
            }
        }
    }

    let inner_html = inner_html.unwrap_or_else(|| raw_html.to_string());

    // Extract plain text from the content
    let content_doc = Html::parse_fragment(&inner_html);
    let text_content: String = content_doc.root_element().text().collect::<Vec<_>>().join(" ");
    let text_content = text_content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    // Sanitize with ammonia
    let base = url::Url::parse(base_url).ok();
    let clean_html = ammonia::Builder::new()
        .tags(std::collections::HashSet::from([
            "p", "h1", "h2", "h3", "h4", "h5", "h6",
            "a", "img", "ul", "ol", "li",
            "blockquote", "pre", "code", "em", "strong",
            "table", "thead", "tbody", "tr", "td", "th",
            "br", "hr", "div", "span",
            "figure", "figcaption", "sup", "sub",
        ]))
        .link_rel(Some("noopener noreferrer"))
        .url_relative(if let Some(ref base) = base {
            ammonia::UrlRelative::RewriteWithBase(base.clone())
        } else {
            ammonia::UrlRelative::Deny
        })
        .clean(&inner_html)
        .to_string();

    Ok((clean_html, text_content, title))
}

#[tauri::command]
pub fn team_get_fetched_content(
    state: State<'_, TeamState>,
    node_id: String,
) -> Result<Option<FetchedContent>, String> {
    let conn = state.local_db.raw_conn();
    let conn = conn.lock().map_err(|e| e.to_string())?;
    let result = conn.query_row(
        "SELECT node_id, url, html, text_content, title, fetched_at
         FROM fetched_content WHERE node_id = ?1",
        params![node_id],
        |row| {
            Ok(FetchedContent {
                node_id: row.get(0)?,
                url: row.get(1)?,
                html: row.get(2)?,
                text_content: row.get(3)?,
                title: row.get(4)?,
                fetched_at: row.get(5)?,
            })
        },
    );
    match result {
        Ok(fc) => Ok(Some(fc)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
