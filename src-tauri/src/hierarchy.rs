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

use crate::db::{Database, Edge, EdgeType, Node, NodeType, Position};
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
use crate::utils::safe_truncate;

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

// ==================== Coherence-Based Hierarchy Refinement ====================

/// Compute intra-category coherence: how tight are children around their centroid?
/// Returns mean similarity of children to centroid (0.0 = scattered, 1.0 = tight cluster)
/// O(n) complexity - NOT O(n^2) pairwise comparison
fn compute_category_coherence(db: &Database, category_id: &str) -> Result<Option<f32>, String> {
    let children = db.get_children(category_id).map_err(|e| e.to_string())?;
    if children.len() < 2 {
        return Ok(Some(1.0)); // Single child trivially coherent
    }

    // Get centroid embedding (categories have pre-computed centroids)
    let centroid = match db.get_node_embedding(category_id).map_err(|e| e.to_string())? {
        Some(c) => c,
        None => return Ok(None), // No centroid = can't compute
    };

    // Measure each child's similarity to centroid (O(n), not O(n^2))
    let mut total_sim = 0.0f32;
    let mut count = 0usize;

    for child in &children {
        if let Some(child_emb) = db.get_node_embedding(&child.id).map_err(|e| e.to_string())? {
            total_sim += cosine_similarity(&centroid, &child_emb);
            count += 1;
        }
    }

    if count < 2 {
        return Ok(None);
    }

    // Coherence = mean similarity to centroid
    // Tight cluster = all children close to centroid = high coherence
    Ok(Some(total_sim / count as f32))
}

/// Find sibling categories that should be merged (sim >= threshold)
fn find_mergeable_siblings(
    db: &Database,
    parent_id: &str,
    threshold: f32,
) -> Result<Vec<(String, String, f32)>, String> {
    let siblings = db.get_children(parent_id).map_err(|e| e.to_string())?;
    let categories: Vec<&Node> = siblings.iter().filter(|n| !n.is_item).collect();

    if categories.len() < 2 {
        return Ok(vec![]);
    }

    let embeddings: Vec<(String, Vec<f32>)> = categories.iter()
        .filter_map(|c| db.get_node_embedding(&c.id).ok().flatten()
            .map(|emb| (c.id.clone(), emb)))
        .collect();

    let mut candidates = Vec::new();
    let n = embeddings.len();

    for i in 0..n {
        for j in (i + 1)..n {
            let sim = cosine_similarity(&embeddings[i].1, &embeddings[j].1);
            if sim >= threshold {
                candidates.push((embeddings[i].0.clone(), embeddings[j].0.clone(), sim));
            }
        }
    }

    // Sort by similarity descending
    candidates.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    Ok(candidates)
}

// ==================== Graph-Based Hierarchy Refinement ====================

/// In-memory edge graph built from bulk DB query
/// Maps node_id -> Vec<(neighbor_id, weight)>
type EdgeGraph = HashMap<String, Vec<(String, f32)>>;

/// Build edge graph for a set of node IDs using bulk query
/// Much faster than per-node queries: O(1) DB call instead of O(N)
fn build_edge_graph_bulk(
    db: &Database,
    node_ids: &HashSet<String>,
) -> Result<EdgeGraph, String> {
    if node_ids.is_empty() {
        return Ok(HashMap::new());
    }

    // Convert to Vec<&str> for the DB method
    let ids_vec: Vec<&str> = node_ids.iter().map(|s| s.as_str()).collect();

    // Bulk fetch all edges involving these nodes
    let edges = db.get_edges_for_nodes_bulk(&ids_vec).map_err(|e| e.to_string())?;

    // Build graph - only include edges where BOTH endpoints are in our set
    let mut graph: EdgeGraph = HashMap::new();
    for id in node_ids {
        graph.insert(id.clone(), Vec::new());
    }

    for (source, target, weight) in edges {
        let w = weight as f32;

        // Only add edge if both endpoints are in our set (internal edges only)
        if node_ids.contains(&source) && node_ids.contains(&target) {
            graph.get_mut(&source).unwrap().push((target.clone(), w));
            graph.get_mut(&target).unwrap().push((source.clone(), w));
        }
    }

    Ok(graph)
}

/// Build adjacency list for papers (only edges between papers in set)
fn build_paper_adjacency(
    edge_graph: &EdgeGraph,
    paper_ids: &HashSet<String>,
) -> HashMap<String, Vec<String>> {
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();

    for paper_id in paper_ids {
        let neighbors: Vec<String> = edge_graph
            .get(paper_id)
            .map(|edges| {
                edges.iter()
                    .filter(|(neighbor, _)| paper_ids.contains(neighbor))
                    .map(|(neighbor, _)| neighbor.clone())
                    .collect()
            })
            .unwrap_or_default();
        graph.insert(paper_id.clone(), neighbors);
    }

    graph
}

/// Find connected components in a paper graph using union-find
fn find_connected_components(
    graph: &HashMap<String, Vec<String>>,
) -> Vec<Vec<String>> {
    let ids: Vec<&String> = graph.keys().collect();
    let id_to_idx: HashMap<&String, usize> = ids.iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();

    // Union-find parent array
    let mut parent: Vec<usize> = (0..ids.len()).collect();

    fn find(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i {
            parent[i] = find(parent, parent[i]);
        }
        parent[i]
    }

    fn union(parent: &mut [usize], i: usize, j: usize) {
        let pi = find(parent, i);
        let pj = find(parent, j);
        if pi != pj {
            parent[pi] = pj;
        }
    }

    // Union connected papers
    for (paper_id, neighbors) in graph {
        let i = id_to_idx[paper_id];
        for neighbor_id in neighbors {
            let j = id_to_idx[neighbor_id];
            union(&mut parent, i, j);
        }
    }

    // Group by root
    let mut components: HashMap<usize, Vec<String>> = HashMap::new();
    for (i, paper_id) in ids.iter().enumerate() {
        let root = find(&mut parent, i);
        components.entry(root).or_default().push((*paper_id).clone());
    }

    components.into_values().collect()
}

// ==================== Topic-Level Graph Functions (for uber-category grouping) ====================

/// Build affinity graph between child categories of a parent, based on cross-edges between their items.
/// Uses efficient SQL joins - O(E) where E = edges, not O(T²) where T = topics.
///
/// Returns adjacency list: topic_id -> Vec<connected_topic_id>
fn build_topic_affinity_graph_for_parent(
    db: &Database,
    parent_id: &str,
    topic_ids: &[String],
    min_cross_edges: usize,
) -> Result<HashMap<String, Vec<String>>, String> {
    if topic_ids.len() < 2 {
        return Ok(HashMap::new());
    }

    // Get cross-edge counts using efficient SQL join (O(E) not O(T²))
    let cross_edges = db.get_cross_edge_counts_for_children(parent_id)
        .map_err(|e| e.to_string())?;

    // Build set of valid topic IDs for filtering
    let valid_topics: HashSet<&str> = topic_ids.iter().map(|s| s.as_str()).collect();

    // Build adjacency list
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    for topic_id in topic_ids {
        graph.insert(topic_id.clone(), Vec::new());
    }

    for (topic_a, topic_b, count) in cross_edges {
        // Only include edges between topics in our list
        if count >= min_cross_edges
            && valid_topics.contains(topic_a.as_str())
            && valid_topics.contains(topic_b.as_str())
        {
            if let Some(neighbors) = graph.get_mut(&topic_a) {
                neighbors.push(topic_b.clone());
            }
            if let Some(neighbors) = graph.get_mut(&topic_b) {
                neighbors.push(topic_a.clone());
            }
        }
    }

    Ok(graph)
}

/// Group child topics of a parent by edge connectivity of their items.
/// Uses efficient SQL joins - O(E) where E = edges, not O(T²) where T = topics.
///
/// Returns Vec of groups, where each group is a Vec of topic IDs that should form an uber-category.
fn group_topics_by_edges(
    db: &Database,
    parent_id: &str,
    topic_ids: &[String],
    min_cross_edges: usize,
) -> Result<Vec<Vec<String>>, String> {
    if topic_ids.is_empty() {
        return Ok(vec![]);
    }

    if topic_ids.len() == 1 {
        return Ok(vec![topic_ids.to_vec()]);
    }

    // Build topic affinity graph using efficient SQL
    let graph = build_topic_affinity_graph_for_parent(db, parent_id, topic_ids, min_cross_edges)?;

    // Find connected components using existing function
    let components = find_connected_components(&graph);

    Ok(components)
}

// ==================== Recursive Grouping by Paper Connectivity ====================

/// Collect all paper IDs under a category (recursively through subcategories)
fn collect_papers_recursively(db: &Database, category_id: &str) -> Result<Vec<String>, String> {
    let mut papers = Vec::new();
    let mut stack = vec![category_id.to_string()];

    while let Some(node_id) = stack.pop() {
        let children = db.get_children(&node_id).map_err(|e| e.to_string())?;
        for child in children {
            if child.is_item {
                papers.push(child.id);
            } else {
                stack.push(child.id);
            }
        }
    }

    Ok(papers)
}

