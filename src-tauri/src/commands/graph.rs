use crate::db::{Database, Node, Edge};
use crate::clustering::{self, ClusteringResult as ClusterResult};
use crate::ai_client;
use crate::hierarchy;
use crate::import;
use crate::similarity;
use crate::settings;
use tauri::{State, AppHandle, Emitter};
use std::sync::Arc;
use serde::Serialize;

pub struct AppState {
    pub db: Arc<Database>,
}

#[tauri::command]
pub fn get_nodes(state: State<AppState>) -> Result<Vec<Node>, String> {
    state.db.get_all_nodes().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_node(state: State<AppState>, id: String) -> Result<Option<Node>, String> {
    state.db.get_node(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_node(state: State<AppState>, node: Node) -> Result<(), String> {
    state.db.insert_node(&node).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_node(state: State<AppState>, node: Node) -> Result<(), String> {
    state.db.update_node(&node).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_node(state: State<AppState>, id: String) -> Result<(), String> {
    state.db.delete_node(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_edges(state: State<AppState>) -> Result<Vec<Edge>, String> {
    state.db.get_all_edges().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_edges_for_node(state: State<AppState>, node_id: String) -> Result<Vec<Edge>, String> {
    state.db.get_edges_for_node(&node_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_edge(state: State<AppState>, edge: Edge) -> Result<(), String> {
    state.db.insert_edge(&edge).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_edge(state: State<AppState>, id: String) -> Result<(), String> {
    state.db.delete_edge(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn search_nodes(state: State<AppState>, query: String) -> Result<Vec<Node>, String> {
    state.db.search_nodes(&query).map_err(|e| e.to_string())
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
    let needs_clustering = state.db.count_items_needing_clustering().map_err(|e| e.to_string())?;
    let all_items = state.db.get_items().map_err(|e| e.to_string())?;

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
    clustering::run_clustering(&state.db, use_ai).await
}

/// Force re-clustering of all items
#[tauri::command]
pub async fn recluster_all(state: State<'_, AppState>, use_ai: Option<bool>) -> Result<ClusterResult, String> {
    let use_ai = use_ai.unwrap_or(true);
    clustering::recluster_all(&state.db, use_ai).await
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
    let all_nodes = state.db.get_all_nodes().map_err(|e| e.to_string())?;
    let unprocessed = state.db.get_unprocessed_nodes().map_err(|e| e.to_string())?;

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
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HierarchyLogEvent {
    pub message: String,
    pub level: String, // "info", "debug", "warn", "error"
}

#[tauri::command]
pub async fn process_nodes(app: AppHandle, state: State<'_, AppState>) -> Result<ProcessingResult, String> {
    println!("process_nodes called, checking API key availability...");

    if !ai_client::is_available() {
        println!("API key not available!");
        return Err("ANTHROPIC_API_KEY not set".to_string());
    }

    println!("API key is available, fetching unprocessed nodes...");
    let unprocessed = state.db.get_unprocessed_nodes().map_err(|e| e.to_string())?;
    let total = unprocessed.len();
    println!("Found {} unprocessed nodes", total);

    let mut processed = 0;
    let mut failed = 0;
    let mut errors = Vec::new();
    let mut current = 0;

    for node in unprocessed {
        current += 1;
        let content = node.content.as_deref().unwrap_or("");

        // Skip nodes with no content
        if content.is_empty() {
            continue;
        }

        // Emit processing event
        let _ = app.emit("ai-progress", AiProgressEvent {
            current,
            total,
            node_title: node.title.clone(),
            new_title: String::new(),
            emoji: None,
            status: "processing".to_string(),
            error_message: None,
        });

        match ai_client::analyze_node(&node.title, content).await {
            Ok(result) => {
                let tags_json = serde_json::to_string(&result.tags).unwrap_or_default();

                if let Err(e) = state.db.update_node_ai(
                    &node.id,
                    &result.title,
                    &result.summary,
                    &tags_json,
                    &result.emoji,
                ) {
                    let err_msg = format!("DB save failed: {}", e);
                    errors.push(format!("Failed to save node {}: {}", node.id, e));
                    failed += 1;
                    let _ = app.emit("ai-progress", AiProgressEvent {
                        current,
                        total,
                        node_title: node.title.clone(),
                        new_title: String::new(),
                        emoji: None,
                        status: "error".to_string(),
                        error_message: Some(err_msg),
                    });
                } else {
                    processed += 1;
                    println!("Processed node: {} -> {} {}", node.title, result.emoji, result.title);

                    // Generate embedding if OpenAI API key is available
                    let embed_text = format!("{} {}", result.title, result.summary);
                    match ai_client::generate_embedding(&embed_text).await {
                        Ok(embedding) => {
                            if let Err(e) = state.db.update_node_embedding(&node.id, &embedding) {
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

                    let _ = app.emit("ai-progress", AiProgressEvent {
                        current,
                        total,
                        node_title: node.title.clone(),
                        new_title: result.title,
                        emoji: Some(result.emoji),
                        status: "success".to_string(),
                        error_message: None,
                    });
                }
            }
            Err(e) => {
                let err_msg = e.clone();
                errors.push(format!("AI error for node {}: {}", node.id, e));
                failed += 1;
                let _ = app.emit("ai-progress", AiProgressEvent {
                    current,
                    total,
                    node_title: node.title.clone(),
                    new_title: String::new(),
                    emoji: None,
                    status: "error".to_string(),
                    error_message: Some(err_msg),
                });
            }
        }
    }

    // Emit completion event
    let _ = app.emit("ai-progress", AiProgressEvent {
        current: total,
        total,
        node_title: String::new(),
        new_title: String::new(),
        emoji: None,
        status: "complete".to_string(),
        error_message: None,
    });

    Ok(ProcessingResult {
        processed,
        failed,
        errors,
    })
}

#[tauri::command]
pub fn get_learned_emojis(state: State<AppState>) -> Result<std::collections::HashMap<String, String>, String> {
    state.db.get_learned_emojis().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_learned_emoji(state: State<AppState>, keyword: String, emoji: String) -> Result<(), String> {
    state.db.save_learned_emoji(&keyword, &emoji).map_err(|e| e.to_string())
}

// ==================== Hierarchy Navigation Commands ====================

/// Get nodes at a specific depth (0=Universe, increases toward items)
#[tauri::command]
pub fn get_nodes_at_depth(state: State<AppState>, depth: i32) -> Result<Vec<Node>, String> {
    state.db.get_nodes_at_depth(depth).map_err(|e| e.to_string())
}

/// Get children of a specific parent node
#[tauri::command]
pub fn get_children(state: State<AppState>, parent_id: String) -> Result<Vec<Node>, String> {
    state.db.get_children(&parent_id).map_err(|e| e.to_string())
}

/// Get the Universe node (single root, is_universe = true)
#[tauri::command]
pub fn get_universe(state: State<AppState>) -> Result<Option<Node>, String> {
    state.db.get_universe().map_err(|e| e.to_string())
}

/// Get all items (is_item = true) - openable content
#[tauri::command]
pub fn get_items(state: State<AppState>) -> Result<Vec<Node>, String> {
    state.db.get_items().map_err(|e| e.to_string())
}

/// Get the maximum depth in the hierarchy
#[tauri::command]
pub fn get_max_depth(state: State<AppState>) -> Result<i32, String> {
    state.db.get_max_depth().map_err(|e| e.to_string())
}

// ==================== Hierarchy Generation Commands ====================

/// Build the full hierarchy from items
/// Creates intermediate grouping nodes based on collection size
#[tauri::command]
pub fn build_hierarchy(state: State<'_, AppState>) -> Result<hierarchy::HierarchyResult, String> {
    hierarchy::build_hierarchy(&state.db)
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
    hierarchy::build_full_hierarchy(&state.db, should_cluster, Some(&app)).await
}

/// Cluster children of a specific parent node into 8-15 groups using AI
///
/// Returns true if grouping was performed, false if already has ≤15 children.
/// Use this for manual/targeted hierarchy adjustment.
#[tauri::command]
pub async fn cluster_hierarchy_level(
    app: AppHandle,
    state: State<'_, AppState>,
    parent_id: String,
) -> Result<bool, String> {
    hierarchy::cluster_hierarchy_level(&state.db, &parent_id, Some(&app)).await
}

/// Get children of a node, automatically skipping single-child chains
///
/// Useful for navigation - if a level has exactly 1 child, skip to that child's children.
#[tauri::command]
pub fn get_children_flat(state: State<AppState>, parent_id: String) -> Result<Vec<Node>, String> {
    hierarchy::get_children_skip_single_chain(&state.db, &parent_id)
}

// ==================== Multi-Path Association Commands ====================

/// Get all category associations for an item (via BelongsTo edges)
/// Returns edges sorted by weight (highest first)
#[tauri::command]
pub fn get_item_associations(state: State<AppState>, item_id: String) -> Result<Vec<Edge>, String> {
    state.db.get_belongs_to_edges(&item_id).map_err(|e| e.to_string())
}

/// Get items that share categories with this item
/// Returns items connected via BelongsTo edges to any of the same targets
#[tauri::command]
pub fn get_related_items(state: State<AppState>, item_id: String, min_weight: Option<f64>) -> Result<Vec<Node>, String> {
    let min_w = min_weight.unwrap_or(0.3);

    // Get this item's category associations
    let associations = state.db.get_belongs_to_edges(&item_id).map_err(|e| e.to_string())?;

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
            if let Ok(items) = state.db.get_items_in_cluster_via_edges(cid, Some(min_w)) {
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
        if let Ok(Some(node)) = state.db.get_node(&id) {
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
    state.db.get_items_in_cluster_via_edges(cluster_id, min_weight).map_err(|e| e.to_string())
}

// ==================== Conversation Context Commands ====================

/// Get all messages belonging to a conversation, ordered by sequence_index
/// Traces message Leafs back to their parent conversation
#[tauri::command]
pub fn get_conversation_context(state: State<AppState>, conversation_id: String) -> Result<Vec<Node>, String> {
    state.db.get_conversation_messages(&conversation_id).map_err(|e| e.to_string())
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
    import::import_claude_conversations(&state.db, &json_content)
}

// ==================== Quick Access Commands (Sidebar) ====================

/// Pin or unpin a node for quick access
#[tauri::command]
pub fn set_node_pinned(state: State<AppState>, node_id: String, pinned: bool) -> Result<(), String> {
    state.db.set_node_pinned(&node_id, pinned).map_err(|e| e.to_string())
}

/// Update last accessed timestamp for a node (call when opening in Leaf)
#[tauri::command]
pub fn touch_node(state: State<AppState>, node_id: String) -> Result<(), String> {
    state.db.touch_node(&node_id).map_err(|e| e.to_string())
}

/// Get all pinned nodes for Sidebar Pinned tab
#[tauri::command]
pub fn get_pinned_nodes(state: State<AppState>) -> Result<Vec<Node>, String> {
    state.db.get_pinned_nodes().map_err(|e| e.to_string())
}

/// Get recently accessed nodes for Sidebar Recent tab
#[tauri::command]
pub fn get_recent_nodes(state: State<AppState>, limit: Option<i32>) -> Result<Vec<Node>, String> {
    state.db.get_recent_nodes(limit.unwrap_or(15)).map_err(|e| e.to_string())
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
) -> Result<Vec<SimilarNode>, String> {
    let top_n = top_n.unwrap_or(10);

    // Get the target node's embedding
    let target_embedding = state.db.get_node_embedding(&node_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Node {} has no embedding", node_id))?;

    // Get all embeddings
    let all_embeddings = state.db.get_nodes_with_embeddings()
        .map_err(|e| e.to_string())?;

    if all_embeddings.is_empty() {
        return Ok(vec![]);
    }

    // Find similar nodes
    let similar = similarity::find_similar(&target_embedding, &all_embeddings, &node_id, top_n);

    // Fetch full node data for results
    let mut results: Vec<SimilarNode> = Vec::new();
    for (id, sim_score) in similar {
        if let Ok(Some(node)) = state.db.get_node(&id) {
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
    let nodes_with_embeddings = state.db.count_nodes_with_embeddings()
        .map_err(|e| e.to_string())?;
    let all_items = state.db.get_items().map_err(|e| e.to_string())?;

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
