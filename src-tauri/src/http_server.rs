//! HTTP server for browser extension integration
//!
//! Runs on localhost:9876, provides endpoints for:
//! - POST /capture - Create bookmark node from web content
//! - GET /search?q=<query> - Search nodes
//! - GET /status - Check connection status
//! - POST /holerabbit/visit - Record web page visit (Holerabbit extension)

use crate::db::{Database, Node, NodeType, Position};
use crate::holerabbit;
use crate::local_embeddings;
use std::io::Read;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tiny_http::{Header, Method, Request, Response, Server};

const PORT: u16 = 9876;
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Start the HTTP server in a background thread
pub fn start(db: Arc<Database>, app_handle: AppHandle) {
    // Initialize holerabbit - pause all live sessions on startup
    holerabbit::init(&db);

    std::thread::spawn(move || {
        let addr = format!("127.0.0.1:{}", PORT);
        let server = match Server::http(&addr) {
            Ok(s) => {
                println!("[HTTP] Server listening on http://{}", addr);
                s
            }
            Err(e) => {
                eprintln!("[HTTP] Failed to start server on {}: {}", addr, e);
                return;
            }
        };

        for request in server.incoming_requests() {
            let db = db.clone();
            let app = app_handle.clone();
            // Handle request in the same thread (tiny_http is single-threaded by default)
            if let Err(e) = handle_request(request, &db, &app) {
                eprintln!("[HTTP] Error handling request: {}", e);
            }
        }
    });
}

fn handle_request(mut request: Request, db: &Database, app: &AppHandle) -> Result<(), String> {
    let path = request.url().split('?').next().unwrap_or("");
    let method = request.method().clone();

    // Log request
    println!("[HTTP] {} {}", method, request.url());

    let response = match (method, path) {
        (Method::Options, _) => {
            // CORS preflight
            cors_response(Response::from_string(""))
        }
        (Method::Post, "/capture") => {
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body)
                .map_err(|e| format!("Failed to read body: {}", e))?;
            handle_capture(db, &body)
        }
        (Method::Get, "/search") => {
            let query = extract_query_param(request.url(), "q");
            handle_search(db, query.as_deref())
        }
        (Method::Get, "/status") => {
            handle_status()
        }
        // Holerabbit routes
        (Method::Post, "/holerabbit/visit") => {
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body)
                .map_err(|e| format!("Failed to read body: {}", e))?;
            let response = holerabbit::handle_visit(db, &body);
            // Emit event to notify frontend of new visit
            let _ = app.emit("holerabbit:visit", ());
            response
        }
        (Method::Get, "/holerabbit/sessions") => {
            holerabbit::handle_sessions(db)
        }
        (Method::Get, "/holerabbit/live") => {
            holerabbit::handle_live_session(db)
        }
        (Method::Get, path) if path.starts_with("/holerabbit/session/") && !path.contains("/pause") && !path.contains("/resume") && !path.contains("/rename") && !path.contains("/merge") => {
            let session_id = &path["/holerabbit/session/".len()..];
            holerabbit::handle_session_detail(db, session_id)
        }
        // Session control endpoints
        (Method::Post, path) if path.ends_with("/pause") && path.starts_with("/holerabbit/session/") => {
            let session_id = &path["/holerabbit/session/".len()..path.len() - 6]; // strip /pause
            holerabbit::handle_pause_session(db, session_id)
        }
        (Method::Post, path) if path.ends_with("/resume") && path.starts_with("/holerabbit/session/") => {
            let session_id = &path["/holerabbit/session/".len()..path.len() - 7]; // strip /resume
            holerabbit::handle_resume_session(db, session_id)
        }
        (Method::Post, path) if path.ends_with("/rename") && path.starts_with("/holerabbit/session/") => {
            let session_id = path["/holerabbit/session/".len()..path.len() - 7].to_string(); // strip /rename
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body)
                .map_err(|e| format!("Failed to read body: {}", e))?;
            holerabbit::handle_rename_session(db, &session_id, &body)
        }
        (Method::Post, path) if path.ends_with("/merge") && path.starts_with("/holerabbit/session/") => {
            let session_id = path["/holerabbit/session/".len()..path.len() - 6].to_string(); // strip /merge
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body)
                .map_err(|e| format!("Failed to read body: {}", e))?;
            holerabbit::handle_merge_sessions(db, &session_id, &body)
        }
        (Method::Delete, path) if path.starts_with("/holerabbit/session/") => {
            // DELETE /holerabbit/session/{id} - delete session
            let session_id = &path["/holerabbit/session/".len()..];
            println!("[HTTP] DELETE session_id extracted: '{}'", session_id);
            // Ensure it's just the ID (no sub-path like /pause)
            if !session_id.is_empty() && !session_id.contains('/') {
                holerabbit::handle_delete_session(db, session_id)
            } else {
                cors_response(json_response(404, r#"{"error":"Invalid session ID"}"#))
            }
        }
        _ => {
            cors_response(json_response(404, r#"{"error":"Not found"}"#))
        }
    };

    request.respond(response)
        .map_err(|e| format!("Failed to send response: {}", e))
}

fn extract_query_param(url: &str, param: &str) -> Option<String> {
    let query_start = url.find('?')?;
    let query = &url[query_start + 1..];
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
            if key == param {
                return Some(urlencoding::decode(value).unwrap_or_default().to_string());
            }
        }
    }
    None
}

