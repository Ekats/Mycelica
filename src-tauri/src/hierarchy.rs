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
use serde::Serialize;
use std::collections::HashMap;
use tauri::{AppHandle, Emitter};

/// Log event for frontend dev console
#[derive(Clone, Serialize)]
pub struct HierarchyLogEvent {
    pub message: String,
    pub level: String,
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

/// Cluster children of a specific parent into 8-15 groups using AI
///
/// If parent has <= MAX_CHILDREN_PER_LEVEL children, returns Ok(false) - no grouping needed.
/// Otherwise, creates new intermediate nodes and reparents children.
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

    // 5. Build context
    let context = ai_client::GroupingContext {
        parent_name: parent_node.cluster_label.clone().unwrap_or_else(|| parent_node.title.clone()),
        parent_description: parent_node.summary.clone(),
        hierarchy_path: hierarchy_path.clone(),
        current_depth: parent_node.depth,
        sibling_names: sibling_names.clone(),
        forbidden_names: forbidden_names.clone(),
    };

    emit_log(app, "info", &format!("Context: parent='{}', depth={}, path={:?}, {} siblings, {} forbidden",
             context.parent_name, context.current_depth,
             hierarchy_path, sibling_names.len(), forbidden_names.len()));

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

    // Get AI to group topics (with context)
    let groupings = ai_client::group_topics_into_categories(&topics, &context).await?;

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

    // Create intermediate nodes and reparent children
    let mut categories_created = 0;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

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

        // Create intermediate node
        let category_id = format!("{}-cat-{}", parent_id, idx);

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

        // Reparent children to this category
        for child in matching_children {
            db.update_parent(&child.id, &category_id).map_err(|e| e.to_string())?;
            // Increment depth of child and all its descendants
            increment_subtree_depth(db, &child.id)?;
        }
    }

    // Update parent's child count
    db.update_child_count(parent_id, categories_created)
        .map_err(|e| e.to_string())?;

    emit_log(app, "info", &format!("Created {} intermediate categories under {}", categories_created, parent_id));

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