/// For each topic, determine which connected component most of its papers belong to
/// Returns: topic_id -> component_index (0-based)
fn map_topics_to_components(
    db: &Database,
    topic_ids: &[String],
    min_component_size: usize,
) -> Result<(HashMap<String, usize>, Vec<Vec<String>>), String> {
    // Collect all papers from all topics
    let mut topic_papers: HashMap<String, Vec<String>> = HashMap::new();
    let mut all_papers: HashSet<String> = HashSet::new();

    for topic_id in topic_ids {
        let papers = collect_papers_recursively(db, topic_id)?;
        for paper in &papers {
            all_papers.insert(paper.clone());
        }
        topic_papers.insert(topic_id.clone(), papers);
    }

    if all_papers.is_empty() {
        return Ok((HashMap::new(), vec![]));
    }

    // Build edge graph among all papers
    let edge_graph = build_edge_graph_bulk(db, &all_papers)?;
    let adjacency = build_paper_adjacency(&edge_graph, &all_papers);

    // Find connected components among papers
    let components = find_connected_components(&adjacency);

    // Create paper -> component_index mapping
    let mut paper_to_component: HashMap<String, usize> = HashMap::new();
    for (idx, component) in components.iter().enumerate() {
        for paper_id in component {
            paper_to_component.insert(paper_id.clone(), idx);
        }
    }

    // For each topic, count papers in each component and assign to majority
    let mut topic_to_component: HashMap<String, usize> = HashMap::new();

    for topic_id in topic_ids {
        let papers = topic_papers.get(topic_id).unwrap();
        if papers.is_empty() {
            continue;
        }

        // Count papers per component
        let mut component_counts: HashMap<usize, usize> = HashMap::new();
        for paper_id in papers {
            if let Some(&comp_idx) = paper_to_component.get(paper_id) {
                *component_counts.entry(comp_idx).or_insert(0) += 1;
            }
        }

        // Assign to majority component
        if let Some((&majority_comp, _)) = component_counts.iter().max_by_key(|(_, count)| *count) {
            topic_to_component.insert(topic_id.clone(), majority_comp);
        }
    }

    // Filter to significant components (with >= min_component_size papers)
    let significant_components: Vec<Vec<String>> = components.into_iter()
        .filter(|c| c.len() >= min_component_size)
        .collect();

    Ok((topic_to_component, significant_components))
}

/// Group topics by edge connectivity of their papers.
/// Uses efficient SQL joins - O(E) where E = edges, not O(P²) where P = papers.
///
/// Returns Vec of groups, where each group is topics that should stay together.
fn group_topics_by_paper_connectivity(
    db: &Database,
    parent_id: &str,
    topic_ids: &[String],
    min_group_size: usize,
    _min_component_size: usize,  // Unused with SQL approach
) -> Result<Vec<Vec<String>>, String> {
    if topic_ids.is_empty() {
        return Ok(vec![]);
    }

    if topic_ids.len() == 1 {
        return Ok(vec![topic_ids.to_vec()]);
    }

    // Get cross-edge counts using efficient SQL join (O(E) not O(P²))
    // Any edge between topics counts as a connection (threshold = 1)
    let cross_edges = db.get_cross_edge_counts_for_children(parent_id)
        .map_err(|e| e.to_string())?;

    // Build set of valid topic IDs
    let valid_topics: HashSet<&str> = topic_ids.iter().map(|s| s.as_str()).collect();

    // Build adjacency list (any cross-edge = connected)
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    for topic_id in topic_ids {
        graph.insert(topic_id.clone(), Vec::new());
    }

    for (topic_a, topic_b, count) in cross_edges {
        // Any edge counts as connection (count >= 1)
        if count >= 1
            && valid_topics.contains(topic_a.as_str())
            && valid_topics.contains(topic_b.as_str())
        {
            if let Some(neighbors) = graph.get_mut(&topic_a) {
                neighbors.push(topic_b.clone());
            }
            if let Some(neighbors) = graph.get_mut(&topic_b) {
                neighbors.push(topic_a.clone());
            }
        }
    }

    // Find connected components
    let components = find_connected_components(&graph);

    // Filter to groups with >= min_group_size topics
    let groups: Vec<Vec<String>> = components.into_iter()
        .filter(|topics| topics.len() >= min_group_size)
        .collect();

    // Check if too many topics would be singletons (>50%)
    let grouped_count: usize = groups.iter().map(|g| g.len()).sum();
    if grouped_count < topic_ids.len() / 2 {
        // Too sparse - most topics would be orphaned
        return Ok(vec![]);
    }

    Ok(groups)
}

// ==================== Category Edges from Paper Cross-Counts ====================

/// Create edges between sibling categories based on paper cross-edge counts.
/// Uses efficient SQL joins - O(E) where E = edges, not O(T²) where T = categories.
///
/// Weight reflects fraction of smaller category's papers with cross-connections:
/// weight = raw_cross_edges / min(papers_in_a, papers_in_b), capped at 1.0.
pub fn create_category_edges_from_cross_counts(
    db: &Database,
    app: Option<&AppHandle>,
) -> Result<usize, String> {
    // Delete existing sibling edges
    db.delete_edges_by_type("sibling").map_err(|e| e.to_string())?;

    let mut created = 0;

    // Get all unique parent_ids that have category children
    let all_nodes = db.get_all_nodes(false).map_err(|e| e.to_string())?;
    let parent_ids: HashSet<String> = all_nodes.iter()
        .filter(|n| !n.is_item)
        .filter_map(|n| n.parent_id.clone())
        .collect();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    // Process each parent's sibling group using efficient SQL
    for parent_id in &parent_ids {
        // Get all sibling pairs with their sizes
        let all_pairs = db.get_all_sibling_pairs(parent_id).map_err(|e| e.to_string())?;
        if all_pairs.is_empty() {
            continue;
        }

        // Get cross-edge counts using efficient SQL join (O(E) not O(T²))
        let cross_edges = db.get_sibling_cross_edge_counts(parent_id).map_err(|e| e.to_string())?;

        // Build lookup map for cross-edge counts
        let cross_edge_map: HashMap<(String, String), (usize, usize, usize)> = cross_edges
            .into_iter()
            .map(|(a, b, count, size_a, size_b)| ((a, b), (count, size_a, size_b)))
            .collect();

        // Create edges for all sibling pairs
        for (cat_a, cat_b, size_a, size_b) in all_pairs {
            // Look up cross-edge count (canonical order already guaranteed)
            let raw_count = cross_edge_map
                .get(&(cat_a.clone(), cat_b.clone()))
                .map(|(count, _, _)| *count)
                .unwrap_or(0);

            // Normalize by smaller category size
            let min_size = size_a.min(size_b).max(1);
            let weight = (raw_count as f64 / min_size as f64).min(1.0);

            // Insert edge (always, even if weight is 0)
            let edge_id = format!("sibling-{}-{}", cat_a, cat_b);
            db.insert_edge(&Edge {
                id: edge_id,
                source: cat_a,
                target: cat_b,
                edge_type: EdgeType::Sibling,
                label: None,
                weight: Some(weight),
                edge_source: Some("ai".to_string()),
                evidence_id: None,
                confidence: None,
                created_at: now,
            }).map_err(|e| e.to_string())?;

            created += 1;
        }
    }

    emit_log(app, "info", &format!("Created {} sibling edges from paper cross-counts (O(E) SQL)", created));
    Ok(created)
}

/// Result of splitting a leaf category by connectivity
#[derive(Debug, Default)]
struct SplitResult {
    pub papers_moved: usize,
    pub subcategories_created: usize,
    pub did_split: bool,
}

/// Split a leaf category (all children are papers) by edge connectivity.
/// If papers form multiple disconnected components, create subcategories for each.
///
/// Only splits if at least 2 components have >= min_component_size papers.
/// Papers in smaller components stay in the parent (acceptable orphans).
async fn split_leaf_category_by_connectivity(
    db: &Database,
    category_id: &str,
    min_component_size: usize,
    app: Option<&AppHandle>,
) -> Result<SplitResult, String> {
    let mut result = SplitResult::default();

    let children = db.get_children(category_id).map_err(|e| e.to_string())?;

    // Must be a leaf category (all children are papers)
    let all_papers = children.iter().all(|c| c.is_item);
    if !all_papers || children.is_empty() {
        return Ok(result);
    }

    // Need enough papers to potentially split
    if children.len() < min_component_size * 2 {
        return Ok(result);
    }

    // Build edge graph among papers
    let paper_ids: HashSet<String> = children.iter().map(|c| c.id.clone()).collect();
    let edge_graph = build_edge_graph_bulk(db, &paper_ids)?;
    let paper_adjacency = build_paper_adjacency(&edge_graph, &paper_ids);
    let components = find_connected_components(&paper_adjacency);

    // Count qualifying components (>= min_component_size)
    let qualifying_components: Vec<&Vec<String>> = components.iter()
        .filter(|c| c.len() >= min_component_size)
        .collect();

    // Only split if there are at least 2 qualifying components
    if qualifying_components.len() < 2 {
        return Ok(result);
    }

    let category = db.get_node(category_id).map_err(|e| e.to_string())?
        .ok_or("Category not found")?;
    let category_name = category.ai_title.as_deref().unwrap_or(&category.title);

    emit_log(app, "info", &format!(
        "  Splitting '{}': {} papers -> {} components",
        category_name, children.len(), qualifying_components.len()
    ));

    let new_depth = category.depth + 1;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap().as_millis();

    // Collect existing sibling names to avoid duplicates
    let parent_id = category.parent_id.as_deref().unwrap_or("");
    let siblings = db.get_children(parent_id).map_err(|e| e.to_string())?;
    let existing_names: Vec<String> = siblings.iter()
        .filter_map(|s| s.ai_title.clone())
        .collect();

    for (idx, component) in qualifying_components.iter().enumerate() {
        // Get paper nodes for naming
        let component_papers: Vec<Node> = component.iter()
            .filter_map(|id| db.get_node(id).ok().flatten())
            .collect();

        // Name the new subcategory
        let refs: Vec<&Node> = component_papers.iter().collect();
        let name = name_cluster_from_items(&refs, &existing_names, app).await
            .unwrap_or_else(|_| format!("{} - Group {}", category_name, idx + 1));

        let sub_id = format!("{}-split-{}-{}", category_id, timestamp, idx);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap().as_secs() as i64;

        // Create subcategory
        db.insert_node(&Node {
            id: sub_id.clone(),
            node_type: NodeType::Cluster,
            title: name.clone(),
            url: None,
            content: None,
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: None,
            depth: new_depth,
            is_item: false,
            is_universe: false,
            parent_id: Some(category_id.to_string()),
            child_count: component.len() as i32,
            ai_title: Some(name.clone()),
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
        }).map_err(|e| e.to_string())?;

        // Move papers into new subcategory
        for paper_id in *component {
            db.update_parent(paper_id, &sub_id).map_err(|e| e.to_string())?;
            db.set_node_depth(paper_id, new_depth + 1).map_err(|e| e.to_string())?;
            result.papers_moved += 1;
        }

        compute_and_store_centroid(db, &sub_id);
        result.subcategories_created += 1;

        emit_log(app, "info", &format!(
            "    Created '{}' with {} papers", name, component.len()
        ));
    }

    // Update parent's child count (now has subcategories + orphan papers)
    let new_child_count = db.get_children(category_id).map_err(|e| e.to_string())?.len();
    db.update_child_count(category_id, new_child_count as i32).map_err(|e| e.to_string())?;
    compute_and_store_centroid(db, category_id);

    result.did_split = true;
    Ok(result)
}

