use crate::db::{Database, Node, Edge, Position, NodeType};
use crate::clustering::{self, ClusteringResult as ClusterResult};
use crate::ai_client;
use crate::hierarchy;
use crate::import;
use crate::similarity;
use crate::settings;
use tauri::{State, AppHandle, Emitter};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use serde::Serialize;

use crate::utils::safe_truncate;

// Global cancellation flags
static CANCEL_PROCESSING: AtomicBool = AtomicBool::new(false);
pub static CANCEL_REBUILD: AtomicBool = AtomicBool::new(false);

/// Cache for similarity search results with TTL
pub struct SimilarityCache {
    results: HashMap<String, (Vec<(String, f32)>, Instant)>,
    ttl: Duration,
}

impl SimilarityCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            results: HashMap::new(),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    pub fn get(&self, node_id: &str) -> Option<Vec<(String, f32)>> {
        self.results.get(node_id)
            .filter(|(_, time)| time.elapsed() < self.ttl)
            .map(|(results, _)| results.clone())
    }

    pub fn insert(&mut self, node_id: String, results: Vec<(String, f32)>) {
        self.results.insert(node_id, (results, Instant::now()));
    }

    pub fn invalidate(&mut self) {
        self.results.clear();
    }
}

pub struct AppState {
    pub db: RwLock<Arc<Database>>,
    pub similarity_cache: RwLock<SimilarityCache>,
    pub openaire_cancel: std::sync::atomic::AtomicBool,
}

#[tauri::command]
pub fn get_nodes(state: State<AppState>, include_hidden: Option<bool>) -> Result<Vec<Node>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_all_nodes(include_hidden.unwrap_or(false)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_node(state: State<AppState>, id: String) -> Result<Option<Node>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_node(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cancel_processing() -> Result<(), String> {
    println!("[Cancel] AI Processing cancel requested");
    CANCEL_PROCESSING.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub fn cancel_rebuild() -> Result<(), String> {
    println!("[Cancel] Rebuild/Clustering cancel requested");
    CANCEL_REBUILD.store(true, Ordering::SeqCst);
    Ok(())
}

/// Cancel ALL operations (AI processing, clustering, hierarchy, embeddings)
/// Use this for a single "Stop" button that halts everything
#[tauri::command]
pub fn cancel_all() -> Result<(), String> {
    println!("[Cancel] ALL operations cancel requested");
    CANCEL_PROCESSING.store(true, Ordering::SeqCst);
    CANCEL_REBUILD.store(true, Ordering::SeqCst);
    Ok(())
}

/// Check if any operation was cancelled
pub fn is_cancelled() -> bool {
    CANCEL_PROCESSING.load(Ordering::SeqCst) || CANCEL_REBUILD.load(Ordering::SeqCst)
}

/// Check if rebuild was cancelled (for use in other modules)
pub fn is_rebuild_cancelled() -> bool {
    CANCEL_REBUILD.load(Ordering::SeqCst)
}

/// Reset rebuild cancel flag (call at start of rebuild)
pub fn reset_rebuild_cancel() {
    CANCEL_REBUILD.store(false, Ordering::SeqCst);
}

/// Reset all cancel flags (call at start of any operation)
pub fn reset_all_cancel() {
    CANCEL_PROCESSING.store(false, Ordering::SeqCst);
    CANCEL_REBUILD.store(false, Ordering::SeqCst);
}

#[tauri::command]
pub fn create_node(state: State<AppState>, node: Node) -> Result<(), String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.insert_node(&node).map_err(|e| e.to_string())
}

/// Add a quick note - creates note under "Recent Notes" container
#[tauri::command]
pub fn add_note(state: State<AppState>, title: String, content: String) -> Result<String, String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    use uuid::Uuid;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    // 1. Find or create "Recent Notes" container
    let container_id = "container-recent-notes";
    let container_exists = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_node(container_id).ok().flatten().is_some();

    if !container_exists {
        // Get Universe to set as parent
        let universe = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_universe()
            .map_err(|e| e.to_string())?
            .ok_or("No Universe found")?;

        // Create container at depth 1
        let container_node = Node {
            id: container_id.to_string(),
            node_type: NodeType::Cluster,
            title: "Recent Notes".to_string(),
            emoji: Some("📝".to_string()),
            depth: 1,
            is_item: false,
            is_universe: false,
            parent_id: Some(universe.id.clone()),
            child_count: 0,
            created_at: now,
            updated_at: now,
            position: Position { x: 0.0, y: 0.0 },
            url: None,
            content: None,
            cluster_id: None,
            cluster_label: Some("Recent Notes".to_string()),
            ai_title: None,
            summary: Some("User-created notes".to_string()),
            tags: None,
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
        };
        state.db.read().map_err(|e| format!("DB lock error: {}", e))?.insert_node(&container_node).map_err(|e| e.to_string())?;

        // Update Universe child_count
        state.db.read().map_err(|e| format!("DB lock error: {}", e))?.update_child_count(&universe.id, universe.child_count + 1)
            .map_err(|e| e.to_string())?;
    }

    // 2. Create the note
    let note_id = format!("note-{}", Uuid::new_v4());
    let note_title = if title.trim().is_empty() {
        "Untitled Note".to_string()
    } else {
        title
    };

    let note = Node {
        id: note_id.clone(),
        node_type: NodeType::Thought,
        title: note_title,
        content: Some(content),
        depth: 2,
        is_item: true,
        is_universe: false,
        parent_id: Some(container_id.to_string()),
        child_count: 0,
        is_processed: false,
        created_at: now,
        updated_at: now,
        position: Position { x: 0.0, y: 0.0 },
        url: None,
        cluster_id: None,
        cluster_label: None,
        ai_title: None,
        summary: None,
        tags: None,
        emoji: None,
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
    };

    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.insert_node(&note).map_err(|e| e.to_string())?;

    // 3. Update container child_count
    let children = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_children(container_id).map_err(|e| e.to_string())?;
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.update_child_count(container_id, children.len() as i32)
        .map_err(|e| e.to_string())?;

    Ok(note_id)
}

#[tauri::command]
pub fn update_node(state: State<AppState>, node: Node) -> Result<(), String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.update_node(&node).map_err(|e| e.to_string())
}

