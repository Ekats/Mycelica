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
use crate::ai_client;
use crate::similarity::{cosine_similarity, compute_centroid};
use serde::Serialize;
use std::collections::HashMap;
use std::collections::HashSet;

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

// ==================== Coherence-Based Hierarchy Refinement ====================


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
const DEFAULT_MERGE_THRESHOLD: f32 = 0.65;

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

    // Delete edges and node
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
        let words: Vec<String> = title
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 2 && !stop_words.contains(w))
            .map(|w| w.to_string())
            .collect();

        let unique: HashSet<_> = words.into_iter().collect();
        for word in unique {
            *word_counts.entry(word).or_insert(0) += 1;
        }
    }

    let threshold = titles.len() / 2;
    let mut common: Vec<(String, usize)> = word_counts
        .into_iter()
        .filter(|(_, count)| *count > threshold)
        .collect();

    common.sort_by(|a, b| b.1.cmp(&a.1));
    common.into_iter().map(|(word, _)| word).collect()
}

/// Generate a name for a cluster of items using AI
async fn name_cluster_from_items(
    items: &[&Node],
    forbidden_names: &[String],
    app: Option<&AppHandle>,
) -> Result<String, String> {
    let sample_size = items.len().min(10);
    let mut sample_indices: Vec<usize> = Vec::new();

    if items.len() <= 10 {
        sample_indices = (0..items.len()).collect();
    } else {
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

    let titles: Vec<String> = sample_indices.iter()
        .map(|&i| items[i].ai_title.clone()
            .or_else(|| items[i].cluster_label.clone())
            .unwrap_or_else(|| items[i].title.clone()))
        .collect();

    match crate::ai_client::name_cluster_from_samples(&titles, forbidden_names).await {
        Ok(name) if !is_garbage_name(&name) => return Ok(name),
        Ok(_) => {}
        Err(e) => {
            emit_log(app, "warn", &format!("AI naming failed: {}", e));
        }
    }

    let common = find_common_keywords(&titles);
    if !common.is_empty() {
        return Ok(common.into_iter().take(3).collect::<Vec<_>>().join(" "));
    }

    Ok(format!("Group ({} items)", items.len()))
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

/// Create the Universe root node (depth 0, is_universe = true)
fn create_universe(db: &Database, child_ids: &[String]) -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let universe_id = "universe".to_string();

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
                    // Third: delete all edges referencing this node
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