/// POST /capture - Create a bookmark node from web content
fn handle_capture(db: &Database, body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    #[derive(serde::Deserialize)]
    struct CaptureRequest {
        title: String,
        url: String,
        content: String,
        #[serde(default)]
        timestamp: Option<i64>,
    }

    let req: CaptureRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return json_response(400, &format!(r#"{{"error":"Invalid JSON: {}"}}"#, e));
        }
    };

    let node_id = uuid::Uuid::new_v4().to_string();
    let now = req.timestamp.unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

    let node = Node {
        id: node_id.clone(),
        node_type: NodeType::Bookmark,
        title: req.title.clone(),
        url: Some(req.url),
        content: Some(req.content.clone()),
        position: Position { x: 0.0, y: 0.0 },
        created_at: now,
        updated_at: now,
        cluster_id: None,
        cluster_label: None,
        ai_title: None,
        summary: None,
        tags: None,
        emoji: Some("🌐".to_string()),
        is_processed: false,
        depth: 0,          // Will be set when added to hierarchy
        is_item: true,     // Bookmarks are leaf items
        is_universe: false,
        parent_id: None,
        child_count: 0,
        conversation_id: None,
        sequence_index: None,
        is_pinned: false,
        last_accessed_at: Some(now),
        latest_child_date: None,
        is_private: None,
        privacy_reason: None,
        source: Some("firefox".to_string()),
        pdf_available: Some(false),
        content_type: Some("bookmark".to_string()),
        associated_idea_id: None,
        privacy: None,
    };

    // Insert node
    if let Err(e) = db.insert_node(&node) {
        return json_response(500, &format!(r#"{{"error":"Failed to insert node: {}"}}"#, e));
    }

    // Generate and store embedding for the content
    if !req.content.is_empty() {
        // Use title + first part of content for embedding
        let embed_text = format!("{}\n{}", req.title, &req.content[..req.content.len().min(2000)]);
        match local_embeddings::generate(&embed_text) {
            Ok(embedding) => {
                if let Err(e) = db.update_node_embedding(&node_id, &embedding) {
                    eprintln!("[HTTP] Failed to store embedding: {}", e);
                }
            }
            Err(e) => {
                eprintln!("[HTTP] Failed to generate embedding: {}", e);
            }
        }
    }

    println!("[HTTP] Created bookmark node: {} - {}", &node_id[..8], node.title);
    cors_response(json_response(200, &format!(r#"{{"success":true,"nodeId":"{}"}}"#, node_id)))
}

/// GET /search?q=<query> - Search nodes
fn handle_search(db: &Database, query: Option<&str>) -> Response<std::io::Cursor<Vec<u8>>> {
    let query = match query {
        Some(q) if !q.is_empty() => q,
        _ => {
            return json_response(400, r#"{"error":"Missing query parameter 'q'"}"#);
        }
    };

    let results = match db.search_nodes(query) {
        Ok(nodes) => nodes,
        Err(e) => {
            return json_response(500, &format!(r#"{{"error":"Search failed: {}"}}"#, e));
        }
    };

    #[derive(serde::Serialize)]
    struct SearchResult {
        id: String,
        title: String,
        #[serde(rename = "type")]
        node_type: String,
        emoji: Option<String>,
        url: Option<String>,
    }

    let results: Vec<SearchResult> = results.iter().take(20).map(|n| SearchResult {
        id: n.id.clone(),
        title: n.ai_title.clone().unwrap_or_else(|| n.title.clone()),
        node_type: n.node_type.as_str().to_string(),
        emoji: n.emoji.clone(),
        url: n.url.clone(),
    }).collect();

    let json = serde_json::to_string(&serde_json::json!({ "results": results }))
        .unwrap_or_else(|_| r#"{"results":[]}"#.to_string());

    cors_response(json_response(200, &json))
}

/// GET /status - Check connection status
fn handle_status() -> Response<std::io::Cursor<Vec<u8>>> {
    let json = format!(r#"{{"connected":true,"version":"{}"}}"#, VERSION);
    cors_response(json_response(200, &json))
}

fn json_response(status: u16, body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let data = body.as_bytes().to_vec();
    let cursor = std::io::Cursor::new(data);
    Response::new(
        tiny_http::StatusCode(status),
        vec![
            Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
        ],
        cursor,
        Some(body.len()),
        None,
    )
}

fn cors_response(mut response: Response<std::io::Cursor<Vec<u8>>>) -> Response<std::io::Cursor<Vec<u8>>> {
    response.add_header(
        Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap()
    );
    response.add_header(
        Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"GET, POST, DELETE, OPTIONS"[..]).unwrap()
    );
    response.add_header(
        Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type"[..]).unwrap()
    );
    response
}