/// Update just the content of a node (simpler API for editing)
#[tauri::command]
pub fn update_node_content(state: State<AppState>, node_id: String, content: String) -> Result<(), String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.update_node_content(&node_id, &content).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_node(state: State<AppState>, id: String) -> Result<(), String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.delete_node(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_edges(state: State<AppState>) -> Result<Vec<Edge>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_all_edges().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_edges_for_node(state: State<AppState>, node_id: String) -> Result<Vec<Edge>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_edges_for_node(&node_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_edges_for_fos(state: State<AppState>, fos_id: String) -> Result<Vec<Edge>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_edges_for_fos(&fos_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_edges_for_view(state: State<AppState>, parent_id: String) -> Result<Vec<Edge>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_edges_for_view(&parent_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_edge(state: State<AppState>, edge: Edge) -> Result<(), String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.insert_edge(&edge).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_edge(state: State<AppState>, id: String) -> Result<(), String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.delete_edge(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn search_nodes(state: State<AppState>, query: String) -> Result<Vec<Node>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.search_nodes(&query).map_err(|e| e.to_string())
}

// ==================== Clustering Commands ====================

/// Status of items needing clustering
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusteringStatus {
    pub items_needing_clustering: i32,
    pub total_items: usize,
    pub ai_available: bool,
}

/// Get clustering status
#[tauri::command]
pub fn get_clustering_status(state: State<AppState>) -> Result<ClusteringStatus, String> {
    let needs_clustering = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.count_items_needing_clustering().map_err(|e| e.to_string())?;
    let all_items = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_items().map_err(|e| e.to_string())?;

    Ok(ClusteringStatus {
        items_needing_clustering: needs_clustering,
        total_items: all_items.len(),
        ai_available: ai_client::is_available(),
    })
}

/// Run clustering on items that need it
/// Uses AI when available, falls back to TF-IDF
#[tauri::command]
pub async fn run_clustering(state: State<'_, AppState>, use_ai: Option<bool>) -> Result<ClusterResult, String> {
    let use_ai = use_ai.unwrap_or(true); // Default to using AI
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clone();
    clustering::run_clustering(&db, use_ai).await
}

/// Force re-clustering of all items
#[tauri::command]
pub async fn recluster_all(state: State<'_, AppState>, use_ai: Option<bool>) -> Result<ClusterResult, String> {
    let use_ai = use_ai.unwrap_or(true);
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clone();
    clustering::recluster_all(&db, use_ai).await
}

/// Result of cluster naming operation
#[derive(Serialize)]
pub struct NamingResult {
    pub clusters_named: usize,
    pub clusters_skipped: usize,
}

/// Name clusters that have keyword-only names (runs AI naming)
#[tauri::command]
pub async fn name_clusters(state: State<'_, AppState>) -> Result<NamingResult, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clone();
    let result = clustering::name_unnamed_clusters(&db).await?;
    Ok(NamingResult {
        clusters_named: result.clusters_named,
        clusters_skipped: result.clusters_skipped,
    })
}

#[derive(Serialize)]
pub struct AiStatus {
    pub available: bool,
    pub total_nodes: usize,
    pub processed_nodes: usize,
    pub unprocessed_nodes: usize,
}

#[tauri::command]
pub fn get_ai_status(state: State<AppState>) -> Result<AiStatus, String> {
    let all_nodes = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_all_nodes(false).map_err(|e| e.to_string())?;
    let unprocessed = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_unprocessed_nodes().map_err(|e| e.to_string())?;

    Ok(AiStatus {
        available: ai_client::is_available(),
        total_nodes: all_nodes.len(),
        processed_nodes: all_nodes.len() - unprocessed.len(),
        unprocessed_nodes: unprocessed.len(),
    })
}

#[derive(Serialize)]
pub struct ProcessingResult {
    pub processed: usize,
    pub failed: usize,
    pub errors: Vec<String>,
    pub cancelled: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProgressEvent {
    pub current: usize,
    pub total: usize,
    pub node_title: String,
    pub new_title: String,
    pub content_type: Option<String>,
    pub status: String, // "processing", "success", "error", "complete"
    pub error_message: Option<String>,
    pub elapsed_secs: Option<f64>,      // Time elapsed so far
    pub estimate_secs: Option<f64>,     // Estimated total time
    pub remaining_secs: Option<f64>,    // Estimated time remaining
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HierarchyLogEvent {
    pub message: String,
    pub level: String, // "info", "debug", "warn", "error"
}

#[tauri::command]
pub async fn process_nodes(app: AppHandle, state: State<'_, AppState>) -> Result<ProcessingResult, String> {
    use std::time::Instant;

    println!("═══════════════════════════════════════════════════════════");
    println!("[AI Processing] Starting...");
    println!("═══════════════════════════════════════════════════════════");

    if !ai_client::is_available() {
        println!("[AI Processing] Error: API key not available!");
        return Err("ANTHROPIC_API_KEY not set".to_string());
    }

    // Clone the Arc to use across await points
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clone();

    // Get unprocessed nodes, excluding protected (Recent Notes) and hidden content types
    let all_unprocessed = db.get_unprocessed_nodes().map_err(|e| e.to_string())?;
    let protected_ids = db.get_protected_node_ids();

    let mut hidden_count = 0;
    let unprocessed: Vec<_> = all_unprocessed
        .into_iter()
        .filter(|node| !protected_ids.contains(&node.id))
        .filter(|node| {
            // Skip items already classified as hidden (debug, code, paste, trivial)
            if let Some(ct) = &node.content_type {
                if let Some(content_type) = crate::classification::ContentType::from_str(ct) {
                    if content_type.is_hidden() {
                        hidden_count += 1;
                        return false;
                    }
                }
            }
            true
        })
        .collect();

    if !protected_ids.is_empty() {
        println!("[AI Processing] Excluding {} protected items (Recent Notes)", protected_ids.len());
    }
    if hidden_count > 0 {
        println!("[AI Processing] Skipping {} hidden items (pre-classified)", hidden_count);
    }

    let total = unprocessed.len();
    println!("[AI Processing] Processing {} nodes", total);

    // Reset cancel flag at start
    CANCEL_PROCESSING.store(false, Ordering::SeqCst);

    let mut processed = 0;
    let mut failed = 0;
    let mut errors = Vec::new();
    let mut current = 0;

    let start_time = Instant::now();
    let mut avg_time_per_node: Option<f64> = None;

    for node in unprocessed {
        // Check for cancellation
        if CANCEL_PROCESSING.load(Ordering::SeqCst) {
            println!("[AI Processing] Cancelled by user after {} nodes", processed);
            let _ = app.emit("ai-progress", AiProgressEvent {
                current,
                total,
                node_title: "Cancelled".to_string(),
                new_title: String::new(),
                content_type: None,
                status: "cancelled".to_string(),
                error_message: Some("Cancelled by user".to_string()),
                elapsed_secs: Some(start_time.elapsed().as_secs_f64()),
                estimate_secs: None,
                remaining_secs: None,
            });
            return Ok(ProcessingResult { processed, failed, errors, cancelled: true });
        }

        current += 1;
        let content = node.content.as_deref().unwrap_or("");

        // Skip nodes with no content
        if content.is_empty() {
            continue;
        }

        // Skip papers (already have title/summary/content_type from import)
        if node.content_type.as_deref() == Some("paper") {
            continue;
        }

        // Skip bookmarks (web captures have fixed content_type, never reclassify)
        if node.content_type.as_deref() == Some("bookmark") {
            continue;
        }

        // Note: Hidden items (debug, code, paste, trivial) are filtered out before the loop

        // Calculate time estimates
        let elapsed = start_time.elapsed().as_secs_f64();
        let (estimate, remaining) = if current > 1 {
            let avg = elapsed / (current - 1) as f64;
            avg_time_per_node = Some(avg);
            let est = avg * total as f64;
            let rem = avg * (total - current + 1) as f64;
            (Some(est), Some(rem))
        } else {
            (None, None)
        };

        // Emit processing event
        let _ = app.emit("ai-progress", AiProgressEvent {
            current,
            total,
            node_title: node.title.clone(),
            new_title: String::new(),
            content_type: None,
            status: "processing".to_string(),
            error_message: None,
            elapsed_secs: Some(elapsed),
            estimate_secs: estimate,
            remaining_secs: remaining,
        });

        match ai_client::analyze_node(&node.title, content).await {
            Ok(result) => {
                // Preserve code_* content_types AND tags (from code import)
                // Code nodes have file_path metadata in tags that must not be overwritten
                let is_code_node = node.content_type
                    .as_ref()
                    .map(|ct| ct.starts_with("code_"))
                    .unwrap_or(false);

                let final_content_type = if is_code_node {
                    node.content_type.clone().unwrap_or_default()
                } else {
                    result.content_type.clone()
                };

                // For code nodes, keep original tags (file_path metadata); for others, use AI tags
                let final_tags = if is_code_node {
                    node.tags.clone().unwrap_or_default()
                } else {
                    serde_json::to_string(&result.tags).unwrap_or_default()
                };

                if let Err(e) = db.update_node_ai(
                    &node.id,
                    &result.title,
                    &result.summary,
                    &final_tags,
                    &final_content_type,
                ) {
                    let err_msg = format!("DB save failed: {}", e);
                    errors.push(format!("Failed to save node {}: {}", node.id, e));
                    failed += 1;
                    let elapsed_now = start_time.elapsed().as_secs_f64();
                    let _ = app.emit("ai-progress", AiProgressEvent {
                        current,
                        total,
                        node_title: node.title.clone(),
                        new_title: String::new(),
                        content_type: None,
                        status: "error".to_string(),
                        error_message: Some(err_msg),
                        elapsed_secs: Some(elapsed_now),
                        estimate_secs: estimate,
                        remaining_secs: remaining,
                    });
                } else {
                    processed += 1;
                    println!("Processed node: {} -> [{}] {}", node.title, result.content_type, result.title);

                    // Generate embedding from content (truncated to ~1000 bytes)
                    // Content is more semantically meaningful for clustering
                    let embed_text = safe_truncate(content, 1000);
                    match ai_client::generate_embedding(embed_text).await {
                        Ok(embedding) => {
                            if let Err(e) = db.update_node_embedding(&node.id, &embedding) {
                                eprintln!("Failed to save embedding for {}: {}", node.id, e);
                            } else {
                                println!("  + Generated embedding for node {}", node.id);
                            }
                        }
                        Err(e) => {
                            // Log but don't fail - embeddings are optional
                            eprintln!("Embedding generation skipped for {}: {}", node.id, e);
                        }
                    }

                    let elapsed_now = start_time.elapsed().as_secs_f64();
                    let remaining_now = avg_time_per_node.map(|avg| avg * (total - current) as f64);
                    let _ = app.emit("ai-progress", AiProgressEvent {
                        current,
                        total,
                        node_title: node.title.clone(),
                        new_title: result.title,
                        content_type: Some(result.content_type.clone()),
                        status: "success".to_string(),
                        error_message: None,
                        elapsed_secs: Some(elapsed_now),
                        estimate_secs: estimate,
                        remaining_secs: remaining_now,
                    });
                }
            }
            Err(e) => {
                let err_msg = e.clone();
                errors.push(format!("AI error for node {}: {}", node.id, e));
                failed += 1;
                let elapsed_now = start_time.elapsed().as_secs_f64();
                let _ = app.emit("ai-progress", AiProgressEvent {
                    current,
                    total,
                    node_title: node.title.clone(),
                    new_title: String::new(),
                    content_type: None,
                    status: "error".to_string(),
                    error_message: Some(err_msg),
                    elapsed_secs: Some(elapsed_now),
                    estimate_secs: estimate,
                    remaining_secs: remaining,
                });
            }
        }
    }

    // Emit completion event
    let final_elapsed = start_time.elapsed().as_secs_f64();
    println!("[AI Processing] Complete: {} processed, {} failed ({:.1}s)", processed, failed, final_elapsed);
    let _ = app.emit("ai-progress", AiProgressEvent {
        current: total,
        total,
        node_title: String::new(),
        new_title: String::new(),
        content_type: None,
        status: "complete".to_string(),
        error_message: None,
        elapsed_secs: Some(final_elapsed),
        estimate_secs: Some(final_elapsed),
        remaining_secs: Some(0.0),
    });

    Ok(ProcessingResult {
        processed,
        failed,
        errors,
        cancelled: false,
    })
}

#[tauri::command]
pub fn get_learned_emojis(state: State<AppState>) -> Result<std::collections::HashMap<String, String>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_learned_emojis().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_learned_emoji(state: State<AppState>, keyword: String, emoji: String) -> Result<(), String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.save_learned_emoji(&keyword, &emoji).map_err(|e| e.to_string())
}

// ==================== Hierarchy Navigation Commands ====================

/// Get nodes at a specific depth (0=Universe, increases toward items)
#[tauri::command]
pub fn get_nodes_at_depth(state: State<AppState>, depth: i32) -> Result<Vec<Node>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_nodes_at_depth(depth).map_err(|e| e.to_string())
}

/// Get children of a specific parent node
#[tauri::command]
pub fn get_children(state: State<AppState>, parent_id: String) -> Result<Vec<Node>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_children(&parent_id).map_err(|e| e.to_string())
}

/// Get the Universe node (single root, is_universe = true)
#[tauri::command]
pub fn get_universe(state: State<AppState>) -> Result<Option<Node>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_universe().map_err(|e| e.to_string())
}

/// Get all items (is_item = true) - openable content
#[tauri::command]
pub fn get_items(state: State<AppState>) -> Result<Vec<Node>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_items().map_err(|e| e.to_string())
}

/// Get the maximum depth in the hierarchy
#[tauri::command]
pub fn get_max_depth(state: State<AppState>) -> Result<i32, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_max_depth().map_err(|e| e.to_string())
}

// ==================== Hierarchy Generation Commands ====================

/// Build the full hierarchy from items
/// Creates intermediate grouping nodes based on collection size
#[tauri::command]
pub fn build_hierarchy(state: State<'_, AppState>) -> Result<hierarchy::HierarchyResult, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
    hierarchy::build_hierarchy(&db)
}

/// Build full navigable hierarchy with recursive AI grouping
///
/// Flow:
/// 1. Optionally run clustering to assign items to fine-grained topics
/// 2. Build initial hierarchy (flat topics under Universe)
/// 3. Recursively group any level with >15 children until navigable (8-15 children per level)
#[tauri::command]
pub async fn build_full_hierarchy(
    app: AppHandle,
    state: State<'_, AppState>,
    run_clustering: Option<bool>,
) -> Result<hierarchy::FullHierarchyResult, String> {
    let should_cluster = run_clustering.unwrap_or(false);
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clone();
    hierarchy::build_full_hierarchy(&db, should_cluster, Some(&app)).await
}

/// Cluster children of a specific parent node into groups using AI
///
/// Returns true if grouping was performed, false if node has <2 children.
/// Use this for manual/targeted hierarchy adjustment.
/// max_groups: optional maximum number of groups to create (default 5)
/// This is a manual split, so force=true bypasses automatic threshold checks.
#[tauri::command]
pub async fn cluster_hierarchy_level(
    app: AppHandle,
    state: State<'_, AppState>,
    parent_id: String,
    max_groups: Option<usize>,
) -> Result<bool, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clone();
    // force=true: manual split from UI bypasses automatic thresholds
    hierarchy::cluster_hierarchy_level(&db, &parent_id, Some(&app), max_groups, true).await
}

/// Split a node - delete it and move its children up to its parent
///
/// The selected node is deleted, and all its children become children of the node's parent.
/// Returns the number of children that were moved up.
#[tauri::command]
pub fn unsplit_node(state: State<AppState>, parent_id: String) -> Result<usize, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;

    // Get the node to be split (deleted)
    let node = db.get_node(&parent_id).map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Node {} not found", parent_id))?;

    // Can't split the Universe
    if node.is_universe {
        return Err("Cannot split the Universe node".to_string());
    }

    // Can't split items
    if node.is_item {
        return Err("Cannot split an item node".to_string());
    }

    // Get the node's parent (where children will go)
    let grandparent_id = node.parent_id.clone()
        .ok_or_else(|| "Cannot split a node without a parent".to_string())?;

    // Get direct children of this node
    let children = db.get_children(&parent_id).map_err(|e| e.to_string())?;
    let moved_count = children.len();

    // Reparent all children to the grandparent
    for child in &children {
        db.update_node_parent(&child.id, &grandparent_id)
            .map_err(|e| e.to_string())?;
        // Decrement depth of child and all its descendants
        db.decrement_subtree_depth(&child.id).map_err(|e| e.to_string())?;
    }

    // Delete the node itself
    db.delete_node(&parent_id).map_err(|e| e.to_string())?;

    // Update child count of grandparent
    let new_child_count = db.get_children(&grandparent_id)
        .map_err(|e| e.to_string())?
        .len() as i32;
    db.update_child_count(&grandparent_id, new_child_count)
        .map_err(|e| e.to_string())?;

    println!("[Split] Deleted '{}', moved {} children up to parent", node.title, moved_count);

    Ok(moved_count)
}

/// Get children of a node, automatically skipping single-child chains
///
/// Useful for navigation - if a level has exactly 1 child, skip to that child's children.
#[tauri::command]
pub fn get_children_flat(state: State<AppState>, parent_id: String) -> Result<Vec<Node>, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
    hierarchy::get_children_skip_single_chain(&db, &parent_id)
}

// ==================== Mini-Clustering Commands ====================

/// Get only idea nodes for graph rendering (filters out code/debug/paste)
/// If include_hidden is true, also includes HIDDEN tier items
#[tauri::command]
pub fn get_graph_children(state: State<AppState>, parent_id: String, include_hidden: Option<bool>) -> Result<Vec<Node>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_graph_children(&parent_id, include_hidden.unwrap_or(false)).map_err(|e| e.to_string())
}

/// Get supporting items (code/debug/paste) under a parent
#[tauri::command]
pub fn get_supporting_items(state: State<AppState>, parent_id: String) -> Result<Vec<Node>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_supporting_items(&parent_id).map_err(|e| e.to_string())
}

