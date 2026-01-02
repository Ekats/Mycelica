//! Dynamic hierarchy generation for the knowledge graph
//!
//! NEW: Recursive hierarchy building with AI-powered topic grouping
//!
//! The hierarchy builder now works in two phases:
//! 1. Bottom-up: Items are clustered into fine-grained topics (via clustering.rs)
//! 2. Top-down: Topics are recursively grouped into parent categories
//!    using tiered limits (L0-1=10, L2=25, L3=50, L4=100) for navigation
//!
//! Key insight: Start with natural clusters, then organize them into a
//! navigable tree. Both directions meeting in the middle.

use crate::db::{Database, Node, NodeType, Position};
use crate::ai_client::{self, TopicInfo};
use crate::settings;
use crate::commands::is_rebuild_cancelled;
use crate::classification;
use crate::similarity::{cosine_similarity, compute_centroid};
use rand::Rng;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Instant;

/// Safely truncate a string at a UTF-8 boundary
fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if max_bytes >= s.len() { return s; }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Compute and store centroid embedding for a parent node from its children's embeddings
/// This enables similarity-based grouping of intermediate nodes (topics/categories)
fn compute_and_store_centroid(db: &Database, parent_id: &str) {
    let children = match db.get_children(parent_id) {
        Ok(c) => c,
        Err(_) => return,
    };

    let child_embeddings: Vec<Vec<f32>> = children
        .iter()
        .filter_map(|c| db.get_node_embedding(&c.id).ok().flatten())
        .collect();

    if !child_embeddings.is_empty() {
        let refs: Vec<&[f32]> = child_embeddings.iter().map(|e| e.as_slice()).collect();
        if let Some(centroid) = compute_centroid(&refs) {
            let _ = db.update_node_embedding(parent_id, &centroid);
        }
    }
}

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

/// Get maximum children allowed for a given depth level
/// Tiered limits for clean navigation at top, more permissive at depth
fn max_children_for_depth(depth: i32) -> usize {
    match depth {
        0 | 1 => 10,  // Universe + L1: strict navigation
        2 => 25,      // L2: buffer layer
        3 => 50,      // L3: topic groupings
        4 => 100,     // L4: normal max depth
        _ => 150,     // L5+: coherent mega-clusters only
    }
}

/// Check if children are coherent enough to warrant deep splitting (L5+)
/// Returns true if splitting makes sense, false if it would just create noise
fn is_coherent_for_deep_split(children: &[Node]) -> bool {
    // Separate items from categories
    let items: Vec<_> = children.iter().filter(|c| c.is_item).collect();
    let categories: Vec<_> = children.iter().filter(|c| !c.is_item).collect();

    // If mostly categories → coherent (they're already grouped)
    if categories.len() > items.len() {
        return true;
    }

    // If mostly items → check cluster_id coherence
    let mut cluster_counts: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
    for item in &items {
        if let Some(cluster_id) = item.cluster_id {
            *cluster_counts.entry(cluster_id).or_default() += 1;
        }
    }

    if let Some(max_count) = cluster_counts.values().max() {
        if *max_count as f32 / items.len() as f32 >= 0.8 {
            return true; // 80%+ items from same cluster = coherent
        }
    }

    // No cluster_ids at all? Default to coherent (can't prove incoherence)
    if cluster_counts.is_empty() {
        return true;
    }

    false // Diverse cluster_ids = incoherent noise
}

/// Names that indicate AI couldn't produce a meaningful grouping
const GARBAGE_NAMES: &[&str] = &[
    "empty", "cluster", "misc", "other", "general", "various",
    "uncategorized", "miscellaneous", "group", "collection",
    "related", "topics", "items", "content", "stuff", "things",
    "mixed", "assorted", "combined", "merged", "grouped", "sorted",
];

/// Check if a category name is garbage (indicates AI failure)
fn is_garbage_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Check if name is ONLY garbage words (allow "Rust Development" but not "Empty Cluster")
    let words: Vec<&str> = lower.split_whitespace().collect();

    // If all words are garbage, it's a garbage name
    let garbage_word_count = words.iter()
        .filter(|w| GARBAGE_NAMES.iter().any(|g| w.contains(g)))
        .count();

    // Name is garbage if >50% of words are garbage terms
    garbage_word_count > 0 && garbage_word_count >= (words.len() + 1) / 2
}

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
/// will add intermediate levels as needed based on tiered limits per depth.
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

