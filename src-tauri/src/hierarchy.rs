//! Dynamic hierarchy generation for the knowledge graph
//!
//! NEW: Recursive hierarchy building with AI-powered topic grouping
//!
//! The hierarchy builder now works in two phases:
//! 1. Bottom-up: Items are clustered into fine-grained topics (via clustering.rs)
//! 2. Top-down: Topics are recursively grouped into parent categories until
//!    each level has 8-15 children (manageable for navigation)
//!
//! Key insight: Start with natural clusters, then organize them into a
//! navigable tree. Both directions meeting in the middle.

use crate::db::{Database, Node, NodeType, Position};
use crate::ai_client::{self, TopicInfo};
use crate::settings;
use crate::commands::is_rebuild_cancelled;
use serde::Serialize;
use std::collections::HashMap;
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::time::{timeout, Duration};

/// Log event for frontend dev console
#[derive(Clone, Serialize)]
pub struct HierarchyLogEvent {
    pub message: String,
    pub level: String,
}

/// Progress event for AI operations (reuses the same format as process_nodes)
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
    pub elapsed_secs: Option<f64>,
    pub estimate_secs: Option<f64>,
    pub remaining_secs: Option<f64>,
}

/// Emit progress event
fn emit_progress(app: Option<&AppHandle>, event: AiProgressEvent) {
    if let Some(app) = app {
        let _ = app.emit("ai-progress", event);
    }
}

/// Emit a log to the frontend dev console (if app handle available)
fn emit_log(app: Option<&AppHandle>, level: &str, message: &str) {
    // Always print to terminal
    match level {
        "error" => eprintln!("[Hierarchy] {}", message),
        "warn" => eprintln!("[Hierarchy] {}", message),
        _ => println!("[Hierarchy] {}", message),
    }

    // Also emit to frontend if app handle is available
    if let Some(app) = app {
        let _ = app.emit("hierarchy-log", HierarchyLogEvent {
            message: message.to_string(),
            level: level.to_string(),
        });
    }
}

/// Compute topic centroids from their child items' embeddings
///
/// During hierarchy grouping (Step 3), topic nodes don't have embeddings yet
/// (those are generated in Step 4). But their child items DO have embeddings
/// from the Process AI step. This function computes topic "centroids" by
/// averaging the embeddings of each topic's child items.
fn compute_topic_centroids_from_items(
    db: &Database,
    topic_nodes: &[Node],
) -> Vec<(String, Vec<f32>)> {
    let mut centroids: Vec<(String, Vec<f32>)> = Vec::new();

    for topic in topic_nodes {
        // Get children of this topic (items)
        let children = match db.get_children(&topic.id) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if children.is_empty() {
            continue;
        }

        // Collect embeddings from children
        let mut embeddings: Vec<Vec<f32>> = Vec::new();
        for child in &children {
            if let Ok(Some(emb)) = db.get_node_embedding(&child.id) {
                embeddings.push(emb);
            }
        }

        if embeddings.is_empty() {
            continue;
        }

        // Compute centroid (element-wise average)
        let dim = embeddings[0].len();
        let mut centroid = vec![0.0f32; dim];
        for emb in &embeddings {
            for (i, val) in emb.iter().enumerate() {
                if i < dim {
                    centroid[i] += val;
                }
            }
        }
        let count = embeddings.len() as f32;
        for val in &mut centroid {
            *val /= count;
        }

        centroids.push((topic.id.clone(), centroid));
    }

    centroids
}

/// Maximum children per node before we need to group them
const MAX_CHILDREN_PER_LEVEL: usize = 15;

/// Result of hierarchy generation
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HierarchyResult {
    pub levels_created: usize,
    pub intermediate_nodes_created: usize,
    pub items_organized: usize,
    pub max_depth: i32,
}

/// Calculate how many levels are needed based on cluster count
///
/// IMPORTANT: This now returns a FLAT structure (Universe → Topics → Items)
/// regardless of cluster count. The recursive AI grouping in build_full_hierarchy
/// will add intermediate levels as needed to ensure 8-15 children per level.
///
/// This prevents the "Uncategorized wrapper" problem where intermediate nodes
/// without cluster_ids all collapse into one group.
fn calculate_levels_needed(item_count: usize, cluster_count: usize) -> usize {
    if item_count == 0 {
        return 1; // Just Universe (empty state)
    }
    if cluster_count <= 1 {
        return 1; // Universe -> Items (no intermediate levels needed)
    }
    // Always create flat structure: Universe -> Topics -> Items
    // Recursive AI grouping will add levels as needed
    2
}

/// Get semantic name for a level based on its position
fn level_name(depth: i32, max_depth: i32) -> &'static str {
    // Universe is always depth 0
    // Items are always at max_depth
    // In between are dynamic groupings
    if depth == 0 {
        return "Universe";
    }

    let distance_from_items = max_depth - depth;
    match distance_from_items {
        0 => "Item",       // This shouldn't happen - items aren't created here
        1 => "Topic",      // One level above items
        2 => "Domain",     // Two levels above
        3 => "Galaxy",     // Three levels above
        _ => "Region",     // Deeper hierarchies
    }
}

/// Get emoji for a level
fn level_emoji(depth: i32, max_depth: i32) -> &'static str {
    if depth == 0 {
        return "🌌"; // Universe
    }

    let distance_from_items = max_depth - depth;
    match distance_from_items {
        1 => "🗂️",  // Topic
        2 => "🌍",  // Domain
        3 => "🌀",  // Galaxy
        _ => "📁",  // Generic folder
    }
}

