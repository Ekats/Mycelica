//! Holerabbit browser extension backend
//!
//! Handles web browsing sessions from Firefox extension
//! Creates web page nodes, session containers, and navigation edges

use crate::db::{Database, Edge, EdgeType, Node, NodeType, Position};
use crate::local_embeddings;
use crate::settings::HOLERABBIT_CONTAINER_ID;
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tiny_http::Response;

// ==================== Initialization ====================

/// Initialize holerabbit on startup:
/// 1. Create Holerabbit container node if missing
/// 2. Reparent orphan sessions under the container
/// 3. Pause all live sessions
pub fn init(db: &Database) {
    // Ensure container exists
    let container_id = get_or_create_container(db);

    if let Ok(sessions) = db.get_nodes_by_content_type("session") {
        for s in &sessions {
            // Reparent orphan sessions
            if s.parent_id.is_none() || s.parent_id.as_deref() != Some(container_id.as_str()) {
                if let Err(e) = db.update_node_parent(&s.id, &container_id) {
                    eprintln!("[Holerabbit] Failed to reparent session {}: {}", s.id, e);
                }
            }

            // Pause live sessions
            let tags_str = s.tags.clone().unwrap_or_default();
            if let Ok(mut tags) = serde_json::from_str::<serde_json::Value>(&tags_str) {
                if tags["status"].as_str() == Some("live") {
                    tags["status"] = json!("paused");
                    if let Err(e) = db.update_node_tags(&s.id, &tags.to_string()) {
                        eprintln!("[Holerabbit] Failed to pause session {}: {}", s.id, e);
                    } else {
                        println!("[Holerabbit] Paused session {} on startup", s.id);
                    }
                }
            }
        }

        // Update container child count
        let _ = db.update_child_count(&container_id, sessions.len() as i32);
    }
}

/// Get or create the Holerabbit container node (under Universe)
fn get_or_create_container(db: &Database) -> String {
    // Check if container exists
    if let Ok(Some(_)) = db.get_node(HOLERABBIT_CONTAINER_ID) {
        return HOLERABBIT_CONTAINER_ID.to_string();
    }

    // Get Universe as parent
    let universe_id = db.get_universe()
        .ok()
        .flatten()
        .map(|u| u.id)
        .unwrap_or_default();

    let now = Utc::now().timestamp_millis();
    let node = Node {
        id: HOLERABBIT_CONTAINER_ID.to_string(),
        node_type: NodeType::Cluster,
        title: "Holerabbit".to_string(),
        url: None,
        content: None,
        position: Position { x: 0.0, y: 0.0 },
        created_at: now,
        updated_at: now,
        cluster_id: None,
        cluster_label: None,
        ai_title: Some("Browsing Sessions".to_string()),
        summary: Some("Web browsing sessions tracked by Holerabbit extension".to_string()),
        tags: None,
        emoji: Some("🐇".to_string()),
        is_processed: true,
        depth: 1,
        is_item: false,
        is_universe: false,
        parent_id: if universe_id.is_empty() { None } else { Some(universe_id) },
        child_count: 0,
        conversation_id: None,
        sequence_index: None,
        is_pinned: false,
        last_accessed_at: Some(now),
        latest_child_date: None,
        is_private: None,
        privacy_reason: None,
        source: Some("holerabbit".to_string()),
        pdf_available: Some(false),
        content_type: Some("holerabbit_container".to_string()),
        associated_idea_id: None,
        privacy: None,
        human_edited: None,
        human_created: false,
        author: None,
        agent_id: None,
        node_class: None,
        meta_type: None,
    };

    if let Err(e) = db.insert_node(&node) {
        eprintln!("[Holerabbit] Failed to create container: {}", e);
    } else {
        println!("[Holerabbit] Created Holerabbit container node");
    }

    HOLERABBIT_CONTAINER_ID.to_string()
}

// ==================== Request/Response Types ====================

#[derive(Deserialize)]
pub struct VisitRequest {
    pub url: String,
    pub referrer: Option<String>,
    pub timestamp: i64,
    pub tab_id: i32,
    pub navigation_type: String, // "clicked", "searched", "backtracked"
    pub previous_dwell_time_ms: i64,
    #[serde(default = "default_gap")]
    pub session_gap_minutes: i64,
    pub title: Option<String>,
    pub content: Option<String>,
    /// Session ID from extension - if provided, use this session directly
    pub session_id: Option<String>,
}

fn default_gap() -> i64 {
    30
}

#[derive(Serialize)]
pub struct VisitResponse {
    pub success: bool,
    pub node_id: String,
    pub session_id: String,
    pub session_name: String,
    pub is_new_session: bool,
}

// ==================== Helper Functions ====================