/// Get items associated with a specific idea
#[tauri::command]
pub fn get_associated_items(state: State<AppState>, idea_id: String) -> Result<Vec<Node>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_associated_items(&idea_id).map_err(|e| e.to_string())
}

/// Supporting item counts for badge display
#[derive(serde::Serialize)]
pub struct SupportingCounts {
    pub code: i32,
    pub debug: i32,
    pub paste: i32,
}

/// Get counts of supporting items for badge display
#[tauri::command]
pub fn get_supporting_counts(state: State<AppState>, parent_id: String) -> Result<SupportingCounts, String> {
    let (code, debug, paste) = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_supporting_counts(&parent_id).map_err(|e| e.to_string())?;
    Ok(SupportingCounts { code, debug, paste })
}

/// Run classification and association on all items
#[tauri::command]
pub fn classify_and_associate(state: State<AppState>) -> Result<(usize, usize), String> {
    use crate::classification;

    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;

    // Step 1: Classify all unclassified items
    let classified = classification::classify_all_items(&db)?;

    // Step 2: Compute associations for all topics
    let associated = classification::compute_all_associations(&db)?;

    Ok((classified, associated))
}

/// Run classification and association on items under a specific parent
#[tauri::command]
pub fn classify_and_associate_children(state: State<AppState>, parent_id: String) -> Result<(usize, usize), String> {
    use crate::classification;

    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;

    // Step 1: Classify children
    let classified = classification::classify_children(&db, &parent_id)?;

    // Step 2: Compute associations for this parent
    let associated = classification::compute_associations(&db, &parent_id)?;

    Ok((classified, associated))
}

/// Propagate latest_child_date from leaves up through the hierarchy
///
/// Fast operation (~seconds) - no AI or embeddings involved.
/// Leaves get their created_at, groups get MAX of their children's latest_child_date.
#[tauri::command]
pub fn propagate_latest_dates(state: State<AppState>) -> Result<(), String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.propagate_latest_dates().map_err(|e| e.to_string())
}

// ==================== Multi-Path Association Commands ====================

/// Get all category associations for an item (via BelongsTo edges)
/// Returns edges sorted by weight (highest first)
#[tauri::command]
pub fn get_item_associations(state: State<AppState>, item_id: String) -> Result<Vec<Edge>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_belongs_to_edges(&item_id).map_err(|e| e.to_string())
}

/// Get items that share categories with this item
/// Returns items connected via BelongsTo edges to any of the same targets
#[tauri::command]
pub fn get_related_items(state: State<AppState>, item_id: String, min_weight: Option<f64>) -> Result<Vec<Node>, String> {
    let min_w = min_weight.unwrap_or(0.3);

    // Get this item's category associations
    let associations = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_belongs_to_edges(&item_id).map_err(|e| e.to_string())?;

    if associations.is_empty() {
        return Ok(vec![]);
    }

    // Collect all related items from shared categories
    let mut related_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for edge in &associations {
        // Skip weak associations
        if let Some(weight) = edge.weight {
            if weight < min_w {
                continue;
            }
        }

        // Find the cluster_id from the edge target (format: "topic-{id}" or "cluster-{id}")
        let target = &edge.target;
        let cluster_id: Option<i32> = if target.starts_with("topic-") {
            target.strip_prefix("topic-").and_then(|s| s.parse().ok())
        } else if target.starts_with("cluster-") {
            target.strip_prefix("cluster-").and_then(|s| s.parse().ok())
        } else {
            None
        };

        if let Some(cid) = cluster_id {
            // Get all items in this cluster via edges
            if let Ok(items) = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_items_in_cluster_via_edges(cid, Some(min_w)) {
                for item in items {
                    if item.id != item_id { // Don't include self
                        related_ids.insert(item.id);
                    }
                }
            }
        }
    }

    // Fetch full node data for related items
    let mut related_nodes: Vec<Node> = Vec::new();
    for id in related_ids {
        if let Ok(Some(node)) = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_node(&id) {
            related_nodes.push(node);
        }
    }

    // Sort by title for consistent ordering
    related_nodes.sort_by(|a, b| a.title.cmp(&b.title));

    Ok(related_nodes)
}

/// Get all items in a category (via BelongsTo edges, not just cluster_id)
/// More comprehensive than hierarchy navigation - includes secondary associations
#[tauri::command]
pub fn get_category_items(state: State<AppState>, cluster_id: i32, min_weight: Option<f64>) -> Result<Vec<Node>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_items_in_cluster_via_edges(cluster_id, min_weight).map_err(|e| e.to_string())
}

// ==================== Conversation Context Commands ====================

/// Get all messages belonging to a conversation, ordered by sequence_index
/// Traces message Leafs back to their parent conversation
#[tauri::command]
pub fn get_conversation_context(state: State<AppState>, conversation_id: String) -> Result<Vec<Node>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_conversation_messages(&conversation_id).map_err(|e| e.to_string())
}

// ==================== Import Commands ====================

/// Import Claude conversations from JSON content.
///
/// Creates:
/// 1. Container nodes for each conversation (is_item = false, not clustered)
/// 2. Individual message nodes (is_item = true, will be clustered)
///
/// Each message gets conversation_id and sequence_index for context reconstruction.
#[tauri::command]
pub fn import_claude_conversations(state: State<AppState>, json_content: String) -> Result<import::ImportResult, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
    import::import_claude_conversations(&db, &json_content)
}

/// Import ChatGPT conversations from JSON export
///
/// Handles tree-structured conversations from ChatGPT data export.
/// Creates exchange nodes (user + assistant pairs).
#[tauri::command]
pub fn import_chatgpt_conversations(state: State<AppState>, json_content: String) -> Result<import::ImportResult, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
    import::import_chatgpt_conversations(&db, &json_content)
}

/// Import markdown files as notes
///
/// Each .md file becomes a note under "Recent Notes" container.
/// Title is extracted from first # heading or filename.
#[tauri::command]
pub fn import_markdown_files(state: State<AppState>, file_paths: Vec<String>) -> Result<import::ImportResult, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
    import::import_markdown_files(&db, &file_paths)
}

/// Import Google Keep notes from a Google Takeout zip file.
///
/// Extracts JSON files from Takeout/Keep/ in the zip, parses notes,
/// and creates thought nodes with is_item=true and source="googlekeep".
#[tauri::command]
pub fn import_google_keep(state: State<AppState>, zip_path: String) -> Result<import::GoogleKeepImportResult, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
    import::import_google_keep(&db, &zip_path)
}

/// Import scientific papers from OpenAIRE (EU open science graph).
///
/// Fetches papers matching the search query, creates paper nodes,
/// and optionally downloads PDFs.
#[tauri::command]
pub async fn import_openaire(
    app: AppHandle,
    state: State<'_, AppState>,
    query: String,
    country: Option<String>,
    fos: Option<String>,
    from_year: Option<String>,
    to_year: Option<String>,
    max_papers: u32,
    download_pdfs: bool,
    max_pdf_size_mb: Option<u32>,
) -> Result<import::OpenAireImportResult, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clone();
    let max_size = max_pdf_size_mb.unwrap_or(20);

    // Get OpenAIRE API key from settings (optional - public API also works)
    let api_key = settings::get_openaire_api_key();

    // Progress callback that emits events to frontend
    let app_handle = app.clone();
    let on_progress = move |current: usize, total: usize| {
        let _ = app_handle.emit("openaire-progress", serde_json::json!({
            "current": current,
            "total": total,
        }));
    };

    import::import_openaire_papers(
        &db,
        query,
        country,
        fos,
        from_year,
        to_year,
        max_papers,
        download_pdfs,
        max_size,
        api_key,
        on_progress,
    ).await
}

/// Count papers matching OpenAIRE query without importing
/// Returns (total_count, already_imported_count)
/// Just gets the count from the API header, doesn't paginate
#[tauri::command]
pub async fn count_openaire_papers(
    state: State<'_, AppState>,
    query: String,
    country: Option<String>,
    fos: Option<String>,
    from_year: Option<String>,
    to_year: Option<String>,
) -> Result<(u32, u32), String> {
    use crate::openaire::{OpenAireClient, OpenAireQuery};
    use std::sync::atomic::Ordering;

    // Reset cancel flag at start
    state.openaire_cancel.store(false, Ordering::SeqCst);

    // Get OpenAIRE API key from settings (optional)
    let api_key = settings::get_openaire_api_key();
    let client = OpenAireClient::new_with_key(api_key);

    let query_obj = OpenAireQuery {
        search: query,
        country,
        fos,
        from_year,
        to_year,
        access_right: Some("OPEN".to_string()),
        page_size: 1,  // Just need the count, not the papers
        page: 1,
        sort_by: None,
    };

    // Check cancel before API call
    if state.openaire_cancel.load(Ordering::SeqCst) {
        return Err("Cancelled".to_string());
    }

    let total_count = client.count_papers(&query_obj).await?;

    // We don't count already-imported anymore (too expensive)
    Ok((total_count, 0))
}

/// Cancel any ongoing OpenAIRE operations
#[tauri::command]
pub fn cancel_openaire(state: State<'_, AppState>) {
    use std::sync::atomic::Ordering;
    state.openaire_cancel.store(true, Ordering::SeqCst);
    println!("[OpenAIRE] Cancel requested");
}

/// Get count of already-imported papers from local database
#[tauri::command]
pub fn get_imported_paper_count(state: State<AppState>) -> Result<i32, String> {
    state.db.read()
        .map_err(|e| format!("DB lock error: {}", e))?
        .get_paper_count()
        .map_err(|e| e.to_string())
}

// ==================== Paper Retrieval Commands ====================

/// Get paper metadata by node ID
#[tauri::command]
pub fn get_paper_metadata(state: State<AppState>, node_id: String) -> Result<Option<crate::db::Paper>, String> {
    let result = state.db.read()
        .map_err(|e| format!("DB lock error: {}", e))?
        .get_paper_by_node_id(&node_id)
        .map_err(|e| e.to_string())?;
    if let Some(ref paper) = result {
        eprintln!("[PDF] get_paper_metadata: node_id={}, pdfAvailable={}, pdfUrl={:?}",
            node_id, paper.pdf_available, paper.pdf_url);
    }
    Ok(result)
}

/// Get PDF binary for a paper
#[tauri::command]
pub fn get_paper_pdf(state: State<AppState>, node_id: String) -> Result<Option<Vec<u8>>, String> {
    eprintln!("[PDF] get_paper_pdf called for node_id: {}", node_id);
    let result = state.db.read()
        .map_err(|e| format!("DB lock error: {}", e))?
        .get_paper_pdf(&node_id)
        .map_err(|e| e.to_string())?;
    eprintln!("[PDF] get_paper_pdf result: {} bytes", result.as_ref().map(|v| v.len()).unwrap_or(0));
    Ok(result)
}

/// Check if a paper has a PDF available
#[tauri::command]
pub fn has_paper_pdf(state: State<AppState>, node_id: String) -> Result<bool, String> {
    state.db.read()
        .map_err(|e| format!("DB lock error: {}", e))?
        .has_paper_pdf(&node_id)
        .map_err(|e| e.to_string())
}

/// Get document binary and format for a paper (supports PDF, DOCX, DOC)
#[tauri::command]
pub fn get_paper_document(state: State<AppState>, node_id: String) -> Result<Option<(Vec<u8>, String)>, String> {
    eprintln!("[Document] get_paper_document called for node_id: {}", node_id);
    let result = state.db.read()
        .map_err(|e| format!("DB lock error: {}", e))?
        .get_paper_document(&node_id)
        .map_err(|e| e.to_string())?;
    if let Some((ref bytes, ref format)) = result {
        eprintln!("[Document] get_paper_document result: {} bytes, format: {}", bytes.len(), format);
    }
    Ok(result)
}