/// Build hierarchy: dynamic levels based on item count
///
/// Flow:
/// 1. Get all items (is_item = true)
/// 2. Calculate levels needed based on count
/// 3. Cluster items if needed (requires cluster_id from clustering)
/// 4. Build parent levels bottom-up
/// 5. Create Universe root
pub fn build_hierarchy(db: &Database) -> Result<HierarchyResult, String> {
    // Step 1: Clean up old hierarchy completely
    cleanup_hierarchy(db)?;

    // Step 2: Get all items
    let items = db.get_items().map_err(|e| e.to_string())?;
    let item_count = items.len();

    println!("Building hierarchy for {} items", item_count);

    if item_count == 0 {
        // Empty collection - just create Universe
        let universe_id = create_universe(db, &[])?;
        println!("Created empty Universe: {}", universe_id);
        return Ok(HierarchyResult {
            levels_created: 1,
            intermediate_nodes_created: 1,
            items_organized: 0,
            max_depth: 0,
        });
    }

    // Step 3: Group items by cluster_id to create topics
    let mut clusters: HashMap<i32, Vec<&Node>> = HashMap::new();
    let mut unclustered: Vec<&Node> = Vec::new();

    for item in &items {
        if let Some(cluster_id) = item.cluster_id {
            clusters.entry(cluster_id).or_default().push(item);
        } else {
            unclustered.push(item);
        }
    }

    // Add unclustered items to their own group if any
    if !unclustered.is_empty() {
        clusters.insert(-1, unclustered);
    }

    let cluster_count = clusters.len();
    println!("Found {} clusters from {} items", cluster_count, item_count);

    // Structure: Universe (depth 0) -> Topics (depth 1) -> Items (depth 2)
    let item_depth = 2;
    let topic_depth = 1;

    // Step 4: Set all items to depth 2
    for item in &items {
        db.update_node_hierarchy(&item.id, None, item_depth)
            .map_err(|e| e.to_string())?;
    }

    // Step 5: Create topic nodes (one per cluster)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let mut topic_ids: Vec<String> = Vec::new();
    let mut topics_created = 0;

    for (cluster_id, cluster_items) in &clusters {
        // Get cluster label from first item with one
        let cluster_label = cluster_items.iter()
            .find_map(|item| item.cluster_label.clone())
            .unwrap_or_else(|| {
                if *cluster_id == -1 {
                    "Uncategorized".to_string()
                } else {
                    format!("Topic {}", cluster_id)
                }
            });

        let topic_id = format!("topic-{}", cluster_id);

        // Generate summary from first few children's titles
        let child_titles: Vec<String> = cluster_items.iter()
            .take(3)
            .map(|n| n.ai_title.clone().unwrap_or_else(|| n.title.clone()))
            .collect();
        let topic_summary = if child_titles.is_empty() {
            format!("Collection of {} related items", cluster_items.len())
        } else {
            format!("Including {}", child_titles.join(", "))
        };

        let topic_node = Node {
            id: topic_id.clone(),
            node_type: NodeType::Cluster,
            title: cluster_label.clone(),
            url: None,
            content: None,
            parent_id: None,  // Will be set to Universe after creation
            cluster_id: Some(*cluster_id),
            cluster_label: Some(cluster_label.clone()),
            depth: topic_depth,
            is_item: false,
            is_universe: false,
            child_count: cluster_items.len() as i32,
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            ai_title: None,
            summary: Some(topic_summary),
            tags: None,
            emoji: None,
            is_processed: false,
            conversation_id: None,
            sequence_index: None,
            is_pinned: false,
            last_accessed_at: None,
        };

        db.insert_node(&topic_node).map_err(|e| e.to_string())?;
        topic_ids.push(topic_id.clone());
        topics_created += 1;

        // Assign items to this topic
        for item in cluster_items {
            db.update_node_hierarchy(&item.id, Some(&topic_id), item_depth)
                .map_err(|e| e.to_string())?;
        }
    }

    println!("Created {} topic nodes", topics_created);

    // Step 6: Create Universe (depth 0) and attach topics to it
    let universe_id = create_universe(db, &topic_ids)?;

    // Update topics to point to Universe
    for topic_id in &topic_ids {
        db.update_node_hierarchy(topic_id, Some(&universe_id), topic_depth)
            .map_err(|e| e.to_string())?;
    }

    // Update child count on Universe
    db.update_child_count(&universe_id, topic_ids.len() as i32)
        .map_err(|e| e.to_string())?;

    println!("Hierarchy complete: Universe -> {} topics -> {} items",
             topics_created, item_count);

    Ok(HierarchyResult {
        levels_created: 3,  // Universe, Topics, Items
        intermediate_nodes_created: topics_created + 1,  // topics + universe
        items_organized: item_count,
        max_depth: item_depth,
    })
}

/// Clean up existing hierarchy (delete intermediate nodes, clear item parents)
fn cleanup_hierarchy(db: &Database) -> Result<(), String> {
    // Delete all intermediate nodes (non-items, non-universe)
    let deleted = db.delete_hierarchy_nodes().map_err(|e| e.to_string())?;
    println!("Deleted {} old hierarchy nodes", deleted);

    // Delete any existing universe
    if let Ok(Some(universe)) = db.get_universe() {
        db.delete_node(&universe.id).map_err(|e| e.to_string())?;
        println!("Deleted old Universe node");
    }

    // Clear parent_id on all items
    db.clear_item_parents().map_err(|e| e.to_string())?;
    println!("Cleared parent references on items");

    Ok(())
}