/// Result struct for graph-based refinement
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefineGraphResult {
    pub categories_analyzed: usize,
    pub papers_moved: usize,
    pub subcategories_created: usize,
    pub categories_merged: usize,
    pub iterations: usize,
}

const MAX_REFINE_GRAPH_ITERATIONS: usize = 5;

/// Configuration for graph-based refinement
pub struct RefineGraphConfig {
    pub merge_threshold: f32,
    pub min_component_size: usize,  // Min papers to form new subcategory
    pub dry_run: bool,
}

impl Default for RefineGraphConfig {
    fn default() -> Self {
        Self {
            merge_threshold: DEFAULT_MERGE_THRESHOLD,
            min_component_size: 3,
            dry_run: false,
        }
    }
}

/// Helper: merge cat_b into cat_a (reparent children, delete cat_b)
/// Simplified version without logging (used by graph-based refinement)
fn merge_category_into_silent(db: &Database, survivor_id: &str, to_delete_id: &str) -> Result<(), String> {
    let children = db.get_children(to_delete_id).map_err(|e| e.to_string())?;
    for child in &children {
        db.update_parent(&child.id, survivor_id).map_err(|e| e.to_string())?;
    }

    // Delete in FK order
    db.delete_fos_edges_for_node(to_delete_id).map_err(|e| e.to_string())?;
    db.delete_edges_for_node(to_delete_id).map_err(|e| e.to_string())?;
    db.delete_node(to_delete_id).map_err(|e| e.to_string())?;

    // Update survivor
    let new_count = db.get_children(survivor_id).map_err(|e| e.to_string())?.len();
    db.update_child_count(survivor_id, new_count as i32).map_err(|e| e.to_string())?;
    compute_and_store_centroid(db, survivor_id);

    Ok(())
}

/// Main graph-based refinement function
pub async fn refine_hierarchy_by_graph(
    db: &Database,
    app: Option<&AppHandle>,
    config: RefineGraphConfig,
) -> Result<RefineGraphResult, String> {
    emit_log(app, "info", "Starting graph-based hierarchy refinement...");

    let protected = db.get_protected_node_ids();
    let mut result = RefineGraphResult {
        categories_analyzed: 0,
        papers_moved: 0,
        subcategories_created: 0,
        categories_merged: 0,
        iterations: 0,
    };

    loop {
        result.iterations += 1;
        if result.iterations > MAX_REFINE_GRAPH_ITERATIONS {
            emit_log(app, "warn", &format!("Hit max iterations ({})", MAX_REFINE_GRAPH_ITERATIONS));
            break;
        }

        let mut changes = 0;
        let max_depth = db.get_max_depth().map_err(|e| e.to_string())?;

        // Phase A: Top-down merge similar sibling categories (keep existing logic)
        emit_log(app, "info", &format!("  Iteration {} Phase A: Merging similar categories", result.iterations));

        for depth in 0..max_depth {
            let parents = db.get_nodes_at_depth(depth).map_err(|e| e.to_string())?;

            for parent in parents {
                if parent.is_item || protected.contains(&parent.id) {
                    continue;
                }

                let candidates = find_mergeable_siblings(db, &parent.id, config.merge_threshold)?;

                for (cat_a, cat_b, sim) in &candidates {
                    // Check both nodes still exist
                    let node_a = match db.get_node(cat_a).map_err(|e| e.to_string())? {
                        Some(n) => n,
                        None => continue,
                    };
                    let node_b = match db.get_node(cat_b).map_err(|e| e.to_string())? {
                        Some(n) => n,
                        None => continue,
                    };

                    // Survivor = category with more children
                    let (survivor_id, to_delete_id) = if node_b.child_count > node_a.child_count {
                        (cat_b.as_str(), cat_a.as_str())
                    } else {
                        (cat_a.as_str(), cat_b.as_str())
                    };

                    emit_log(app, "debug", &format!("  Merging: sim={:.3}", sim));

                    if !config.dry_run {
                        merge_category_into_silent(db, survivor_id, to_delete_id)?;
                        result.categories_merged += 1;
                        changes += 1;
                    }
                }
            }
        }

        // Phase B: Split leaf categories by edge connectivity
        // Categories whose papers aren't connected should be split
        emit_log(app, "info", &format!("  Iteration {} Phase B: Splitting by connectivity", result.iterations));

        // Work bottom-up so we process leaves first
        for depth in (1..=max_depth).rev() {
            let nodes = db.get_nodes_at_depth(depth).map_err(|e| e.to_string())?;

            for node in nodes {
                if node.is_item || protected.contains(&node.id) {
                    continue;
                }

                // Check if node still exists (may have been modified)
                if db.get_node(&node.id).map_err(|e| e.to_string())?.is_none() {
                    continue;
                }

                result.categories_analyzed += 1;

                // Try to split leaf categories by connectivity
                if !config.dry_run {
                    let split = split_leaf_category_by_connectivity(
                        db, &node.id, config.min_component_size, app
                    ).await?;

                    if split.did_split {
                        result.papers_moved += split.papers_moved;
                        result.subcategories_created += split.subcategories_created;
                        changes += split.subcategories_created;
                    }
                }
            }
        }

        if changes == 0 {
            emit_log(app, "info", &format!("Converged after {} iterations", result.iterations));
            break;
        }
    }

    // Index edges for the reorganized hierarchy
    emit_log(app, "info", "  Indexing edge parents...");
    let indexed = db.update_edge_parents().map_err(|e| e.to_string())?;
    emit_log(app, "info", &format!("  Indexed {} edges", indexed));

    Ok(result)
}

/// Relocate misplaced children from an incoherent category to better-fitting sibling categories.
/// Returns number of moves executed.
fn relocate_from_incoherent_category(
    db: &Database,
    category_id: &str,
    app: Option<&AppHandle>,
) -> Result<usize, String> {
    let category = db.get_node(category_id).map_err(|e| e.to_string())?
        .ok_or("Category not found")?;

    let children = db.get_children(category_id).map_err(|e| e.to_string())?;
    if children.is_empty() {
        return Ok(0);
    }

    // Get current category's centroid
    let current_centroid = match db.get_node_embedding(category_id).map_err(|e| e.to_string())? {
        Some(c) => c,
        None => return Ok(0),  // No centroid, can't compare
    };

    // Build sibling-only centroid map
    let parent_id = match &category.parent_id {
        Some(id) => id.clone(),
        None => return Ok(0),
    };
    let siblings = db.get_children(&parent_id).map_err(|e| e.to_string())?;
    let sibling_centroids: HashMap<String, Vec<f32>> = siblings.iter()
        .filter(|s| !s.is_item && s.id != category.id)
        .filter_map(|s| db.get_node_embedding(&s.id).ok().flatten()
            .map(|emb| (s.id.clone(), emb)))
        .collect();
    if sibling_centroids.is_empty() {
        return Ok(0);
    }

    // Collect moves: (child_id, new_parent_id)
    let mut moves: Vec<(String, String)> = Vec::new();

    // Analyze each child
    for child in &children {
        if !child.is_item {
            continue;
        }
        let child_emb = match db.get_node_embedding(&child.id).map_err(|e| e.to_string())? {
            Some(e) => e,
            None => continue,
        };

        // Similarity to current parent
        let current_sim = cosine_similarity(&child_emb, &current_centroid);

        // Find best alternative
        let mut best_id: Option<String> = None;
        let mut best_sim = current_sim;

        for (other_id, centroid) in sibling_centroids.iter() {
            if other_id == &child.id {
                continue;
            }

            let sim = cosine_similarity(&child_emb, centroid);
            if sim > best_sim + 0.05 {
                best_sim = sim;
                best_id = Some(other_id.clone());
            }
        }

        if let Some(new_parent_id) = best_id {
            moves.push((child.id.clone(), new_parent_id));
        }
    }

    if moves.is_empty() {
        return Ok(0);
    }

    emit_log(app, "info", &format!(
        "  Relocating {} children from '{}'",
        moves.len(),
        category.ai_title.as_deref().unwrap_or(&category.title)
    ));

    // Track affected parents
    let mut affected_parents: HashSet<String> = HashSet::new();
    affected_parents.insert(category_id.to_string());

    // Execute all moves
    for (child_id, new_parent_id) in &moves {
        db.update_parent(child_id, new_parent_id).map_err(|e| e.to_string())?;

        if let Some(new_parent) = db.get_node(new_parent_id).map_err(|e| e.to_string())? {
            db.set_node_depth(child_id, new_parent.depth + 1).map_err(|e| e.to_string())?;
        }

        affected_parents.insert(new_parent_id.clone());
    }

    // Update counts and centroids for all affected parents
    for parent_id in &affected_parents {
        compute_and_store_centroid(db, parent_id);
        let actual_count = db.get_children(parent_id).map_err(|e| e.to_string())?.len() as i32;
        db.update_child_count(parent_id, actual_count).map_err(|e| e.to_string())?;
    }

    // Delete source category if now empty
    let remaining = db.get_children(category_id).map_err(|e| e.to_string())?.len();
    if remaining == 0 {
        emit_log(app, "debug", &format!(
            "    Deleting empty category '{}'",
            category.ai_title.as_deref().unwrap_or(&category.title)
        ));
        db.delete_fos_edges_for_node(category_id).map_err(|e| e.to_string())?;
        db.delete_edges_for_node(category_id).map_err(|e| e.to_string())?;
        db.delete_node(category_id).map_err(|e| e.to_string())?;
    }

    Ok(moves.len())
}

