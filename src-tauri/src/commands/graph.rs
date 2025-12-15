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
use serde::Serialize;

// Global cancellation flags
static CANCEL_PROCESSING: AtomicBool = AtomicBool::new(false);
pub static CANCEL_REBUILD: AtomicBool = AtomicBool::new(false);

pub struct AppState {
    pub db: RwLock<Arc<Database>>,
}

#[tauri::command]
pub fn get_nodes(state: State<AppState>) -> Result<Vec<Node>, String> {
    state.db.read().unwrap().get_all_nodes().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_node(state: State<AppState>, id: String) -> Result<Option<Node>, String> {
    state.db.read().unwrap().get_node(&id).map_err(|e| e.to_string())
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

/// Check if rebuild was cancelled (for use in other modules)
pub fn is_rebuild_cancelled() -> bool {
    CANCEL_REBUILD.load(Ordering::SeqCst)
}

/// Reset rebuild cancel flag (call at start of rebuild)
pub fn reset_rebuild_cancel() {
    CANCEL_REBUILD.store(false, Ordering::SeqCst);
}

#[tauri::command]
pub fn create_node(state: State<AppState>, node: Node) -> Result<(), String> {
    state.db.read().unwrap().insert_node(&node).map_err(|e| e.to_string())
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
    let container_exists = state.db.read().unwrap().get_node(container_id).ok().flatten().is_some();

    if !container_exists {
        // Get Universe to set as parent
        let universe = state.db.read().unwrap().get_universe()
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
        };
        state.db.read().unwrap().insert_node(&container_node).map_err(|e| e.to_string())?;

        // Update Universe child_count
        state.db.read().unwrap().update_child_count(&universe.id, universe.child_count + 1)
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
    };

    state.db.read().unwrap().insert_node(&note).map_err(|e| e.to_string())?;

    // 3. Update container child_count
    let children = state.db.read().unwrap().get_children(container_id).map_err(|e| e.to_string())?;
    state.db.read().unwrap().update_child_count(container_id, children.len() as i32)
        .map_err(|e| e.to_string())?;

    Ok(note_id)
}

#[tauri::command]
pub fn update_node(state: State<AppState>, node: Node) -> Result<(), String> {
    state.db.read().unwrap().update_node(&node).map_err(|e| e.to_string())
}

/// Update just the content of a node (simpler API for editing)
#[tauri::command]
pub fn update_node_content(state: State<AppState>, node_id: String, content: String) -> Result<(), String> {
    state.db.read().unwrap().update_node_content(&node_id, &content).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_node(state: State<AppState>, id: String) -> Result<(), String> {
    state.db.read().unwrap().delete_node(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_edges(state: State<AppState>) -> Result<Vec<Edge>, String> {
    state.db.read().unwrap().get_all_edges().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_edges_for_node(state: State<AppState>, node_id: String) -> Result<Vec<Edge>, String> {
    state.db.read().unwrap().get_edges_for_node(&node_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_edge(state: State<AppState>, edge: Edge) -> Result<(), String> {
    state.db.read().unwrap().insert_edge(&edge).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_edge(state: State<AppState>, id: String) -> Result<(), String> {
    state.db.read().unwrap().delete_edge(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn search_nodes(state: State<AppState>, query: String) -> Result<Vec<Node>, String> {
    state.db.read().unwrap().search_nodes(&query).map_err(|e| e.to_string())
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
    let needs_clustering = state.db.read().unwrap().count_items_needing_clustering().map_err(|e| e.to_string())?;
    let all_items = state.db.read().unwrap().get_items().map_err(|e| e.to_string())?;

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
    let db = state.db.read().unwrap().clone();
    clustering::run_clustering(&db, use_ai).await
}

/// Force re-clustering of all items
#[tauri::command]
pub async fn recluster_all(state: State<'_, AppState>, use_ai: Option<bool>) -> Result<ClusterResult, String> {
    let use_ai = use_ai.unwrap_or(true);
    let db = state.db.read().unwrap().clone();
    clustering::recluster_all(&db, use_ai).await
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
    let all_nodes = state.db.read().unwrap().get_all_nodes().map_err(|e| e.to_string())?;
    let unprocessed = state.db.read().unwrap().get_unprocessed_nodes().map_err(|e| e.to_string())?;

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
    pub emoji: Option<String>,
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
    let db = state.db.read().unwrap().clone();

    // Get unprocessed nodes, excluding protected (Recent Notes)
    let all_unprocessed = db.get_unprocessed_nodes().map_err(|e| e.to_string())?;
    let protected_ids = db.get_protected_node_ids();
    let unprocessed: Vec<_> = all_unprocessed
        .into_iter()
        .filter(|node| !protected_ids.contains(&node.id))
        .collect();

    if !protected_ids.is_empty() {
        println!("[AI Processing] Excluding {} protected items (Recent Notes)", protected_ids.len());
    }

    let total = unprocessed.len();
    println!("[AI Processing] Found {} unprocessed nodes", total);

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
                emoji: None,
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
            emoji: None,
            status: "processing".to_string(),
            error_message: None,
            elapsed_secs: Some(elapsed),
            estimate_secs: estimate,
            remaining_secs: remaining,
        });

        match ai_client::analyze_node(&node.title, content).await {
            Ok(result) => {
                let tags_json = serde_json::to_string(&result.tags).unwrap_or_default();

                if let Err(e) = db.update_node_ai(
                    &node.id,
                    &result.title,
                    &result.summary,
                    &tags_json,
                    &result.emoji,
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
                        emoji: None,
                        status: "error".to_string(),
                        error_message: Some(err_msg),
                        elapsed_secs: Some(elapsed_now),
                        estimate_secs: estimate,
                        remaining_secs: remaining,
                    });
                } else {
                    processed += 1;
                    println!("Processed node: {} -> {} {}", node.title, result.emoji, result.title);

                    // Generate embedding if OpenAI API key is available
                    let embed_text = format!("{} {}", result.title, result.summary);
                    match ai_client::generate_embedding(&embed_text).await {
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
                        emoji: Some(result.emoji),
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
                    emoji: None,
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
        emoji: None,
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
    state.db.read().unwrap().get_learned_emojis().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_learned_emoji(state: State<AppState>, keyword: String, emoji: String) -> Result<(), String> {
    state.db.read().unwrap().save_learned_emoji(&keyword, &emoji).map_err(|e| e.to_string())
}

// ==================== Hierarchy Navigation Commands ====================

/// Get nodes at a specific depth (0=Universe, increases toward items)
#[tauri::command]
pub fn get_nodes_at_depth(state: State<AppState>, depth: i32) -> Result<Vec<Node>, String> {
    state.db.read().unwrap().get_nodes_at_depth(depth).map_err(|e| e.to_string())
}

/// Get children of a specific parent node
#[tauri::command]
pub fn get_children(state: State<AppState>, parent_id: String) -> Result<Vec<Node>, String> {
    state.db.read().unwrap().get_children(&parent_id).map_err(|e| e.to_string())
}

/// Get the Universe node (single root, is_universe = true)
#[tauri::command]
pub fn get_universe(state: State<AppState>) -> Result<Option<Node>, String> {
    state.db.read().unwrap().get_universe().map_err(|e| e.to_string())
}

/// Get all items (is_item = true) - openable content
#[tauri::command]
pub fn get_items(state: State<AppState>) -> Result<Vec<Node>, String> {
    state.db.read().unwrap().get_items().map_err(|e| e.to_string())
}

/// Get the maximum depth in the hierarchy
#[tauri::command]
pub fn get_max_depth(state: State<AppState>) -> Result<i32, String> {
    state.db.read().unwrap().get_max_depth().map_err(|e| e.to_string())
}

// ==================== Hierarchy Generation Commands ====================

/// Build the full hierarchy from items
/// Creates intermediate grouping nodes based on collection size
#[tauri::command]
pub fn build_hierarchy(state: State<'_, AppState>) -> Result<hierarchy::HierarchyResult, String> {
    let db = state.db.read().unwrap();
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
    let db = state.db.read().unwrap().clone();
    hierarchy::build_full_hierarchy(&db, should_cluster, Some(&app)).await
}

/// Cluster children of a specific parent node into groups using AI
///
/// Returns true if grouping was performed, false if already has ≤15 children.
/// Use this for manual/targeted hierarchy adjustment.
/// max_groups: optional maximum number of groups to create (default 5)
#[tauri::command]
pub async fn cluster_hierarchy_level(
    app: AppHandle,
    state: State<'_, AppState>,
    parent_id: String,
    max_groups: Option<usize>,
) -> Result<bool, String> {
    let db = state.db.read().unwrap().clone();
    hierarchy::cluster_hierarchy_level(&db, &parent_id, Some(&app), max_groups).await
}

/// Unsplit a node - flatten intermediate categories back into the parent
///
/// Moves all grandchildren up to be direct children and deletes the intermediate category nodes.
/// Returns the number of nodes that were flattened.
#[tauri::command]
pub fn unsplit_node(state: State<AppState>, parent_id: String) -> Result<usize, String> {
    let db = state.db.read().unwrap();

    // Get direct children of this node
    let children = db.get_children(&parent_id).map_err(|e| e.to_string())?;

    let mut flattened_count = 0;
    let mut categories_to_delete: Vec<String> = Vec::new();

    for child in &children {
        // Only process category nodes (not items)
        if child.is_item {
            continue;
        }

        // Get grandchildren (children of this category)
        let grandchildren = db.get_children(&child.id).map_err(|e| e.to_string())?;

        if grandchildren.is_empty() {
            // Empty category - just delete it
            categories_to_delete.push(child.id.clone());
            continue;
        }

        // Reparent grandchildren to the parent node
        for grandchild in &grandchildren {
            db.update_node_parent(&grandchild.id, &parent_id)
                .map_err(|e| e.to_string())?;
            flattened_count += 1;
        }

        // Mark this intermediate category for deletion
        categories_to_delete.push(child.id.clone());
    }

    // Delete the intermediate categories
    for cat_id in &categories_to_delete {
        db.delete_node(cat_id).map_err(|e| e.to_string())?;
    }

    // Update child count of parent
    let new_child_count = db.get_children(&parent_id)
        .map_err(|e| e.to_string())?
        .len() as i32;
    db.update_child_count(&parent_id, new_child_count)
        .map_err(|e| e.to_string())?;

    Ok(flattened_count)
}

/// Get children of a node, automatically skipping single-child chains
///
/// Useful for navigation - if a level has exactly 1 child, skip to that child's children.
#[tauri::command]
pub fn get_children_flat(state: State<AppState>, parent_id: String) -> Result<Vec<Node>, String> {
    let db = state.db.read().unwrap();
    hierarchy::get_children_skip_single_chain(&db, &parent_id)
}

/// Propagate latest_child_date from leaves up through the hierarchy
///
/// Fast operation (~seconds) - no AI or embeddings involved.
/// Leaves get their created_at, groups get MAX of their children's latest_child_date.
#[tauri::command]
pub fn propagate_latest_dates(state: State<AppState>) -> Result<(), String> {
    state.db.read().unwrap().propagate_latest_dates().map_err(|e| e.to_string())
}

// ==================== Multi-Path Association Commands ====================

/// Get all category associations for an item (via BelongsTo edges)
/// Returns edges sorted by weight (highest first)
#[tauri::command]
pub fn get_item_associations(state: State<AppState>, item_id: String) -> Result<Vec<Edge>, String> {
    state.db.read().unwrap().get_belongs_to_edges(&item_id).map_err(|e| e.to_string())
}

/// Get items that share categories with this item
/// Returns items connected via BelongsTo edges to any of the same targets
#[tauri::command]
pub fn get_related_items(state: State<AppState>, item_id: String, min_weight: Option<f64>) -> Result<Vec<Node>, String> {
    let min_w = min_weight.unwrap_or(0.3);

    // Get this item's category associations
    let associations = state.db.read().unwrap().get_belongs_to_edges(&item_id).map_err(|e| e.to_string())?;

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
            if let Ok(items) = state.db.read().unwrap().get_items_in_cluster_via_edges(cid, Some(min_w)) {
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
        if let Ok(Some(node)) = state.db.read().unwrap().get_node(&id) {
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
    state.db.read().unwrap().get_items_in_cluster_via_edges(cluster_id, min_weight).map_err(|e| e.to_string())
}

// ==================== Conversation Context Commands ====================

/// Get all messages belonging to a conversation, ordered by sequence_index
/// Traces message Leafs back to their parent conversation
#[tauri::command]
pub fn get_conversation_context(state: State<AppState>, conversation_id: String) -> Result<Vec<Node>, String> {
    state.db.read().unwrap().get_conversation_messages(&conversation_id).map_err(|e| e.to_string())
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
    let db = state.db.read().unwrap();
    import::import_claude_conversations(&db, &json_content)
}

/// Import markdown files as notes
///
/// Each .md file becomes a note under "Recent Notes" container.
/// Title is extracted from first # heading or filename.
#[tauri::command]
pub fn import_markdown_files(state: State<AppState>, file_paths: Vec<String>) -> Result<import::ImportResult, String> {
    let db = state.db.read().unwrap();
    import::import_markdown_files(&db, &file_paths)
}

// ==================== Quick Access Commands (Sidebar) ====================

/// Pin or unpin a node for quick access
#[tauri::command]
pub fn set_node_pinned(state: State<AppState>, node_id: String, pinned: bool) -> Result<(), String> {
    state.db.read().unwrap().set_node_pinned(&node_id, pinned).map_err(|e| e.to_string())
}

/// Update last accessed timestamp for a node (call when opening in Leaf)
#[tauri::command]
pub fn touch_node(state: State<AppState>, node_id: String) -> Result<(), String> {
    state.db.read().unwrap().touch_node(&node_id).map_err(|e| e.to_string())
}

/// Get all pinned nodes for Sidebar Pinned tab
#[tauri::command]
pub fn get_pinned_nodes(state: State<AppState>) -> Result<Vec<Node>, String> {
    state.db.read().unwrap().get_pinned_nodes().map_err(|e| e.to_string())
}

/// Get recently accessed nodes for Sidebar Recent tab
#[tauri::command]
pub fn get_recent_nodes(state: State<AppState>, limit: Option<i32>) -> Result<Vec<Node>, String> {
    state.db.read().unwrap().get_recent_nodes(limit.unwrap_or(15)).map_err(|e| e.to_string())
}

/// Remove a node from recents (clear last_accessed_at)
#[tauri::command]
pub fn clear_recent(state: State<AppState>, node_id: String) -> Result<(), String> {
    state.db.read().unwrap().clear_recent(&node_id).map_err(|e| e.to_string())
}

// ==================== Semantic Similarity Commands ====================

/// Similar node result with similarity score
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarNode {
    pub id: String,
    pub title: String,
    pub emoji: Option<String>,
    pub summary: Option<String>,
    pub similarity: f32,
}

/// Find nodes semantically similar to a given node
/// Uses embedding cosine similarity
#[tauri::command]
pub fn get_similar_nodes(
    state: State<AppState>,
    node_id: String,
    top_n: Option<usize>,
    min_similarity: Option<f32>,
) -> Result<Vec<SimilarNode>, String> {
    let top_n = top_n.unwrap_or(10);
    let min_similarity = min_similarity.unwrap_or(0.0);

    // Get the target node's embedding - return empty if none (e.g., category nodes)
    let target_embedding = match state.db.read().unwrap().get_node_embedding(&node_id) {
        Ok(Some(emb)) => emb,
        _ => return Ok(vec![]), // No embedding = no similar nodes, but not an error
    };

    // Get all embeddings
    let all_embeddings = state.db.read().unwrap().get_nodes_with_embeddings()
        .map_err(|e| e.to_string())?;

    if all_embeddings.is_empty() {
        return Ok(vec![]);
    }

    // Find similar nodes
    let similar = similarity::find_similar(&target_embedding, &all_embeddings, &node_id, top_n, min_similarity);

    // Fetch full node data for results
    let mut results: Vec<SimilarNode> = Vec::new();
    for (id, sim_score) in similar {
        if let Ok(Some(node)) = state.db.read().unwrap().get_node(&id) {
            results.push(SimilarNode {
                id: node.id,
                title: node.ai_title.unwrap_or(node.title),
                emoji: node.emoji,
                summary: node.summary,
                similarity: sim_score,
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
    let nodes_with_embeddings = state.db.read().unwrap().count_nodes_with_embeddings()
        .map_err(|e| e.to_string())?;
    let all_items = state.db.read().unwrap().get_items().map_err(|e| e.to_string())?;

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

// ==================== Leaf View Commands ====================

/// Get the full content of a node for Leaf view rendering
#[tauri::command]
pub fn get_leaf_content(state: State<AppState>, node_id: String) -> Result<String, String> {
    let node = state.db.read().unwrap().get_node(&node_id)
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
    let edges_deleted = state.db.read().unwrap().delete_all_edges().map_err(|e| e.to_string())?;
    let nodes_deleted = state.db.read().unwrap().delete_all_nodes().map_err(|e| e.to_string())?;
    Ok(DeleteResult { nodes_deleted, edges_deleted })
}

/// Reset AI processing flag on all items
#[tauri::command]
pub fn reset_ai_processing(state: State<AppState>) -> Result<usize, String> {
    state.db.read().unwrap().reset_ai_processing().map_err(|e| e.to_string())
}

/// Reset clustering flag on all items
#[tauri::command]
pub fn reset_clustering(state: State<AppState>) -> Result<usize, String> {
    state.db.read().unwrap().mark_all_items_need_clustering().map_err(|e| e.to_string())
}

/// Clear all embeddings
#[tauri::command]
pub fn clear_embeddings(state: State<AppState>) -> Result<usize, String> {
    // Also delete semantic edges since they depend on embeddings
    let _ = state.db.read().unwrap().delete_semantic_edges();
    state.db.read().unwrap().clear_all_embeddings().map_err(|e| e.to_string())
}

/// Clear hierarchy (delete intermediate nodes, keep items)
#[tauri::command]
pub fn clear_hierarchy(state: State<AppState>) -> Result<usize, String> {
    // Also clear parent_id on items
    let _ = state.db.read().unwrap().clear_item_parents();
    state.db.read().unwrap().delete_hierarchy_nodes().map_err(|e| e.to_string())
}

/// Delete empty nodes (items with no content/raw data)
#[tauri::command]
pub fn delete_empty_nodes(state: State<AppState>) -> Result<usize, String> {
    state.db.read().unwrap().delete_empty_items().map_err(|e| e.to_string())
}

/// Result of quick_add_to_hierarchy
#[derive(Debug, serde::Serialize)]
pub struct QuickAddResult {
    pub items_added: usize,
    pub topics_created: usize,
    pub items_skipped: usize,
    pub inbox_count: usize,  // Items in Inbox - prompt rebuild when > 20
}

/// Quickly add orphaned items to existing hierarchy without full rebuild
/// For small additions (1-10 items) - takes ~2 seconds instead of 30 minutes
/// New clusters go into "📥 Inbox" category to avoid cluttering top-level
#[tauri::command]
pub async fn quick_add_to_hierarchy(
    state: State<'_, AppState>,
) -> Result<QuickAddResult, String> {
    use crate::db::{Node, NodeType, Position};
    use std::time::{SystemTime, UNIX_EPOCH};

    const INBOX_ID: &str = "inbox-category";
    const INBOX_TITLE: &str = "📥 Inbox";

    // Get orphaned items that have been clustered but not added to hierarchy
    let orphans = state.db.read().unwrap().get_orphaned_clustered_items().map_err(|e| e.to_string())?;

    if orphans.is_empty() {
        // Still return inbox count for UI
        let inbox_count = match state.db.read().unwrap().get_node(INBOX_ID) {
            Ok(Some(inbox)) => inbox.child_count as usize,
            _ => 0,
        };
        return Ok(QuickAddResult {
            items_added: 0,
            topics_created: 0,
            items_skipped: 0,
            inbox_count,
        });
    }

    println!("[QuickAdd] Found {} orphaned items to add", orphans.len());

    let mut items_added = 0;
    let mut topics_created = 0;
    let mut items_skipped = 0;

    // Get Universe for fallback parent
    let universe = state.db.read().unwrap().get_universe()
        .map_err(|e| e.to_string())?
        .ok_or("No Universe node found - run full hierarchy build first")?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    // Get or create Inbox category for new topics
    let inbox_depth = universe.depth + 1;
    let inbox = match state.db.read().unwrap().get_node(INBOX_ID).map_err(|e| e.to_string())? {
        Some(existing) => existing,
        None => {
            // Create Inbox category
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
            };
            state.db.read().unwrap().insert_node(&inbox_node).map_err(|e| e.to_string())?;
            state.db.read().unwrap().increment_child_count(&universe.id).map_err(|e| e.to_string())?;
            println!("[QuickAdd] Created 📥 Inbox category");
            inbox_node
        }
    };

    for item in orphans {
        // Skip items without cluster_label
        let cluster_label = match &item.cluster_label {
            Some(label) => label.clone(),
            None => {
                println!("[QuickAdd] Skipping {} - no cluster_label", item.id);
                items_skipped += 1;
                continue;
            }
        };

        // Try to find existing topic with matching cluster_label
        let topic = state.db.read().unwrap().find_topic_by_cluster_label(&cluster_label)
            .map_err(|e| e.to_string())?;

        match topic {
            Some(existing_topic) => {
                // Add item to existing topic
                let item_depth = existing_topic.depth + 1;
                state.db.read().unwrap().set_node_parent(&item.id, &existing_topic.id, item_depth)
                    .map_err(|e| e.to_string())?;
                state.db.read().unwrap().increment_child_count(&existing_topic.id)
                    .map_err(|e| e.to_string())?;

                println!("[QuickAdd] Added '{}' to existing topic '{}'",
                    item.ai_title.as_ref().unwrap_or(&item.title),
                    existing_topic.title);
                items_added += 1;
            }
            None => {
                // Create new topic under Inbox (not Universe)
                let topic_id = format!("topic-quick-{}", item.cluster_id.unwrap_or(0));

                // Check if this topic already exists (might have been created in this batch)
                if let Ok(Some(_)) = state.db.read().unwrap().get_node(&topic_id) {
                    // Topic was just created, add item to it
                    let topic_depth = inbox_depth + 1;
                    let item_depth = topic_depth + 1;
                    state.db.read().unwrap().set_node_parent(&item.id, &topic_id, item_depth)
                        .map_err(|e| e.to_string())?;
                    state.db.read().unwrap().increment_child_count(&topic_id)
                        .map_err(|e| e.to_string())?;
                    items_added += 1;
                    continue;
                }

                // Create new topic node under Inbox
                let topic_depth = inbox_depth + 1;
                let topic_node = Node {
                    id: topic_id.clone(),
                    title: cluster_label.clone(),
                    node_type: NodeType::Cluster,
                    url: None,
                    content: None,
                    position: Position { x: 0.0, y: 0.0 },
                    created_at: now,
                    updated_at: now,
                    cluster_id: item.cluster_id,
                    cluster_label: Some(cluster_label.clone()),
                    depth: topic_depth,
                    is_item: false,
                    is_universe: false,
                    parent_id: Some(inbox.id.clone()),  // Under Inbox, not Universe
                    child_count: 1, // The item we're adding
                    ai_title: None,
                    summary: Some(format!("New topic: {}", cluster_label)),
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
                };

                state.db.read().unwrap().insert_node(&topic_node).map_err(|e| e.to_string())?;
                state.db.read().unwrap().increment_child_count(&inbox.id).map_err(|e| e.to_string())?;
                topics_created += 1;

                // Add item to new topic
                let item_depth = topic_depth + 1;
                state.db.read().unwrap().set_node_parent(&item.id, &topic_id, item_depth)
                    .map_err(|e| e.to_string())?;

                println!("[QuickAdd] Created new topic '{}' under Inbox for '{}'",
                    cluster_label,
                    item.ai_title.as_ref().unwrap_or(&item.title));
                items_added += 1;
            }
        }
    }

    // Get final inbox count
    let inbox_count = match state.db.read().unwrap().get_node(INBOX_ID) {
        Ok(Some(inbox)) => inbox.child_count as usize,
        _ => 0,
    };

    println!("[QuickAdd] Done: {} items added, {} topics created, {} skipped, {} in Inbox",
        items_added, topics_created, items_skipped, inbox_count);

    Ok(QuickAddResult {
        items_added,
        topics_created,
        items_skipped,
        inbox_count,
    })
}

/// Flatten empty passthrough levels in hierarchy
/// Removes single-child chains and "Uncategorized" passthrough nodes
#[tauri::command]
pub fn flatten_hierarchy(state: State<AppState>) -> Result<usize, String> {
    state.db.read().unwrap().flatten_empty_levels().map_err(|e| e.to_string())
}

/// Consolidate Universe's direct children into 4-8 uber-categories
/// Creates new depth-1 nodes with single-word ALL-CAPS names (TECH, LIFE, MIND, etc.)
#[tauri::command]
pub async fn consolidate_root(state: State<'_, AppState>) -> Result<ConsolidateResult, String> {
    use crate::ai_client::{self, TopicInfo};
    use crate::db::{Node, NodeType, Position};

    // Get Universe
    let universe = state.db.read().unwrap().get_universe()
        .map_err(|e| e.to_string())?
        .ok_or("No Universe node found")?;

    // Get Universe's direct children (excluding protected)
    let all_children = state.db.read().unwrap().get_children(&universe.id).map_err(|e| e.to_string())?;
    let protected_ids = state.db.read().unwrap().get_protected_node_ids();
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

    // Call AI to group into uber-categories
    let groupings = ai_client::group_into_uber_categories(&categories).await?;

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
        };

        state.db.read().unwrap().insert_node(&uber_node).map_err(|e| e.to_string())?;
        uber_categories_created += 1;

        // Reparent children to this uber-category
        for child in &matching_children {
            state.db.read().unwrap().update_parent(&child.id, &uber_id).map_err(|e| e.to_string())?;
            all_children_to_update.push(child.id.clone());
            children_reparented += 1;
        }

        println!("[Consolidate] Created '{}' with {} children", grouping.name, matching_children.len());
    }

    // Batch update depths
    if !all_children_to_update.is_empty() {
        println!("[Consolidate] Updating depths for {} reparented nodes...", all_children_to_update.len());
        state.db.read().unwrap().increment_multiple_subtrees_depth(&all_children_to_update).map_err(|e| e.to_string())?;
    }

    // Update Universe's child count (new uber-categories + children that stayed at top level)
    let children_stayed_at_top = children.len() - children_reparented;
    let new_universe_child_count = uber_categories_created + children_stayed_at_top;
    state.db.read().unwrap().update_child_count(&universe.id, new_universe_child_count as i32)
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
    let db = state.db.read().unwrap();

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
        state.db.read().unwrap().get_stats().map_err(|e| e.to_string())?;
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
    Ok(state.db.read().unwrap().get_path())
}

/// Switch to a different database file (hot-swap without restart)
#[tauri::command]
pub fn switch_database(_app: AppHandle, state: State<AppState>, db_path: String) -> Result<DbStats, String> {
    use std::path::PathBuf;
    use crate::db::Database;
    use crate::hierarchy;
    use crate::settings;

    let path = PathBuf::from(&db_path);
    if !path.exists() {
        return Err(format!("Database file not found: {}", db_path));
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
        let mut db_guard = state.db.write().unwrap();
        *db_guard = Arc::new(new_db);
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