/// Download paper document on demand from source URL
/// Falls back to cached version if available, otherwise downloads from pdf_url
#[tauri::command]
pub async fn download_paper_on_demand(
    state: State<'_, AppState>,
    node_id: String,
    cache_locally: bool,
) -> Result<Option<(Vec<u8>, String)>, String> {
    use crate::openaire::OpenAireClient;

    // First check if we have it cached locally
    if let Ok(Some(doc)) = state.db.read()
        .map_err(|e| format!("DB lock: {}", e))?
        .get_paper_document(&node_id)
    {
        eprintln!("[OnDemand] Found cached document for {}", node_id);
        return Ok(Some(doc));
    }

    // Get the PDF URL from paper metadata
    let pdf_url = state.db.read()
        .map_err(|e| format!("DB lock: {}", e))?
        .get_paper_by_node_id(&node_id)
        .map_err(|e| e.to_string())?
        .and_then(|p| p.pdf_url);

    let url = match pdf_url {
        Some(u) => u,
        None => return Err("No PDF URL available for this paper".to_string()),
    };

    eprintln!("[OnDemand] Downloading from: {}", url);

    // Download on demand
    let client = OpenAireClient::new();
    let result = client.download_document(&url, 20).await?;

    if let Some((ref bytes, ref format)) = result {
        eprintln!("[OnDemand] Downloaded {} KB, format: {}", bytes.len() / 1024, format);

        // Optionally cache to DB for faster access next time
        if cache_locally {
            if let Ok(db) = state.db.read() {
                if let Err(e) = db.update_paper_document(&node_id, bytes, format) {
                    eprintln!("[OnDemand] Failed to cache: {}", e);
                } else {
                    eprintln!("[OnDemand] Cached to local DB");
                }
            }
        }
    }

    Ok(result)
}

/// Open a paper's document in the system's default viewer
#[tauri::command]
pub fn open_paper_external(state: State<AppState>, node_id: String, title: String) -> Result<(), String> {
    use std::io::Write;

    let (doc_data, format) = state.db.read()
        .map_err(|e| format!("DB lock error: {}", e))?
        .get_paper_document(&node_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Document not available".to_string())?;

    // Create safe filename
    let safe_name: String = title.chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
        .take(50)
        .collect();
    let safe_name = safe_name.trim().replace(' ', "_");

    // Write to temp file with correct extension
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join(format!("{}.{}", safe_name, format));

    let mut file = std::fs::File::create(&file_path)
        .map_err(|e| format!("Failed to create temp file: {}", e))?;
    file.write_all(&doc_data)
        .map_err(|e| format!("Failed to write document: {}", e))?;

    eprintln!("[Document] Opening external: {:?}", file_path);

    // Open with system default viewer
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&file_path)
            .spawn()
            .map_err(|e| format!("Failed to open document: {}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&file_path)
            .spawn()
            .map_err(|e| format!("Failed to open document: {}", e))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &file_path.to_string_lossy()])
            .spawn()
            .map_err(|e| format!("Failed to open document: {}", e))?;
    }

    Ok(())
}

/// Reformat all paper abstracts (for papers imported before the formatter)
#[tauri::command]
pub fn reformat_paper_abstracts(state: State<AppState>) -> Result<usize, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
    let count = db.reformat_all_paper_abstracts().map_err(|e| e.to_string())?;
    eprintln!("[Papers] Reformatted {} abstracts with detected structure", count);
    Ok(count)
}

/// Sync pdf_available from papers table to nodes table
#[tauri::command]
pub fn sync_paper_pdf_status(state: State<AppState>) -> Result<usize, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
    let count = db.sync_paper_pdf_status().map_err(|e| e.to_string())?;
    eprintln!("[Papers] Synced pdf_available for {} paper nodes", count);
    Ok(count)
}

/// Sync paper dates from papers.publication_date to nodes.created_at
/// Re-parses publication_date strings and fixes nodes with wrong dates
/// Papers with missing/unparseable dates get 0 (unknown)
#[tauri::command]
pub fn sync_paper_dates(state: State<AppState>) -> Result<(usize, usize), String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
    let (updated, unknown) = db.sync_paper_dates().map_err(|e| e.to_string())?;
    eprintln!("[Papers] Synced dates: {} updated, {} set to unknown", updated, unknown);
    Ok((updated, unknown))
}

// ==================== Quick Access Commands (Sidebar) ====================

/// Pin or unpin a node for quick access
#[tauri::command]
pub fn set_node_pinned(state: State<AppState>, node_id: String, pinned: bool) -> Result<(), String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.set_node_pinned(&node_id, pinned).map_err(|e| e.to_string())
}

/// Update last accessed timestamp for a node (call when opening in Leaf)
#[tauri::command]
pub fn touch_node(state: State<AppState>, node_id: String) -> Result<(), String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.touch_node(&node_id).map_err(|e| e.to_string())
}

/// Get all pinned nodes for Sidebar Pinned tab
#[tauri::command]
pub fn get_pinned_nodes(state: State<AppState>) -> Result<Vec<Node>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_pinned_nodes().map_err(|e| e.to_string())
}

/// Get recently accessed nodes for Sidebar Recent tab
#[tauri::command]
pub fn get_recent_nodes(state: State<AppState>, limit: Option<i32>) -> Result<Vec<Node>, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_recent_nodes(limit.unwrap_or(15)).map_err(|e| e.to_string())
}

/// Remove a node from recents (clear last_accessed_at)
#[tauri::command]
pub fn clear_recent(state: State<AppState>, node_id: String) -> Result<(), String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clear_recent(&node_id).map_err(|e| e.to_string())
}

// ==================== Semantic Similarity Commands ====================

/// Similar node result with similarity score or edge relationship
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarNode {
    pub id: String,
    pub title: String,
    pub emoji: Option<String>,
    pub summary: Option<String>,
    pub similarity: f32,
    /// Edge type if this is an edge-based relationship (e.g., "calls", "called_by", "documents")
    /// None for embedding-based similarity
    pub edge_type: Option<String>,
}

/// Find nodes semantically similar to a given node
/// Uses embedding cosine similarity with caching for performance
#[tauri::command]
pub fn get_similar_nodes(
    state: State<AppState>,
    node_id: String,
    top_n: Option<usize>,
    min_similarity: Option<f32>,
) -> Result<Vec<SimilarNode>, String> {
    let top_n = top_n.unwrap_or(10);
    let min_similarity = min_similarity.unwrap_or(0.0);

    // Check cache first
    let cached = state.similarity_cache.read().map_err(|e| format!("Cache lock error: {}", e))?.get(&node_id);

    let similar = if let Some(cached_results) = cached {
        // Use cached results, but filter and limit
        cached_results.into_iter()
            .filter(|(_, score)| *score >= min_similarity)
            .take(top_n)
            .collect::<Vec<_>>()
    } else {
        // Get the target node's embedding - return empty if none (e.g., category nodes)
        let target_embedding = match state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_node_embedding(&node_id) {
            Ok(Some(emb)) => emb,
            _ => return Ok(vec![]), // No embedding = no similar nodes, but not an error
        };

        // Get all embeddings
        let all_embeddings = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_nodes_with_embeddings()
            .map_err(|e| e.to_string())?;

        if all_embeddings.is_empty() {
            return Ok(vec![]);
        }

        // Find similar nodes - get more than requested for caching
        let max_cache_results = 50;
        let similar = similarity::find_similar(&target_embedding, &all_embeddings, &node_id, max_cache_results, 0.0);

        // Cache the full results
        state.similarity_cache.write().map_err(|e| format!("Cache lock error: {}", e))?.insert(node_id.clone(), similar.clone());

        // Filter and limit for this request
        similar.into_iter()
            .filter(|(_, score)| *score >= min_similarity)
            .take(top_n)
            .collect::<Vec<_>>()
    };

    // Fetch full node data for results (only items, not categories/clusters)
    let mut results: Vec<SimilarNode> = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    // First, add edge-based relationships (calls, documents) - these come first
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
    if let Ok(edges) = db.get_edges_for_node(&node_id) {
        for edge in edges {
            let (related_id, edge_label) = if edge.source == node_id {
                // Outgoing edge: this node -> target
                let label = match edge.edge_type.as_str() {
                    "calls" => "calls",
                    "documents" => "documents",
                    _ => continue, // Skip other edge types for now
                };
                (edge.target, label)
            } else {
                // Incoming edge: source -> this node
                let label = match edge.edge_type.as_str() {
                    "calls" => "called by",
                    "documents" => "documented by",
                    _ => continue,
                };
                (edge.source, label)
            };

            if seen_ids.contains(&related_id) {
                continue;
            }

            if let Ok(Some(node)) = db.get_node(&related_id) {
                if !node.is_item {
                    continue;
                }
                seen_ids.insert(related_id.clone());
                results.push(SimilarNode {
                    id: node.id,
                    title: node.ai_title.unwrap_or(node.title),
                    emoji: node.emoji,
                    summary: node.summary,
                    similarity: 1.0, // Edge relationships are "100%" related
                    edge_type: Some(edge_label.to_string()),
                });
            }
        }
    }
    drop(db);

    // Then add embedding-based similar nodes
    for (id, sim_score) in similar {
        if seen_ids.contains(&id) {
            continue; // Skip if already added via edge
        }
        if let Ok(Some(node)) = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_node(&id) {
            // Skip non-item nodes (categories, clusters) - they shouldn't open in leaf view
            if !node.is_item {
                continue;
            }
            results.push(SimilarNode {
                id: node.id,
                title: node.ai_title.unwrap_or(node.title),
                emoji: node.emoji,
                summary: node.summary,
                similarity: sim_score,
                edge_type: None,
            });
        }
    }

    Ok(results)
}

/// Get embedding status for nodes
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingStatus {
    pub nodes_with_embeddings: i32,
    pub total_items: usize,
    pub openai_available: bool,
}

#[tauri::command]
pub fn get_embedding_status(state: State<AppState>) -> Result<EmbeddingStatus, String> {
    let nodes_with_embeddings = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.count_nodes_with_embeddings()
        .map_err(|e| e.to_string())?;
    let all_items = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_items().map_err(|e| e.to_string())?;

    Ok(EmbeddingStatus {
        nodes_with_embeddings,
        total_items: all_items.len(),
        openai_available: settings::has_openai_api_key(),
    })
}

// ==================== OpenAI API Key Commands ====================

#[tauri::command]
pub fn get_openai_api_key_status() -> Result<Option<String>, String> {
    // Return masked key for display, or None if not set
    Ok(settings::get_masked_openai_api_key())
}

#[tauri::command]
pub fn save_openai_api_key(key: String) -> Result<(), String> {
    settings::set_openai_api_key(key)
}

#[tauri::command]
pub fn clear_openai_api_key() -> Result<(), String> {
    settings::set_openai_api_key(String::new())
}

// ==================== OpenAIRE API Key Commands ====================

#[tauri::command]
pub fn get_openaire_api_key_status() -> Result<Option<String>, String> {
    // Return masked key for display, or None if not set
    Ok(settings::get_masked_openaire_api_key())
}

#[tauri::command]
pub fn save_openaire_api_key(key: String) -> Result<(), String> {
    settings::set_openaire_api_key(key)
}

#[tauri::command]
pub fn clear_openaire_api_key() -> Result<(), String> {
    settings::set_openaire_api_key(String::new())
}

// ==================== Leaf View Commands ====================

/// Get the full content of a node for Leaf view rendering
#[tauri::command]
pub fn get_leaf_content(state: State<AppState>, node_id: String) -> Result<String, String> {
    let node = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_node(&node_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Node {} not found", node_id))?;

    // Return content, falling back to empty string
    Ok(node.content.unwrap_or_default())
}

// ==================== Settings Panel Commands ====================

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResult {
    pub nodes_deleted: usize,
    pub edges_deleted: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbStats {
    pub total_nodes: usize,
    pub total_items: usize,
    pub processed_items: usize,
    pub items_with_embeddings: usize,
    // For API cost estimation
    pub unprocessed_items: usize,      // Items needing AI processing
    pub unclustered_items: usize,      // Items without cluster_id
    pub orphan_items: usize,           // Items with cluster_id but no parent_id
    pub topics_count: usize,           // Number of topic nodes (for hierarchy grouping)
}

/// Delete all data (nodes + edges)
#[tauri::command]
pub fn delete_all_data(state: State<AppState>) -> Result<DeleteResult, String> {
    let edges_deleted = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.delete_all_edges().map_err(|e| e.to_string())?;
    let nodes_deleted = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.delete_all_nodes().map_err(|e| e.to_string())?;
    Ok(DeleteResult { nodes_deleted, edges_deleted })
}

/// Reset AI processing flag on all items
#[tauri::command]
pub fn reset_ai_processing(state: State<AppState>) -> Result<usize, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.reset_ai_processing().map_err(|e| e.to_string())
}

/// Reset clustering flag on all items
#[tauri::command]
pub fn reset_clustering(state: State<AppState>) -> Result<usize, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.mark_all_items_need_clustering().map_err(|e| e.to_string())
}