/// Result struct for refinement
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefineCoherenceResult {
    pub categories_analyzed: usize,
    pub incoherent_found: usize,
    pub children_relocated: usize,
    pub categories_merged: usize,
    pub iterations: usize,
}

// Default thresholds (used when not specified via CLI)
const DEFAULT_COHERENCE_THRESHOLD: f32 = 0.45;
const DEFAULT_MERGE_THRESHOLD: f32 = 0.65;
const MAX_REFINE_ITERATIONS: usize = 10;

/// Configuration for coherence refinement
pub struct RefineConfig {
    pub coherence_threshold: f32,  // Below = split category
    pub merge_threshold: f32,      // Above = auto-merge
    pub dry_run: bool,
}

impl Default for RefineConfig {
    fn default() -> Self {
        Self {
            coherence_threshold: DEFAULT_COHERENCE_THRESHOLD,
            merge_threshold: DEFAULT_MERGE_THRESHOLD,
            dry_run: false,
        }
    }
}

/// Helper: merge cat_b into cat_a (reparent children, delete cat_b)
fn merge_category_into(db: &Database, survivor_id: &str, to_delete_id: &str, app: Option<&AppHandle>) -> Result<(), String> {
    let survivor = db.get_node(survivor_id).map_err(|e| e.to_string())?
        .ok_or("Survivor category not found")?;
    let to_delete = db.get_node(to_delete_id).map_err(|e| e.to_string())?
        .ok_or("Category to delete not found")?;

    emit_log(app, "info", &format!(
        "    Merging '{}' into '{}'",
        to_delete.ai_title.as_deref().unwrap_or(&to_delete.title),
        survivor.ai_title.as_deref().unwrap_or(&survivor.title)
    ));

    let children = db.get_children(to_delete_id).map_err(|e| e.to_string())?;
    for child in &children {
        db.update_parent(&child.id, survivor_id).map_err(|e| e.to_string())?;
    }

    // Delete in FK order: fos_edges -> edges -> node
    db.delete_fos_edges_for_node(to_delete_id).map_err(|e| e.to_string())?;
    db.delete_edges_for_node(to_delete_id).map_err(|e| e.to_string())?;
    db.delete_node(to_delete_id).map_err(|e| e.to_string())?;

    // Update survivor
    let new_count = db.get_children(survivor_id).map_err(|e| e.to_string())?.len();
    db.update_child_count(survivor_id, new_count as i32).map_err(|e| e.to_string())?;
    compute_and_store_centroid(db, survivor_id);

    Ok(())
}

/// Main refinement function - iterates until stable
pub async fn refine_hierarchy_coherence(
    db: &Database,
    app: Option<&AppHandle>,
    config: RefineConfig,
) -> Result<RefineCoherenceResult, String> {
    emit_log(app, "info", "Starting coherence-based refinement...");

    let protected = db.get_protected_node_ids();
    let mut analyzed = 0usize;
    let mut incoherent = 0usize;
    let mut relocated = 0usize;
    let mut merged = 0usize;
    let mut iterations = 0;

    loop {
        iterations += 1;
        if iterations > MAX_REFINE_ITERATIONS {
            emit_log(app, "warn", &format!("Hit max iterations ({})", MAX_REFINE_ITERATIONS));
            break;
        }

        let mut changes = 0;
        let max_depth = db.get_max_depth().map_err(|e| e.to_string())?;

        // Phase A: Top-down merge similar siblings first (consolidate before splitting)
        emit_log(app, "debug", &format!("Iteration {} - Phase A: merging similar siblings", iterations));

        for depth in 0..max_depth {
            let parents = db.get_nodes_at_depth(depth).map_err(|e| e.to_string())?;

            for parent in parents {
                if parent.is_item || protected.contains(&parent.id) {
                    continue;
                }

                let candidates = find_mergeable_siblings(db, &parent.id, config.merge_threshold)?;

                for (cat_a, cat_b, sim) in &candidates {
                    let node_a = match db.get_node(cat_a).map_err(|e| e.to_string())? {
                        Some(n) => n,
                        None => continue,
                    };
                    let node_b = match db.get_node(cat_b).map_err(|e| e.to_string())? {
                        Some(n) => n,
                        None => continue,
                    };

                    // Survivor = category with more children (or first if equal)
                    let (survivor_id, to_delete_id) = if node_b.child_count > node_a.child_count {
                        (cat_b.as_str(), cat_a.as_str())
                    } else {
                        (cat_a.as_str(), cat_b.as_str())
                    };

                    emit_log(app, "info", &format!("  Merging: sim={:.3}", sim));

                    if !config.dry_run {
                        merge_category_into(db, survivor_id, to_delete_id, app)?;
                        merged += 1;
                        changes += 1;
                    }
                }
            }
        }

        // Phase B: Relocate misplaced children from incoherent categories
        emit_log(app, "debug", &format!("Iteration {} - Phase B: relocating misplaced children", iterations));

        // Bottom-up: check each category for coherence (depth >= 2 only)
        // Skip top-layer categories to preserve hierarchy structure
        for depth in (2..max_depth).rev() {
            let nodes = db.get_nodes_at_depth(depth).map_err(|e| e.to_string())?;

            for node in nodes {
                if node.is_item || protected.contains(&node.id) {
                    continue;
                }

                // Check if node still exists (may have been deleted)
                if db.get_node(&node.id).map_err(|e| e.to_string())?.is_none() {
                    continue;
                }

                analyzed += 1;

                if let Some(coherence) = compute_category_coherence(db, &node.id)? {
                    if coherence < config.coherence_threshold {
                        incoherent += 1;
                        emit_log(app, "debug", &format!(
                            "Incoherent: '{}' (coh={:.3})",
                            node.ai_title.as_deref().unwrap_or(&node.title), coherence
                        ));

                        if !config.dry_run {
                            let moves = relocate_from_incoherent_category(
                                db, &node.id, app
                            )?;

                            if moves > 0 {
                                relocated += moves;
                                changes += moves;
                            }
                        }
                    }
                }
            }
        }

        if changes == 0 {
            emit_log(app, "info", &format!("Converged after {} iterations", iterations));
            break;
        }
    }

    Ok(RefineCoherenceResult {
        categories_analyzed: analyzed,
        incoherent_found: incoherent,
        children_relocated: relocated,
        categories_merged: merged,
        iterations,
    })
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
    // Check if FOS nodes exist - if so, preserve them and only rebuild topics underneath
    let fos_nodes = db.get_nodes_at_depth(1)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|n| !n.is_item && n.id.starts_with("fos-"))
        .count();

    if fos_nodes > 0 {
        println!("[Hierarchy] Found {} FOS category nodes - preserving them", fos_nodes);
        return build_hierarchy_preserving_parents(db);
    }

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

    // Step 9: Ensure Holerabbit container exists (for browsing sessions)
    let holerabbit_container_id = crate::settings::HOLERABBIT_CONTAINER_ID;
    if db.get_node(holerabbit_container_id).map_err(|e| e.to_string())?.is_none() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let holerabbit_node = Node {
            id: holerabbit_container_id.to_string(),
            node_type: NodeType::Cluster,
            title: "Holerabbit".to_string(),
            url: None,
            content: None,
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: Some("Holerabbit".to_string()),
            ai_title: Some("Holerabbit".to_string()),
            summary: Some("Browser session tracking".to_string()),
            tags: None,
            emoji: Some("🐰".to_string()),
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
            privacy: Some(0.5),
        };

        db.insert_node(&holerabbit_node).map_err(|e| e.to_string())?;

        // Update Universe child count to include Holerabbit
        universe_child_count += 1;
        db.update_child_count(&universe_id, universe_child_count)
            .map_err(|e| e.to_string())?;

        println!("[Hierarchy] Created Holerabbit container (protected)");
    }

    // Step 10: Ensure import containers exist (for code imports by language)
    let import_containers_created = ensure_import_containers_exist(db, &universe_id, topic_depth, &mut universe_child_count)?;

    let total_items = item_count + private_count;
    println!("Hierarchy complete: Universe -> {} topics + Personal + Notes + Holerabbit + {} import containers -> {} items",
             topics_created, import_containers_created, total_items);

    // Update edge parent columns for fast per-view lookups
    if let Ok(count) = db.update_edge_parents() {
        println!("[Hierarchy] Indexed {} edges for fast view lookups", count);
    }

    Ok(HierarchyResult {
        levels_created: 3,  // Universe, Topics, Items
        intermediate_nodes_created: topics_created + 1 + (if private_count > 0 { 1 } else { 0 }),  // topics + universe + personal
        items_organized: total_items,
        max_depth: item_depth,
    })
}