/// Create parent nodes from clustered children
/// Groups children by cluster_id and creates one parent per cluster
/// `child_depth` is the actual depth of children (passed from caller since Node structs may be stale)
fn create_parent_level(
    db: &Database,
    children: &[Node],
    parent_depth: i32,
    child_depth: i32,
    max_depth: i32,
) -> Result<(Vec<Node>, usize), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    // Group children by cluster_id
    let mut clusters: HashMap<i32, Vec<&Node>> = HashMap::new();
    let mut unclustered: Vec<&Node> = Vec::new();

    for node in children {
        if let Some(cluster_id) = node.cluster_id {
            clusters.entry(cluster_id).or_default().push(node);
        } else {
            unclustered.push(node);
        }
    }

    println!("  {} clusters, {} unclustered nodes", clusters.len(), unclustered.len());

    // If no clusters, put everything under one "Uncategorized" parent
    if clusters.is_empty() {
        if unclustered.is_empty() {
            return Ok((Vec::new(), 0));
        }
        clusters.insert(-1, unclustered);
    } else if !unclustered.is_empty() {
        // Add unclustered to their own group
        clusters.insert(-1, unclustered);
    }

    // Note: Redundant 1:1 levels are now prevented at the source:
    // - calculate_levels_needed uses cluster_count to determine structure
    // - Intermediate nodes have cluster_id = None, so they collapse into one group at parent level

    let level_prefix = match level_name(parent_depth, max_depth) {
        "Topic" => "topic",
        "Domain" => "domain",
        "Galaxy" => "galaxy",
        "Region" => "region",
        _ => "group",
    };

    let mut parent_nodes = Vec::new();
    let mut total_assigned = 0;

    for (cluster_id, nodes) in clusters {
        let parent_id = format!("{}-{}", level_prefix, cluster_id);

        // Get cluster label from first node, or use default
        let cluster_label = if cluster_id == -1 {
            "Uncategorized".to_string()
        } else {
            nodes.first()
                .and_then(|n| n.cluster_label.clone())
                .unwrap_or_else(|| format!("Group {}", cluster_id))
        };

        // Calculate centroid position
        let (sum_x, sum_y) = nodes.iter().fold((0.0, 0.0), |(x, y), n| {
            (x + n.position.x, y + n.position.y)
        });
        let centroid = Position {
            x: sum_x / nodes.len() as f64,
            y: sum_y / nodes.len() as f64,
        };

        // Create parent node
        // IMPORTANT: cluster_id is NOT set on intermediate nodes
        // This prevents the next level up from inheriting the same grouping

        // Generate summary from first few children's titles/summaries
        let child_summaries: Vec<String> = nodes.iter()
            .take(3)
            .filter_map(|n| n.ai_title.clone().or_else(|| Some(n.title.clone())))
            .collect();
        let topic_summary = if child_summaries.is_empty() {
            format!("Collection of {} related items", nodes.len())
        } else {
            format!("Topics including {}", child_summaries.join(", "))
        };

        let parent_node = Node {
            id: parent_id.clone(),
            node_type: NodeType::Cluster,
            title: cluster_label.clone(),
            url: None,
            content: Some(format!("{} items", nodes.len())),
            position: centroid,
            created_at: now,
            updated_at: now,
            cluster_id: None, // Don't inherit - prevents 1:1 parent creation at next level
            cluster_label: Some(cluster_label),
            depth: parent_depth,
            is_item: false,
            is_universe: false,
            parent_id: None, // Will be set by next level up
            child_count: nodes.len() as i32,
            ai_title: None,
            summary: Some(topic_summary),
            tags: None,
            emoji: None,  // Let frontend keyword matcher assign meaningful emoji
            is_processed: false,
            conversation_id: None,  // Cluster nodes don't belong to conversations
            sequence_index: None,
            is_pinned: false,
            last_accessed_at: None,
        };

        db.insert_node(&parent_node).map_err(|e| e.to_string())?;

        // Update children to point to this parent (use child_depth, not stale node.depth)
        for node in &nodes {
            db.update_node_hierarchy(&node.id, Some(&parent_id), child_depth)
                .map_err(|e| e.to_string())?;
            total_assigned += 1;
        }

        // Update child count
        db.update_child_count(&parent_id, nodes.len() as i32)
            .map_err(|e| e.to_string())?;

        parent_nodes.push(parent_node);
    }

    Ok((parent_nodes, total_assigned))
}

/// Create the Universe root node (depth 0, is_universe = true)
fn create_universe(db: &Database, child_ids: &[String]) -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let universe_id = "universe-root".to_string();

    let universe_node = Node {
        id: universe_id.clone(),
        node_type: NodeType::Cluster,
        title: "All Knowledge".to_string(),
        url: None,
        content: Some("The root of everything".to_string()),
        position: Position { x: 0.0, y: 0.0 },
        created_at: now,
        updated_at: now,
        cluster_id: None,
        cluster_label: Some("Universe".to_string()),
        depth: 0,
        is_item: false,
        is_universe: true,
        parent_id: None, // Root has no parent
        child_count: child_ids.len() as i32,
        ai_title: None,
        summary: None,
        tags: None,
        emoji: Some("🌌".to_string()),
        is_processed: false,
        conversation_id: None,  // Universe doesn't belong to a conversation
        sequence_index: None,
        is_pinned: false,
        last_accessed_at: None,
    };

    db.insert_node(&universe_node).map_err(|e| e.to_string())?;

    println!("Created Universe root with {} children", child_ids.len());

    Ok(universe_id)
}

// ==================== Recursive Hierarchy Building ====================

/// Result of full hierarchy building (clustering + recursive grouping)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FullHierarchyResult {
    pub clustering_result: Option<crate::clustering::ClusteringResult>,
    pub hierarchy_result: HierarchyResult,
    pub levels_created: usize,
    pub grouping_iterations: usize,
    pub embeddings_generated: usize,
    pub embeddings_skipped: usize,
}

/// Maximum children to send in a single AI grouping request
/// Larger batches risk JSON truncation in AI response
const BATCH_SIZE_FOR_GROUPING: usize = 150;

/// Threshold above which we split into batches
const BATCH_THRESHOLD: usize = 200;