/// Clear all embeddings
#[tauri::command]
pub fn clear_embeddings(state: State<AppState>) -> Result<usize, String> {
    // Also delete semantic edges since they depend on embeddings
    let _ = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.delete_semantic_edges();
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clear_all_embeddings().map_err(|e| e.to_string())
}

/// Clear hierarchy (delete intermediate nodes, keep items)
#[tauri::command]
pub fn clear_hierarchy(state: State<AppState>) -> Result<usize, String> {
    // Also clear parent_id on items
    let _ = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clear_item_parents();
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.delete_hierarchy_nodes().map_err(|e| e.to_string())
}

/// Clear all tags and item_tags (for tag regeneration)
#[tauri::command]
pub fn clear_tags(state: State<AppState>) -> Result<usize, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.delete_all_tags().map_err(|e| e.to_string())
}

/// Delete empty nodes (items with no content/raw data)
#[tauri::command]
pub fn delete_empty_nodes(state: State<AppState>) -> Result<usize, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.delete_empty_items().map_err(|e| e.to_string())
}

/// Flatten empty passthrough levels in hierarchy
/// Removes single-child chains and "Uncategorized" passthrough nodes
#[tauri::command]
pub fn flatten_hierarchy(state: State<AppState>) -> Result<usize, String> {
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.flatten_empty_levels().map_err(|e| e.to_string())
}

/// Consolidate Universe's direct children into 4-8 uber-categories
/// Creates new depth-1 nodes with single-word ALL-CAPS names (TECH, LIFE, MIND, etc.)
#[tauri::command]
pub async fn consolidate_root(state: State<'_, AppState>) -> Result<ConsolidateResult, String> {
    use crate::ai_client::{self, TopicInfo};
    use crate::db::{Node, NodeType, Position};

    // Get Universe
    let universe = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_universe()
        .map_err(|e| e.to_string())?
        .ok_or("No Universe node found")?;

    // Get Universe's direct children (excluding protected)
    let all_children = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_children(&universe.id).map_err(|e| e.to_string())?;
    let protected_ids = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_protected_node_ids();
    let children: Vec<Node> = all_children
        .into_iter()
        .filter(|child| !protected_ids.contains(&child.id))
        .collect();

    if !protected_ids.is_empty() {
        println!("[Consolidate] Excluding {} protected nodes (Recent Notes)", protected_ids.len());
    }

    if children.is_empty() {
        return Err("Universe has no children to consolidate".to_string());
    }

    if children.len() <= 8 {
        return Err(format!("Universe only has {} children - already consolidated enough", children.len()));
    }

    println!("[Consolidate] Grouping {} categories into uber-categories", children.len());

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

    // Pre-fetch embeddings for similarity-sorted batching (before async call to avoid lock issues)
    let embeddings_map: std::collections::HashMap<String, Vec<f32>> = {
        let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
        categories
            .iter()
            .filter_map(|c| {
                db.get_node_embedding(&c.id)
                    .ok()
                    .flatten()
                    .map(|emb| (c.id.clone(), emb))
            })
            .collect()
    };
    println!("[Consolidate] Fetched {}/{} topic embeddings for similarity sorting", embeddings_map.len(), categories.len());

    // Call AI to group into uber-categories (similarity-sorted batching for coherent groups)
    let groupings = ai_client::group_into_uber_categories(&categories, &embeddings_map, None).await?;

    if groupings.is_empty() {
        return Err("AI returned no uber-categories".to_string());
    }

    println!("[Consolidate] AI created {} uber-categories", groupings.len());

    // Create map from label -> child nodes
    let mut label_to_children: std::collections::HashMap<String, Vec<&Node>> = std::collections::HashMap::new();
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
        .unwrap()
        .as_millis();
    let now = timestamp as i64;

    let mut uber_categories_created = 0;
    let mut children_reparented = 0;
    let mut all_children_to_update: Vec<String> = Vec::new();

    for (idx, grouping) in groupings.iter().enumerate() {
        // Find matching children
        let matching_children: Vec<&Node> = grouping.children
            .iter()
            .flat_map(|label| label_to_children.get(label).cloned().unwrap_or_default())
            .collect();

        if matching_children.is_empty() {
            println!("[Consolidate] Warning: '{}' has no matching children", grouping.name);
            continue;
        }

        if matching_children.len() < 2 {
            // Single-child groups don't need an uber-category wrapper - child stays at top level
            let child_name = matching_children.first().map(|c| c.cluster_label.as_deref().or(c.ai_title.as_deref()).unwrap_or(&c.title)).unwrap_or("unknown");
            println!("[Consolidate] '{}' stays at top level (no wrapper needed for single child)", child_name);
            continue;
        }

        // Create uber-category node
        let uber_id = format!("uber-{}-{}", timestamp, idx);

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
        };

        state.db.read().map_err(|e| format!("DB lock error: {}", e))?.insert_node(&uber_node).map_err(|e| e.to_string())?;
        uber_categories_created += 1;

        // Reparent children to this uber-category
        for child in &matching_children {
            state.db.read().map_err(|e| format!("DB lock error: {}", e))?.update_parent(&child.id, &uber_id).map_err(|e| e.to_string())?;
            all_children_to_update.push(child.id.clone());
            children_reparented += 1;
        }

        println!("[Consolidate] Created '{}' with {} children", grouping.name, matching_children.len());
    }

    // Batch update depths
    if !all_children_to_update.is_empty() {
        println!("[Consolidate] Updating depths for {} reparented nodes...", all_children_to_update.len());
        state.db.read().map_err(|e| format!("DB lock error: {}", e))?.increment_multiple_subtrees_depth(&all_children_to_update).map_err(|e| e.to_string())?;
    }

    // Update Universe's child count (new uber-categories + children that stayed at top level)
    let children_stayed_at_top = children.len().saturating_sub(children_reparented);
    let new_universe_child_count = uber_categories_created + children_stayed_at_top;
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.update_child_count(&universe.id, new_universe_child_count as i32)
        .map_err(|e| e.to_string())?;

    println!("[Consolidate] Complete: {} uber-categories, {} children reparented, {} stayed at top level",
             uber_categories_created, children_reparented, children_stayed_at_top);

    Ok(ConsolidateResult {
        uber_categories_created: uber_categories_created as i32,
        children_reparented: children_reparented as i32,
    })
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsolidateResult {
    pub uber_categories_created: i32,
    pub children_reparented: i32,
}

// ==================== Tidy Database Command ====================

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TidyReport {
    pub same_name_merged: usize,
    pub chains_flattened: usize,
    pub empties_removed: usize,
    pub empty_items_removed: usize,
    pub child_counts_fixed: usize,
    pub depths_fixed: usize,
    pub orphans_reparented: usize,
    pub dead_edges_pruned: usize,
    pub duplicate_edges_removed: usize,
    pub duration_ms: u64,
}

