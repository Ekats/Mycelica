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
}

impl TeamConfig {
    fn config_dir() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".mycelica-team")
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

        let client = RemoteClient::new(&config.server_url);

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
        Ok(RemoteClient::new(c.base_url()))
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
    let base_url = {
        let c = state.client.lock().map_err(|e| e.to_string())?;
        c.base_url().to_string()
    };
    let client = RemoteClient::new(&base_url);

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
    let client = state.make_client()?;
    client.search(&query, limit.unwrap_or(20)).await
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
        *client = RemoteClient::new(&new_config.server_url);
    }
    {
        let mut config = state.config.lock().map_err(|e| e.to_string())?;
        *config = new_config;
    }

    Ok(())
}