/// Process topics in batches and merge results
///
/// For large datasets (>200 topics), this:
/// 1. Splits topics into chunks of 150
/// 2. Calls AI grouping for each chunk
/// 3. Merges similar categories across batches (fuzzy name matching)
/// 4. Returns unified groupings
async fn group_topics_in_batches(
    topics: &[TopicInfo],
    context: &ai_client::GroupingContext,
    app: Option<&AppHandle>,
) -> Result<Vec<ai_client::CategoryGrouping>, String> {
    let batch_count = (topics.len() + BATCH_SIZE_FOR_GROUPING - 1) / BATCH_SIZE_FOR_GROUPING;
    emit_log(app, "info", &format!("Splitting {} topics into {} batches of ~{}",
             topics.len(), batch_count, BATCH_SIZE_FOR_GROUPING));

    let mut all_groupings: Vec<ai_client::CategoryGrouping> = Vec::new();

    for (batch_idx, batch) in topics.chunks(BATCH_SIZE_FOR_GROUPING).enumerate() {
        emit_log(app, "info", &format!("  Processing batch {}/{} ({} topics)",
                 batch_idx + 1, batch_count, batch.len()));

        // For subsequent batches, include existing category names as hints
        let mut batch_context = context.clone();
        if !all_groupings.is_empty() {
            let existing_names: Vec<String> = all_groupings.iter()
                .map(|g| g.name.clone())
                .collect();
            batch_context.sibling_names.extend(existing_names);
        }

        match timeout(
            Duration::from_secs(120),
            ai_client::group_topics_into_categories(batch, &batch_context)
        ).await {
            Ok(Ok(batch_groupings)) => {
                emit_log(app, "info", &format!("    Batch {} returned {} categories",
                         batch_idx + 1, batch_groupings.len()));

                // Merge with existing groupings
                for new_grouping in batch_groupings {
                    // Try to find existing category with similar name
                    if let Some(existing) = find_similar_category(&mut all_groupings, &new_grouping.name) {
                        // Merge children into existing category
                        existing.children.extend(new_grouping.children);
                        emit_log(app, "debug", &format!("    Merged '{}' into existing '{}'",
                                 new_grouping.name, existing.name));
                    } else {
                        // Add as new category
                        all_groupings.push(new_grouping);
                    }
                }
            }
            Ok(Err(e)) => {
                emit_log(app, "error", &format!("    Batch {} failed: {}", batch_idx + 1, e));
                // Continue with other batches rather than failing entirely
            }
            Err(_) => {
                emit_log(app, "error", &format!("    Batch {} timed out after 120s", batch_idx + 1));
                // Continue with other batches rather than failing entirely
            }
        }
    }

    emit_log(app, "info", &format!("Batch processing complete: {} total categories", all_groupings.len()));
    Ok(all_groupings)
}

/// Find a category with a similar name (case-insensitive, handles minor variations)
fn find_similar_category<'a>(
    categories: &'a mut [ai_client::CategoryGrouping],
    name: &str,
) -> Option<&'a mut ai_client::CategoryGrouping> {
    let name_lower = name.to_lowercase();
    let name_normalized = normalize_category_name(&name_lower);

    for cat in categories.iter_mut() {
        let cat_lower = cat.name.to_lowercase();
        let cat_normalized = normalize_category_name(&cat_lower);

        // Exact match (case-insensitive)
        if cat_lower == name_lower {
            return Some(cat);
        }

        // Normalized match (ignores minor word differences)
        if cat_normalized == name_normalized {
            return Some(cat);
        }

        // One contains the other (for cases like "Programming" vs "Programming & Development")
        if cat_lower.contains(&name_lower) || name_lower.contains(&cat_lower) {
            // Only merge if they share significant overlap (at least 60% of shorter name)
            let shorter = cat_lower.len().min(name_lower.len());
            let common = common_prefix_len(&cat_lower, &name_lower);
            if common as f64 / shorter as f64 > 0.6 {
                return Some(cat);
            }
        }
    }

    None
}