/// Increment depth of a node and all its descendants
fn increment_subtree_depth(db: &Database, node_id: &str) -> Result<(), String> {
    let node = db.get_node(node_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Node {} not found", node_id))?;

    // Update this node's depth
    db.update_node_hierarchy(node_id, node.parent_id.as_deref(), node.depth + 1)
        .map_err(|e| e.to_string())?;

    // Recursively update children
    let children = db.get_children(node_id).map_err(|e| e.to_string())?;
    for child in children {
        increment_subtree_depth(db, &child.id)?;
    }

    Ok(())
}

/// Build full navigable hierarchy with recursive grouping
///
/// Flow:
/// 1. Run clustering to assign items to fine-grained topics
/// 2. Build initial hierarchy (flat topics under Universe)
/// 3. Recursively group any level with >15 children until navigable
pub async fn build_full_hierarchy(db: &Database, run_clustering: bool, app: Option<&AppHandle>) -> Result<FullHierarchyResult, String> {
    // Step 1: Optionally run clustering
    let clustering_result = if run_clustering && ai_client::is_available() {
        emit_log(app, "info", "Step 1: Running AI clustering on items...");
        let result = crate::clustering::run_clustering(db, true).await?;
        emit_log(app, "info", &format!("Clustering complete: {} items in {} clusters", result.items_assigned, result.clusters_created));
        Some(result)
    } else {
        emit_log(app, "info", "Step 1: Skipping clustering (already done or AI not available)");
        None
    };

    // Step 2: Build initial hierarchy
    emit_log(app, "info", "Step 2: Building initial hierarchy from clustered items...");
    let hierarchy_result = build_hierarchy(db)?;

    // Step 3: Recursively group levels with too many children
    emit_log(app, "info", "Step 3: Recursively grouping levels with >15 children...");
    let mut grouping_iterations = 0;
    let max_iterations = 10; // Safety limit

    loop {
        if grouping_iterations >= max_iterations {
            emit_log(app, "warn", &format!("Hit max grouping iterations ({})", max_iterations));
            break;
        }

        // Find a node that needs grouping (has >15 children)
        let node_to_group = find_node_needing_grouping(db, app)?;

        match node_to_group {
            Some(node_id) => {
                emit_log(app, "info", &format!("Grouping children of: {}", node_id));
                let grouped = cluster_hierarchy_level(db, &node_id, app).await?;
                if grouped {
                    grouping_iterations += 1;
                }
            }
            None => {
                emit_log(app, "info", "All levels have ≤15 children, hierarchy is navigable");
                break;
            }
        }
    }

    // Recalculate final depth
    let final_max_depth = db.get_max_depth().map_err(|e| e.to_string())?;

    // Step 4: Generate embeddings for ALL nodes that need them (if OpenAI key available)
    let (embeddings_generated, embeddings_skipped) = if settings::has_openai_api_key() {
        emit_log(app, "info", "Step 4: Generating embeddings for all nodes...");
        let nodes_needing_embeddings = db.get_nodes_needing_embeddings().map_err(|e| e.to_string())?;
        let total_needing = nodes_needing_embeddings.len();

        if total_needing == 0 {
            emit_log(app, "info", "All nodes already have embeddings");
            (0, 0)
        } else {
            emit_log(app, "info", &format!("Generating embeddings for {} nodes...", total_needing));
            let mut generated = 0;
            let mut skipped = 0;

            for (i, node) in nodes_needing_embeddings.iter().enumerate() {
                // Build text from ai_title + summary
                let title = node.ai_title.as_deref().unwrap_or(&node.title);
                let summary = node.summary.as_deref().unwrap_or("");
                let embed_text = format!("{} {}", title, summary);

                match ai_client::generate_embedding(&embed_text).await {
                    Ok(embedding) => {
                        if let Err(e) = db.update_node_embedding(&node.id, &embedding) {
                            emit_log(app, "warn", &format!("Failed to save embedding for {}: {}", node.id, e));
                            skipped += 1;
                        } else {
                            generated += 1;
                            if (i + 1) % 10 == 0 || i + 1 == total_needing {
                                emit_log(app, "info", &format!("Embeddings: {}/{} complete", i + 1, total_needing));
                            }
                        }
                    }
                    Err(e) => {
                        emit_log(app, "warn", &format!("Embedding failed for {}: {}", node.id, e));
                        skipped += 1;
                    }
                }
            }

            emit_log(app, "info", &format!("Embeddings complete: {} generated, {} skipped", generated, skipped));
            (generated, skipped)
        }
    } else {
        emit_log(app, "info", "Step 4: Skipping embeddings (OpenAI API key not set)");
        (0, 0)
    };

    // Step 5: Create semantic edges based on embedding similarity
    let semantic_edges_created = if embeddings_generated > 0 || db.get_nodes_with_embeddings().map(|v| v.len()).unwrap_or(0) > 1 {
        emit_log(app, "info", "Step 5: Creating semantic edges from embeddings...");

        // Delete old AI-generated semantic edges first
        if let Ok(deleted) = db.delete_semantic_edges() {
            if deleted > 0 {
                emit_log(app, "info", &format!("Cleared {} old semantic edges", deleted));
            }
        }

        // Create new edges: min 50% similarity, max 5 edges per node
        match db.create_semantic_edges(0.5, 5) {
            Ok(created) => {
                emit_log(app, "info", &format!("Created {} semantic edges", created));
                created
            }
            Err(e) => {
                emit_log(app, "warn", &format!("Failed to create semantic edges: {}", e));
                0
            }
        }
    } else {
        emit_log(app, "info", "Step 5: Skipping semantic edges (no embeddings)");
        0
    };

    emit_log(app, "info", &format!("Full hierarchy complete: {} grouping iterations, max depth = {}, {} semantic edges", grouping_iterations, final_max_depth, semantic_edges_created));

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
fn find_node_needing_grouping(db: &Database, app: Option<&AppHandle>) -> Result<Option<String>, String> {
    // Start from Universe and work down using BFS (proper queue with remove(0))
    let universe = db.get_universe().map_err(|e| e.to_string())?;

    if let Some(universe) = universe {
        let mut queue = vec![(universe.id.clone(), 0i32)]; // (node_id, depth)
        let mut nodes_checked = 0;

        while !queue.is_empty() {
            // BFS: remove from front (FIFO) to prioritize shallower nodes
            let (node_id, depth) = queue.remove(0);
            nodes_checked += 1;

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