/// Generate node ID from URL (32 hex chars = 128 bits)
fn hash_url(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result[..16])
}

/// Extract domain from URL
fn extract_domain(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_default()
}

/// Extract title from URL path (fallback when no title provided)
fn extract_title_from_url(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back())
                .map(|s| s.to_string())
        })
        .map(|s| {
            // Decode percent-encoding and replace underscores/dashes with spaces
            let decoded = urlencoding::decode(&s).unwrap_or_default().to_string();
            decoded.replace('_', " ").replace('-', " ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Untitled".to_string())
}

/// Format timestamp as human-readable datetime
fn format_datetime(timestamp_ms: i64) -> String {
    Utc.timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

/// Fetch page content server-side
fn fetch_page(url: &str) -> Result<(String, String), String> {
    // Use blocking reqwest since we're in a sync context
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (compatible; Mycelica/1.0)")
        .build()
        .map_err(|e| e.to_string())?;

    let response = client.get(url).send().map_err(|e| e.to_string())?;
    let html = response.text().map_err(|e| e.to_string())?;

    // Extract title from HTML
    let title = html
        .find("<title>")
        .and_then(|start| {
            let rest = &html[start + 7..];
            rest.find("</title>").map(|end| {
                // Decode HTML entities in title
                html_escape::decode_html_entities(&rest[..end]).to_string()
            })
        })
        .unwrap_or_else(|| extract_title_from_url(url));

    Ok((title, html))
}

// ==================== Session Management ====================

/// Get or create a session for the current visit
/// Priority: live session > extension's session_id > create new
fn get_or_create_session(
    db: &Database,
    timestamp: i64,
    _gap_minutes: i64,
    explicit_session_id: Option<&str>,
) -> Result<(String, bool), String> {
    let sessions = db
        .get_nodes_by_content_type("session")
        .map_err(|e| e.to_string())?;

    // Priority 1: Use existing live session (set by app or extension)
    for s in &sessions {
        let tags_str = s.tags.clone().unwrap_or_default();
        if let Ok(tags) = serde_json::from_str::<serde_json::Value>(&tags_str) {
            if tags["status"].as_str() == Some("live") {
                // Update last_activity and continue this session
                let mut tags = tags;
                tags["last_activity"] = json!(timestamp);
                db.update_node_tags(&s.id, &tags.to_string())
                    .map_err(|e| e.to_string())?;
                return Ok((s.id.clone(), false));
            }
        }
    }

    // Priority 2: Use extension's session_id if provided (and set it live)
    if let Some(sid) = explicit_session_id {
        let (session_id, is_new) = ensure_session_exists(db, sid, timestamp)?;
        // Set it live
        if let Some(session) = db.get_node(&session_id).map_err(|e| e.to_string())? {
            let mut tags: serde_json::Value =
                serde_json::from_str(&session.tags.unwrap_or_default()).unwrap_or(json!({}));
            tags["status"] = json!("live");
            tags["last_activity"] = json!(timestamp);
            db.update_node_tags(&session_id, &tags.to_string())
                .map_err(|e| e.to_string())?;
        }
        return Ok((session_id, is_new));
    }

    // Priority 3: Create new live session
    let session_id = format!("session-{}", timestamp);
    let now = Utc::now().timestamp_millis();
    let container_id = get_or_create_container(db);

    let node = Node {
        id: session_id.clone(),
        node_type: NodeType::Cluster,
        title: format!("Session {}", format_datetime(timestamp)),
        url: None,
        content: None,
        position: Position { x: 0.0, y: 0.0 },
        created_at: now,
        updated_at: now,
        cluster_id: None,
        cluster_label: None,
        ai_title: None,
        summary: None,
        tags: Some(
            json!({
                "start_time": timestamp,
                "last_activity": timestamp,
                "status": "live"
            })
            .to_string(),
        ),
        emoji: Some("🐇".to_string()),
        is_processed: true,
        depth: 2,  // Container is depth 1, sessions are depth 2
        is_item: false,
        is_universe: false,
        parent_id: Some(container_id),
        child_count: 0,
        conversation_id: None,
        sequence_index: None,
        is_pinned: false,
        last_accessed_at: Some(now),
        latest_child_date: None,
        is_private: None,
        privacy_reason: None,
        source: Some("holerabbit".to_string()),
        pdf_available: Some(false),
        content_type: Some("session".to_string()),
        associated_idea_id: None,
        privacy: None,
        human_edited: None,
        human_created: false,
        author: None,
        agent_id: None,
        node_class: None,
        meta_type: None,
    };

    db.insert_node(&node).map_err(|e| e.to_string())?;
    println!(
        "[Holerabbit] Created new session: {} at {}",
        session_id,
        format_datetime(timestamp)
    );

    Ok((session_id, true))
}

/// Ensure a session exists with the given ID, create if missing
/// Used when extension provides explicit session_id
fn ensure_session_exists(
    db: &Database,
    session_id: &str,
    timestamp: i64,
) -> Result<(String, bool), String> {
    // Check if session already exists
    if let Some(session) = db.get_node(session_id).map_err(|e| e.to_string())? {
        // Update last_activity
        let mut tags: serde_json::Value =
            serde_json::from_str(&session.tags.unwrap_or_default()).unwrap_or(json!({}));
        tags["last_activity"] = json!(timestamp);
        db.update_node_tags(session_id, &tags.to_string())
            .map_err(|e| e.to_string())?;
        return Ok((session_id.to_string(), false));
    }

    // Create new session with the provided ID
    let now = Utc::now().timestamp_millis();
    let container_id = get_or_create_container(db);

    let node = Node {
        id: session_id.to_string(),
        node_type: NodeType::Cluster,
        title: format!("Session {}", format_datetime(timestamp)),
        url: None,
        content: None,
        position: Position { x: 0.0, y: 0.0 },
        created_at: now,
        updated_at: now,
        cluster_id: None,
        cluster_label: None,
        ai_title: None,
        summary: None,
        tags: Some(
            json!({
                "start_time": timestamp,
                "last_activity": timestamp,
                "status": "live"
            })
            .to_string(),
        ),
        emoji: Some("🐇".to_string()),
        is_processed: true,
        depth: 2,  // Container is depth 1, sessions are depth 2
        is_item: false,
        is_universe: false,
        parent_id: Some(container_id),
        child_count: 0,
        conversation_id: None,
        sequence_index: None,
        is_pinned: false,
        last_accessed_at: Some(now),
        latest_child_date: None,
        is_private: None,
        privacy_reason: None,
        source: Some("holerabbit".to_string()),
        pdf_available: Some(false),
        content_type: Some("session".to_string()),
        associated_idea_id: None,
        privacy: None,
        human_edited: None,
        human_created: false,
        author: None,
        agent_id: None,
        node_class: None,
        meta_type: None,
    };

    db.insert_node(&node).map_err(|e| e.to_string())?;
    println!(
        "[Holerabbit] Created session from extension: {} at {}",
        session_id,
        format_datetime(timestamp)
    );

    Ok((session_id.to_string(), true))
}

// ==================== Main Visit Handler ====================

/// Handle POST /holerabbit/visit
pub fn handle_visit(db: &Database, body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    // Parse request
    let req: VisitRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return error_response(400, &format!("Invalid JSON: {}", e));
        }
    };

    // Process the visit
    match process_visit(db, &req) {
        Ok(response) => {
            let json = serde_json::to_string(&response).unwrap_or_else(|_| {
                r#"{"success":false,"error":"Serialization failed"}"#.to_string()
            });
            success_response(&json)
        }
        Err(e) => error_response(500, &e),
    }
}