/// Normalize a category name for comparison
/// Removes common filler words and punctuation
fn normalize_category_name(name: &str) -> String {
    let stopwords = ["and", "the", "of", "&", "-", "related", "topics", "items"];
    name.split_whitespace()
        .filter(|word| !stopwords.contains(&word.to_lowercase().as_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Calculate length of common prefix between two strings
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars()
        .zip(b.chars())
        .take_while(|(ac, bc)| ac == bc)
        .count()
}

/// Cluster children of a specific parent into 8-15 groups using AI
///
/// If parent has <= MAX_CHILDREN_PER_LEVEL children, returns Ok(false) - no grouping needed.
/// Otherwise, creates new intermediate nodes and reparents children.
///
/// For large datasets (>200 children), splits into batches of 150, calls AI for each,
/// then merges similar categories across batches to prevent fragmentation.
pub async fn cluster_hierarchy_level(db: &Database, parent_id: &str, app: Option<&AppHandle>) -> Result<bool, String> {
    // Get children of this parent
    let children = db.get_children(parent_id).map_err(|e| e.to_string())?;

    if children.len() <= MAX_CHILDREN_PER_LEVEL {
        emit_log(app, "info", &format!("Parent {} has {} children (≤{}), no grouping needed",
                 parent_id, children.len(), MAX_CHILDREN_PER_LEVEL));
        return Ok(false);
    }

    emit_log(app, "info", &format!("Grouping {} children of {} into 8-15 categories", children.len(), parent_id));

    // === Gather hierarchy context for AI ===

    // 1. Get parent node info
    let parent_node = db.get_node(parent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Parent node {} not found", parent_id))?;

    // 2. Build hierarchy path (walk up to Universe)
    let hierarchy_path = build_hierarchy_path(db, parent_id)?;

    // 3. Get sibling names (other children of grandparent)
    let sibling_names = if let Some(ref grandparent_id) = parent_node.parent_id {
        db.get_children(grandparent_id)
            .map_err(|e| e.to_string())?
            .iter()
            .filter(|n| n.id != parent_id)
            .map(|n| n.cluster_label.clone().unwrap_or_else(|| n.title.clone()))
            .collect()
    } else {
        vec![]
    };

    // 4. Collect all existing category names (forbidden for reuse)
    let forbidden_names = db.get_all_category_names().map_err(|e| e.to_string())?;

    // Build topic info for AI
    let topics: Vec<TopicInfo> = children
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

    // 5. Detect embedding-based project clusters using topic centroids
    // Topic nodes don't have embeddings yet (generated in Step 4), but their
    // child items do. Compute centroids from item embeddings.
    let topic_centroids = compute_topic_centroids_from_items(db, &children);

    let mandatory_clusters = if topic_centroids.len() >= 4 {
        emit_log(app, "info", &format!("Detecting project clusters from {} topics (centroids from child items)", topic_centroids.len()));
        let clusters = ai_client::detect_project_clusters_from_embeddings(db, &topics, &topic_centroids, 4, 0.60);
        if !clusters.is_empty() {
            emit_log(app, "info", &format!("Found {} project clusters: {:?}",
                clusters.len(),
                clusters.iter().map(|c| format!("{}({} topics)", c.name, c.topic_ids.len())).collect::<Vec<_>>()
            ));
        }
        clusters
    } else {
        emit_log(app, "info", &format!("Only {} topics have centroids (need 4+), skipping project cluster detection", topic_centroids.len()));
        vec![]
    };

    // 6. Build context with mandatory clusters
    let context = ai_client::GroupingContext {
        parent_name: parent_node.cluster_label.clone().unwrap_or_else(|| parent_node.title.clone()),
        parent_description: parent_node.summary.clone(),
        hierarchy_path: hierarchy_path.clone(),
        current_depth: parent_node.depth,
        sibling_names: sibling_names.clone(),
        forbidden_names: forbidden_names.clone(),
        mandatory_clusters,
    };

    emit_log(app, "info", &format!("Context: parent='{}', depth={}, path={:?}, {} siblings, {} forbidden, {} mandatory clusters",
             context.parent_name, context.current_depth,
             hierarchy_path, sibling_names.len(), forbidden_names.len(), context.mandatory_clusters.len()));

    // Get AI to group topics - use batching for large datasets
    // Both paths have 120s timeout to prevent hanging
    let groupings = if topics.len() > BATCH_THRESHOLD {
        emit_log(app, "info", &format!("Large dataset ({} topics) - using batch processing", topics.len()));
        group_topics_in_batches(&topics, &context, app).await?
    } else {
        match timeout(
            Duration::from_secs(120),
            ai_client::group_topics_into_categories(&topics, &context)
        ).await {
            Ok(Ok(g)) => g,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                emit_log(app, "error", &format!("AI grouping timed out for {} after 120s", parent_id));
                return Ok(false); // Signal failure, will be added to failed_nodes
            }
        }
    };

    if groupings.is_empty() {
        return Err("AI returned no groupings".to_string());
    }

    emit_log(app, "info", &format!("AI created {} parent categories", groupings.len()));

    // Get parent node to determine new depth
    let parent_node = db.get_node(parent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Parent node {} not found", parent_id))?;

    let new_intermediate_depth = parent_node.depth + 1;

    // Create map from label -> ALL child nodes with that label
    // (multiple topics can have the same cluster_label)
    let mut label_to_children: HashMap<String, Vec<&Node>> = HashMap::new();
    for child in &children {
        let label = child.cluster_label
            .as_ref()
            .or(child.ai_title.as_ref())
            .unwrap_or(&child.title)
            .clone();
        label_to_children.entry(label).or_default().push(child);
    }

    // Generate unique timestamp suffix to avoid ID collisions across iterations
    let timestamp_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();

    // Create intermediate nodes and reparent children
    let mut categories_created = 0;
    let now = timestamp_suffix as i64;

    // Collect all children that need depth updates for batch processing
    let mut all_children_to_update: Vec<String> = Vec::new();

    for (idx, grouping) in groupings.iter().enumerate() {
        // Find ALL child nodes matching this grouping's labels
        let matching_children: Vec<&Node> = grouping.children
            .iter()
            .flat_map(|label| label_to_children.get(label).cloned().unwrap_or_default())
            .collect();

        if matching_children.is_empty() {
            emit_log(app, "warn", &format!("Category '{}' has no matching children", grouping.name));
            continue;
        }

        // Create intermediate node with unique ID (timestamp prevents collision across iterations)
        let category_id = format!("{}-cat-{}-{}", parent_id, timestamp_suffix, idx);

        let category_node = Node {
            id: category_id.clone(),
            node_type: NodeType::Cluster,
            title: grouping.name.clone(),
            url: None,
            content: grouping.description.clone(),
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: Some(grouping.name.clone()),
            depth: new_intermediate_depth,
            is_item: false,
            is_universe: false,
            parent_id: Some(parent_id.to_string()),
            child_count: matching_children.len() as i32,
            ai_title: None,
            summary: grouping.description.clone(),
            tags: None,
            emoji: None,  // Let frontend keyword matcher assign meaningful emoji
            is_processed: false,
            conversation_id: None,
            sequence_index: None,
            is_pinned: false,
            last_accessed_at: None,
        };

        db.insert_node(&category_node).map_err(|e| e.to_string())?;
        categories_created += 1;

        // Reparent children to this category and collect for batch depth update
        for child in matching_children {
            db.update_parent(&child.id, &category_id).map_err(|e| e.to_string())?;
            all_children_to_update.push(child.id.clone());
        }
    }

    // Batch update depths for all reparented children in ONE query
    if !all_children_to_update.is_empty() {
        emit_log(app, "info", &format!("Updating depths for {} reparented nodes...", all_children_to_update.len()));
        db.increment_multiple_subtrees_depth(&all_children_to_update).map_err(|e| e.to_string())?;
    }

    // Update parent's child count
    db.update_child_count(parent_id, categories_created)
        .map_err(|e| e.to_string())?;

    emit_log(app, "info", &format!("Created {} intermediate categories under {}", categories_created, parent_id));

    // Detect pathological grouping - if grouping didn't actually split children meaningfully
    let original_child_count = children.len();
    let reparented_count = all_children_to_update.len();

    if categories_created <= 1 {
        // Only created 0 or 1 categories - grouping failed to split
        emit_log(app, "warn", &format!(
            "Pathological grouping detected for {}: {} children → {} categories. AI labels didn't match topic labels.",
            parent_id, original_child_count, categories_created
        ));
        // Return false to signal grouping didn't meaningfully reduce children
        // This prevents infinite recursion
        return Ok(false);
    }

    if reparented_count < original_child_count / 2 {
        // Less than half the children were matched - most fell through
        emit_log(app, "warn", &format!(
            "Grouping mostly failed for {}: only {}/{} children were categorized. Stopping recursion.",
            parent_id, reparented_count, original_child_count
        ));
        return Ok(false);
    }

    Ok(true)
}

/// Get emoji for a hierarchy depth level
fn get_emoji_for_depth(depth: i32) -> String {
    match depth {
        0 => "🌌".to_string(),  // Universe
        1 => "🌀".to_string(),  // Galaxy/Domain
        2 => "🌍".to_string(),  // Region
        3 => "🗂️".to_string(), // Topic
        _ => "📁".to_string(),  // Generic folder
    }
}

/// Build path from Universe to this node
/// Returns a list of node names from root to the specified node
pub fn build_hierarchy_path(db: &Database, node_id: &str) -> Result<Vec<String>, String> {
    let mut path = vec![];
    let mut current_id = Some(node_id.to_string());

    while let Some(id) = current_id {
        let node = db.get_node(&id).map_err(|e| e.to_string())?;
        if let Some(n) = node {
            // Use cluster_label if available, otherwise title
            let name = n.cluster_label.unwrap_or(n.title);
            path.push(name);
            current_id = n.parent_id;
        } else {
            break;
        }
    }

    path.reverse(); // Universe first
    Ok(path)
}

/// Increment depth of a node and all its descendants by 1
/// Uses a single SQL statement with recursive CTE to avoid lock issues
fn increment_subtree_depth(db: &Database, node_id: &str) -> Result<(), String> {
    db.increment_subtree_depth(node_id).map_err(|e| e.to_string())
}

/// Build full navigable hierarchy with recursive grouping
///
/// Flow:
/// 1. Run clustering to assign items to fine-grained topics
/// 2. Build initial hierarchy (flat topics under Universe)
/// 3. Recursively group any level with >15 children until navigable
pub async fn build_full_hierarchy(db: &Database, run_clustering: bool, app: Option<&AppHandle>) -> Result<FullHierarchyResult, String> {
    emit_log(app, "info", "═══════════════════════════════════════════════════════════");
    emit_log(app, "info", "Starting Full Hierarchy Build");
    emit_log(app, "info", "═══════════════════════════════════════════════════════════");

    // Step 1: Optionally run clustering
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ STEP 1/5: Clustering items into topics");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    let clustering_result = if run_clustering && ai_client::is_available() {
        emit_log(app, "info", "Running AI clustering on items...");
        let result = crate::clustering::run_clustering(db, true).await?;
        emit_log(app, "info", &format!("✓ Clustering complete: {} items → {} clusters", result.items_assigned, result.clusters_created));
        Some(result)
    } else {
        emit_log(app, "info", "Skipping (already done or AI not available)");
        None
    };

    // Step 2: Build initial hierarchy
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ STEP 2/5: Building initial hierarchy structure");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_log(app, "info", "Creating Universe and topic nodes from clusters...");
    let hierarchy_result = build_hierarchy(db)?;
    emit_log(app, "info", &format!("✓ Created {} intermediate nodes, organized {} items", hierarchy_result.intermediate_nodes_created, hierarchy_result.items_organized));

    // Step 3: Detect and create project umbrellas
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ STEP 3/5: Creating project umbrella categories");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_log(app, "info", "Scanning item titles for major project names...");

    let major_projects = ai_client::detect_major_projects_globally(db);
    let mut project_umbrellas_created = 0;

    if major_projects.is_empty() {
        emit_log(app, "info", "No major projects detected (need 2%+ of items, min 20)");
    } else {
        emit_log(app, "info", &format!("Found {} major projects:", major_projects.len()));
        for project in &major_projects {
            emit_log(app, "info", &format!("  • {} ({} items, {:.1}%)", project.name, project.item_count, project.percentage));
        }

        // Get universe node
        let universe = db.get_universe().map_err(|e| e.to_string())?
            .ok_or("No universe node found")?;

        for project in &major_projects {
            // Find parent topics of items containing this project name
            let mut topic_ids_to_move: std::collections::HashSet<String> = std::collections::HashSet::new();

            for item_id in &project.item_ids {
                if let Ok(Some(item)) = db.get_node(item_id) {
                    if let Some(parent_id) = &item.parent_id {
                        // Only move topics (non-items) that are direct children of universe
                        if let Ok(Some(parent)) = db.get_node(parent_id) {
                            if !parent.is_item && parent.parent_id.as_ref() == Some(&universe.id) {
                                topic_ids_to_move.insert(parent_id.clone());
                            }
                        }
                    }
                }
            }

            if topic_ids_to_move.is_empty() {
                emit_log(app, "info", &format!("  Skipping '{}': no eligible topics to move", project.name));
                continue;
            }

            // Create project umbrella node under universe
            let umbrella_id = format!("project-{}", project.name.to_lowercase().replace(' ', "-"));
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64;
            let umbrella = crate::db::Node {
                id: umbrella_id.clone(),
                node_type: crate::db::NodeType::Thought,
                title: project.name.clone(),
                url: None,
                content: None,
                position: crate::db::Position { x: 0.0, y: 0.0 },
                created_at: now,
                updated_at: now,
                cluster_id: None,
                cluster_label: Some(project.name.clone()),
                depth: 1,
                is_item: false,
                is_universe: false,
                parent_id: Some(universe.id.clone()),
                child_count: topic_ids_to_move.len() as i32,
                ai_title: Some(project.name.clone()),
                summary: Some(format!("Project umbrella for {} ({} items)", project.name, project.item_count)),
                tags: None,
                emoji: Some("📦".to_string()),
                is_processed: true,
                conversation_id: None,
                sequence_index: None,
                is_pinned: false,
                last_accessed_at: None,
            };

            // Insert umbrella node
            if let Err(e) = db.insert_node(&umbrella) {
                emit_log(app, "warn", &format!("  Failed to create umbrella for '{}': {}", project.name, e));
                continue;
            }

            // Reparent topics under the umbrella
            let mut moved_count = 0;
            for topic_id in &topic_ids_to_move {
                if let Err(e) = db.update_node_parent(topic_id, &umbrella_id) {
                    emit_log(app, "warn", &format!("    Failed to move topic {}: {}", topic_id, e));
                } else {
                    // Update depth of moved topic and its subtree
                    if let Err(e) = db.increment_subtree_depth_by(topic_id, 1) {
                        emit_log(app, "warn", &format!("    Failed to update depth for {}: {}", topic_id, e));
                    }
                    moved_count += 1;
                }
            }

            emit_log(app, "info", &format!("  ✓ Created '{}' umbrella with {} topics", project.name, moved_count));
            project_umbrellas_created += 1;
        }
    }

    if project_umbrellas_created > 0 {
        emit_log(app, "info", &format!("✓ Created {} project umbrella categories", project_umbrellas_created));
    }

    // Step 4: Recursively group levels with too many children
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ STEP 4/5: Recursive grouping (target: 8-15 children per level)");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    let mut grouping_iterations = 0;
    let max_iterations = 50; // Safety limit
    let mut failed_nodes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let grouping_start = std::time::Instant::now();

    // Emit initial progress for Step 3
    emit_progress(app, AiProgressEvent {
        current: 0,
        total: 0, // Unknown upfront
        node_title: "Starting hierarchy grouping...".to_string(),
        new_title: "Analyzing structure".to_string(),
        emoji: Some("🔄".to_string()),
        status: "processing".to_string(),
        error_message: None,
        elapsed_secs: Some(0.0),
        estimate_secs: None,
        remaining_secs: None,
    });

    loop {
        // Check for cancellation
        if is_rebuild_cancelled() {
            emit_log(app, "warn", "[Hierarchy] Cancelled by user during grouping");
            return Err("Hierarchy build cancelled by user".to_string());
        }

        if grouping_iterations >= max_iterations {
            emit_log(app, "warn", &format!("⚠ Hit max grouping iterations ({})", max_iterations));
            break;
        }

        // Find a node that needs grouping (has >15 children), excluding failed nodes
        let node_to_group = find_node_needing_grouping_excluding(db, app, &failed_nodes)?;

        match node_to_group {
            Some(node_id) => {
                // Get node name for progress display
                let node_name = db.get_node(&node_id)
                    .ok()
                    .flatten()
                    .map(|n| n.cluster_label.unwrap_or(n.title))
                    .unwrap_or_else(|| node_id.clone());

                let elapsed = grouping_start.elapsed().as_secs_f64();

                // Emit progress event for this iteration
                emit_progress(app, AiProgressEvent {
                    current: grouping_iterations + 1,
                    total: grouping_iterations + 2, // Show as "N of N+1" to indicate ongoing
                    node_title: format!("Grouping: {}", node_name),
                    new_title: "AI organizing into categories...".to_string(),
                    emoji: Some("🧠".to_string()),
                    status: "processing".to_string(),
                    error_message: None,
                    elapsed_secs: Some(elapsed),
                    estimate_secs: None,
                    remaining_secs: None,
                });

                emit_log(app, "info", &format!("  Iteration {}: Grouping children of {}", grouping_iterations + 1, node_id));
                let grouped = cluster_hierarchy_level(db, &node_id, app).await?;
                if grouped {
                    grouping_iterations += 1;
                } else {
                    // Grouping failed (pathological case) - skip this node in future iterations
                    emit_log(app, "warn", &format!("  Marking {} as ungroupable", node_id));
                    failed_nodes.insert(node_id);
                }
            }
            None => {
                if failed_nodes.is_empty() {
                    emit_log(app, "info", &format!("✓ Hierarchy is navigable after {} grouping iterations", grouping_iterations));
                } else {
                    emit_log(app, "warn", &format!(
                        "✓ Hierarchy grouping complete after {} iterations ({} nodes couldn't be grouped)",
                        grouping_iterations, failed_nodes.len()
                    ));
                }
                break;
            }
        }
    }

    // Emit completion for Step 3
    let grouping_elapsed = grouping_start.elapsed().as_secs_f64();
    emit_progress(app, AiProgressEvent {
        current: grouping_iterations,
        total: grouping_iterations,
        node_title: "Hierarchy grouping".to_string(),
        new_title: format!("{} levels organized", grouping_iterations),
        emoji: Some("✓".to_string()),
        status: if grouping_iterations > 0 { "success".to_string() } else { "complete".to_string() },
        error_message: None,
        elapsed_secs: Some(grouping_elapsed),
        estimate_secs: Some(grouping_elapsed),
        remaining_secs: Some(0.0),
    });

    // Recalculate final depth
    let final_max_depth = db.get_max_depth().map_err(|e| e.to_string())?;
    emit_log(app, "info", &format!("  Final hierarchy depth: {}", final_max_depth));

    // Step 5: Generate embeddings for ALL nodes that need them (if OpenAI key available)
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ STEP 5/5: Generating embeddings for semantic search");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    let (embeddings_generated, embeddings_skipped) = if settings::has_openai_api_key() {
        let nodes_needing_embeddings = db.get_nodes_needing_embeddings().map_err(|e| e.to_string())?;
        let total_needing = nodes_needing_embeddings.len();

        if total_needing == 0 {
            emit_log(app, "info", "All nodes already have embeddings");
            (0, 0)
        } else {
            emit_log(app, "info", &format!("Generating embeddings for {} nodes...", total_needing));
            let mut generated = 0;
            let mut skipped = 0;
            let start_time = Instant::now();

            for (i, node) in nodes_needing_embeddings.iter().enumerate() {
                let current = i + 1;
                let title = node.ai_title.as_deref().unwrap_or(&node.title);
                let summary = node.summary.as_deref().unwrap_or("");
                let embed_text = format!("{} {}", title, summary);

                // Calculate time estimates
                let elapsed = start_time.elapsed().as_secs_f64();
                let (estimate, remaining) = if current > 1 {
                    let avg = elapsed / (current - 1) as f64;
                    let est = avg * total_needing as f64;
                    let rem = avg * (total_needing - current + 1) as f64;
                    (Some(est), Some(rem))
                } else {
                    (None, None)
                };

                // Emit processing event
                emit_progress(app, AiProgressEvent {
                    current,
                    total: total_needing,
                    node_title: title.to_string(),
                    new_title: "Generating embedding...".to_string(),
                    emoji: None,
                    status: "processing".to_string(),
                    error_message: None,
                    elapsed_secs: Some(elapsed),
                    estimate_secs: estimate,
                    remaining_secs: remaining,
                });

                match ai_client::generate_embedding(&embed_text).await {
                    Ok(embedding) => {
                        if let Err(e) = db.update_node_embedding(&node.id, &embedding) {
                            emit_log(app, "warn", &format!("Failed to save embedding for {}: {}", node.id, e));
                            skipped += 1;
                        } else {
                            generated += 1;
                            // Emit success event
                            let elapsed_now = start_time.elapsed().as_secs_f64();
                            emit_progress(app, AiProgressEvent {
                                current,
                                total: total_needing,
                                node_title: title.to_string(),
                                new_title: "Embedding generated".to_string(),
                                emoji: Some("✓".to_string()),
                                status: "success".to_string(),
                                error_message: None,
                                elapsed_secs: Some(elapsed_now),
                                estimate_secs: estimate,
                                remaining_secs: remaining,
                            });
                            if (i + 1) % 10 == 0 || i + 1 == total_needing {
                                emit_log(app, "info", &format!("  Progress: {}/{} embeddings", i + 1, total_needing));
                            }
                        }
                    }
                    Err(e) => {
                        emit_log(app, "warn", &format!("Embedding failed for {}: {}", node.id, e));
                        skipped += 1;
                    }
                }
            }

            // Emit complete event
            let total_elapsed = start_time.elapsed().as_secs_f64();
            emit_progress(app, AiProgressEvent {
                current: total_needing,
                total: total_needing,
                node_title: String::new(),
                new_title: format!("{} embeddings generated", generated),
                emoji: Some("✓".to_string()),
                status: "complete".to_string(),
                error_message: None,
                elapsed_secs: Some(total_elapsed),
                estimate_secs: Some(total_elapsed),
                remaining_secs: Some(0.0),
            });

            emit_log(app, "info", &format!("✓ Embeddings complete: {} generated, {} skipped", generated, skipped));
            (generated, skipped)
        }
    } else {
        emit_log(app, "info", "Skipping (OpenAI API key not set)");
        (0, 0)
    };

    // Step 5: Create semantic edges based on embedding similarity
    // (This is a bonus step, not numbered in the main 4)
    let semantic_edges_created = if embeddings_generated > 0 || db.get_nodes_with_embeddings().map(|v| v.len()).unwrap_or(0) > 1 {
        emit_log(app, "info", "");
        emit_log(app, "info", "Creating semantic edges from embeddings...");

        // Delete old AI-generated semantic edges first
        if let Ok(deleted) = db.delete_semantic_edges() {
            if deleted > 0 {
                emit_log(app, "info", &format!("  Cleared {} old semantic edges", deleted));
            }
        }

        // Create new edges: min 50% similarity, max 5 edges per node
        match db.create_semantic_edges(0.5, 5) {
            Ok(created) => {
                emit_log(app, "info", &format!("✓ Created {} semantic edges", created));
                created
            }
            Err(e) => {
                emit_log(app, "warn", &format!("Failed to create semantic edges: {}", e));
                0
            }
        }
    } else {
        0
    };

    emit_log(app, "info", "");
    emit_log(app, "info", "═══════════════════════════════════════════════════════════");
    emit_log(app, "info", "✓ HIERARCHY BUILD COMPLETE");
    emit_log(app, "info", &format!("  • {} grouping iterations", grouping_iterations));
    emit_log(app, "info", &format!("  • {} hierarchy levels (depth 0-{})", final_max_depth + 1, final_max_depth));
    emit_log(app, "info", &format!("  • {} semantic edges", semantic_edges_created));
    emit_log(app, "info", "═══════════════════════════════════════════════════════════");

    Ok(FullHierarchyResult {
        clustering_result,
        hierarchy_result: HierarchyResult {
            levels_created: (final_max_depth + 1) as usize,
            intermediate_nodes_created: hierarchy_result.intermediate_nodes_created,
            items_organized: hierarchy_result.items_organized,
            max_depth: final_max_depth,
        },
        levels_created: (final_max_depth + 1) as usize,
        grouping_iterations,
        embeddings_generated,
        embeddings_skipped,
    })
}

/// Find a node that has >15 children and needs grouping
/// Prioritizes nodes closer to Universe (lower depth) for top-down grouping
/// Excludes nodes in the `skip_nodes` set (nodes that failed grouping)
fn find_node_needing_grouping_excluding(
    db: &Database,
    app: Option<&AppHandle>,
    skip_nodes: &std::collections::HashSet<String>,
) -> Result<Option<String>, String> {
    // Start from Universe and work down using BFS (proper queue with remove(0))
    let universe = db.get_universe().map_err(|e| e.to_string())?;

    if let Some(universe) = universe {
        let mut queue = vec![(universe.id.clone(), 0i32)]; // (node_id, depth)
        let mut nodes_checked = 0;

        while !queue.is_empty() {
            // BFS: remove from front (FIFO) to prioritize shallower nodes
            let (node_id, depth) = queue.remove(0);
            nodes_checked += 1;

            // Skip nodes that failed grouping (pathological case)
            if skip_nodes.contains(&node_id) {
                let children = db.get_children(&node_id).map_err(|e| e.to_string())?;
                for child in children {
                    if !child.is_item {
                        queue.push((child.id, depth + 1));
                    }
                }
                continue;
            }

            let children = db.get_children(&node_id).map_err(|e| e.to_string())?;
            let child_count = children.len();

            // Count non-item children (categories that could be grouped)
            let non_item_children: Vec<_> = children.iter().filter(|c| !c.is_item).collect();
            let non_item_count = non_item_children.len();

            // Check if this node has too many children
            if child_count > MAX_CHILDREN_PER_LEVEL && non_item_count > 0 {
                emit_log(app, "debug", &format!(
                    "Found node needing grouping: {} (depth {}, {} children, {} non-items)",
                    node_id, depth, child_count, non_item_count
                ));
                return Ok(Some(node_id));
            }

            // Add non-item children to queue for BFS traversal
            for child in children {
                if !child.is_item {
                    queue.push((child.id, depth + 1));
                }
            }
        }

        emit_log(app, "debug", &format!("Checked {} nodes, none need grouping", nodes_checked));
    }

    Ok(None)
}

/// Flatten single-child chains in navigation
/// When getting children, if a node has exactly 1 child, skip to that child's children
pub fn get_children_skip_single_chain(db: &Database, parent_id: &str) -> Result<Vec<Node>, String> {
    let mut current_id = parent_id.to_string();
    let mut depth_skipped = 0;
    const MAX_SKIP: usize = 5; // Safety limit

    loop {
        let children = db.get_children(&current_id).map_err(|e| e.to_string())?;

        // If exactly 1 non-item child, skip to its children
        if children.len() == 1 && !children[0].is_item && depth_skipped < MAX_SKIP {
            current_id = children[0].id.clone();
            depth_skipped += 1;
            println!("Skipping single-child node: {}", children[0].title);
            continue;
        }

        return Ok(children);
    }
}