/// Clear the existing hierarchy (delete all intermediate nodes, clear parent refs)
///
/// Used by rebuild_lite to start fresh without AI.
pub fn clear_hierarchy(db: &Database) -> Result<(), String> {
    cleanup_hierarchy(db)
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

    // Step 2: Get only VISIBLE tier items (excluding protected)
    // VISIBLE: insight, exploration, synthesis, question, planning
    // HIDDEN/SUPPORTING items keep cluster_id but don't appear in hierarchy
    let all_items = db.get_visible_items().map_err(|e| e.to_string())?;
    let protected_ids = db.get_protected_node_ids();
    let after_protected: Vec<Node> = all_items
        .into_iter()
        .filter(|item| !protected_ids.contains(&item.id))
        .collect();

    if !protected_ids.is_empty() {
        println!("[Hierarchy] Excluding {} protected items (Recent Notes)", protected_ids.len());
    }

    // Step 2.5: Separate private items (privacy < threshold) - they go to Personal category
    let privacy_threshold = crate::settings::get_privacy_threshold() as f64;
    let (private_items, items): (Vec<Node>, Vec<Node>) = after_protected
        .into_iter()
        .partition(|item| item.privacy.map(|p| p < privacy_threshold).unwrap_or(false));

    if !private_items.is_empty() {
        println!("[Hierarchy] Found {} private items (privacy < {}) - will go to Personal category", private_items.len(), privacy_threshold);
    }

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
            latest_child_date: None,
            is_private: None,
            privacy_reason: None,
            source: None,
            pdf_available: None,
            content_type: None,
            associated_idea_id: None,
            privacy: None,
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

    // Compute centroid embeddings for topic nodes (enables similarity-based uber-category grouping)
    for topic_id in &topic_ids {
        compute_and_store_centroid(db, topic_id);
    }
    println!("Computed centroid embeddings for {} topics", topic_ids.len());

    // Step 6: Create Universe (depth 0) and attach topics to it
    let universe_id = create_universe(db, &topic_ids)?;

    // Update topics to point to Universe
    for topic_id in &topic_ids {
        db.update_node_hierarchy(topic_id, Some(&universe_id), topic_depth)
            .map_err(|e| e.to_string())?;
    }

    // Update child count on Universe (will be updated again if we add Personal)
    let mut universe_child_count = topic_ids.len() as i32;
    db.update_child_count(&universe_id, universe_child_count)
        .map_err(|e| e.to_string())?;

    // Step 7: Handle private items - create Personal category
    let mut private_count = 0;
    if !private_items.is_empty() {
        let personal_id = "category-personal".to_string();

        // Create Personal category at depth 1 (same level as topics)
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let personal_node = Node {
            id: personal_id.clone(),
            node_type: NodeType::Cluster,
            title: "Personal".to_string(),
            url: None,
            content: None,
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: Some("Personal".to_string()),
            ai_title: Some("Personal".to_string()),
            summary: Some(format!("Private items (privacy score < {})", privacy_threshold)),
            tags: None,
            emoji: Some("🔒".to_string()),
            is_processed: true,
            depth: topic_depth,
            is_item: false,
            is_universe: false,
            parent_id: Some(universe_id.clone()),
            child_count: private_items.len() as i32,
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
            privacy: Some(0.0), // Category is private since it contains private items
        };

        db.insert_node(&personal_node).map_err(|e| e.to_string())?;

        // Reparent private items under Personal
        for item in &private_items {
            db.update_node_hierarchy(&item.id, Some(&personal_id), item_depth)
                .map_err(|e| e.to_string())?;
            private_count += 1;
        }

        // Update Universe child count to include Personal
        universe_child_count += 1;
        db.update_child_count(&universe_id, universe_child_count)
            .map_err(|e| e.to_string())?;

        println!("[Hierarchy] Created Personal category with {} private items", private_count);
    }

    // Step 8: Ensure Recent Notes container exists (for in-app notes)
    let notes_container_id = crate::settings::RECENT_NOTES_CONTAINER_ID;
    if db.get_node(notes_container_id).map_err(|e| e.to_string())?.is_none() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let notes_node = Node {
            id: notes_container_id.to_string(),
            node_type: NodeType::Cluster,
            title: "Recent Notes".to_string(),
            url: None,
            content: None,
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: Some("Recent Notes".to_string()),
            ai_title: Some("Recent Notes".to_string()),
            summary: Some("User-created notes".to_string()),
            tags: None,
            emoji: Some("📝".to_string()),
            is_processed: true,
            depth: topic_depth,
            is_item: false,
            is_universe: false,
            parent_id: Some(universe_id.clone()),
            child_count: 0,
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
            privacy: Some(0.5), // Notes are semi-private by default
        };

        db.insert_node(&notes_node).map_err(|e| e.to_string())?;

        // Update Universe child count to include Notes
        universe_child_count += 1;
        db.update_child_count(&universe_id, universe_child_count)
            .map_err(|e| e.to_string())?;

        println!("[Hierarchy] Created Notes container (protected)");
    }

    let total_items = item_count + private_count;
    println!("Hierarchy complete: Universe -> {} topics + Personal + Notes -> {} items",
             topics_created, total_items);

    Ok(HierarchyResult {
        levels_created: 3,  // Universe, Topics, Items
        intermediate_nodes_created: topics_created + 1 + (if private_count > 0 { 1 } else { 0 }),  // topics + universe + personal
        items_organized: total_items,
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
            latest_child_date: None,
            is_private: None,
            privacy_reason: None,
            source: None,
            pdf_available: None,
            content_type: None,
            associated_idea_id: None,
            privacy: None,
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
        latest_child_date: None,
        is_private: None,
        privacy_reason: None,
        source: None,
        pdf_available: None,
        content_type: None,
        associated_idea_id: None,
        privacy: None,
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
    max_groups: usize,
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
            ai_client::group_topics_into_categories(batch, &batch_context, Some(max_groups))
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

/// Cluster children of a specific parent into groups using AI
///
/// If parent has <= max_children_for_depth(depth) children, returns Ok(false) - no grouping needed.
/// Tiered limits: L0/L1=10, L2=25, L3=50, L4=100
/// Otherwise, creates new intermediate nodes and reparents children.
///
/// For large datasets (>200 children), splits into batches of 150, calls AI for each,
/// then merges similar categories across batches to prevent fragmentation.
/// max_groups: maximum number of categories to create (default 9 if None for manual splits)
pub async fn cluster_hierarchy_level(db: &Database, parent_id: &str, app: Option<&AppHandle>, max_groups: Option<usize>, force: bool) -> Result<bool, String> {
    emit_log(app, "debug", &format!("cluster_hierarchy_level called: parent={}, force={}, max_groups={:?}", parent_id, force, max_groups));
    let max_groups = max_groups.unwrap_or(9); // Default to 9 for manual splits (Universe target: 9 + Notes = 10)
    // 1. Get parent node info first (need depth for tiered limits)
    let parent_node = db.get_node(parent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Parent node {} not found", parent_id))?;

    let parent_depth = parent_node.depth;
    let max_for_depth = max_children_for_depth(parent_depth);

    // Get children of this parent (excluding protected)
    let all_children = db.get_children(parent_id).map_err(|e| e.to_string())?;
    let all_children_count = all_children.len();
    let protected_ids = db.get_protected_node_ids();
    let children: Vec<Node> = all_children
        .into_iter()
        .filter(|child| !protected_ids.contains(&child.id))
        .collect();

    let excluded_count = all_children_count - children.len();
    if excluded_count > 0 {
        emit_log(app, "info", &format!("Excluding {} protected nodes (Recent Notes) from grouping", excluded_count));
    }

    // For manual splits (force=true), require at least 2 children
    // For automatic splits, use depth-based thresholds
    if force {
        if children.len() < 2 {
            emit_log(app, "info", &format!("Parent {} has {} children, need at least 2 to split", parent_id, children.len()));
            return Ok(false);
        }
    } else if children.len() <= max_for_depth {
        emit_log(app, "info", &format!("Parent {} (depth {}) has {} children (≤{}), no grouping needed",
                 parent_id, parent_depth, children.len(), max_for_depth));
        return Ok(false);
    }

    // L5+ coherence gate: don't split incoherent noise deeper
    if parent_depth >= 4 && !is_coherent_for_deep_split(&children) {
        emit_log(app, "warn", &format!(
            "Parent {} (depth {}) has {} children but they're incoherent - skipping L5 split",
            parent_id, parent_depth, children.len()
        ));
        return Ok(false);
    }

    // Check if ALL children are items (leaf nodes) - use embedding-based clustering
    let non_item_count = children.iter().filter(|c| !c.is_item).count();
    if non_item_count == 0 && children.len() > max_for_depth {
        emit_log(app, "info", &format!(
            "All {} children are items - using embedding-based clustering",
            children.len()
        ));
        return cluster_items_by_embedding(db, parent_id, &children, app).await;
    }

    emit_log(app, "info", &format!("Grouping {} children of {} (depth {}, max {}) into categories",
             children.len(), parent_id, parent_depth, max_for_depth));

    // === Gather hierarchy context for AI ===

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

    // Build topic info for AI - use same label logic as label_to_children
    let topics: Vec<TopicInfo> = children
        .iter()
        .map(|child| TopicInfo {
            id: child.id.clone(),
            label: if child.is_item {
                // Items: prefer ai_title (unique) over cluster_label (shared with siblings)
                child.ai_title
                    .clone()
                    .or_else(|| child.cluster_label.clone())
                    .unwrap_or_else(|| child.title.clone())
            } else {
                // Categories: use cluster_label as primary
                child.cluster_label
                    .clone()
                    .or_else(|| child.ai_title.clone())
                    .unwrap_or_else(|| child.title.clone())
            },
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

        // Filter out clusters with garbage names (e.g., "Empty", "Cluster")
        let valid_clusters: Vec<_> = clusters.into_iter()
            .filter(|c| !is_garbage_name(&c.name))
            .collect();

        if !valid_clusters.is_empty() {
            emit_log(app, "info", &format!("Found {} valid project clusters: {:?}",
                valid_clusters.len(),
                valid_clusters.iter().map(|c| format!("{}({} topics)", c.name, c.topic_ids.len())).collect::<Vec<_>>()
            ));
        }
        valid_clusters
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
        group_topics_in_batches(&topics, &context, app, max_groups).await?
    } else {
        match timeout(
            Duration::from_secs(120),
            ai_client::group_topics_into_categories(&topics, &context, Some(max_groups))
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

    // Filter out garbage category names (indicates AI couldn't produce meaningful groups)
    let original_count = groupings.len();
    let groupings: Vec<_> = groupings.into_iter()
        .filter(|g| !is_garbage_name(&g.name))
        .collect();

    let filtered_count = original_count - groupings.len();
    if filtered_count > 0 {
        emit_log(app, "warn", &format!(
            "Filtered {} garbage category names (e.g., 'Empty', 'Cluster')", filtered_count
        ));
    }

    // If all groupings were garbage or <2 remain, stop grouping
    if groupings.len() < 2 {
        emit_log(app, "warn", &format!(
            "Only {} valid categories remain after filtering garbage names. Stopping grouping.",
            groupings.len()
        ));
        return Ok(false);
    }

    emit_log(app, "info", &format!("AI created {} valid parent categories", groupings.len()));

    // Get parent node to determine new depth
    let parent_node = db.get_node(parent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Parent node {} not found", parent_id))?;

    let new_intermediate_depth = parent_node.depth + 1;

    // Safety: prevent runaway depth explosion
    const MAX_HIERARCHY_DEPTH: i32 = 15;
    if new_intermediate_depth > MAX_HIERARCHY_DEPTH {
        emit_log(app, "warn", &format!(
            "Max hierarchy depth {} reached at parent '{}'. Stopping grouping to prevent explosion.",
            MAX_HIERARCHY_DEPTH, parent_id
        ));
        return Ok(false);
    }

    // Create map from label -> ALL child nodes with that label
    // For ITEMS: use ai_title or title (cluster_label is parent topic's label, not unique)
    // For CATEGORIES: use cluster_label, ai_title, or title
    let mut label_to_children: HashMap<String, Vec<&Node>> = HashMap::new();
    for child in &children {
        let label = if child.is_item {
            // Items: prefer ai_title (unique per item) over cluster_label (shared with siblings)
            child.ai_title
                .as_ref()
                .or(child.cluster_label.as_ref())
                .unwrap_or(&child.title)
                .clone()
        } else {
            // Categories: use cluster_label as primary
            child.cluster_label
                .as_ref()
                .or(child.ai_title.as_ref())
                .unwrap_or(&child.title)
                .clone()
        };
        label_to_children.entry(label).or_default().push(child);
    }

    // Generate unique timestamp suffix to avoid ID collisions across iterations
    let timestamp_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();

    // Create intermediate nodes and reparent children
    let mut categories_created = 0;
    let mut max_category_size = 0usize;
    let now = timestamp_suffix as i64;

    // Collect all children that need depth updates for batch processing
    let mut all_children_to_update: Vec<String> = Vec::new();
    // Track created category IDs to update their child_count after reparenting
    let mut created_category_ids: Vec<String> = Vec::new();

    for (idx, grouping) in groupings.iter().enumerate() {
        // Find ALL child nodes matching this grouping's labels
        // Deduplicate by node ID to handle duplicate labels in AI response
        let mut seen_ids = std::collections::HashSet::new();
        let matching_children: Vec<&Node> = grouping.children
            .iter()
            .flat_map(|label| label_to_children.get(label).cloned().unwrap_or_default())
            .filter(|node| seen_ids.insert(node.id.clone()))
            .collect();

        if matching_children.is_empty() {
            emit_log(app, "warn", &format!("Category '{}' has no matching children", grouping.name));
            continue;
        }

        // Track max category size for pathological detection
        max_category_size = max_category_size.max(matching_children.len());

        // Check for duplicate name: if a category with same name already exists under this parent, merge into it
        let existing_siblings = db.get_children(parent_id).map_err(|e| e.to_string())?;
        let existing_category = existing_siblings.iter()
            .find(|c| !c.is_item && c.title.to_lowercase() == grouping.name.to_lowercase());

        if let Some(existing) = existing_category {
            // Skip if trying to merge a category into itself (pathological case)
            let non_self_children: Vec<_> = matching_children.iter()
                .filter(|c| c.id != existing.id)
                .collect();

            if non_self_children.is_empty() {
                emit_log(app, "debug", &format!("Skipping self-referential merge for '{}'", existing.title));
                continue;
            }

            // Merge into existing category instead of creating duplicate
            emit_log(app, "info", &format!("Merging {} children into existing '{}'", non_self_children.len(), existing.title));

            for child in &non_self_children {
                db.update_parent(&child.id, &existing.id).map_err(|e| e.to_string())?;
                all_children_to_update.push(child.id.clone());
            }

            // Update child_count on existing category
            let new_count = existing.child_count + non_self_children.len() as i32;
            db.update_child_count(&existing.id, new_count).map_err(|e| e.to_string())?;
            continue; // Skip creating new node
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
            latest_child_date: None,
            is_private: None,
            privacy_reason: None,
            source: None,
            pdf_available: None,
            content_type: None,
            associated_idea_id: None,
            privacy: None,
        };

        db.insert_node(&category_node).map_err(|e| e.to_string())?;
        categories_created += 1;
        created_category_ids.push(category_id.clone());

        // Reparent children to this category and collect for batch depth update
        for child in &matching_children {
            db.update_parent(&child.id, &category_id).map_err(|e| e.to_string())?;
            all_children_to_update.push(child.id.clone());
        }
    }

    // Update child_count for all created categories to reflect ACTUAL children
    // (matching_children may have been reassigned to other categories due to label overlap)
    for category_id in &created_category_ids {
        let actual_count = db.get_children(category_id).map_err(|e| e.to_string())?.len() as i32;
        db.update_child_count(category_id, actual_count).map_err(|e| e.to_string())?;
    }

    // Compute centroid embeddings for new categories (enables similarity-based grouping)
    for category_id in &created_category_ids {
        compute_and_store_centroid(db, category_id);
    }
    if !created_category_ids.is_empty() {
        emit_log(app, "info", &format!("Computed centroid embeddings for {} categories", created_category_ids.len()));
    }

    // SET depths for reparented children to correct value (not increment!)
    // Children are now under categories at new_intermediate_depth, so they should be at new_intermediate_depth + 1
    // This prevents depth explosion from accumulated increments across multiple grouping iterations.
    if !all_children_to_update.is_empty() {
        let depth_start = std::time::Instant::now();
        let child_depth = new_intermediate_depth + 1;
        emit_log(app, "info", &format!("Setting depths for {} reparented nodes to {}...", all_children_to_update.len(), child_depth));
        db.set_reparented_nodes_depth(&all_children_to_update, child_depth).map_err(|e| e.to_string())?;
        let depth_elapsed = depth_start.elapsed().as_secs_f64();
        emit_log(app, "info", &format!("  Depth update completed in {:.1}s", depth_elapsed));
    }

    // Update parent's child count to actual count (not just categories_created,
    // since some children may have been merged into existing categories)
    let actual_child_count = db.get_children(parent_id).map_err(|e| e.to_string())?.len() as i32;
    db.update_child_count(parent_id, actual_child_count)
        .map_err(|e| e.to_string())?;

    emit_log(app, "info", &format!("Created {} intermediate categories under {} (total children: {})",
        categories_created, parent_id, actual_child_count));

    // Detect pathological grouping - if grouping didn't actually split children meaningfully
    let original_child_count = children.len();
    let reparented_count = all_children_to_update.len();

    if categories_created == 0 && reparented_count == 0 {
        // Nothing happened - likely because children are already the categories AI would create
        emit_log(app, "info", &format!(
            "Node {} already has {} organized categories - nothing to split. Try drilling into a category to split its contents.",
            parent_id, original_child_count
        ));
        return Ok(false);
    }

    if categories_created <= 1 && reparented_count > 0 {
        // Only created 0 or 1 categories but did reparent some - grouping failed to split
        emit_log(app, "warn", &format!(
            "Grouping incomplete for {}: {} children → {} categories. AI labels didn't match topic labels.",
            parent_id, original_child_count, categories_created
        ));
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

    // Check if one category dominates (>80% of children)
    // This indicates grouping didn't really split the problem
    if max_category_size > original_child_count * 4 / 5 {
        emit_log(app, "warn", &format!(
            "Grouping ineffective for {}: largest category has {}/{} children (>80%). Stopping recursion.",
            parent_id, max_category_size, original_child_count
        ));
        return Ok(false);
    }

    Ok(true)
}

// ============================================================================
// EMBEDDING-BASED CLUSTERING FOR ITEM-ONLY CONTAINERS
// ============================================================================

/// Calculate target cluster count based on dataset size
/// Scales cap higher for large datasets to avoid re-subdivision
fn calculate_target_k(n: usize) -> usize {
    if n < 2 {
        return n;
    }
    let sqrt_n = (n as f64).sqrt() as usize;

    // Scale cap based on dataset size to avoid re-subdivision
    // Target: ~40-50 items per cluster for large datasets
    let cap = if n > 1000 { 35 }      // 1000+ items: up to 35 clusters
              else if n > 500 { 25 }   // 500-1000: up to 25 clusters
              else if n > 200 { 20 }   // 200-500: up to 20 clusters
              else { 15 };             // <200: up to 15 clusters

    sqrt_n.max(8).min(cap).min(n)
}

/// K-means clustering on embeddings
/// Returns Vec<Vec<String>> - each inner vec is item IDs in that cluster
fn kmeans_cluster(
    embeddings: &[(String, Vec<f32>)],
    k: usize,
) -> Result<Vec<Vec<String>>, String> {
    if embeddings.is_empty() || k == 0 {
        return Ok(vec![]);
    }

    let n = embeddings.len();
    let dim = embeddings[0].1.len();

    if dim == 0 {
        return Err("Embeddings have zero dimensions".into());
    }

    // k-means++ initialization
    let mut centroids = kmeans_plusplus_init(embeddings, k);

    let mut assignments: Vec<usize> = vec![0; n];
    let max_iterations = 50;

    for _ in 0..max_iterations {
        // Assign each point to nearest centroid
        let mut changed = false;
        for (i, (_, emb)) in embeddings.iter().enumerate() {
            let mut best_cluster = 0;
            let mut best_sim = f32::MIN;

            for (c, centroid) in centroids.iter().enumerate() {
                let sim = cosine_similarity(emb, centroid);
                if sim > best_sim {
                    best_sim = sim;
                    best_cluster = c;
                }
            }

            if assignments[i] != best_cluster {
                assignments[i] = best_cluster;
                changed = true;
            }
        }

        if !changed {
            break;
        }

        // Recompute centroids
        centroids = recompute_centroids(embeddings, &assignments, k, dim);
    }

    // Group item IDs by cluster
    let mut clusters: Vec<Vec<String>> = vec![vec![]; k];
    for (i, (id, _)) in embeddings.iter().enumerate() {
        clusters[assignments[i]].push(id.clone());
    }

    // Remove empty clusters
    clusters.retain(|c| !c.is_empty());

    Ok(clusters)
}

/// k-means++ initialization for better starting centroids
fn kmeans_plusplus_init(embeddings: &[(String, Vec<f32>)], k: usize) -> Vec<Vec<f32>> {
    let mut rng = rand::thread_rng();
    let n = embeddings.len();

    if n == 0 || k == 0 {
        return vec![];
    }

    // First centroid: random
    let mut centroids = vec![embeddings[rng.gen_range(0..n)].1.clone()];

    // Remaining centroids: probability proportional to distance squared
    while centroids.len() < k && centroids.len() < n {
        let mut distances: Vec<f32> = Vec::with_capacity(n);

        for (_, emb) in embeddings {
            // Distance to nearest existing centroid
            let min_dist = centroids.iter()
                .map(|c| 1.0 - cosine_similarity(emb, c))
                .fold(f32::MAX, |a, b| a.min(b));
            distances.push(min_dist * min_dist);
        }

        // Weighted random selection
        let total: f32 = distances.iter().sum();
        if total <= 0.0 {
            // All points are identical to existing centroids
            break;
        }

        let threshold = rng.gen::<f32>() * total;
        let mut cumsum = 0.0;

        for (i, d) in distances.iter().enumerate() {
            cumsum += d;
            if cumsum >= threshold {
                centroids.push(embeddings[i].1.clone());
                break;
            }
        }
    }

    centroids
}

fn recompute_centroids(
    embeddings: &[(String, Vec<f32>)],
    assignments: &[usize],
    k: usize,
    dim: usize,
) -> Vec<Vec<f32>> {
    let mut rng = rand::thread_rng();

    let mut centroids: Vec<Vec<f32>> = vec![vec![0.0; dim]; k];
    let mut counts: Vec<usize> = vec![0; k];

    for (i, (_, emb)) in embeddings.iter().enumerate() {
        let c = assignments[i];
        counts[c] += 1;
        for (j, val) in emb.iter().enumerate() {
            centroids[c][j] += val;
        }
    }

    // Average first, then normalize
    for c in 0..k {
        if counts[c] > 0 {
            // Step 1: Average (divide by count)
            for j in 0..dim {
                centroids[c][j] /= counts[c] as f32;
            }
            // Step 2: Normalize to unit vector
            let norm: f32 = centroids[c].iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for j in 0..dim {
                    centroids[c][j] /= norm;
                }
            }
        } else {
            // Empty cluster: reinitialize to random point to avoid NaN
            let random_idx = rng.gen_range(0..embeddings.len());
            centroids[c] = embeddings[random_idx].1.clone();
        }
    }

    centroids
}

/// Extract common keywords from a list of titles
fn find_common_keywords(titles: &[String]) -> Vec<String> {
    let stop_words: HashSet<&str> = [
        "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for",
        "of", "with", "by", "from", "as", "is", "was", "are", "were", "be",
        "been", "being", "have", "has", "had", "do", "does", "did", "will",
        "would", "could", "should", "may", "might", "must", "shall", "can",
        "this", "that", "these", "those", "it", "its", "i", "you", "we", "they",
        "my", "your", "our", "their", "what", "which", "who", "whom", "how",
    ].iter().cloned().collect();

    let mut word_counts: HashMap<String, usize> = HashMap::new();

    for title in titles {
        // Split into words, normalize
        let words: Vec<String> = title
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 2 && !stop_words.contains(w))
            .map(|w| w.to_string())
            .collect();

        // Count unique words per title (not total occurrences)
        let unique: HashSet<_> = words.into_iter().collect();
        for word in unique {
            *word_counts.entry(word).or_insert(0) += 1;
        }
    }

    // Find words appearing in >50% of titles
    let threshold = titles.len() / 2;
    let mut common: Vec<(String, usize)> = word_counts
        .into_iter()
        .filter(|(_, count)| *count > threshold)
        .collect();

    // Sort by frequency descending
    common.sort_by(|a, b| b.1.cmp(&a.1));

    common.into_iter().map(|(word, _)| word).collect()
}

/// Generate a name for a cluster of items using AI
async fn name_cluster_from_items(
    items: &[&Node],
    forbidden_names: &[String],
    app: Option<&AppHandle>,
) -> Result<String, String> {
    // Sample 8-10 items (diverse: first, middle, last, spread)
    let sample_size = items.len().min(10);
    let mut sample_indices: Vec<usize> = Vec::new();

    if items.len() <= 10 {
        sample_indices = (0..items.len()).collect();
    } else {
        // First, last, middle, then spread
        sample_indices.push(0);
        sample_indices.push(items.len() - 1);
        sample_indices.push(items.len() / 2);

        let step = items.len() / (sample_size - 3).max(1);
        for i in (step..items.len()).step_by(step) {
            if sample_indices.len() < sample_size && !sample_indices.contains(&i) {
                sample_indices.push(i);
            }
        }
    }

    // Build titles list
    let titles: Vec<String> = sample_indices.iter()
        .map(|&i| items[i].ai_title.clone()
            .or_else(|| items[i].cluster_label.clone())
            .unwrap_or_else(|| items[i].title.clone()))
        .collect();

    // Try AI naming first
    match crate::ai_client::name_cluster_from_samples(&titles, forbidden_names).await {
        Ok(name) if !is_garbage_name(&name) => return Ok(name),
        Ok(_) => {} // Garbage name, fall through
        Err(e) => {
            emit_log(app, "warn", &format!("AI naming failed: {}", e));
        }
    }

    // Fallback: extract common words
    let common = find_common_keywords(&titles);
    if !common.is_empty() {
        return Ok(common.into_iter().take(3).collect::<Vec<_>>().join(" "));
    }

    // Ultimate fallback
    Ok(format!("Group ({} items)", items.len()))
}

/// Cluster items by embedding similarity when all children are items
/// Returns true if grouping was successful
async fn cluster_items_by_embedding(
    db: &Database,
    parent_id: &str,
    items: &[Node],
    app: Option<&AppHandle>,
) -> Result<bool, String> {
    let item_count = items.len();
    emit_log(app, "info", &format!(
        "Using embedding clustering for {} items under {}",
        item_count, parent_id
    ));

    // 1. Fetch embeddings for all items
    let mut embeddings: Vec<(String, Vec<f32>)> = Vec::new();
    for item in items {
        if let Ok(Some(emb)) = db.get_node_embedding(&item.id) {
            embeddings.push((item.id.clone(), emb));
        }
    }

    if embeddings.len() < items.len() / 2 {
        emit_log(app, "warn", &format!(
            "Only {}/{} items have embeddings, falling back to AI grouping",
            embeddings.len(), items.len()
        ));
        return Ok(false); // Fall through to AI-based grouping
    }

    // 2. Determine target cluster count
    let target_k = calculate_target_k(embeddings.len());
    emit_log(app, "info", &format!("Target {} clusters for {} items with embeddings", target_k, embeddings.len()));

    // 3. Cluster embeddings using k-means
    let clusters = kmeans_cluster(&embeddings, target_k)?;

    if clusters.len() < 2 {
        emit_log(app, "warn", "Clustering produced < 2 groups, falling back to AI");
        return Ok(false);
    }

    // 4. Name each cluster using AI
    let parent_node = db.get_node(parent_id).map_err(|e| e.to_string())?
        .ok_or("Parent not found")?;
    let forbidden_names = db.get_all_category_names().map_err(|e| e.to_string())?;

    let mut categories_created = 0;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let now = timestamp as i64;
    let new_depth = parent_node.depth + 1;
    let child_depth = new_depth + 1;

    let mut all_reparented: Vec<String> = Vec::new();
    let mut created_category_ids: Vec<String> = Vec::new();
    let mut max_category_size = 0usize;

    for (idx, cluster_item_ids) in clusters.iter().enumerate() {
        if cluster_item_ids.is_empty() {
            continue;
        }

        max_category_size = max_category_size.max(cluster_item_ids.len());

        // Get items in this cluster
        let cluster_items: Vec<&Node> = cluster_item_ids.iter()
            .filter_map(|id| items.iter().find(|i| &i.id == id))
            .collect();

        // Generate name from cluster items
        let name = name_cluster_from_items(&cluster_items, &forbidden_names, app).await?;

        emit_log(app, "info", &format!(
            "  Cluster {}: \"{}\" ({} items)",
            idx, name, cluster_item_ids.len()
        ));

        // Create category node
        let category_id = format!("{}-emb-{}-{}", parent_id, timestamp, idx);
        let category_node = Node {
            id: category_id.clone(),
            node_type: NodeType::Cluster,
            title: name.clone(),
            cluster_label: Some(name.clone()),
            depth: new_depth,
            is_item: false,
            is_universe: false,
            parent_id: Some(parent_id.to_string()),
            child_count: cluster_item_ids.len() as i32,
            created_at: now,
            updated_at: now,
            ai_title: None,
            summary: None,
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
            cluster_id: None,
            position: Position { x: 0.0, y: 0.0 },
            url: None,
            content: None,
        };

        db.insert_node(&category_node).map_err(|e| e.to_string())?;
        created_category_ids.push(category_id.clone());
        categories_created += 1;

        // Reparent items to this category
        for item_id in cluster_item_ids {
            db.update_parent(item_id, &category_id).map_err(|e| e.to_string())?;
            all_reparented.push(item_id.clone());
        }
    }

    // 5. Set correct depths (not increment!)
    if !all_reparented.is_empty() {
        emit_log(app, "info", &format!("Setting depths for {} reparented items to {}", all_reparented.len(), child_depth));
        db.set_reparented_nodes_depth(&all_reparented, child_depth).map_err(|e| e.to_string())?;
    }

    // 6. Update child counts
    for category_id in &created_category_ids {
        let actual = db.get_children(category_id).map_err(|e| e.to_string())?.len() as i32;
        db.update_child_count(category_id, actual).map_err(|e| e.to_string())?;
    }

    // 7. Compute centroid embeddings for new categories (enables similarity-based grouping)
    for category_id in &created_category_ids {
        compute_and_store_centroid(db, category_id);
    }
    if !created_category_ids.is_empty() {
        emit_log(app, "info", &format!("Computed centroid embeddings for {} categories", created_category_ids.len()));
    }

    // Update parent's child count
    let parent_children = db.get_children(parent_id).map_err(|e| e.to_string())?.len() as i32;
    db.update_child_count(parent_id, parent_children).map_err(|e| e.to_string())?;

    emit_log(app, "info", &format!(
        "Created {} embedding-based categories under {} (largest: {} items)",
        categories_created, parent_id, max_category_size
    ));

    // Check if one category dominates (>80% of items) - same check as AI grouping
    if max_category_size > item_count * 4 / 5 {
        emit_log(app, "warn", &format!(
            "Embedding clustering ineffective: largest category has {}/{} items (>80%)",
            max_category_size, item_count
        ));
        return Ok(false);
    }

    Ok(categories_created >= 2)
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
    let total_start = std::time::Instant::now();

    emit_log(app, "info", "═══════════════════════════════════════════════════════════");
    emit_log(app, "info", "Starting Full Hierarchy Build");
    emit_log(app, "info", "═══════════════════════════════════════════════════════════");

    // Emit initial progress
    emit_progress(app, AiProgressEvent {
        current: 0,
        total: 7,
        node_title: "Starting rebuild...".to_string(),
        new_title: "Preparing hierarchy build".to_string(),
        emoji: Some("🚀".to_string()),
        status: "processing".to_string(),
        error_message: None,
        elapsed_secs: Some(0.0),
        estimate_secs: None,
        remaining_secs: None,
    });

    // Step 0: Clean up incomplete items (queries with no Claude response)
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ Cleanup: Removing incomplete conversations");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    let deleted_incomplete = db.delete_incomplete_conversations().map_err(|e| e.to_string())?;
    if deleted_incomplete > 0 {
        emit_log(app, "info", &format!("✓ Deleted {} incomplete items (queries with no response)", deleted_incomplete));
    } else {
        emit_log(app, "info", "No incomplete items found");
    }

    // Step 1: Classify content types BEFORE clustering
    // This ensures supporting items (code/debug/paste) are excluded from clustering
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ STEP 1/7: Classifying content types");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_progress(app, AiProgressEvent {
        current: 1,
        total: 7,
        node_title: "Step 1/7: Classification".to_string(),
        new_title: "Classifying content types...".to_string(),
        emoji: Some("🏷️".to_string()),
        status: "processing".to_string(),
        error_message: None,
        elapsed_secs: Some(total_start.elapsed().as_secs_f64()),
        estimate_secs: None,
        remaining_secs: None,
    });
    emit_log(app, "info", "Classifying items as idea/code/debug/paste...");
    let classified_count = classification::classify_all_items(db)?;
    emit_log(app, "info", &format!("✓ Classified {} items by content type", classified_count));

    // Step 2: Optionally run clustering (only on ideas, not supporting items)
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ STEP 2/7: Clustering ideas into topics");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_progress(app, AiProgressEvent {
        current: 2,
        total: 7,
        node_title: "Step 2/7: Clustering".to_string(),
        new_title: if run_clustering { "Clustering ideas into topics..." } else { "Skipping (using existing clusters)" }.to_string(),
        emoji: Some("🧩".to_string()),
        status: "processing".to_string(),
        error_message: None,
        elapsed_secs: Some(total_start.elapsed().as_secs_f64()),
        estimate_secs: None,
        remaining_secs: None,
    });
    let clustering_result = if run_clustering && ai_client::is_available() {
        emit_log(app, "info", "Running AI clustering on idea items (excluding code/debug/paste)...");
        let result = crate::clustering::run_clustering(db, true).await?;
        emit_log(app, "info", &format!("✓ Clustering complete: {} ideas → {} clusters", result.items_assigned, result.clusters_created));
        Some(result)
    } else {
        emit_log(app, "info", "Skipping (already done or AI not available)");
        None
    };

    // Bootstrap persistent tags from item vocabulary (one-time, if tags table is empty)
    match crate::tags::generate_tags_from_item_vocabulary(db) {
        Ok(0) => {}, // Tags already exist or no item tags
        Ok(n) => emit_log(app, "info", &format!("✓ Bootstrapped {} persistent tags from vocabulary", n)),
        Err(e) => emit_log(app, "warn", &format!("Tag bootstrap failed (non-fatal): {}", e)),
    }

    // Step 3: Build initial hierarchy
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ STEP 3/7: Building initial hierarchy structure");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_progress(app, AiProgressEvent {
        current: 3,
        total: 7,
        node_title: "Step 3/7: Building hierarchy".to_string(),
        new_title: "Creating Universe and topic nodes...".to_string(),
        emoji: Some("🏗️".to_string()),
        status: "processing".to_string(),
        error_message: None,
        elapsed_secs: Some(total_start.elapsed().as_secs_f64()),
        estimate_secs: None,
        remaining_secs: None,
    });
    emit_log(app, "info", "Creating Universe and topic nodes from clusters...");
    let hierarchy_result = build_hierarchy(db)?;
    emit_log(app, "info", &format!("✓ Created {} intermediate nodes, organized {} items", hierarchy_result.intermediate_nodes_created, hierarchy_result.items_organized));

    // Step 4: Detect and create project umbrellas
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ STEP 4/7: Creating project umbrella categories");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_progress(app, AiProgressEvent {
        current: 4,
        total: 7,
        node_title: "Step 4/7: Project detection".to_string(),
        new_title: "Detecting major projects...".to_string(),
        emoji: Some("📦".to_string()),
        status: "processing".to_string(),
        error_message: None,
        elapsed_secs: Some(total_start.elapsed().as_secs_f64()),
        estimate_secs: None,
        remaining_secs: None,
    });
    emit_log(app, "info", "Collecting capitalized words from titles...");

    // Step 1: Collect ALL capitalized words (very loose, 5+ occurrences)
    let candidates = ai_client::collect_capitalized_words(db);
    let mut project_umbrellas_created = 0;

    if candidates.is_empty() {
        emit_log(app, "info", "No capitalized words found (need 5+ occurrences)");
    } else {
        emit_log(app, "info", &format!("Found {} capitalized words, asking AI to detect projects...", candidates.len()));

        // Show top 10 candidates
        for candidate in candidates.iter().take(10) {
            emit_log(app, "info", &format!("  {} ({} occurrences)", candidate.word, candidate.count));
        }
        if candidates.len() > 10 {
            emit_log(app, "info", &format!("  ... and {} more", candidates.len() - 10));
        }

        // Step 2: AI detects which are actual user projects
        let major_projects = ai_client::detect_projects_with_ai(db, candidates).await;

        if major_projects.is_empty() {
            emit_log(app, "info", "AI detected no user projects");
        } else {
            emit_log(app, "info", &format!("AI detected {} user projects:", major_projects.len()));
            for project in &major_projects {
                emit_log(app, "info", &format!("  ✓ {} ({} items, {:.1}%)", project.name, project.item_count, project.percentage));
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
                latest_child_date: None,
                is_private: None,
                privacy_reason: None,
                source: None,
                pdf_available: None,
                content_type: None,
                associated_idea_id: None,
                privacy: None,
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
        } // end inner else (AI validated projects)
    } // end outer else (candidate_projects not empty)

    if project_umbrellas_created > 0 {
        emit_log(app, "info", &format!("✓ Created {} project umbrella categories", project_umbrellas_created));
    }

    // Step 4.5: Consolidate Universe into uber-categories if it has too many direct children
    let universe = db.get_universe().map_err(|e| e.to_string())?
        .ok_or("No Universe found")?;
    let universe_children = db.get_children(&universe.id).map_err(|e| e.to_string())?;
    let mut uber_categories_created = 0;
    let universe_max = max_children_for_depth(0); // L0 = 10

    if universe_children.len() > universe_max {
        emit_log(app, "info", "");
        emit_log(app, "info", &format!("▶ Consolidating Universe ({} direct children > {}, grouping into uber-categories)", universe_children.len(), universe_max));
        emit_log(app, "info", "───────────────────────────────────────────────────────────");

        // Build TopicInfo for uber-category grouping
        // IMPORTANT: Filter out project-* and category-personal nodes - they should stay at depth 1
        let categories: Vec<ai_client::TopicInfo> = universe_children.iter()
            .filter(|c| !c.id.starts_with("project-") && c.id != "category-personal")
            .map(|c| ai_client::TopicInfo {
                id: c.id.clone(),
                label: c.cluster_label.clone()
                    .or(c.ai_title.clone())
                    .unwrap_or_else(|| c.title.clone()),
                item_count: c.child_count.max(1),
            })
            .collect();

        // Get persistent tag anchors for uber-category hints
        let tag_anchors = crate::tags::get_tag_anchors(db);
        if !tag_anchors.is_empty() {
            emit_log(app, "info", &format!("  Using {} persistent tags as category anchors", tag_anchors.len()));
        }

        // Pre-fetch embeddings for similarity-sorted batching
        let embeddings_map: std::collections::HashMap<String, Vec<f32>> = categories
            .iter()
            .filter_map(|c| {
                db.get_node_embedding(&c.id)
                    .ok()
                    .flatten()
                    .map(|emb| (c.id.clone(), emb))
            })
            .collect();
        emit_log(app, "info", &format!("  Fetched {}/{} topic embeddings for similarity sorting", embeddings_map.len(), categories.len()));

        // Call AI to group into uber-categories (similarity-sorted batching for coherent groups)
        match ai_client::group_into_uber_categories(&categories, &embeddings_map, Some(&tag_anchors)).await {
            Ok(groupings) if !groupings.is_empty() => {
                // Filter out garbage names from uber-categories (same pattern as cluster_hierarchy_level)
                let groupings: Vec<_> = groupings.into_iter()
                    .filter(|g| !is_garbage_name(&g.name))
                    .collect();

                if groupings.is_empty() {
                    emit_log(app, "warn", "  All uber-categories were garbage names, skipping consolidation");
                } else {
                emit_log(app, "info", &format!("  AI created {} uber-categories (after filtering)", groupings.len()));

                // Create map from label -> child nodes
                let mut label_to_children: std::collections::HashMap<String, Vec<&Node>> = std::collections::HashMap::new();
                for child in &universe_children {
                    let label = child.cluster_label.as_ref()
                        .or(child.ai_title.as_ref())
                        .unwrap_or(&child.title)
                        .clone();
                    label_to_children.entry(label).or_default().push(child);
                }

                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis();
                let now = timestamp as i64;

                for (idx, grouping) in groupings.iter().enumerate() {
                    // Find matching children
                    let matching_children: Vec<&Node> = grouping.children.iter()
                        .flat_map(|label| label_to_children.get(label).cloned().unwrap_or_default())
                        .collect();

                    if matching_children.len() < 2 {
                        continue; // Skip single-child groups
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

                    db.insert_node(&uber_node).map_err(|e| e.to_string())?;

                    // Reparent children to the uber-category (skip project/personal nodes - safety check)
                    let reparentable: Vec<_> = matching_children.iter()
                        .filter(|c| !c.id.starts_with("project-") && c.id != "category-personal")
                        .collect();

                    for child in &reparentable {
                        db.update_parent(&child.id, &uber_id).map_err(|e| e.to_string())?;
                    }

                    // Increment depths of reparented subtrees
                    let child_ids: Vec<String> = reparentable.iter().map(|c| c.id.clone()).collect();
                    db.increment_multiple_subtrees_depth(&child_ids).map_err(|e| e.to_string())?;

                    uber_categories_created += 1;
                    emit_log(app, "info", &format!("  Created '{}' with {} children", grouping.name, matching_children.len()));
                }

                emit_log(app, "info", &format!("✓ Consolidated into {} uber-categories", uber_categories_created));
                } // end if groupings not empty after filtering
            }
            Ok(_) => {
                emit_log(app, "warn", "  AI returned no uber-categories, skipping consolidation");
            }
            Err(e) => {
                emit_log(app, "warn", &format!("  Uber-category grouping failed: {}", e));
            }
        }
    }

    // Step 5: Recursively group levels with too many children
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ STEP 5/7: Recursive grouping (tiered limits: L0-1=10, L2=25, L3=50, L4=100)");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    let mut grouping_iterations = 0;
    let max_iterations = 50; // Safety limit
    let mut failed_nodes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let grouping_start = std::time::Instant::now();

    // Emit initial progress for Step 5
    emit_progress(app, AiProgressEvent {
        current: 5,
        total: 7,
        node_title: "Step 5/7: Recursive grouping".to_string(),
        new_title: "Analyzing structure for grouping...".to_string(),
        emoji: Some("🔄".to_string()),
        status: "processing".to_string(),
        error_message: None,
        elapsed_secs: Some(total_start.elapsed().as_secs_f64()),
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

                // Emit progress event for this iteration
                emit_progress(app, AiProgressEvent {
                    current: 5,
                    total: 7,
                    node_title: format!("Step 5/7: Grouping (iter {})", grouping_iterations + 1),
                    new_title: format!("Organizing {}...", node_name),
                    emoji: Some("🧠".to_string()),
                    status: "processing".to_string(),
                    error_message: None,
                    elapsed_secs: Some(total_start.elapsed().as_secs_f64()),
                    estimate_secs: None,
                    remaining_secs: None,
                });

                emit_log(app, "info", &format!("  Iteration {}: Grouping children of {}", grouping_iterations + 1, node_id));
                let iter_start = std::time::Instant::now();
                let grouped = cluster_hierarchy_level(db, &node_id, app, None, false).await?;
                let iter_elapsed = iter_start.elapsed().as_secs_f64();
                emit_log(app, "info", &format!("  Iteration {} completed in {:.1}s", grouping_iterations + 1, iter_elapsed));
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

    // Log Step 5 completion time
    let grouping_elapsed = grouping_start.elapsed().as_secs_f64();
    emit_log(app, "info", &format!("  Step 5 completed in {:.1}s ({} iterations)", grouping_elapsed, grouping_iterations));

    // Recalculate final depth
    let final_max_depth = db.get_max_depth().map_err(|e| e.to_string())?;
    emit_log(app, "info", &format!("  Final hierarchy depth: {}", final_max_depth));

    // Step 6: Generate embeddings for ALL nodes that need them (if OpenAI key available)
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ STEP 6/7: Generating embeddings for semantic search");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_progress(app, AiProgressEvent {
        current: 6,
        total: 7,
        node_title: "Step 6/7: Embeddings".to_string(),
        new_title: "Generating semantic embeddings...".to_string(),
        emoji: Some("🔢".to_string()),
        status: "processing".to_string(),
        error_message: None,
        elapsed_secs: Some(total_start.elapsed().as_secs_f64()),
        estimate_secs: None,
        remaining_secs: None,
    });
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
                // Use content for embeddings (more semantically meaningful)
                let embed_text = if let Some(content) = &node.content {
                    safe_truncate(content, 1000).to_string()
                } else {
                    // Fallback for nodes without content
                    let summary = node.summary.as_deref().unwrap_or("");
                    format!("{} {}", title, summary)
                };

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

                // Emit processing event with step info
                emit_progress(app, AiProgressEvent {
                    current: 6,
                    total: 7,
                    node_title: format!("Step 6/7: Embeddings ({}/{})", current, total_needing),
                    new_title: format!("Processing {}...", safe_truncate(title, 30)),
                    emoji: None,
                    status: "processing".to_string(),
                    error_message: None,
                    elapsed_secs: Some(total_start.elapsed().as_secs_f64()),
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

            emit_log(app, "info", &format!("✓ Embeddings complete: {} generated, {} skipped", generated, skipped));
            (generated, skipped)
        }
    } else {
        emit_log(app, "info", "Skipping (OpenAI API key not set)");
        (0, 0)
    };

    // Create semantic edges based on embedding similarity (part of Step 6)
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

    // ═══════════════════════════════════════════════════════════════════════════
    // STEP 7/7: Associate supporting items with ideas
    // ═══════════════════════════════════════════════════════════════════════════
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ STEP 7/7: Associating supporting items with ideas");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_progress(app, AiProgressEvent {
        current: 7,
        total: 7,
        node_title: "Step 7/7: Associations".to_string(),
        new_title: "Linking supporting items to ideas...".to_string(),
        emoji: Some("🔗".to_string()),
        status: "processing".to_string(),
        error_message: None,
        elapsed_secs: Some(total_start.elapsed().as_secs_f64()),
        estimate_secs: None,
        remaining_secs: None,
    });
    emit_log(app, "info", "Linking code/debug/paste items to their related ideas...");
    let associations_created = classification::compute_all_associations(db)?;
    emit_log(app, "info", &format!("✓ Associated {} supporting items with ideas", associations_created));

    // Final step: Propagate latest dates from leaves up through hierarchy
    emit_log(app, "info", "");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_log(app, "info", "Propagating latest dates...");

    if let Err(e) = db.propagate_latest_dates() {
        emit_log(app, "warn", &format!("  Failed to propagate dates: {}", e));
    } else {
        emit_log(app, "info", "  ✓ Latest dates propagated to all nodes");
    }

    // Propagate privacy scores from items up to categories
    emit_log(app, "info", "");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    let privacy_propagated = match propagate_privacy_scores(db, app) {
        Ok(count) => count,
        Err(e) => {
            emit_log(app, "warn", &format!("  Failed to propagate privacy: {}", e));
            0
        }
    };

    let total_elapsed = total_start.elapsed().as_secs_f64();
    emit_log(app, "info", "");
    emit_log(app, "info", "═══════════════════════════════════════════════════════════");
    emit_log(app, "info", "✓ HIERARCHY BUILD COMPLETE");
    emit_log(app, "info", &format!("  • {} items classified", classified_count));
    emit_log(app, "info", &format!("  • {} supporting items associated", associations_created));
    emit_log(app, "info", &format!("  • {} grouping iterations", grouping_iterations));
    emit_log(app, "info", &format!("  • {} hierarchy levels (depth 0-{})", final_max_depth + 1, final_max_depth));
    emit_log(app, "info", &format!("  • {} semantic edges", semantic_edges_created));
    emit_log(app, "info", &format!("  • {} categories with propagated privacy", privacy_propagated));
    emit_log(app, "info", &format!("  • Total time: {:.1}s", total_elapsed));
    emit_log(app, "info", "═══════════════════════════════════════════════════════════");

    // Emit final complete event
    emit_progress(app, AiProgressEvent {
        current: 7,
        total: 7,
        node_title: "Complete".to_string(),
        new_title: format!("Built in {:.0}s", total_elapsed),
        emoji: Some("✓".to_string()),
        status: "complete".to_string(),
        error_message: None,
        elapsed_secs: Some(total_elapsed),
        estimate_secs: Some(total_elapsed),
        remaining_secs: Some(0.0),
    });

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

            // Skip nodes whose parent is ungroupable - they likely inherited the same problem
            // This prevents the "bounce to child" loop where AI creates 1 category with all items
            if let Ok(Some(node)) = db.get_node(&node_id) {
                if let Some(ref parent_id) = node.parent_id {
                    if skip_nodes.contains(parent_id) {
                        emit_log(app, "debug", &format!(
                            "Skipping {} - parent {} was ungroupable (inherited problem)",
                            node_id, parent_id
                        ));
                        let children = db.get_children(&node_id).map_err(|e| e.to_string())?;
                        for child in children {
                            if !child.is_item {
                                queue.push((child.id, depth + 1));
                            }
                        }
                        continue;
                    }
                }
            }

            let children = db.get_children(&node_id).map_err(|e| e.to_string())?;
            let child_count = children.len();
            let max_for_this_depth = max_children_for_depth(depth);

            // Check if this node has too many children (items OR groups - fixes mega-topic bug)
            if child_count > max_for_this_depth {
                let non_item_count = children.iter().filter(|c| !c.is_item).count();

                // L5+ coherence gate: don't split incoherent noise deeper
                if depth >= 4 && !is_coherent_for_deep_split(&children) {
                    emit_log(app, "debug", &format!(
                        "Skipping {} (depth {}, {} children) - incoherent for L5 split",
                        node_id, depth, child_count
                    ));
                } else {
                    // Items CAN be grouped by creating subcategories and reparenting
                    emit_log(app, "debug", &format!(
                        "Found node needing grouping: {} (depth {}, {} children, {} non-items, max {})",
                        node_id, depth, child_count, non_item_count, max_for_this_depth
                    ));
                    return Ok(Some(node_id));
                }
            }

            // Add non-item children to queue for BFS traversal
            for child in &children {
                if !child.is_item {
                    queue.push((child.id.clone(), depth + 1));
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

/// Propagate privacy scores from items up through the category hierarchy
///
/// Category privacy = minimum of children's privacy scores (most restrictive wins)
/// This ensures a category is only as public as its most private child.
pub fn propagate_privacy_scores(db: &Database, app: Option<&tauri::AppHandle>) -> Result<usize, String> {
    emit_log(app, "info", "Propagating privacy scores to categories...");

    let max_depth = db.get_max_depth().map_err(|e| e.to_string())?;
    let mut categories_updated = 0;

    // Bottom-up: start from deepest level (above items) and work to root
    // Items are at max_depth, so categories start at max_depth - 1
    for depth in (0..max_depth).rev() {
        let nodes_at_depth = db.get_nodes_at_depth(depth).map_err(|e| e.to_string())?;

        for node in nodes_at_depth {
            // Skip items (they have their own scores from AI)
            if node.is_item {
                continue;
            }

            // Get children's privacy scores
            let children = db.get_children(&node.id).map_err(|e| e.to_string())?;
            if children.is_empty() {
                continue;
            }

            // Category privacy = min of children (most restrictive wins)
            // Only consider children that have privacy scores
            let min_privacy: Option<f64> = children.iter()
                .filter_map(|c| c.privacy)
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

            if let Some(privacy) = min_privacy {
                if let Err(e) = db.update_privacy_score(&node.id, privacy) {
                    emit_log(app, "warn", &format!("Failed to update privacy for {}: {}", node.id, e));
                } else {
                    categories_updated += 1;
                }
            }
        }
    }

    emit_log(app, "info", &format!("✓ Privacy propagated to {} categories", categories_updated));
    Ok(categories_updated)
}