fn process_visit(db: &Database, req: &VisitRequest) -> Result<VisitResponse, String> {
    let node_id = format!("web-{}", hash_url(&req.url));
    let now = Utc::now().timestamp_millis();

    // Get or fetch content
    let (title, content) = match (&req.title, &req.content) {
        (Some(t), Some(c)) => (t.clone(), c.clone()),
        (Some(t), None) => {
            // Have title, try to fetch content
            match fetch_page(&req.url) {
                Ok((_, c)) => (t.clone(), c),
                Err(_) => (t.clone(), String::new()),
            }
        }
        _ => {
            // Server-side fetch both
            match fetch_page(&req.url) {
                Ok((t, c)) => (t, c),
                Err(_) => (extract_title_from_url(&req.url), String::new()),
            }
        }
    };

    // Check if node exists
    let existing = db.get_node(&node_id).map_err(|e| e.to_string())?;

    if let Some(node) = existing {
        // Update visit count in tags
        let mut tags: serde_json::Value =
            serde_json::from_str(&node.tags.unwrap_or_default()).unwrap_or(json!({}));
        let visit_count = tags["visit_count"].as_i64().unwrap_or(0) + 1;
        tags["visit_count"] = json!(visit_count);
        tags["last_visit"] = json!(req.timestamp);
        let total_dwell = tags["total_dwell_time_ms"].as_i64().unwrap_or(0) + req.previous_dwell_time_ms;
        tags["total_dwell_time_ms"] = json!(total_dwell);

        db.update_node_tags(&node_id, &tags.to_string())
            .map_err(|e| e.to_string())?;

        println!(
            "[Holerabbit] Updated visit #{} for: {}",
            visit_count,
            &node_id[..12]
        );
    } else {
        // Create new web node
        let node = Node {
            id: node_id.clone(),
            node_type: NodeType::Bookmark, // Web pages are like bookmarks
            title: title.clone(),
            url: Some(req.url.clone()),
            content: Some(content.clone()),
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: None,
            ai_title: None,
            summary: None,
            tags: Some(
                json!({
                    "url": req.url,
                    "domain": extract_domain(&req.url),
                    "first_visit": req.timestamp,
                    "visit_count": 1,
                    "total_dwell_time_ms": req.previous_dwell_time_ms
                })
                .to_string(),
            ),
            emoji: Some("🌐".to_string()),
            is_processed: true,
            depth: 3,  // Container=1, Session=2, Visit=3
            is_item: true,
            is_universe: false,
            parent_id: None,  // Linked to session via edges, not hierarchy
            child_count: 0,
            conversation_id: None,
            sequence_index: None,
            is_pinned: false,
            last_accessed_at: Some(now),
            latest_child_date: None,
            is_private: None,
            privacy_reason: None,
            source: Some("holerabbit".to_string()),
            pdf_available: Some(false),
            content_type: Some("web".to_string()),
            associated_idea_id: None,
            privacy: None,
            human_edited: None,
            human_created: false,
            author: None,
            agent_id: None,
            node_class: None,
            meta_type: None,
        };

        db.insert_node(&node).map_err(|e| e.to_string())?;

        // Generate embedding for new nodes
        if !content.is_empty() {
            let embed_text = format!("{}\n{}", title, &content[..content.len().min(2000)]);
            if let Ok(embedding) = local_embeddings::generate(&embed_text) {
                if let Err(e) = db.update_node_embedding(&node_id, &embedding) {
                    eprintln!("[Holerabbit] Failed to store embedding: {}", e);
                }
            }
        }

        println!("[Holerabbit] Created web node: {} - {}", &node_id[..12], title);
    }

    // Get or create session
    let (session_id, is_new_session) =
        get_or_create_session(db, req.timestamp, req.session_gap_minutes, req.session_id.as_deref())?;

    // Get session name for response
    let session_name = db.get_node(&session_id)
        .ok()
        .flatten()
        .map(|n| n.title)
        .unwrap_or_else(|| session_id.clone());

    // Set entry_point if this is the first item in session
    if is_new_session {
        let mut tags: serde_json::Value = db
            .get_node(&session_id)
            .map_err(|e| e.to_string())?
            .and_then(|n| n.tags)
            .map(|t| serde_json::from_str(&t).unwrap_or(json!({})))
            .unwrap_or(json!({}));
        tags["entry_point"] = json!(node_id);
        db.update_node_tags(&session_id, &tags.to_string())
            .map_err(|e| e.to_string())?;
    }

    // Create session_item edge
    let item_count = db
        .get_edge_count_by_source_and_type(&session_id, "session_item")
        .map_err(|e| e.to_string())?;

    let session_edge = Edge {
        id: format!("session-{}-{}-{}", &session_id, &node_id, req.timestamp),
        source: session_id.clone(),
        target: node_id.clone(),
        edge_type: EdgeType::SessionItem,
        label: Some(
            json!({
                "order": item_count,
                "timestamp": req.timestamp,
                "dwell_time_ms": req.previous_dwell_time_ms
            })
            .to_string(),
        ),
        weight: Some(1.0),
        edge_source: Some("holerabbit".to_string()),
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

    db.insert_edge(&session_edge).map_err(|e| e.to_string())?;

    // Create navigation edge based on type
    if let Some(ref referrer) = req.referrer {
        let ref_id = format!("web-{}", hash_url(referrer));

        // Only create edge if referrer node exists
        if db.get_node(&ref_id).map_err(|e| e.to_string())?.is_some() {
            let edge_type = match req.navigation_type.as_str() {
                "backtracked" => EdgeType::Backtracked,
                _ => EdgeType::Clicked,
            };

            let nav_edge = Edge {
                id: format!("nav-{}-{}-{}", &ref_id, &node_id, req.timestamp),
                source: ref_id,
                target: node_id.clone(),
                edge_type,
                label: Some(
                    json!({
                        "timestamp": req.timestamp,
                        "session_id": session_id,
                        "tab_id": req.tab_id
                    })
                    .to_string(),
                ),
                weight: Some(1.0),
                edge_source: Some("holerabbit".to_string()),
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

            db.insert_edge(&nav_edge).map_err(|e| e.to_string())?;
            println!(
                "[Holerabbit] Created {} edge from referrer",
                req.navigation_type
            );
        }
    }

    Ok(VisitResponse {
        success: true,
        node_id,
        session_id,
        session_name,
        is_new_session,
    })
}

// ==================== Session List/Detail Response Types ====================

#[derive(Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub start_time: i64,
    pub duration_ms: i64,
    pub item_count: i32,
    pub status: String, // "live", "paused", "archived"
    pub entry_point: Option<String>,
    pub entry_title: Option<String>,
}

#[derive(Serialize)]
pub struct SessionItem {
    pub node_id: String,
    pub title: String,
    pub url: String,
    pub order: i32,
    pub timestamp: i64,
    pub dwell_time_ms: i64,
    pub visit_count: i32,
}

#[derive(Serialize)]
pub struct NavigationEdge {
    pub from_id: String,
    pub to_id: String,
    pub edge_type: String, // "clicked" or "backtracked"
    pub timestamp: i64,
}

#[derive(Serialize)]
pub struct SessionDetail {
    pub session: SessionSummary,
    pub items: Vec<SessionItem>,
    pub edges: Vec<NavigationEdge>,
}

#[derive(Serialize)]
pub struct SessionsResponse {
    pub sessions: Vec<SessionSummary>,
}

#[derive(Serialize)]
pub struct LiveSessionResponse {
    pub session: Option<SessionSummary>,
}

// ==================== GET /holerabbit/live ====================

/// Handle GET /holerabbit/live - get current live session (if any)
pub fn handle_live_session(db: &Database) -> Response<std::io::Cursor<Vec<u8>>> {
    match get_live_session(db) {
        Ok(session) => {
            let response = LiveSessionResponse { session };
            let json = serde_json::to_string(&response)
                .unwrap_or_else(|_| r#"{"session":null}"#.to_string());
            success_response(&json)
        }
        Err(e) => error_response(500, &e),
    }
}

fn get_live_session(db: &Database) -> Result<Option<SessionSummary>, String> {
    let sessions = db
        .get_nodes_by_content_type("session")
        .map_err(|e| e.to_string())?;

    for s in sessions {
        let tags_str = s.tags.clone().unwrap_or_default();
        if let Ok(tags) = serde_json::from_str::<serde_json::Value>(&tags_str) {
            if tags["status"].as_str() == Some("live") {
                let entry_point = tags["entry_point"].as_str().map(String::from);
                let entry_title = entry_point.as_ref().and_then(|id| {
                    db.get_node(id)
                        .ok()
                        .flatten()
                        .map(|n| n.ai_title.unwrap_or(n.title))
                });

                let item_count = db
                    .get_edge_count_by_source_and_type(&s.id, "session_item")
                    .unwrap_or(0);

                return Ok(Some(SessionSummary {
                    id: s.id,
                    title: s.ai_title.unwrap_or(s.title),
                    start_time: tags["start_time"].as_i64().unwrap_or(0),
                    duration_ms: tags["last_activity"].as_i64().unwrap_or(0)
                        - tags["start_time"].as_i64().unwrap_or(0),
                    item_count,
                    status: "live".to_string(),
                    entry_point,
                    entry_title,
                }));
            }
        }
    }

    Ok(None)
}

// ==================== GET /holerabbit/sessions ====================

/// Handle GET /holerabbit/sessions - list all sessions
pub fn handle_sessions(db: &Database) -> Response<std::io::Cursor<Vec<u8>>> {
    match get_sessions_list(db) {
        Ok(sessions) => {
            let response = SessionsResponse { sessions };
            let json = serde_json::to_string(&response)
                .unwrap_or_else(|_| r#"{"sessions":[]}"#.to_string());
            success_response(&json)
        }
        Err(e) => error_response(500, &e),
    }
}

fn get_sessions_list(db: &Database) -> Result<Vec<SessionSummary>, String> {
    let sessions = db
        .get_nodes_by_content_type("session")
        .map_err(|e| e.to_string())?;

    let summaries: Vec<SessionSummary> = sessions
        .into_iter()
        .filter_map(|s| {
            let tags_str = s.tags.clone().unwrap_or_default();
            let tags: serde_json::Value = serde_json::from_str(&tags_str).ok()?;

            let entry_point = tags["entry_point"].as_str().map(String::from);
            let entry_title = entry_point.as_ref().and_then(|id| {
                db.get_node(id)
                    .ok()
                    .flatten()
                    .map(|n| n.ai_title.unwrap_or(n.title))
            });

            let item_count = db
                .get_edge_count_by_source_and_type(&s.id, "session_item")
                .unwrap_or(0);

            Some(SessionSummary {
                id: s.id.clone(),
                title: s.ai_title.unwrap_or(s.title),
                start_time: tags["start_time"].as_i64().unwrap_or(0),
                duration_ms: tags["last_activity"].as_i64().unwrap_or(0)
                    - tags["start_time"].as_i64().unwrap_or(0),
                item_count,
                status: tags["status"].as_str().unwrap_or("archived").to_string(),
                entry_point,
                entry_title,
            })
        })
        .collect();

    Ok(summaries)
}

// ==================== GET /holerabbit/session/{id} ====================

/// Handle GET /holerabbit/session/{id} - get session detail
pub fn handle_session_detail(db: &Database, session_id: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    match get_session_detail(db, session_id) {
        Ok(detail) => {
            let json = serde_json::to_string(&detail)
                .unwrap_or_else(|_| r#"{"error":"Serialization failed"}"#.to_string());
            success_response(&json)
        }
        Err(e) => error_response(404, &e),
    }
}

fn get_session_detail(db: &Database, session_id: &str) -> Result<SessionDetail, String> {
    // Get session node
    let session_node = db
        .get_node(session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Session not found".to_string())?;

    let tags_str = session_node.tags.clone().unwrap_or_default();
    let tags: serde_json::Value =
        serde_json::from_str(&tags_str).unwrap_or(json!({}));

    // Build summary
    let entry_point = tags["entry_point"].as_str().map(String::from);
    let entry_title = entry_point.as_ref().and_then(|id| {
        db.get_node(id)
            .ok()
            .flatten()
            .map(|n| n.ai_title.unwrap_or(n.title))
    });

    let item_count = db
        .get_edge_count_by_source_and_type(session_id, "session_item")
        .map_err(|e| e.to_string())?;

    let summary = SessionSummary {
        id: session_node.id.clone(),
        title: session_node.ai_title.unwrap_or(session_node.title),
        start_time: tags["start_time"].as_i64().unwrap_or(0),
        duration_ms: tags["last_activity"].as_i64().unwrap_or(0)
            - tags["start_time"].as_i64().unwrap_or(0),
        item_count,
        status: tags["status"].as_str().unwrap_or("archived").to_string(),
        entry_point,
        entry_title,
    };

    // Get all session_item edges and their target nodes
    let session_edges = db
        .get_edges_by_source_and_type(session_id, "session_item")
        .map_err(|e| e.to_string())?;

    let mut items: Vec<SessionItem> = Vec::new();

    for edge in &session_edges {
        if let Some(node) = db.get_node(&edge.target).map_err(|e| e.to_string())? {
            let node_tags_str = node.tags.clone().unwrap_or_default();
            let node_tags: serde_json::Value =
                serde_json::from_str(&node_tags_str).unwrap_or(json!({}));

            let edge_meta_str = edge.label.clone().unwrap_or_default();
            let edge_meta: serde_json::Value =
                serde_json::from_str(&edge_meta_str).unwrap_or(json!({}));

            items.push(SessionItem {
                node_id: node.id,
                title: node.ai_title.unwrap_or(node.title),
                url: node_tags["url"].as_str().unwrap_or("").to_string(),
                order: edge_meta["order"].as_i64().unwrap_or(0) as i32,
                timestamp: edge_meta["timestamp"].as_i64().unwrap_or(0),
                dwell_time_ms: edge_meta["dwell_time_ms"].as_i64().unwrap_or(0),
                visit_count: node_tags["visit_count"].as_i64().unwrap_or(1) as i32,
            });
        }
    }

    // Sort by order
    items.sort_by_key(|i| i.order);

    // Get navigation edges (clicked/backtracked) between items in this session
    let item_ids: std::collections::HashSet<_> = items.iter().map(|i| i.node_id.clone()).collect();
    let mut nav_edges: Vec<NavigationEdge> = Vec::new();

    for item in &items {
        // Get outgoing clicked/backtracked edges
        for edge_type_str in &["clicked", "backtracked"] {
            if let Ok(edges) = db.get_edges_by_source_and_type(&item.node_id, edge_type_str) {
                for edge in edges {
                    // Only include if target is in this session
                    if item_ids.contains(&edge.target) {
                        let meta_str = edge.label.clone().unwrap_or_default();
                        let meta: serde_json::Value =
                            serde_json::from_str(&meta_str).unwrap_or(json!({}));

                        // Verify edge belongs to this session
                        if meta["session_id"].as_str() == Some(session_id) {
                            nav_edges.push(NavigationEdge {
                                from_id: edge.source,
                                to_id: edge.target,
                                edge_type: edge_type_str.to_string(),
                                timestamp: meta["timestamp"].as_i64().unwrap_or(0),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(SessionDetail {
        session: summary,
        items,
        edges: nav_edges,
    })
}

// ==================== Session Control Endpoints ====================

/// Handle POST /holerabbit/session/{id}/pause
pub fn handle_pause_session(db: &Database, session_id: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    match set_session_status(db, session_id, "paused") {
        Ok(_) => success_response(r#"{"success":true}"#),
        Err(e) => error_response(500, &e),
    }
}

/// Handle POST /holerabbit/session/{id}/resume
pub fn handle_resume_session(db: &Database, session_id: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    match set_session_status(db, session_id, "live") {
        Ok(_) => success_response(r#"{"success":true}"#),
        Err(e) => error_response(500, &e),
    }
}

fn set_session_status(db: &Database, session_id: &str, status: &str) -> Result<(), String> {
    // When resuming to "live", first pause ALL other live sessions (only one live at a time)
    if status == "live" {
        let sessions = db
            .get_nodes_by_content_type("session")
            .map_err(|e| e.to_string())?;

        for s in sessions {
            if s.id == session_id {
                continue;
            }
            let tags_str = s.tags.clone().unwrap_or_default();
            if let Ok(mut tags) = serde_json::from_str::<serde_json::Value>(&tags_str) {
                if tags["status"].as_str() == Some("live") {
                    tags["status"] = json!("paused");
                    db.update_node_tags(&s.id, &tags.to_string())
                        .map_err(|e| e.to_string())?;
                    println!("[Holerabbit] Auto-paused session {} (only one live allowed)", &s.id);
                }
            }
        }
    }

    let session = db
        .get_node(session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Session not found".to_string())?;

    let mut tags: serde_json::Value =
        serde_json::from_str(&session.tags.unwrap_or_default()).unwrap_or(json!({}));
    tags["status"] = json!(status);

    // When resuming, update last_activity to now
    if status == "live" {
        let now = chrono::Utc::now().timestamp_millis();
        tags["last_activity"] = json!(now);
    }

    db.update_node_tags(session_id, &tags.to_string())
        .map_err(|e| e.to_string())?;

    println!("[Holerabbit] Session {} status set to {}", session_id, status);
    Ok(())
}

/// Handle POST /holerabbit/session/{id}/rename with body {"title": "new name"}
pub fn handle_rename_session(
    db: &Database,
    session_id: &str,
    body: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    #[derive(Deserialize)]
    struct RenameRequest {
        title: String,
    }

    let req: RenameRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return error_response(400, &format!("Invalid JSON: {}", e)),
    };

    match db.update_node_title(session_id, &req.title) {
        Ok(_) => {
            println!("[Holerabbit] Session {} renamed to: {}", session_id, req.title);
            success_response(r#"{"success":true}"#)
        }
        Err(e) => error_response(500, &e.to_string()),
    }
}

/// Handle POST /holerabbit/session/{id}/merge with body {"target_id": "session-xxx"}
/// Merges source session INTO target session (source is deleted)
pub fn handle_merge_sessions(
    db: &Database,
    source_id: &str,
    body: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    #[derive(Deserialize)]
    struct MergeRequest {
        target_id: String,
    }

    let req: MergeRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return error_response(400, &format!("Invalid JSON: {}", e)),
    };

    match merge_sessions(db, source_id, &req.target_id) {
        Ok(count) => {
            let json = format!(r#"{{"success":true,"items_moved":{}}}"#, count);
            success_response(&json)
        }
        Err(e) => error_response(500, &e),
    }
}

fn merge_sessions(db: &Database, source_id: &str, target_id: &str) -> Result<i32, String> {
    // Verify both sessions exist
    let source = db
        .get_node(source_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Source session not found".to_string())?;
    let target = db
        .get_node(target_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Target session not found".to_string())?;

    // Get source session items
    let source_edges = db
        .get_edges_by_source_and_type(source_id, "session_item")
        .map_err(|e| e.to_string())?;

    // Get current item count in target for ordering
    let target_count = db
        .get_edge_count_by_source_and_type(target_id, "session_item")
        .map_err(|e| e.to_string())?;

    let now = chrono::Utc::now().timestamp_millis();
    let mut moved = 0;

    // Move each item to target session
    for (i, edge) in source_edges.iter().enumerate() {
        // Parse existing edge metadata
        let edge_meta_str = edge.label.clone().unwrap_or_default();
        let mut edge_meta: serde_json::Value =
            serde_json::from_str(&edge_meta_str).unwrap_or(json!({}));

        // Update order to append after existing target items
        edge_meta["order"] = json!(target_count + i as i32);

        // Create new edge in target session
        let new_edge = Edge {
            id: format!("session-{}-{}-{}", target_id, &edge.target, now + i as i64),
            source: target_id.to_string(),
            target: edge.target.clone(),
            edge_type: EdgeType::SessionItem,
            label: Some(edge_meta.to_string()),
            weight: edge.weight,
            edge_source: Some("holerabbit".to_string()),
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

        db.insert_edge(&new_edge).map_err(|e| e.to_string())?;
        moved += 1;
    }

    // Update navigation edges to reference target session
    for edge in &source_edges {
        // Get clicked/backtracked edges that reference source session
        for edge_type_str in &["clicked", "backtracked"] {
            if let Ok(nav_edges) = db.get_edges_by_source_and_type(&edge.target, edge_type_str) {
                for nav_edge in nav_edges {
                    let meta_str = nav_edge.label.clone().unwrap_or_default();
                    let mut meta: serde_json::Value =
                        serde_json::from_str(&meta_str).unwrap_or(json!({}));

                    if meta["session_id"].as_str() == Some(source_id) {
                        // Update session_id reference
                        meta["session_id"] = json!(target_id);
                        // Note: We can't easily update edge label, so we delete and recreate
                        let _ = db.delete_edge(&nav_edge.id);
                        let updated_edge = Edge {
                            label: Some(meta.to_string()),
                            ..nav_edge
                        };
                        let _ = db.insert_edge(&updated_edge);
                    }
                }
            }
        }
    }

    // Update target session metadata
    let source_tags: serde_json::Value =
        serde_json::from_str(&source.tags.unwrap_or_default()).unwrap_or(json!({}));
    let mut target_tags: serde_json::Value =
        serde_json::from_str(&target.tags.unwrap_or_default()).unwrap_or(json!({}));

    // Extend time range if needed
    let source_start = source_tags["start_time"].as_i64().unwrap_or(i64::MAX);
    let target_start = target_tags["start_time"].as_i64().unwrap_or(i64::MAX);
    if source_start < target_start {
        target_tags["start_time"] = json!(source_start);
        // Update entry_point if source started earlier
        if let Some(entry) = source_tags["entry_point"].as_str() {
            target_tags["entry_point"] = json!(entry);
        }
    }

    let source_last = source_tags["last_activity"].as_i64().unwrap_or(0);
    let target_last = target_tags["last_activity"].as_i64().unwrap_or(0);
    if source_last > target_last {
        target_tags["last_activity"] = json!(source_last);
    }

    db.update_node_tags(target_id, &target_tags.to_string())
        .map_err(|e| e.to_string())?;

    // Delete source session edges and node
    db.delete_edges_by_source_and_type(source_id, "session_item")
        .map_err(|e| e.to_string())?;
    db.delete_node(source_id).map_err(|e| e.to_string())?;

    println!(
        "[Holerabbit] Merged session {} into {} ({} items moved)",
        source_id, target_id, moved
    );

    Ok(moved)
}

/// Handle DELETE /holerabbit/session/{id}
pub fn handle_delete_session(db: &Database, session_id: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    match delete_session(db, session_id) {
        Ok(_) => success_response(r#"{"success":true}"#),
        Err(e) => error_response(500, &e),
    }
}

fn delete_session(db: &Database, session_id: &str) -> Result<(), String> {
    // Verify session exists
    let _ = db
        .get_node(session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Session not found".to_string())?;

    // Delete all session_item edges
    db.delete_edges_by_source_and_type(session_id, "session_item")
        .map_err(|e| e.to_string())?;

    // Delete the session node
    db.delete_node(session_id).map_err(|e| e.to_string())?;

    println!("[Holerabbit] Deleted session: {}", session_id);
    Ok(())
}

// ==================== HTTP Response Helpers ====================

fn success_response(body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let data = body.as_bytes().to_vec();
    let cursor = std::io::Cursor::new(data);
    let mut response = Response::new(
        tiny_http::StatusCode(200),
        vec![tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap()],
        cursor,
        Some(body.len()),
        None,
    );
    add_cors_headers(&mut response);
    response
}

fn error_response(status: u16, message: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = format!(r#"{{"success":false,"error":"{}"}}"#, message);
    let data = body.as_bytes().to_vec();
    let cursor = std::io::Cursor::new(data);
    let mut response = Response::new(
        tiny_http::StatusCode(status),
        vec![tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap()],
        cursor,
        Some(body.len()),
        None,
    );
    add_cors_headers(&mut response);
    response
}

fn add_cors_headers(response: &mut Response<std::io::Cursor<Vec<u8>>>) {
    response.add_header(
        tiny_http::Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
    );
    response.add_header(
        tiny_http::Header::from_bytes(
            &b"Access-Control-Allow-Methods"[..],
            &b"GET, POST, DELETE, OPTIONS"[..],
        )
        .unwrap(),
    );
    response.add_header(
        tiny_http::Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type"[..])
            .unwrap(),
    );
}