/// Run safe, fast cleanup operations on the database
/// Order: merge same-name → flatten chains → remove empties → fix counts/depths → orphans → edges
#[tauri::command]
pub fn tidy_database(state: State<AppState>) -> Result<TidyReport, String> {
    let start = std::time::Instant::now();
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;

    // Run operations in order (logging done in schema.rs)
    let same_name_merged = db.merge_same_name_children().map_err(|e| e.to_string())?;
    let chains_flattened = db.flatten_single_child_chains().map_err(|e| e.to_string())?;
    let empties_removed = db.remove_empty_categories().map_err(|e| e.to_string())?;
    let empty_items_removed = db.delete_empty_items().map_err(|e| e.to_string())?;
    let child_counts_fixed = db.fix_all_child_counts().map_err(|e| e.to_string())?;
    let depths_fixed = db.fix_all_depths().map_err(|e| e.to_string())?;
    let orphans_reparented = db.reparent_orphans().map_err(|e| e.to_string())?;
    let dead_edges_pruned = db.prune_dead_edges().map_err(|e| e.to_string())?;
    let duplicate_edges_removed = db.deduplicate_edges().map_err(|e| e.to_string())?;

    Ok(TidyReport {
        same_name_merged,
        chains_flattened,
        empties_removed,
        empty_items_removed,
        child_counts_fixed,
        depths_fixed,
        orphans_reparented,
        dead_edges_pruned,
        duplicate_edges_removed,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// Get database stats for settings panel
#[tauri::command]
pub fn get_db_stats(state: State<AppState>) -> Result<DbStats, String> {
    let (total_nodes, total_items, processed_items, items_with_embeddings,
         unprocessed_items, unclustered_items, orphan_items, topics_count) =
        state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_stats().map_err(|e| e.to_string())?;
    Ok(DbStats {
        total_nodes,
        total_items,
        processed_items,
        items_with_embeddings,
        unprocessed_items,
        unclustered_items,
        orphan_items,
        topics_count,
    })
}

/// Get current database path
#[tauri::command]
pub fn get_db_path(state: State<AppState>) -> Result<String, String> {
    Ok(state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_path())
}

/// Switch to a different database file (hot-swap without restart)
/// Creates a new database if the file doesn't exist
#[tauri::command]
pub fn switch_database(app: AppHandle, state: State<AppState>, db_path: String) -> Result<DbStats, String> {
    use std::path::PathBuf;
    use crate::db::Database;
    use crate::hierarchy;
    use crate::settings;
    use tauri::Manager;

    let path = PathBuf::from(&db_path);
    let is_new = !path.exists();

    if is_new {
        println!("Creating new database at: {:?}", path);
    }

    // Save the custom db path to settings (persists across restarts)
    settings::set_custom_db_path(Some(db_path.clone()))?;

    // Create new database connection
    let new_db = Database::new(&path).map_err(|e| e.to_string())?;

    // Auto-build hierarchy if no Universe exists
    if new_db.get_universe().ok().flatten().is_none() {
        if let Err(e) = hierarchy::build_hierarchy(&new_db) {
            eprintln!("Failed to build hierarchy for new database: {}", e);
        }
    }

    // Get stats before swapping
    let (total_nodes, total_items, processed_items, items_with_embeddings,
         unprocessed_items, unclustered_items, orphan_items, topics_count) =
        new_db.get_stats().map_err(|e| e.to_string())?;

    // Hot-swap the database connection
    {
        let mut db_guard = state.db.write().map_err(|e| format!("DB lock error: {}", e))?;
        *db_guard = Arc::new(new_db);
    }

    // Update window title to show new database path
    if let Some(window) = app.get_webview_window("main") {
        let path_str = db_path.clone();
        // Replace home directory with ~ for cleaner display
        let home = std::env::var("HOME").unwrap_or_default();
        let display_path = if !home.is_empty() && path_str.starts_with(&home) {
            path_str.replacen(&home, "~", 1)
        } else {
            path_str
        };
        let title = format!("Mycelica — {}", display_path);
        if let Err(e) = window.set_title(&title) {
            eprintln!("Failed to set window title: {}", e);
        }
    }

    Ok(DbStats {
        total_nodes,
        total_items,
        processed_items,
        items_with_embeddings,
        unprocessed_items,
        unclustered_items,
        orphan_items,
        topics_count,
    })
}

/// Export a trimmed copy of the database without PDF blobs
/// Reduces database size from ~1.8GB to ~50MB for sharing
#[tauri::command]
pub fn export_trimmed_database(state: State<AppState>, output_path: String) -> Result<String, String> {
    use std::path::PathBuf;

    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
    let source_path = db.get_path();
    let output = PathBuf::from(&output_path);

    // Ensure output directory exists
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create output directory: {}", e))?;
    }

    // Copy database file
    std::fs::copy(&source_path, &output).map_err(|e| format!("Failed to copy database: {}", e))?;
    eprintln!("[Export] Copied database to {:?}", output);

    // Open copy and clear PDF blobs
    let conn = rusqlite::Connection::open(&output).map_err(|e| format!("Failed to open copy: {}", e))?;

    // Get original size
    let original_size: i64 = conn.query_row(
        "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
        [],
        |row| row.get(0)
    ).unwrap_or(0);

    // Clear PDF blobs and mark as not available (they'll be downloaded on-demand)
    let cleared = conn.execute("UPDATE papers SET pdf_blob = NULL, pdf_available = 0 WHERE pdf_blob IS NOT NULL", [])
        .map_err(|e| format!("Failed to clear blobs: {}", e))?;
    // Also update the denormalized pdf_available flag in nodes table
    conn.execute("UPDATE nodes SET pdf_available = 0 WHERE pdf_available = 1", [])
        .map_err(|e| format!("Failed to sync nodes: {}", e))?;
    eprintln!("[Export] Cleared {} PDF blobs, marked for on-demand download", cleared);

    // Vacuum to reclaim space
    conn.execute("VACUUM", []).map_err(|e| format!("Failed to vacuum: {}", e))?;

    // Get final size
    let final_size = std::fs::metadata(&output)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    let saved_mb = (original_size - final_size) as f64 / 1024.0 / 1024.0;
    let final_mb = final_size as f64 / 1024.0 / 1024.0;

    let result = format!(
        "Exported to {:?}: {:.1} MB (saved {:.1} MB by removing {} PDF blobs)",
        output, final_mb, saved_mb, cleared
    );
    eprintln!("[Export] {}", result);

    Ok(result)
}

// ==================== Recent Notes Protection ====================

/// Get Recent Notes protection status
#[tauri::command]
pub fn get_protect_recent_notes() -> bool {
    crate::settings::is_recent_notes_protected()
}

/// Set Recent Notes protection status
#[tauri::command]
pub fn set_protect_recent_notes(protected: bool) -> Result<(), String> {
    crate::settings::set_protect_recent_notes(protected)
}

// ==================== Embedding Regeneration ====================

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegenerateProgress {
    pub current: usize,
    pub total: usize,
    pub status: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegenerateResult {
    pub count: usize,
    pub embedding_source: String,
    pub duration_secs: f64,
}

/// Regenerate all embeddings using current embedding source (local or OpenAI)
/// Used when toggling between local and OpenAI embeddings to prevent dimension mismatch
#[tauri::command]
pub async fn regenerate_all_embeddings(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RegenerateResult, String> {
    use crate::local_embeddings;

    let start = std::time::Instant::now();
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clone();

    let use_local = settings::use_local_embeddings();
    let embedding_source = if use_local {
        "local (384-dim)"
    } else {
        "OpenAI (1536-dim)"
    };

    println!("[Regenerate] Starting embedding regeneration using {}", embedding_source);

    // Get all items that need embeddings
    let items = db.get_items().map_err(|e| e.to_string())?;
    let total = items.len();

    if total == 0 {
        return Ok(RegenerateResult {
            count: 0,
            embedding_source: embedding_source.to_string(),
            duration_secs: 0.0,
        });
    }

    // Clear all existing embeddings first
    db.clear_all_embeddings().map_err(|e| e.to_string())?;
    // Also delete semantic edges since they depend on embeddings
    let _ = db.delete_semantic_edges();

    println!("[Regenerate] Cleared existing embeddings, processing {} items", total);

    // Emit initial progress
    let _ = app.emit("regenerate-progress", RegenerateProgress {
        current: 0,
        total,
        status: format!("Starting regeneration using {}...", embedding_source),
    });

    // Build texts for all items - prioritize content for semantic clustering
    let mut item_texts: Vec<(String, String)> = Vec::with_capacity(total); // (id, text)
    for item in &items {
        let text = if let Some(content) = &item.content {
            // Use content (truncated to ~1000 bytes) - most semantically meaningful
            safe_truncate(content, 1000).to_string()
        } else {
            // Fallback for items without content
            format!(
                "{} {}",
                item.ai_title.as_ref().unwrap_or(&item.title),
                item.summary.as_deref().unwrap_or("")
            )
        };
        item_texts.push((item.id.clone(), text));
    }

    let mut success_count = 0;
    let mut error_count = 0;
    let batch_size = 32;

    if use_local {
        // Batch processing for local embeddings - much faster than one-by-one
        let num_batches = (total + batch_size - 1) / batch_size;
        println!("[Regenerate] Processing {} batches of {} items each", num_batches, batch_size);

        for (batch_idx, chunk) in item_texts.chunks(batch_size).enumerate() {
            let texts: Vec<&str> = chunk.iter().map(|(_, text)| text.as_str()).collect();

            match local_embeddings::generate_batch(&texts) {
                Ok(embeddings) => {
                    for (i, embedding) in embeddings.into_iter().enumerate() {
                        let item_id = &chunk[i].0;
                        if let Err(e) = db.update_node_embedding(item_id, &embedding) {
                            eprintln!("[Regenerate] Failed to save embedding for {}: {}", item_id, e);
                            error_count += 1;
                        } else {
                            success_count += 1;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[Regenerate] Batch {} failed: {}", batch_idx, e);
                    error_count += chunk.len();
                }
            }

            // Emit progress after each batch
            let current = ((batch_idx + 1) * batch_size).min(total);
            let _ = app.emit("regenerate-progress", RegenerateProgress {
                current,
                total,
                status: format!("Batch {} of {}...", batch_idx + 1, num_batches),
            });

            // Yield to let the runtime flush events to the UI
            tokio::task::yield_now().await;

            // Log progress every 10 batches
            if batch_idx % 10 == 0 {
                let elapsed = start.elapsed().as_secs_f64();
                let rate = current as f64 / elapsed;
                println!("[Regenerate] Progress: {}/{} ({:.1}%, {:.0} items/sec)",
                    current, total, (current as f64 / total as f64) * 100.0, rate);
            }
        }
    } else {
        // Sequential processing for OpenAI (API rate limits)
        for (idx, (item_id, text)) in item_texts.iter().enumerate() {
            match ai_client::generate_embedding(text).await {
                Ok(embedding) => {
                    if let Err(e) = db.update_node_embedding(item_id, &embedding) {
                        eprintln!("[Regenerate] Failed to save embedding for {}: {}", item_id, e);
                        error_count += 1;
                    } else {
                        success_count += 1;
                    }
                }
                Err(e) => {
                    eprintln!("[Regenerate] Failed to generate embedding for {}: {}", item_id, e);
                    error_count += 1;
                }
            }

            // Emit progress every 10 items
            if idx % 10 == 0 || idx == total - 1 {
                let _ = app.emit("regenerate-progress", RegenerateProgress {
                    current: idx + 1,
                    total,
                    status: format!("Processing {} of {}...", idx + 1, total),
                });
            }
        }
    }

    let duration_secs = start.elapsed().as_secs_f64();

    println!("[Regenerate] Complete: {} succeeded, {} failed, {:.1}s",
        success_count, error_count, duration_secs);

    // Emit completion
    let _ = app.emit("regenerate-progress", RegenerateProgress {
        current: total,
        total,
        status: format!("Complete! {} embeddings regenerated", success_count),
    });

    Ok(RegenerateResult {
        count: success_count,
        embedding_source: embedding_source.to_string(),
        duration_secs,
    })
}

// ==================== Rebuild Lite Commands ====================

/// Result of lite rebuild operations
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RebuildLiteResult {
    pub items_classified: usize,
    pub clusters_created: usize,
    pub hierarchy_levels: usize,
    pub method: String,  // "pattern" or "ai"
}

/// Pre-classify unclassified items using pattern matching (FREE, instant)
///
/// Only classifies items without content_type.
/// Preserves existing classifications (paper, bookmark, code_*, etc.)
/// Use this BEFORE AI processing to identify hidden items that can skip AI.
#[tauri::command]
pub fn preclassify_items(state: State<AppState>) -> Result<PreclassifyResult, String> {
    use crate::classification::{self, ContentType};

    println!("[Preclassify] === STARTING ===");
    println!("[Preclassify] Mode: Pattern matching (FREE)");

    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;

    // Classify only items without content_type (preserves existing)
    let classified = classification::classify_all_items(&db)?;

    // Count how many are now hidden (will skip AI)
    let items = db.get_items().map_err(|e| e.to_string())?;
    let hidden_count = items.iter()
        .filter(|n| n.content_type.as_ref()
            .and_then(|ct| ContentType::from_str(ct))
            .map(|ct| ct.is_hidden())
            .unwrap_or(false))
        .count();

    let visible_count = items.iter()
        .filter(|n| n.content_type.as_ref()
            .and_then(|ct| ContentType::from_str(ct))
            .map(|ct| ct.is_visible())
            .unwrap_or(false))
        .count();

    println!("[Preclassify] === COMPLETE ===");
    println!("  Classified: {}", classified);
    println!("  Hidden (will skip AI): {}", hidden_count);
    println!("  Visible (will process): {}", visible_count);

    Ok(PreclassifyResult {
        classified,
        hidden_count,
        visible_count,
    })
}

/// Result of pre-classification
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreclassifyResult {
    pub classified: usize,
    pub hidden_count: usize,
    pub visible_count: usize,
}

/// Reclassify all items using pattern matching (FREE, instant)
///
/// Updates content_type based on pattern detection.
/// Does NOT use AI - completely free and instant.
#[tauri::command]
pub fn reclassify_pattern(state: State<AppState>) -> Result<usize, String> {
    use crate::classification;

    println!("[Reclassify Pattern] === STARTING ===");
    println!("[Reclassify Pattern] Mode: Pattern matching (FREE)");

    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;

    // First, clear all existing content_type so they get reclassified
    println!("[Reclassify Pattern] Clearing existing classifications...");
    db.clear_all_content_types().map_err(|e| e.to_string())?;

    // Now classify all items (this function logs the detailed breakdown)
    let classified = classification::classify_all_items(&db)?;

    println!("[Reclassify Pattern] === COMPLETE ===");
    println!("  Total classified: {}", classified);
    println!("  Cost: FREE");

    Ok(classified)
}

/// Reclassify all items using AI (CHEAP, ~$0.04 for 4000 items)
///
/// Uses minimal AI prompt that only returns content_type.
/// ~50 tokens per item with Haiku.
/// Note: API calls are sequential (no batching) but cheap.
#[tauri::command]
pub async fn reclassify_ai(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    use std::collections::HashMap;

    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clone();

    // Get all items
    let items = db.get_items().map_err(|e| e.to_string())?;
    let total = items.len();

    if total == 0 {
        println!("[Reclassify AI] No items to classify");
        return Ok(0);
    }

    println!("[Reclassify AI] === STARTING ===");
    println!("[Reclassify AI] Processing {} items with Haiku...", total);
    println!("[Reclassify AI] Estimated cost: ~${:.2}", total as f64 * 0.00001);

    // Emit progress
    let _ = app.emit("reclassify-progress", serde_json::json!({
        "current": 0,
        "total": total,
        "status": "Starting AI classification..."
    }));

    let mut classified = 0;
    let mut skipped_empty = 0;
    let mut api_errors = 0;
    let mut type_counts: HashMap<String, usize> = HashMap::new();

    for (idx, item) in items.iter().enumerate() {
        // Check cancellation
        if CANCEL_PROCESSING.load(Ordering::SeqCst) {
            println!("[Reclassify AI] Cancelled at {}/{}", idx, total);
            CANCEL_PROCESSING.store(false, Ordering::SeqCst);
            break;
        }

        // Skip papers (fixed content_type, never reclassify)
        if item.content_type.as_deref() == Some("paper") {
            continue;
        }

        // Skip bookmarks (web captures have fixed content_type, never reclassify)
        if item.content_type.as_deref() == Some("bookmark") {
            continue;
        }

        let content = item.content.as_deref().unwrap_or("");
        if content.is_empty() {
            skipped_empty += 1;
            continue;
        }

        let content_type = match ai_client::classify_content_ai(content).await {
            Ok(ct) => ct,
            Err(e) => {
                api_errors += 1;
                if api_errors <= 3 {
                    eprintln!("[Reclassify AI] API error for {}: {}", item.id, e);
                }
                // Use pattern matcher fallback
                use crate::classification::classify_content;
                classify_content(content).as_str().to_string()
            }
        };

        *type_counts.entry(content_type.clone()).or_insert(0) += 1;
        db.set_content_type(&item.id, &content_type)
            .map_err(|e| e.to_string())?;
        classified += 1;

        // Emit progress every 10 items, log every 100
        if idx % 100 == 0 && idx > 0 {
            println!("[Reclassify AI] Progress: {}/{} ({:.1}%)", idx, total, (idx as f64 / total as f64) * 100.0);
        }
        if idx % 10 == 0 || idx == total - 1 {
            let _ = app.emit("reclassify-progress", serde_json::json!({
                "current": idx + 1,
                "total": total,
                "status": format!("Classifying {} of {}...", idx + 1, total)
            }));
        }
    }

    // Log results
    println!("[Reclassify AI] === RESULTS ===");
    println!("  Classified: {}", classified);
    println!("  Skipped (empty): {}", skipped_empty);
    println!("  API errors (used fallback): {}", api_errors);
    println!("[Reclassify AI] === BY TYPE ===");

    let mut sorted_counts: Vec<_> = type_counts.iter().collect();
    sorted_counts.sort_by(|a, b| b.1.cmp(a.1));
    for (content_type, count) in sorted_counts {
        let tier = match content_type.as_str() {
            "insight" | "exploration" | "synthesis" | "question" | "planning" => "VISIBLE",
            "investigation" | "discussion" | "reference" | "creative" => "SUPPORTING",
            "debug" | "code" | "paste" | "trivial" => "HIDDEN",
            _ => "UNKNOWN",
        };
        println!("  {:12} {:5} ({})", content_type, count, tier);
    }

    Ok(classified)
}

/// Rebuild Lite: Reclassify + recluster only (FREE)
///
/// SAFE: Does NOT touch hierarchy structure.
/// Only updates content_type and cluster_id on items.
#[tauri::command]
pub async fn rebuild_lite(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RebuildLiteResult, String> {
    use crate::classification;

    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clone();

    println!("[Rebuild Lite] === STARTING ===");
    println!("[Rebuild Lite] SAFE: Only updating content_type + cluster_id (hierarchy untouched)");

    // Step 1: Pattern classify all items (FREE)
    println!("[Rebuild Lite] Step 1/2: Classifying items...");
    let _ = app.emit("rebuild-lite-progress", serde_json::json!({
        "step": 1,
        "total_steps": 2,
        "status": "Classifying items..."
    }));

    let classified = classification::classify_all_items(&db)?;

    // Step 2: Recluster with existing embeddings (FREE)
    println!("[Rebuild Lite] Step 2/2: Reclustering...");
    let _ = app.emit("rebuild-lite-progress", serde_json::json!({
        "step": 2,
        "total_steps": 2,
        "status": "Reclustering..."
    }));

    let items_marked = db.mark_all_items_need_clustering().map_err(|e| e.to_string())?;
    println!("[Rebuild Lite] Marked {} items for clustering", items_marked);

    let cluster_result = clustering::cluster_with_embeddings_lite(&db).await?;

    let _ = app.emit("rebuild-lite-progress", serde_json::json!({
        "step": 2,
        "total_steps": 2,
        "status": "Complete!"
    }));

    println!("[Rebuild Lite] === COMPLETE ===");
    println!("  Items classified: {}", classified);
    println!("  Clusters: {}", cluster_result.clusters_created);
    println!("  Hierarchy: UNTOUCHED");
    println!("  Cost: FREE");

    Ok(RebuildLiteResult {
        items_classified: classified,
        clusters_created: cluster_result.clusters_created,
        hierarchy_levels: 0,
        method: "lite-safe".to_string(),
    })
}

/// Rebuild Hierarchy Only: Regroup existing topics into uber-categories (CHEAP)
///
/// PRESERVES: Items, clusters, cluster assignments
/// REBUILDS: Only the hierarchy grouping (Universe → categories → topics → items)
///
/// Use this when you have good clusters but want better organization.
#[tauri::command]
pub async fn rebuild_hierarchy_only(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RebuildLiteResult, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clone();

    println!("[Rebuild Hierarchy] === STARTING ===");
    println!("[Rebuild Hierarchy] Keeping items + clusters, only rebuilding hierarchy grouping");

    let _ = app.emit("rebuild-hierarchy-progress", serde_json::json!({
        "step": 1,
        "total_steps": 2,
        "status": "Clearing old hierarchy..."
    }));

    // Clear hierarchy but keep items and their cluster assignments
    hierarchy::clear_hierarchy(&db)?;
    println!("[Rebuild Hierarchy] Cleared old hierarchy (items + clusters preserved)");

    let _ = app.emit("rebuild-hierarchy-progress", serde_json::json!({
        "step": 2,
        "total_steps": 2,
        "status": "Building hierarchy with AI grouping..."
    }));

    // Build full hierarchy with AI grouping
    let result = hierarchy::build_full_hierarchy(&db, false, Some(&app)).await?;

    let _ = app.emit("rebuild-hierarchy-progress", serde_json::json!({
        "step": 2,
        "total_steps": 2,
        "status": "Complete!"
    }));

    println!("[Rebuild Hierarchy] === COMPLETE ===");
    println!("  Levels created: {}", result.levels_created);
    println!("  Grouping iterations: {}", result.grouping_iterations);
    println!("  Cost: ~$0.05-0.15 (AI grouping only)");

    Ok(RebuildLiteResult {
        items_classified: 0,
        clusters_created: 0,
        hierarchy_levels: result.levels_created,
        method: "hierarchy-only".to_string(),
    })
}

// ==================== Smart Add (10-50 items) ====================

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartAddResult {
    pub orphans_found: usize,
    pub matched_to_existing: usize,
    pub new_topics_created: usize,
    pub sent_to_inbox: usize,
    pub processing_time_ms: u64,
}

/// Smart add orphaned items to hierarchy using embedding similarity
/// For medium batches (10-50 items) - faster than full rebuild, smarter than quick add
/// Uses embedding similarity to match items to existing topics
#[tauri::command]
pub async fn smart_add_to_hierarchy(
    state: State<'_, AppState>,
) -> Result<SmartAddResult, String> {
    use crate::db::{Node, NodeType, Position};
    use crate::similarity::cosine_similarity;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};
    use std::collections::HashMap;

    let start = Instant::now();
    const INBOX_ID: &str = "inbox-category";
    const INBOX_TITLE: &str = "📥 Inbox";

    // Content types to exclude (SUPPORTING and HIDDEN tiers)
    const EXCLUDED_TYPES: &[&str] = &[
        "investigation", "discussion", "reference", "creative",  // SUPPORTING
        "debug", "code", "paste", "trivial",                     // HIDDEN
    ];

    // Get clustering thresholds
    let (_, secondary_threshold) = crate::settings::get_clustering_thresholds();
    let similarity_threshold = secondary_threshold.unwrap_or(0.60);

    // Get orphaned items (processed, with cluster_id)
    let all_orphans = state.db.read()
        .map_err(|e| format!("DB lock error: {}", e))?
        .get_orphaned_clustered_items()
        .map_err(|e| e.to_string())?;

    // Filter out SUPPORTING/HIDDEN content types
    let orphans: Vec<Node> = all_orphans
        .into_iter()
        .filter(|item| {
            match &item.content_type {
                Some(ct) => !EXCLUDED_TYPES.contains(&ct.as_str()),
                None => true, // Include items without content_type
            }
        })
        .collect();

    if orphans.is_empty() {
        return Ok(SmartAddResult {
            orphans_found: 0,
            matched_to_existing: 0,
            new_topics_created: 0,
            sent_to_inbox: 0,
            processing_time_ms: start.elapsed().as_millis() as u64,
        });
    }

    println!("[SmartAdd] Found {} orphaned items to process", orphans.len());

    // Get Universe
    let universe = state.db.read()
        .map_err(|e| format!("DB lock error: {}", e))?
        .get_universe()
        .map_err(|e| e.to_string())?
        .ok_or("No Universe node found - run full hierarchy build first")?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    // Get or create Inbox category
    let inbox_depth = universe.depth + 1;
    let inbox = match state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_node(INBOX_ID).map_err(|e| e.to_string())? {
        Some(existing) => existing,
        None => {
            let inbox_node = Node {
                id: INBOX_ID.to_string(),
                title: INBOX_TITLE.to_string(),
                node_type: NodeType::Cluster,
                url: None,
                content: None,
                position: Position { x: 0.0, y: 0.0 },
                created_at: now,
                updated_at: now,
                cluster_id: None,
                cluster_label: Some("Inbox".to_string()),
                depth: inbox_depth,
                is_item: false,
                is_universe: false,
                parent_id: Some(universe.id.clone()),
                child_count: 0,
                ai_title: None,
                summary: Some("New items awaiting organization".to_string()),
                tags: None,
                emoji: Some("📥".to_string()),
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
            };
            state.db.read().map_err(|e| format!("DB lock error: {}", e))?.insert_node(&inbox_node).map_err(|e| e.to_string())?;
            state.db.read().map_err(|e| format!("DB lock error: {}", e))?.increment_child_count(&universe.id).map_err(|e| e.to_string())?;
            println!("[SmartAdd] Created 📥 Inbox category");
            inbox_node
        }
    };

    // Get all topic nodes (non-items with embeddings) and their centroids
    // A topic is any non-item, non-universe node that has children
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;

    // Build a map of topic_id -> stored centroid embedding
    // Topics already have centroids computed during hierarchy build
    let mut topic_centroids: HashMap<String, Vec<f32>> = HashMap::new();

    // Get all non-item nodes as potential topics
    let all_nodes = db.get_all_nodes(false).map_err(|e| e.to_string())?;
    let topic_nodes: Vec<&Node> = all_nodes.iter()
        .filter(|n| !n.is_item && !n.is_universe && n.id != INBOX_ID)
        .collect();

    println!("[SmartAdd] Found {} potential topic nodes", topic_nodes.len());

    // Fetch stored embeddings directly (topics have centroids from hierarchy build)
    for topic in &topic_nodes {
        if let Some(emb) = db.get_node_embedding(&topic.id).ok().flatten() {
            topic_centroids.insert(topic.id.clone(), emb);
        }
    }

    // Pre-fetch orphan embeddings and build topic lookup map
    let mut orphan_embeddings: HashMap<String, Vec<f32>> = HashMap::new();
    let mut topic_nodes_map: HashMap<String, Node> = HashMap::new();

    for orphan in &orphans {
        if let Some(emb) = db.get_node_embedding(&orphan.id).ok().flatten() {
            orphan_embeddings.insert(orphan.id.clone(), emb);
        }
    }

    for topic in topic_nodes {
        topic_nodes_map.insert(topic.id.clone(), topic.clone());
    }

    drop(db); // Release lock before processing

    println!("[SmartAdd] Loaded centroids for {} topics, {} orphan embeddings",
        topic_centroids.len(), orphan_embeddings.len());

    let mut matched_to_existing = 0;
    let mut sent_to_inbox = 0;
    let mut new_topics_created = 0;
    let mut unmatched_items: Vec<Node> = Vec::new();

    // Process each orphan using pre-fetched data
    for item in orphans.iter() {
        // Get pre-fetched embedding
        let item_emb = match orphan_embeddings.get(&item.id) {
            Some(emb) => emb,
            None => {
                // No embedding - fall back to cluster_label matching
                if let Some(ref label) = item.cluster_label {
                    if let Ok(Some(topic)) = state.db.read()
                        .map_err(|e| format!("DB lock error: {}", e))?
                        .find_topic_by_cluster_label(label)
                    {
                        let item_depth = topic.depth + 1;
                        let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
                        db.set_node_parent(&item.id, &topic.id, item_depth)
                            .map_err(|e| e.to_string())?;
                        db.increment_child_count(&topic.id)
                            .map_err(|e| e.to_string())?;
                        matched_to_existing += 1;
                        println!("[SmartAdd] Matched '{}' to '{}' by cluster_label",
                            item.ai_title.as_ref().unwrap_or(&item.title), topic.title);
                        continue;
                    }
                }
                // No embedding and no cluster_label match - add to unmatched
                unmatched_items.push(item.clone());
                continue;
            }
        };

        // Find best matching topic by embedding similarity
        let mut best_topic: Option<(String, f32)> = None;
        for (topic_id, centroid) in &topic_centroids {
            let similarity = cosine_similarity(item_emb, centroid);
            if similarity > similarity_threshold {
                if best_topic.is_none() || similarity > best_topic.as_ref().unwrap().1 {
                    best_topic = Some((topic_id.clone(), similarity));
                }
            }
        }

        if let Some((topic_id, similarity)) = best_topic {
            // Found a good match - use pre-fetched topic data
            let topic = topic_nodes_map.get(&topic_id)
                .ok_or_else(|| format!("Topic {} not found", topic_id))?;

            let item_depth = topic.depth + 1;
            let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
            db.set_node_parent(&item.id, &topic_id, item_depth)
                .map_err(|e| e.to_string())?;
            db.increment_child_count(&topic_id)
                .map_err(|e| e.to_string())?;

            println!("[SmartAdd] Matched '{}' to '{}' (similarity: {:.3})",
                item.ai_title.as_ref().unwrap_or(&item.title),
                topic.title,
                similarity);
            matched_to_existing += 1;
        } else {
            // No good match - add to unmatched list
            unmatched_items.push(item.clone());
        }
    }

    println!("[SmartAdd] {} items matched, {} unmatched", matched_to_existing, unmatched_items.len());

    // Handle unmatched items
    if !unmatched_items.is_empty() {
        if unmatched_items.len() < 5 {
            // Few items - just put in Inbox
            let topic_depth = inbox_depth + 1;
            let item_depth = topic_depth + 1;

            for item in &unmatched_items {
                // Create a topic under Inbox if item has cluster_label
                if let Some(ref label) = item.cluster_label {
                    let topic_id = format!("topic-smart-{}", item.cluster_id.unwrap_or(0));

                    // Check if topic exists
                    let topic_exists = state.db.read()
                        .map_err(|e| format!("DB lock error: {}", e))?
                        .get_node(&topic_id)
                        .map_err(|e| e.to_string())?
                        .is_some();

                    if !topic_exists {
                        // Create new topic under Inbox
                        let topic_node = Node {
                            id: topic_id.clone(),
                            title: label.clone(),
                            node_type: NodeType::Cluster,
                            url: None,
                            content: None,
                            position: Position { x: 0.0, y: 0.0 },
                            created_at: now,
                            updated_at: now,
                            cluster_id: item.cluster_id,
                            cluster_label: Some(label.clone()),
                            depth: topic_depth,
                            is_item: false,
                            is_universe: false,
                            parent_id: Some(inbox.id.clone()),
                            child_count: 0,
                            ai_title: None,
                            summary: Some(format!("New topic: {}", label)),
                            tags: None,
                            emoji: None,
                            is_processed: false,
                            conversation_id: None,
                            sequence_index: None,
                            is_pinned: false,
                            last_accessed_at: None,
                            latest_child_date: Some(item.created_at),
                            is_private: None,
                            privacy_reason: None,
                            source: None,
                            pdf_available: None,
                            content_type: None,
                            associated_idea_id: None,
                            privacy: None,
                        };

                        state.db.read().map_err(|e| format!("DB lock error: {}", e))?
                            .insert_node(&topic_node)
                            .map_err(|e| e.to_string())?;
                        state.db.read().map_err(|e| format!("DB lock error: {}", e))?
                            .increment_child_count(&inbox.id)
                            .map_err(|e| e.to_string())?;
                        new_topics_created += 1;
                    }

                    // Add item to the topic
                    state.db.read().map_err(|e| format!("DB lock error: {}", e))?
                        .set_node_parent(&item.id, &topic_id, item_depth)
                        .map_err(|e| e.to_string())?;
                    state.db.read().map_err(|e| format!("DB lock error: {}", e))?
                        .increment_child_count(&topic_id)
                        .map_err(|e| e.to_string())?;
                } else {
                    // No cluster_label - add directly to Inbox (one level below inbox)
                    state.db.read().map_err(|e| format!("DB lock error: {}", e))?
                        .set_node_parent(&item.id, &inbox.id, inbox_depth + 1)
                        .map_err(|e| e.to_string())?;
                    state.db.read().map_err(|e| format!("DB lock error: {}", e))?
                        .increment_child_count(&inbox.id)
                        .map_err(|e| e.to_string())?;
                }
                sent_to_inbox += 1;
            }
            println!("[SmartAdd] Sent {} items to Inbox", sent_to_inbox);
        } else {
            // 5+ unmatched items - cluster them into new topics
            // Group by cluster_label first
            let mut label_groups: HashMap<String, Vec<&Node>> = HashMap::new();
            let mut no_label: Vec<&Node> = Vec::new();

            for item in &unmatched_items {
                match &item.cluster_label {
                    Some(label) => {
                        label_groups.entry(label.clone()).or_default().push(item);
                    }
                    None => no_label.push(item),
                }
            }

            let topic_depth = inbox_depth + 1;
            let item_depth = topic_depth + 1;

            // Create topics for each label group
            for (label, items) in label_groups {
                let cluster_id = items[0].cluster_id.unwrap_or(0);
                let topic_id = format!("topic-smart-{}", cluster_id);

                // Check if topic exists
                let existing = state.db.read()
                    .map_err(|e| format!("DB lock error: {}", e))?
                    .get_node(&topic_id)
                    .map_err(|e| e.to_string())?;

                if existing.is_none() {
                    // Create new topic
                    let topic_node = Node {
                        id: topic_id.clone(),
                        title: label.clone(),
                        node_type: NodeType::Cluster,
                        url: None,
                        content: None,
                        position: Position { x: 0.0, y: 0.0 },
                        created_at: now,
                        updated_at: now,
                        cluster_id: Some(cluster_id),
                        cluster_label: Some(label.clone()),
                        depth: topic_depth,
                        is_item: false,
                        is_universe: false,
                        parent_id: Some(inbox.id.clone()),
                        child_count: 0,
                        ai_title: None,
                        summary: Some(format!("New topic: {}", label)),
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
                    };

                    state.db.read().map_err(|e| format!("DB lock error: {}", e))?
                        .insert_node(&topic_node)
                        .map_err(|e| e.to_string())?;
                    state.db.read().map_err(|e| format!("DB lock error: {}", e))?
                        .increment_child_count(&inbox.id)
                        .map_err(|e| e.to_string())?;
                    new_topics_created += 1;
                }

                // Add all items to this topic
                for item in items {
                    state.db.read().map_err(|e| format!("DB lock error: {}", e))?
                        .set_node_parent(&item.id, &topic_id, item_depth)
                        .map_err(|e| e.to_string())?;
                    state.db.read().map_err(|e| format!("DB lock error: {}", e))?
                        .increment_child_count(&topic_id)
                        .map_err(|e| e.to_string())?;
                    sent_to_inbox += 1;
                }
            }

            // Items without labels go directly to Inbox (one level below inbox)
            for item in no_label {
                state.db.read().map_err(|e| format!("DB lock error: {}", e))?
                    .set_node_parent(&item.id, &inbox.id, inbox_depth + 1)
                    .map_err(|e| e.to_string())?;
                state.db.read().map_err(|e| format!("DB lock error: {}", e))?
                    .increment_child_count(&inbox.id)
                    .map_err(|e| e.to_string())?;
                sent_to_inbox += 1;
            }

            println!("[SmartAdd] Created {} new topics, sent {} items to Inbox",
                new_topics_created, sent_to_inbox);
        }
    }

    let elapsed = start.elapsed().as_millis() as u64;
    println!("[SmartAdd] Complete in {}ms: {} matched, {} new topics, {} to inbox",
        elapsed, matched_to_existing, new_topics_created, sent_to_inbox);

    Ok(SmartAddResult {
        orphans_found: orphans.len(),
        matched_to_existing,
        new_topics_created,
        sent_to_inbox,
        processing_time_ms: elapsed,
    })
}

// ============================================================================
// Code Import Commands
// ============================================================================

#[derive(Debug, Clone, serde::Serialize)]
pub struct CodeImportResult {
    pub functions: usize,
    pub structs: usize,
    pub enums: usize,
    pub traits: usize,
    pub impls: usize,
    pub modules: usize,
    pub macros: usize,
    pub docs: usize,
    pub files_processed: usize,
    pub files_skipped: usize,
    pub edges_created: usize,
    pub doc_edges: usize,
    pub errors: Vec<String>,
}

impl From<crate::code::CodeImportResult> for CodeImportResult {
    fn from(r: crate::code::CodeImportResult) -> Self {
        Self {
            functions: r.functions,
            structs: r.structs,
            enums: r.enums,
            traits: r.traits,
            impls: r.impls,
            modules: r.modules,
            macros: r.macros,
            docs: r.docs,
            files_processed: r.files_processed,
            files_skipped: r.files_skipped,
            edges_created: r.edges_created,
            doc_edges: r.doc_edges,
            errors: r.errors,
        }
    }
}

/// Import source code from a directory into the graph.
/// Respects .gitignore automatically.
#[tauri::command]
pub fn import_code(
    state: State<AppState>,
    path: String,
    language: Option<String>,
) -> Result<CodeImportResult, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;

    let result = crate::code::import_code(&db, &path, language.as_deref())?;

    Ok(result.into())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CodeEdgesResult {
    pub functions_analyzed: usize,
    pub edges_found: usize,
    pub edges_created: usize,
}

/// Analyze code and create "Calls" edges between functions.
#[tauri::command]
pub fn analyze_code_edges(
    state: State<AppState>,
    path_filter: Option<String>,
) -> Result<CodeEdgesResult, String> {
    use std::collections::{HashMap, HashSet};
    use crate::db::EdgeType;

    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;

    // Get all code function nodes
    let all_nodes = db.get_items().map_err(|e| e.to_string())?;
    let code_functions: Vec<_> = all_nodes
        .iter()
        .filter(|n| {
            n.content_type.as_deref() == Some("code_function")
                && n.source.as_deref().map(|s| s.starts_with("code-")).unwrap_or(false)
        })
        .filter(|n| {
            if let Some(ref filter) = path_filter {
                n.tags.as_ref().map(|t| t.contains(filter)).unwrap_or(false)
            } else {
                true
            }
        })
        .collect();

    // Build function name -> node ID map
    let mut name_to_id: HashMap<String, String> = HashMap::new();
    for func in &code_functions {
        if let Some(name) = extract_fn_name(&func.title) {
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
    let mut edges_found = 0;

    for func in &code_functions {
        let content = match &func.content {
            Some(c) => c,
            None => continue,
        };

        let caller_id = &func.id;
        let caller_name = extract_fn_name(&func.title).unwrap_or_default();

        // Find called functions
        for (name, callee_id) in &name_to_id {
            if *name == caller_name {
                continue; // Skip self
            }

            // Simple heuristic: name followed by '(' in content
            let call_pattern = format!("{}(", name);
            if content.contains(&call_pattern) {
                if existing_edges.contains(&(caller_id.clone(), callee_id.clone())) {
                    continue;
                }

                edges_found += 1;

                let edge = crate::db::Edge {
                    id: format!("calls-{}-{}", &caller_id[..8.min(caller_id.len())], &callee_id[..8.min(callee_id.len())]),
                    source: caller_id.clone(),
                    target: callee_id.clone(),
                    edge_type: EdgeType::Calls,
                    label: Some(format!("{} -> {}", caller_name, name)),
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

    Ok(CodeEdgesResult {
        functions_analyzed: code_functions.len(),
        edges_found,
        edges_created,
    })
}

fn extract_fn_name(title: &str) -> Option<String> {
    let fn_idx = title.find("fn ")?;
    let after_fn = &title[fn_idx + 3..];
    let name_end = after_fn
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(after_fn.len());
    let name = &after_fn[..name_end];
    if name.is_empty() { None } else { Some(name.to_string()) }
}