/// Ensure all import containers exist (created if missing)
/// Returns the number of containers created
fn ensure_import_containers_exist(
    db: &Database,
    universe_id: &str,
    depth: i32,
    universe_child_count: &mut i32,
) -> Result<i32, String> {
    use crate::settings;

    let containers = [
        (settings::RUST_IMPORT_CONTAINER_ID, "Rust Code", "🦀"),
        (settings::TYPESCRIPT_IMPORT_CONTAINER_ID, "TypeScript Code", "📘"),
        (settings::JAVASCRIPT_IMPORT_CONTAINER_ID, "JavaScript Code", "📒"),
        (settings::PYTHON_IMPORT_CONTAINER_ID, "Python Code", "🐍"),
        (settings::C_IMPORT_CONTAINER_ID, "C Code", "⚙️"),
        (settings::DOCS_IMPORT_CONTAINER_ID, "Documentation", "📄"),
    ];

    let mut created = 0;

    for (id, title, emoji) in containers {
        if db.get_node(id).map_err(|e| e.to_string())?.is_some() {
            continue;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let node = Node {
            id: id.to_string(),
            node_type: NodeType::Cluster,
            title: title.to_string(),
            url: None,
            content: Some(format!("Imported {} files", title.to_lowercase())),
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: Some(title.to_string()),
            ai_title: Some(title.to_string()),
            summary: Some(format!("Container for imported {} files", title.to_lowercase())),
            tags: None,
            emoji: Some(emoji.to_string()),
            is_processed: true,
            depth,
            is_item: false,
            is_universe: false,
            parent_id: Some(universe_id.to_string()),
            child_count: 0,
            conversation_id: None,
            sequence_index: None,
            is_pinned: false,
            last_accessed_at: None,
            latest_child_date: None,
            is_private: None,
            privacy_reason: None,
            source: Some("code-import".to_string()),
            pdf_available: None,
            content_type: Some("import-container".to_string()),
            associated_idea_id: None,
            privacy: Some(1.0),
        };

        db.insert_node(&node).map_err(|e| e.to_string())?;
        *universe_child_count += 1;
        db.update_child_count(universe_id, *universe_child_count)
            .map_err(|e| e.to_string())?;
        created += 1;
        println!("[Hierarchy] Created import container: {} (protected)", title);
    }

    Ok(created)
}

/// Build hierarchy while preserving existing top-level parent nodes (e.g., FOS categories)
/// Creates topic nodes UNDER each parent, not directly under Universe
///
/// Flow:
/// 1. Get existing FOS nodes (depth 1, non-items)
/// 2. Delete only topic nodes (depth > 1), preserve FOS nodes
/// 3. For each FOS node, group its children by cluster_id
/// 4. Create topic nodes under each FOS node
pub fn build_hierarchy_preserving_parents(db: &Database) -> Result<HierarchyResult, String> {
    // Step 1: Get existing top-level nodes (depth 1, non-items) - these are FOS nodes
    let top_level_nodes = db.get_nodes_at_depth(1)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|n| !n.is_item)
        .collect::<Vec<_>>();

    if top_level_nodes.is_empty() {
        println!("[Hierarchy] No top-level nodes found, falling back to regular hierarchy");
        return build_hierarchy(db);
    }

    println!("[Hierarchy] Preserving {} top-level nodes (FOS categories)", top_level_nodes.len());

    // Step 2: Delete ONLY topic nodes (depth > 1, non-items), preserve FOS nodes
    let deleted = db.delete_hierarchy_nodes_below_depth(1)
        .map_err(|e| e.to_string())?;
    if deleted > 0 {
        println!("[Hierarchy] Deleted {} old topic nodes (preserved FOS)", deleted);
    }

    // Step 3: Clear parent_id only on items that pointed to deleted nodes
    db.clear_orphaned_item_parents().map_err(|e| e.to_string())?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let mut topics_created = 0;
    let mut items_organized = 0;
    let topic_depth = 2;  // FOS is depth 1, topics are depth 2
    let item_depth = 3;   // Items are depth 3

    // Step 4: For each FOS node, build sub-hierarchy for its children
    for fos_node in &top_level_nodes {
        // Get items that belong to this FOS node
        let fos_children = db.get_children(&fos_node.id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|n| n.is_item)
            .collect::<Vec<_>>();

        if fos_children.is_empty() {
            continue;
        }

        println!("[Hierarchy] FOS '{}': {} items", fos_node.title, fos_children.len());

        // Group by cluster_id
        let mut clusters: HashMap<i32, Vec<Node>> = HashMap::new();
        let mut unclustered: Vec<Node> = Vec::new();

        for item in fos_children {
            if let Some(cid) = item.cluster_id {
                clusters.entry(cid).or_default().push(item);
            } else {
                unclustered.push(item);
            }
        }

        // Add unclustered items to their own group if any
        if !unclustered.is_empty() {
            clusters.insert(-1, unclustered);
        }

        // Create topic nodes under this FOS node
        for (cluster_id, items) in &clusters {
            // Get cluster label from first item with one
            let cluster_label = items.iter()
                .find_map(|item| item.cluster_label.clone())
                .unwrap_or_else(|| {
                    if *cluster_id == -1 {
                        "Uncategorized".to_string()
                    } else {
                        format!("Topic {}", cluster_id)
                    }
                });

            let topic_id = format!("topic-{}-{}", fos_node.id, cluster_id);

            // Generate summary from first few children's titles
            let child_titles: Vec<String> = items.iter()
                .take(3)
                .map(|n| n.ai_title.clone().unwrap_or_else(|| n.title.clone()))
                .collect();
            let summary = if child_titles.len() > 2 {
                format!("{}, {} and {} more", child_titles[0], child_titles[1], items.len() - 2)
            } else {
                child_titles.join(", ")
            };

            let topic_node = Node {
                id: topic_id.clone(),
                node_type: NodeType::Cluster,
                title: cluster_label.clone(),
                url: None,
                content: None,
                position: Position { x: 0.0, y: 0.0 },
                created_at: now,
                updated_at: now,
                cluster_id: Some(*cluster_id),
                cluster_label: Some(cluster_label),
                ai_title: None,
                summary: Some(summary),
                tags: None,
                emoji: Some("📂".to_string()),
                is_processed: true,
                depth: topic_depth,
                is_item: false,
                is_universe: false,
                parent_id: Some(fos_node.id.clone()), // Under FOS, not Universe!
                child_count: items.len() as i32,
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
            topics_created += 1;

            // Update items to point to this topic and set their depth
            for item in items {
                db.update_node_hierarchy(&item.id, Some(&topic_id), item_depth)
                    .map_err(|e| e.to_string())?;
                items_organized += 1;
            }
        }

        // Update FOS node's child count
        let fos_child_count = clusters.len() as i32;
        db.update_child_count(&fos_node.id, fos_child_count)
            .map_err(|e| e.to_string())?;
    }

    println!("[Hierarchy] Created {} topics under {} FOS categories, organized {} items",
             topics_created, top_level_nodes.len(), items_organized);

    // Update edge parent columns for fast per-view lookups
    if let Ok(count) = db.update_edge_parents() {
        println!("[Hierarchy] Indexed {} edges for fast view lookups", count);
    }

    Ok(HierarchyResult {
        levels_created: 4,  // Universe, FOS, Topics, Items
        intermediate_nodes_created: topics_created,
        items_organized,
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

    // === Edge-based grouping (API-free for structural decisions) ===
    // Group topics by connectivity of their papers - topics whose papers share edges stay together
    let child_ids: Vec<String> = children.iter().map(|c| c.id.clone()).collect();
    let min_group_size = 2;  // At least 2 topics to form a group
    let min_component_size = 3;  // At least 3 papers to count as significant component

    let edge_groups = group_topics_by_paper_connectivity(db, parent_id, &child_ids, min_group_size, min_component_size)?;

    let groupings = if edge_groups.len() >= 2 {
        // Log compact summary: group sizes and first topic in each
        let group_summary: String = edge_groups.iter()
            .take(5)
            .map(|g| format!("{}t", g.len()))
            .collect::<Vec<_>>()
            .join("/");
        let extra = if edge_groups.len() > 5 { format!("/+{}", edge_groups.len() - 5) } else { String::new() };
        emit_log(app, "info", &format!("  Edge grouping: {} children → {} groups ({}{})",
            child_ids.len(), edge_groups.len(), group_summary, extra));

        // Convert edge groups to CategoryGrouping format
        let mut groupings: Vec<ai_client::CategoryGrouping> = Vec::new();

        // Build id -> node lookup
        let id_to_node: HashMap<&str, &Node> = children.iter()
            .map(|n| (n.id.as_str(), n))
            .collect();

        for group in &edge_groups {
            // Get nodes in this group
            let group_nodes: Vec<&Node> = group.iter()
                .filter_map(|id| id_to_node.get(id.as_str()).copied())
                .collect();

            if group_nodes.is_empty() {
                continue;
            }

            // Collect papers from all topics in this group for naming
            let mut all_papers: Vec<Node> = Vec::new();
            for topic_id in group {
                if let Ok(topic_children) = db.get_children(topic_id) {
                    for child in topic_children {
                        if child.is_item {
                            all_papers.push(child);
                        }
                    }
                }
            }

            // Name the group using AI (only AI call - for naming, not structure)
            let paper_refs: Vec<&Node> = all_papers.iter().collect();
            let group_name = name_cluster_from_items(&paper_refs, &forbidden_names, app).await
                .unwrap_or_else(|_| format!("Group {}", groupings.len() + 1));

            // Get labels for topics in this group (for matching later)
            let topic_labels: Vec<String> = group_nodes.iter()
                .map(|node| {
                    if node.is_item {
                        node.ai_title.clone()
                            .or_else(|| node.cluster_label.clone())
                            .unwrap_or_else(|| node.title.clone())
                    } else {
                        node.cluster_label.clone()
                            .or_else(|| node.ai_title.clone())
                            .unwrap_or_else(|| node.title.clone())
                    }
                })
                .collect();

            groupings.push(ai_client::CategoryGrouping {
                name: group_name,
                description: None,
                children: topic_labels,
            });
        }

        // Log created group names (truncated)
        let names: String = groupings.iter()
            .take(4)
            .map(|g| {
                let name = &g.name;
                if name.len() > 20 { format!("{}...", &name[..17]) } else { name.clone() }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let extra = if groupings.len() > 4 { format!(", +{}", groupings.len() - 4) } else { String::new() };
        emit_log(app, "info", &format!("  Named: {}{}", names, extra));

        groupings
    } else {
        // All topics are connected (one component) or edge data too sparse
        let reason = if edge_groups.len() == 1 { "all connected" } else { "edges too sparse" };
        emit_log(app, "info", &format!("  Edge grouping: {} children → {} groups ({}) - skip",
            child_ids.len(), edge_groups.len(), reason));

        // Fall back to embedding-based clustering for items, or skip for categories
        let non_item_count = children.iter().filter(|c| !c.is_item).count();
        if non_item_count == 0 {
            emit_log(app, "info", "  Fallback: embedding clustering for items");
            return cluster_items_by_embedding(db, parent_id, &children, app).await;
        } else {
            return Ok(false);
        }
    };

    if groupings.is_empty() {
        emit_log(app, "info", "No valid groupings produced");
        return Ok(false);
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

    // Protected containers should never be reparented
    let recent_notes_id = crate::settings::RECENT_NOTES_CONTAINER_ID;
    let holerabbit_id = crate::settings::HOLERABBIT_CONTAINER_ID;
    let import_container_ids: std::collections::HashSet<&str> =
        crate::settings::IMPORT_CONTAINER_IDS.iter().copied().collect();

    for (idx, grouping) in groupings.iter().enumerate() {
        // Find ALL child nodes matching this grouping's labels
        // Deduplicate by node ID to handle duplicate labels in AI response
        // Exclude protected containers from reparenting
        let mut seen_ids = std::collections::HashSet::new();
        let matching_children: Vec<&Node> = grouping.children
            .iter()
            .flat_map(|label| label_to_children.get(label).cloned().unwrap_or_default())
            .filter(|node| seen_ids.insert(node.id.clone()))
            .filter(|node| {
                node.id != recent_notes_id
                    && node.id != holerabbit_id
                    && !import_container_ids.contains(node.id.as_str())
            })
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

    // Clear FOS edge cache (will be stale after rebuild)
    db.clear_fos_edges().ok();

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
    emit_log(app, "info", "▶ Phase 1: Classifying content types");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_progress(app, AiProgressEvent {
        current: 1,
        total: 7,
        node_title: "Phase 1: Classification".to_string(),
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
    emit_log(app, "info", "▶ Phase 2: Clustering ideas into topics");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_progress(app, AiProgressEvent {
        current: 2,
        total: 7,
        node_title: "Phase 2: Clustering".to_string(),
        new_title: if run_clustering { "Clustering ideas into topics..." } else { "Skipping (using existing clusters)" }.to_string(),
        emoji: Some("🧩".to_string()),
        status: "processing".to_string(),
        error_message: None,
        elapsed_secs: Some(total_start.elapsed().as_secs_f64()),
        estimate_secs: None,
        remaining_secs: None,
    });
    // Check if clusters already exist (skip expensive re-clustering if so)
    let total_items = db.get_items().map(|v| v.len()).unwrap_or(0);
    let clustered_items = db.count_clustered_items().unwrap_or(0);
    let has_existing_clusters = clustered_items > total_items / 2; // >50% clustered = use existing

    let clustering_result = if run_clustering && ai_client::is_available() && !has_existing_clusters {
        emit_log(app, "info", "Running AI clustering on idea items (excluding code/debug/paste)...");
        let result = crate::clustering::run_clustering(db, true).await?;
        emit_log(app, "info", &format!("✓ Clustering complete: {} ideas → {} clusters", result.items_assigned, result.clusters_created));
        Some(result)
    } else if has_existing_clusters {
        emit_log(app, "info", &format!("Using existing clusters ({} items already clustered)", clustered_items));
        None
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

    // Create semantic edges BEFORE hierarchy building (needed for edge-based grouping)
    let semantic_edges_created = if db.get_nodes_with_embeddings().map(|v| v.len()).unwrap_or(0) > 1 {
        emit_log(app, "info", "");
        emit_log(app, "info", "Creating semantic edges from embeddings (for edge-based grouping)...");

        // Delete old semantic edges first
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
        emit_log(app, "info", "Skipping semantic edges (no embeddings yet)");
        0
    };

    // Step 3: Build initial hierarchy
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ Phase 3: Building initial hierarchy structure");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_progress(app, AiProgressEvent {
        current: 3,
        total: 7,
        node_title: "Phase 3: Building hierarchy".to_string(),
        new_title: "Creating Universe and topic nodes...".to_string(),
        emoji: Some("🏗️".to_string()),
        status: "processing".to_string(),
        error_message: None,
        elapsed_secs: Some(total_start.elapsed().as_secs_f64()),
        estimate_secs: None,
        remaining_secs: None,
    });
    // Check if FOS nodes exist - preserve them instead of wiping everything
    let fos_nodes = db.get_nodes_at_depth(1)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|n| !n.is_item && n.id.starts_with("fos-"))
        .count();

    let hierarchy_result = if fos_nodes > 0 {
        emit_log(app, "info", &format!("Found {} FOS category nodes - preserving them", fos_nodes));
        build_hierarchy_preserving_parents(db)?
    } else {
        emit_log(app, "info", "Creating Universe and topic nodes from clusters...");
        build_hierarchy(db)?
    };
    emit_log(app, "info", &format!("✓ Created {} intermediate nodes, organized {} items", hierarchy_result.intermediate_nodes_created, hierarchy_result.items_organized));

    // Check if this is a code-only database (skip project detection for code)
    let all_items = db.get_items().map_err(|e| e.to_string())?;
    let is_code_only = !all_items.is_empty() && all_items.iter().all(|n| {
        n.content_type.as_ref()
            .map(|ct| ct.starts_with("code_"))
            .unwrap_or(false)
    });

    // Phase 4: Project detection - DISABLED
    // Edge-based grouping handles organization better than keyword-based project detection.
    // The detect_projects_with_ai call was also Anthropic-only.
    let _is_code_only = is_code_only; // suppress unused warning
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ Phase 4: Skipped (project detection disabled, using edge-based grouping)");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_progress(app, AiProgressEvent {
        current: 4,
        total: 7,
        node_title: "Phase 4: Skipped".to_string(),
        new_title: "Using edge-based grouping".to_string(),
        emoji: Some("⏭️".to_string()),
        status: "processing".to_string(),
        error_message: None,
        elapsed_secs: Some(total_start.elapsed().as_secs_f64()),
        estimate_secs: None,
        remaining_secs: None,
    });

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
        // Filter out category-personal - it should stay at depth 1
        let categories: Vec<ai_client::TopicInfo> = universe_children.iter()
            .filter(|c| c.id != "category-personal")
            .map(|c| ai_client::TopicInfo {
                id: c.id.clone(),
                label: c.cluster_label.clone()
                    .or(c.ai_title.clone())
                    .unwrap_or_else(|| c.title.clone()),
                item_count: c.child_count.max(1),
            })
            .collect();

        // Edge-based uber-category grouping: topics group by cross-edges between their papers
        // Uses efficient SQL joins - O(E) not O(T²)
        let topic_ids: Vec<String> = categories.iter().map(|c| c.id.clone()).collect();
        let min_cross_edges = 5; // Minimum edges between topics to consider them connected

        emit_log(app, "info", &format!("  Finding connected topic groups (min_cross_edges={}, using O(E) SQL join)", min_cross_edges));

        match group_topics_by_edges(db, &universe.id, &topic_ids, min_cross_edges) {
            Ok(groups) => {
                // Filter to groups with 2+ topics (singleton groups stay under Universe)
                let multi_topic_groups: Vec<_> = groups.into_iter()
                    .filter(|g| g.len() >= 2)
                    .collect();

                if multi_topic_groups.is_empty() {
                    emit_log(app, "info", "  No connected topic groups found - topics stay under Universe");
                } else {
                    emit_log(app, "info", &format!("  Found {} connected groups (2+ topics each)", multi_topic_groups.len()));

                    // Create map from topic_id -> child node
                    let topic_id_to_node: std::collections::HashMap<&str, &Node> = universe_children.iter()
                        .map(|n| (n.id.as_str(), n))
                        .collect();

                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis();
                    let now = timestamp as i64;

                    for (idx, group) in multi_topic_groups.iter().enumerate() {
                        // Get all papers in this group for naming
                        let mut all_papers: Vec<Node> = Vec::new();
                        for topic_id in group {
                            if let Ok(children) = db.get_children(topic_id) {
                                for child in children {
                                    if child.is_item {
                                        all_papers.push(child);
                                    }
                                }
                            }
                        }

                        // Name the uber-category from its papers (AI only for naming)
                        let paper_refs: Vec<&Node> = all_papers.iter().collect();
                        let uber_name = name_cluster_from_items(&paper_refs, &[], app).await
                            .unwrap_or_else(|_| format!("Topic Group {}", idx + 1));

                        // Skip garbage names
                        if is_garbage_name(&uber_name) {
                            emit_log(app, "debug", &format!("  Skipping garbage name: '{}'", uber_name));
                            continue;
                        }

                        // Get matching topic nodes
                        let matching_topics: Vec<&Node> = group.iter()
                            .filter_map(|id| topic_id_to_node.get(id.as_str()).copied())
                            .collect();

                        if matching_topics.len() < 2 {
                            continue;
                        }

                        // Create uber-category node
                        let uber_id = format!("uber-{}-{}", timestamp, idx);
                        let uber_node = Node {
                            id: uber_id.clone(),
                            node_type: NodeType::Cluster,
                            title: uber_name.clone(),
                            url: None,
                            content: None,
                            position: Position { x: 0.0, y: 0.0 },
                            created_at: now,
                            updated_at: now,
                            cluster_id: None,
                            cluster_label: Some(uber_name.clone()),
                            depth: 1,
                            is_item: false,
                            is_universe: false,
                            parent_id: Some(universe.id.clone()),
                            child_count: matching_topics.len() as i32,
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
                        };

                        db.insert_node(&uber_node).map_err(|e| e.to_string())?;

                        // Reparent topics to the uber-category
                        for topic in &matching_topics {
                            db.update_parent(&topic.id, &uber_id).map_err(|e| e.to_string())?;
                        }

                        // Increment depths of reparented subtrees
                        let topic_child_ids: Vec<String> = matching_topics.iter().map(|t| t.id.clone()).collect();
                        db.increment_multiple_subtrees_depth(&topic_child_ids).map_err(|e| e.to_string())?;

                        // Compute centroid for the new uber-category
                        compute_and_store_centroid(db, &uber_id);

                        uber_categories_created += 1;
                        emit_log(app, "info", &format!("  Created '{}' with {} topics ({} papers)", uber_name, matching_topics.len(), all_papers.len()));
                    }

                    if uber_categories_created > 0 {
                        emit_log(app, "info", &format!("✓ Consolidated into {} uber-categories (edge-based, centroids computed)", uber_categories_created));
                    }
                }
            }
            Err(e) => {
                emit_log(app, "warn", &format!("  Edge-based uber-category grouping failed: {}", e));
            }
        }
    }

    // Step 5: Recursively group levels with too many children
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ Phase 5: Recursive grouping (tiered limits: L0-1=10, L2=25, L3=50, L4=100)");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    let mut grouping_iterations = 0;
    let max_iterations = 50; // Safety limit
    let mut failed_nodes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let grouping_start = std::time::Instant::now();

    // Emit initial progress for Step 5
    emit_progress(app, AiProgressEvent {
        current: 5,
        total: 7,
        node_title: "Phase 5: Recursive grouping".to_string(),
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
                    node_title: format!("Phase 5: Grouping (iter {})", grouping_iterations + 1),
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
    emit_log(app, "info", &format!("  Phase 5 completed in {:.1}s ({} iterations)", grouping_elapsed, grouping_iterations));

    // Recalculate final depth
    let final_max_depth = db.get_max_depth().map_err(|e| e.to_string())?;
    emit_log(app, "info", &format!("  Final hierarchy depth: {}", final_max_depth));

    // Phase 5.5: Coherence refinement
    emit_log(app, "info", "");
    emit_log(app, "info", "Phase 5.5: Coherence refinement (split incoherent, merge similar)");
    emit_log(app, "info", "-------------------------------------------------------------");
    emit_progress(app, AiProgressEvent {
        current: 5,
        total: 7,
        node_title: "Phase 5.5: Coherence".to_string(),
        new_title: "Refining hierarchy coherence...".to_string(),
        emoji: Some("".to_string()),
        status: "processing".to_string(),
        error_message: None,
        elapsed_secs: Some(total_start.elapsed().as_secs_f64()),
        estimate_secs: None,
        remaining_secs: None,
    });

    let refine_result = refine_hierarchy_coherence(db, app, RefineConfig::default()).await?;
    emit_log(app, "info", &format!(
        "  Relocated {} children, merged {} similar categories",
        refine_result.children_relocated, refine_result.categories_merged
    ));

    // Fix any depth inconsistencies from splitting
    let depths_fixed = db.fix_all_depths().map_err(|e| e.to_string())?;
    if depths_fixed > 0 {
        emit_log(app, "info", &format!("  Fixed {} node depths after refinement", depths_fixed));
    }

    // Recalculate depth after refinement
    let post_refine_depth = db.get_max_depth().map_err(|e| e.to_string())?;
    if post_refine_depth != final_max_depth {
        emit_log(app, "info", &format!("  Hierarchy depth after refinement: {}", post_refine_depth));
    }

    // Step 6: Generate embeddings for ALL nodes that need them (if OpenAI key available)
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ Phase 6: Generating embeddings for semantic search");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_progress(app, AiProgressEvent {
        current: 6,
        total: 7,
        node_title: "Phase 6: Embeddings".to_string(),
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
                // Use TITLE + CONTENT for embeddings (title helps distinguish similar content)
                let embed_text = if let Some(content) = &node.content {
                    // Prepend title to content (reserve ~100 chars for title, rest for content)
                    format!("{}\n\n{}", title, safe_truncate(content, 900))
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
                    node_title: format!("Phase 6: Embeddings ({}/{})", current, total_needing),
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

    // Note: Semantic edges are now created earlier (before hierarchy building)
    // to enable edge-based grouping. Variable semantic_edges_created is set above.

    // ═══════════════════════════════════════════════════════════════════════════
    // Phase 7: Associate supporting items with ideas
    // ═══════════════════════════════════════════════════════════════════════════
    emit_log(app, "info", "");
    emit_log(app, "info", "▶ Phase 7: Associating supporting items with ideas");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_progress(app, AiProgressEvent {
        current: 7,
        total: 7,
        node_title: "Phase 7: Associations".to_string(),
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

    // Create edges between sibling categories from paper cross-counts
    emit_log(app, "info", "");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_log(app, "info", "Creating category edges from paper cross-counts...");
    let sibling_edges_created = match create_category_edges_from_cross_counts(db, app) {
        Ok(count) => count,
        Err(e) => {
            emit_log(app, "warn", &format!("  Failed to create category edges: {}", e));
            0
        }
    };

    // Update edge parent columns for fast per-view lookups
    emit_log(app, "info", "");
    emit_log(app, "info", "───────────────────────────────────────────────────────────");
    emit_log(app, "info", "Updating edge parent indices...");
    let edges_indexed = match db.update_edge_parents() {
        Ok(count) => {
            emit_log(app, "info", &format!("  ✓ Indexed {} edges for fast view lookups", count));
            count
        }
        Err(e) => {
            emit_log(app, "warn", &format!("  Failed to update edge parents: {}", e));
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
    emit_log(app, "info", &format!("  • {} sibling category edges", sibling_edges_created));
    emit_log(app, "info", &format!("  • {} edges indexed for views", edges_indexed));
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

// ==================== Small Category Merging ====================

/// Result of the merge_small_categories operation
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeCategoriesResult {
    /// Number of categories merged (deleted after reparenting)
    pub categories_merged: usize,
    /// Number of children reparented
    pub children_reparented: usize,
    /// Number of survivor categories renamed
    pub categories_renamed: usize,
    /// Levels processed (bottom-up)
    pub levels_processed: i32,
}

/// Merge small sibling categories that are semantically similar.
///
/// Post-hierarchy pass that consolidates fragmented categories:
/// 1. Works bottom-up (deepest levels first) so merges cascade properly
/// 2. For each parent, finds category children with ≤max_size children
/// 3. Clusters these small siblings by embedding similarity (threshold)
/// 4. Merges similar ones: reparent grandchildren to survivor, delete empty categories
/// 5. Uses AI to rename survivor based on combined children
///
/// Parameters:
/// - `threshold`: Cosine similarity threshold for clustering (default 0.7)
/// - `max_size`: Maximum children for a category to be considered "small" (default 3)
pub async fn merge_small_categories(
    db: &Database,
    app: Option<&AppHandle>,
    threshold: f32,
    max_size: i32,
) -> Result<MergeCategoriesResult, String> {
    emit_log(app, "info", &format!(
        "Starting small category merge (threshold={:.2}, max_size={})",
        threshold, max_size
    ));

    let protected_ids = db.get_protected_node_ids();
    let max_depth = db.get_max_depth().map_err(|e| e.to_string())?;

    let mut total_merged = 0usize;
    let mut total_reparented = 0usize;
    let mut total_renamed = 0usize;
    let mut levels_processed = 0i32;

    // Work bottom-up: start from deepest category level (max_depth - 1) down to depth 1
    // Items are at max_depth, categories start one level above
    // Don't process depth 0 (Universe)
    for depth in (1..max_depth).rev() {
        levels_processed += 1;
        let nodes_at_depth = db.get_nodes_at_depth(depth).map_err(|e| e.to_string())?;

        // Group nodes by parent_id to find siblings
        let mut siblings_by_parent: HashMap<String, Vec<Node>> = HashMap::new();
        for node in nodes_at_depth {
            // Skip items, protected nodes, and nodes without parents
            if node.is_item || protected_ids.contains(&node.id) {
                continue;
            }
            if let Some(parent_id) = &node.parent_id {
                siblings_by_parent.entry(parent_id.clone()).or_default().push(node);
            }
        }

        // Process each group of siblings
        for (parent_id, siblings) in siblings_by_parent {
            // Find "small" categories (≤max_size children)
            let small_cats: Vec<&Node> = siblings.iter()
                .filter(|n| n.child_count <= max_size && !protected_ids.contains(&n.id))
                .collect();

            if small_cats.len() < 2 {
                // Need at least 2 small siblings to potentially merge
                continue;
            }

            emit_log(app, "debug", &format!(
                "Found {} small categories under parent {} at depth {}",
                small_cats.len(), parent_id, depth
            ));

            // Get embeddings for small categories
            let embeddings: Vec<(String, Vec<f32>)> = small_cats.iter()
                .filter_map(|cat| {
                    db.get_node_embedding(&cat.id).ok().flatten()
                        .map(|emb| (cat.id.clone(), emb))
                })
                .collect();

            if embeddings.len() < 2 {
                emit_log(app, "debug", "Not enough embeddings for clustering");
                continue;
            }

            // Cluster by embedding similarity using simple single-linkage
            let clusters = cluster_by_similarity(&embeddings, threshold);

            // Process each cluster of similar categories
            for cluster in clusters {
                if cluster.len() < 2 {
                    continue; // Need at least 2 to merge
                }

                // Find cluster members in small_cats
                let cluster_cats: Vec<&Node> = cluster.iter()
                    .filter_map(|id| small_cats.iter().find(|c| &c.id == id).copied())
                    .collect();

                if cluster_cats.len() < 2 {
                    continue;
                }

                // Choose survivor: the one with most children, or first if tied
                let survivor = cluster_cats.iter()
                    .max_by_key(|c| c.child_count)
                    .unwrap();

                let to_merge: Vec<&&Node> = cluster_cats.iter()
                    .filter(|c| c.id != survivor.id)
                    .collect();

                if to_merge.is_empty() {
                    continue;
                }

                emit_log(app, "info", &format!(
                    "Merging {} categories into '{}' ({})",
                    to_merge.len(), survivor.title, survivor.id
                ));

                // Reparent grandchildren from merged categories to survivor
                let mut children_moved = 0;
                for cat in &to_merge {
                    let grandchildren = db.get_children(&cat.id).map_err(|e| e.to_string())?;
                    for grandchild in &grandchildren {
                        db.update_parent(&grandchild.id, &survivor.id).map_err(|e| e.to_string())?;
                        children_moved += 1;
                    }
                }

                total_reparented += children_moved;
                emit_log(app, "debug", &format!("Reparented {} children to survivor", children_moved));

                // Delete edges and then the now-empty merged categories
                for cat in &to_merge {
                    // First: reparent any remaining children to survivor (safety net)
                    let remaining_children = db.get_children(&cat.id).map_err(|e| e.to_string())?;
                    for child in &remaining_children {
                        db.update_parent(&child.id, &survivor.id).map_err(|e| e.to_string())?;
                        emit_log(app, "debug", &format!("Safety reparent: {} -> {}", child.id, survivor.id));
                    }
                    // Second: clear any orphaned parent_id references
                    db.clear_parent_references(&cat.id).map_err(|e| e.to_string())?;
                    // Third: delete fos_edges FIRST (FK lacks CASCADE, references both nodes and edges)
                    db.delete_fos_edges_for_node(&cat.id).map_err(|e| e.to_string())?;
                    // Fourth: delete all edges referencing this node
                    db.delete_edges_for_node(&cat.id).map_err(|e| e.to_string())?;
                    // Finally: delete the node
                    db.delete_node(&cat.id).map_err(|e| e.to_string())?;
                    total_merged += 1;
                }

                // Update survivor's child_count
                let new_children = db.get_children(&survivor.id).map_err(|e| e.to_string())?;
                db.update_child_count(&survivor.id, new_children.len() as i32)
                    .map_err(|e| e.to_string())?;

                // Rename survivor using AI based on combined children
                if let Ok(Some(new_name)) = generate_merged_category_name(db, &survivor.id, app).await {
                    if new_name != survivor.title {
                        // Update title, cluster_label, and ai_title
                        if let Ok(mut updated_node) = db.get_node(&survivor.id) {
                            if let Some(ref mut node) = updated_node {
                                node.title = new_name.clone();
                                node.cluster_label = Some(new_name.clone());
                                node.ai_title = Some(new_name.clone());
                                if let Err(e) = db.update_node(node) {
                                    emit_log(app, "warn", &format!("Failed to rename survivor: {}", e));
                                } else {
                                    emit_log(app, "info", &format!("Renamed survivor to '{}'", new_name));
                                    total_renamed += 1;
                                }
                            }
                        }
                    }
                }

                // Recompute centroid for survivor
                compute_and_store_centroid(db, &survivor.id);
            }
        }

        emit_log(app, "debug", &format!("Completed depth {} processing", depth));
    }

    // Update parent child counts after all merges
    if let Err(e) = db.fix_all_child_counts() {
        emit_log(app, "warn", &format!("Failed to fix child counts: {}", e));
    }

    emit_log(app, "info", &format!(
        "Merge complete: {} categories merged, {} children reparented, {} renamed across {} levels",
        total_merged, total_reparented, total_renamed, levels_processed
    ));

    Ok(MergeCategoriesResult {
        categories_merged: total_merged,
        children_reparented: total_reparented,
        categories_renamed: total_renamed,
        levels_processed,
    })
}

/// Cluster node IDs by embedding similarity using single-linkage clustering.
/// Returns groups of similar IDs (each group has similarity >= threshold with at least one other member).
fn cluster_by_similarity(embeddings: &[(String, Vec<f32>)], threshold: f32) -> Vec<Vec<String>> {
    let n = embeddings.len();
    if n < 2 {
        return vec![];
    }

    // Union-Find data structure for clustering
    let mut parent: Vec<usize> = (0..n).collect();

    fn find(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i {
            parent[i] = find(parent, parent[i]);
        }
        parent[i]
    }

    fn union(parent: &mut [usize], i: usize, j: usize) {
        let pi = find(parent, i);
        let pj = find(parent, j);
        if pi != pj {
            parent[pi] = pj;
        }
    }

    // Compare all pairs and union similar ones
    for i in 0..n {
        for j in (i + 1)..n {
            let sim = cosine_similarity(&embeddings[i].1, &embeddings[j].1);
            if sim >= threshold {
                union(&mut parent, i, j);
            }
        }
    }

    // Group by root
    let mut clusters: HashMap<usize, Vec<String>> = HashMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        clusters.entry(root).or_default().push(embeddings[i].0.clone());
    }

    // Return only clusters with 2+ members
    clusters.into_values()
        .filter(|c| c.len() >= 2)
        .collect()
}

/// Generate a new name for a merged category based on its combined children.
/// Uses AI to create a meaningful name that encompasses all children.
async fn generate_merged_category_name(
    db: &Database,
    category_id: &str,
    app: Option<&AppHandle>,
) -> Result<Option<String>, String> {
    let children = db.get_children(category_id).map_err(|e| e.to_string())?;
    if children.is_empty() {
        return Ok(None);
    }

    // Get parent context for better naming
    let category = db.get_node(category_id).map_err(|e| e.to_string())?;
    let parent_context = if let Some(ref cat) = category {
        if let Some(ref parent_id) = cat.parent_id {
            db.get_node(parent_id).ok().flatten()
                .map(|p| p.cluster_label.or(Some(p.title)).unwrap_or_default())
        } else {
            None
        }
    } else {
        None
    };

    // Use AI naming for merged categories
    emit_log(app, "info", &format!(
        "Using AI to name merged category with {} children", children.len()
    ));

    let topics: Vec<ai_client::TopicInfo> = children.iter()
        .map(|c| ai_client::TopicInfo {
            id: c.id.clone(),
            label: c.ai_title.clone()
                .or_else(|| c.cluster_label.clone())
                .unwrap_or_else(|| c.title.clone()),
            item_count: c.child_count.max(1),
        })
        .collect();

    let context = ai_client::GroupingContext {
        parent_name: parent_context.unwrap_or_else(|| "Knowledge".to_string()),
        parent_description: None,
        hierarchy_path: vec![],
        current_depth: category.as_ref().map(|c| c.depth).unwrap_or(1),
        sibling_names: vec![],
        forbidden_names: vec![],
        mandatory_clusters: vec![],
    };

    // Ask AI to suggest just ONE category name for these children
    match timeout(
        Duration::from_secs(30),
        ai_client::group_topics_into_categories(&topics, &context, Some(1))
    ).await {
        Ok(Ok(groupings)) if !groupings.is_empty() => {
            let name = groupings[0].name.clone();
            if !is_garbage_name(&name) {
                return Ok(Some(name));
            }
        }
        Ok(Err(e)) => {
            emit_log(app, "warn", &format!("AI naming failed: {}", e));
        }
        Err(_) => {
            emit_log(app, "warn", "AI naming timed out");
        }
        _ => {}
    }

    // AI failed - no name
    Ok(None)
}
