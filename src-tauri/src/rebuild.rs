//! Shared hierarchy rebuild logic.
//!
//! Extracted from cli.rs so both the CLI and the HTTP server can invoke
//! `rebuild_adaptive()` without duplicating the 700-line orchestration.

use crate::ai_client;
use crate::db::{Database, Node, NodeType, Position, Edge, EdgeType};
use crate::dendrogram::{build_adaptive_tree, auto_config, AdaptiveTreeConfig, TreeNode, EdgeIndex};
use crate::settings;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

// ============================================================================
// Public types
// ============================================================================

/// Configuration knobs for the adaptive rebuild.
pub struct RebuildConfig {
    pub min_size: usize,
    pub tight_threshold: f64,
    pub cohesion_threshold: f64,
    pub delta_min: f64,
    pub auto_config: bool,
    pub keywords_only: bool,
    pub fresh: bool,
}

impl Default for RebuildConfig {
    fn default() -> Self {
        Self {
            min_size: 5,
            tight_threshold: 0.001,
            cohesion_threshold: 1.0,
            delta_min: 0.02,
            auto_config: true,
            keywords_only: false,
            fresh: true,
        }
    }
}

/// Summary returned after a successful rebuild.
pub struct RebuildResult {
    pub categories: usize,
    pub papers_assigned: usize,
    pub sibling_edges: usize,
    pub bridges: usize,
}

// ============================================================================
// Helpers
// ============================================================================

/// Returns `true` when >=90 % of characters in the concatenated texts are ASCII.
/// Used to gate local-LLM naming (Ollama hallucinates on non-English text).
pub fn is_predominantly_english(texts: &[String]) -> bool {
    if texts.is_empty() {
        return true;
    }
    let combined: String = texts.join(" ");
    if combined.is_empty() {
        return true;
    }
    let total_chars = combined.chars().count();
    let ascii_chars = combined.chars().filter(|c| c.is_ascii()).count();
    let ascii_ratio = ascii_chars as f64 / total_chars as f64;
    ascii_ratio > 0.90
}

// ============================================================================
// Core rebuild
// ============================================================================

/// Run the full adaptive hierarchy rebuild.
///
/// `progress` is called with human-readable status messages so each caller can
/// decide what to do with them (CLI: print to stderr, server: collect in a Vec).
pub async fn rebuild_adaptive(
    db: &Database,
    config: RebuildConfig,
    progress: &(dyn Fn(&str) + Send + Sync),
) -> Result<RebuildResult, String> {
    let start = Instant::now();
    let now = chrono::Utc::now().timestamp_millis();

    // Step 1: Clear existing hierarchy
    if config.fresh {
        progress("Step 1/7: Fresh rebuild — clearing algorithm-generated categories (preserving human modifications)...");
    } else {
        progress("Step 1/7: Clearing existing hierarchy...");
    }
    db.delete_hierarchy_nodes().map_err(|e| e.to_string())?;

    // Step 2: Load edges and check threshold
    progress("Step 2/7: Loading edges...");
    let edges = db.get_all_item_edges_sorted().map_err(|e| e.to_string())?;
    if edges.is_empty() {
        return Err("No edges found. Run 'mycelica-cli setup' to create semantic edges first.".to_string());
    }
    progress(&format!("  {} edges loaded", edges.len()));

    // Check edge weight distribution
    let min_weight = edges.iter().map(|(_, _, w)| *w).fold(f64::INFINITY, f64::min);
    if min_weight > 0.4 {
        progress(&format!("  Warning: Minimum edge weight is {:.2}. Consider regenerating edges at lower threshold for finer resolution.", min_weight));
    }

    // Sovereignty: extract pinned items (human-edited parent_id)
    let pinned_items = db.get_pinned_items().map_err(|e| e.to_string())?;
    let pinned_ids: HashSet<String> = pinned_items.iter().map(|(id, _)| id.clone()).collect();
    if !pinned_ids.is_empty() {
        progress(&format!("  {} pinned items excluded from clustering (human-edited parent_id)", pinned_ids.len()));
    }

    // Extract unique paper IDs, filtering out pinned items
    let mut paper_set = HashSet::new();
    for (source, target, _) in &edges {
        paper_set.insert(source.clone());
        paper_set.insert(target.clone());
    }
    let papers: Vec<String> = paper_set.into_iter()
        .filter(|id| !pinned_ids.contains(id))
        .collect();
    progress(&format!("  {} unique papers (after excluding pinned)", papers.len()));

    // Always compute edge statistics for dynamic scaling
    let auto_cfg = auto_config(&edges, papers.len());

    // Compute config (auto uses all auto values, manual uses CLI args but keeps iqr/density)
    let tree_config = if config.auto_config {
        progress(&format!("  Auto-config: min_size={}, cohesion={:.2}, delta_min={:.3}",
            auto_cfg.min_size, auto_cfg.cohesion_threshold, auto_cfg.delta_min));
        progress(&format!("  Distribution: IQR={:.3}, density={:.4}", auto_cfg.iqr, auto_cfg.edge_density));
        auto_cfg
    } else {
        AdaptiveTreeConfig {
            min_size: config.min_size,
            tight_threshold: config.tight_threshold,
            cohesion_threshold: config.cohesion_threshold,
            delta_min: config.delta_min,
            // Use actual data statistics for dynamic scaling even in manual mode
            iqr: auto_cfg.iqr,
            edge_density: auto_cfg.edge_density,
            max_depth: 7,
        }
    };

    // Step 3: Build adaptive tree
    progress("Step 3/7: Building adaptive tree...");
    let (root, sibling_edges) = build_adaptive_tree(papers.clone(), edges.clone(), Some(tree_config.clone()));

    // Count tree structure
    fn count_nodes(node: &TreeNode) -> (usize, usize) {
        match node {
            TreeNode::Leaf { papers, .. } => (1, papers.len()),
            TreeNode::Internal { children, papers, .. } => {
                let mut leaves = 0;
                let mut total_papers = papers.len();
                for child in children {
                    let (l, p) = count_nodes(child);
                    leaves += l;
                    total_papers = total_papers.max(p);
                }
                (leaves, total_papers)
            }
        }
    }
    let (leaf_count, _) = count_nodes(&root);
    progress(&format!("  {} leaf categories, {} sibling edges", leaf_count, sibling_edges.len()));

    // Step 4: Create Universe and flatten tree to DB
    progress("Step 4/7: Creating categories in database...");

    // Create or update Universe
    let universe_id = "universe".to_string();
    let universe_exists = db.get_node(&universe_id).map_err(|e| e.to_string())?.is_some();

    if !universe_exists {
        let universe = Node {
            id: universe_id.clone(),
            node_type: NodeType::Cluster,
            title: "All Knowledge".to_string(),
            url: None,
            content: None,
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: None,
            depth: 0,
            is_item: false,
            is_universe: true,
            parent_id: None,
            child_count: 0,
            ai_title: None,
            summary: None,
            tags: None,
            emoji: None,
            is_processed: false,
            conversation_id: None,
            sequence_index: None,
            is_pinned: false,
            last_accessed_at: None,
            source: None,
            is_private: None,
            privacy_reason: None,
            privacy: None,
            content_type: None,
            associated_idea_id: None,
            latest_child_date: None,
            pdf_available: None,
            human_edited: None,
            human_created: false,
            author: None,
            agent_id: None,
            node_class: None,
            meta_type: None,
        };
        db.insert_node(&universe).map_err(|e| e.to_string())?;
    }

    // Flatten tree to categories
    let mut categories_created = 0;
    let mut papers_assigned: HashSet<String> = HashSet::new();
    let mut bridge_count = 0;
    let mut category_id_counter = 0;
    let mut node_to_category: HashMap<String, String> = HashMap::new();

    fn flatten_tree(
        node: &TreeNode,
        parent_id: &str,
        depth: i32,
        db: &Database,
        now: i64,
        categories_created: &mut usize,
        papers_assigned: &mut HashSet<String>,
        bridge_count: &mut usize,
        id_counter: &mut usize,
        node_to_category: &mut HashMap<String, String>,
    ) -> Result<String, String> {
        let category_id = format!("adaptive-{}", *id_counter);
        *id_counter += 1;

        // Map tree node ID to category ID
        node_to_category.insert(node.id().to_string(), category_id.clone());

        match node {
            TreeNode::Leaf { papers, .. } => {
                // Create category node
                let category = Node {
                    id: category_id.clone(),
                    node_type: NodeType::Cluster,
                    title: format!("Category {}", *id_counter - 1),
                    url: None,
                    content: None,
                    position: Position { x: 0.0, y: 0.0 },
                    created_at: now,
                    updated_at: now,
                    cluster_id: None,
                    cluster_label: None,
                    depth,
                    is_item: false,
                    is_universe: false,
                    parent_id: Some(parent_id.to_string()),
                    child_count: papers.len() as i32,
                    ai_title: None,
                    summary: None,
                    tags: None,
                    emoji: None,
                    is_processed: false,
                    conversation_id: None,
                    sequence_index: None,
                    is_pinned: false,
                    last_accessed_at: None,
                    source: None,
                    is_private: None,
                    privacy_reason: None,
                    privacy: None,
                    content_type: None,
                    associated_idea_id: None,
                    latest_child_date: None,
                    pdf_available: None,
                    human_edited: None,
                    human_created: false,
                    author: None,
                    agent_id: None,
                    node_class: None,
                    meta_type: None,
                };
                db.insert_node(&category).map_err(|e| e.to_string())?;
                *categories_created += 1;

                // Assign papers to this category
                for paper_id in papers {
                    db.update_node_hierarchy(paper_id, Some(&category_id), depth + 1)
                        .map_err(|e| e.to_string())?;
                    papers_assigned.insert(paper_id.clone());
                }
            }
            TreeNode::Internal { children, papers, bridges, .. } => {
                // Collect papers that made it into children
                let child_papers: HashSet<&String> = children.iter()
                    .flat_map(|c| match c {
                        TreeNode::Leaf { papers, .. } => papers.iter(),
                        TreeNode::Internal { papers, .. } => papers.iter(),
                    })
                    .collect();

                // Papers in this node but not in any child (orphans from small components)
                let orphan_papers: Vec<&String> = papers.iter()
                    .filter(|p| !child_papers.contains(p))
                    .collect();

                // Create internal category node
                let category = Node {
                    id: category_id.clone(),
                    node_type: NodeType::Cluster,
                    title: format!("Category {}", *id_counter - 1),
                    url: None,
                    content: None,
                    position: Position { x: 0.0, y: 0.0 },
                    created_at: now,
                    updated_at: now,
                    cluster_id: None,
                    cluster_label: None,
                    depth,
                    is_item: false,
                    is_universe: false,
                    parent_id: Some(parent_id.to_string()),
                    child_count: (children.len() + if orphan_papers.is_empty() { 0 } else { orphan_papers.len() }) as i32,
                    ai_title: None,
                    summary: None,
                    tags: None,
                    emoji: None,
                    is_processed: false,
                    conversation_id: None,
                    sequence_index: None,
                    is_pinned: false,
                    last_accessed_at: None,
                    source: None,
                    is_private: None,
                    privacy_reason: None,
                    privacy: None,
                    content_type: None,
                    associated_idea_id: None,
                    latest_child_date: None,
                    pdf_available: None,
                    human_edited: None,
                    human_created: false,
                    author: None,
                    agent_id: None,
                    node_class: None,
                    meta_type: None,
                };
                db.insert_node(&category).map_err(|e| e.to_string())?;
                *categories_created += 1;

                // Assign orphan papers directly to this category
                for paper_id in &orphan_papers {
                    db.update_node_hierarchy(paper_id, Some(&category_id), depth + 1)
                        .map_err(|e| e.to_string())?;
                    papers_assigned.insert(paper_id.to_string());
                }

                // Track bridge count
                *bridge_count += bridges.len();

                // Recurse into children
                for child in children {
                    flatten_tree(
                        child,
                        &category_id,
                        depth + 1,
                        db,
                        now,
                        categories_created,
                        papers_assigned,
                        bridge_count,
                        id_counter,
                        node_to_category,
                    )?;
                }
            }
        }

        Ok(category_id)
    }

    // Special case: if root is a leaf (all papers in one group), handle directly
    match &root {
        TreeNode::Leaf { papers, .. } => {
            // Assign all papers directly under Universe
            for paper_id in papers {
                db.update_node_hierarchy(paper_id, Some(&universe_id), 1)
                    .map_err(|e| e.to_string())?;
                papers_assigned.insert(paper_id.clone());
            }
            categories_created = 0; // No intermediate categories
        }
        TreeNode::Internal { children, papers, .. } => {
            // Collect papers that made it into children
            let child_papers: HashSet<&String> = children.iter()
                .flat_map(|c| match c {
                    TreeNode::Leaf { papers, .. } => papers.iter(),
                    TreeNode::Internal { papers, .. } => papers.iter(),
                })
                .collect();

            // Papers at root that didn't make it into any child (orphans)
            let orphan_papers: Vec<&String> = papers.iter()
                .filter(|p| !child_papers.contains(p))
                .collect();

            // Flatten children under Universe
            for child in children {
                flatten_tree(
                    child,
                    &universe_id,
                    1,
                    db,
                    now,
                    &mut categories_created,
                    &mut papers_assigned,
                    &mut bridge_count,
                    &mut category_id_counter,
                    &mut node_to_category,
                )?;
            }

            // Assign root-level orphan papers directly under Universe
            if !orphan_papers.is_empty() {
                progress(&format!("  {} orphan papers (small components) assigned to Universe", orphan_papers.len()));
                for paper_id in &orphan_papers {
                    db.update_node_hierarchy(paper_id, Some(&universe_id), 1)
                        .map_err(|e| e.to_string())?;
                    papers_assigned.insert(paper_id.to_string());
                }
            }
        }
    }

    // Update Universe child count
    let universe_children = db.get_children(&universe_id).map_err(|e| e.to_string())?;
    db.update_child_count(&universe_id, universe_children.len() as i32)
        .map_err(|e| e.to_string())?;

    progress(&format!("  {} categories, {} papers assigned", categories_created, papers_assigned.len()));

    // Step 5: Create sibling edges with bridge metadata
    progress("Step 5/7: Creating sibling edges...");
    let mut sibling_edges_created = 0;

    for meta in &sibling_edges {
        // Map tree node IDs to category IDs
        let source_cat = node_to_category.get(&meta.source_id);
        let target_cat = node_to_category.get(&meta.target_id);

        if let (Some(source), Some(target)) = (source_cat, target_cat) {
            // Serialize bridges to JSON for label
            let bridges_json = serde_json::to_string(&meta.bridges).unwrap_or_else(|_| "[]".to_string());

            let edge = Edge {
                id: format!("sibling-{}-{}", source, target),
                source: source.clone(),
                target: target.clone(),
                edge_type: EdgeType::Sibling,
                label: Some(bridges_json),
                weight: Some(meta.weight),
                edge_source: Some("adaptive".to_string()),
                evidence_id: None,
                confidence: Some(1.0),
                created_at: now,
                updated_at: Some(now),
                author: None,
                reason: None,
                content: None,
                agent_id: None,
                superseded_by: None,
                metadata: None,
            };
            if db.insert_edge(&edge).is_ok() {
                sibling_edges_created += 1;
            }
        }
    }

    progress(&format!("  {} sibling edges created", sibling_edges_created));

    // Sovereignty: validate pinned parents still exist (follow merges, orphan if deleted)
    if !pinned_ids.is_empty() {
        let warnings = db.validate_pinned_parents().map_err(|e| e.to_string())?;
        for w in &warnings {
            progress(&format!("  Warning: {}", w));
        }
        if warnings.is_empty() {
            progress(&format!("  {} pinned items validated — parents intact", pinned_ids.len()));
        }
    }

    // Step 6: Name categories (bottom-up)
    let llm_available = !config.keywords_only && (ai_client::ollama_available().await || ai_client::is_available());
    let is_local_llm = settings::get_llm_backend() == "ollama";
    if llm_available {
        if is_local_llm {
            progress("Step 6/7: Naming categories with local LLM (keyword fallback for non-English)...");
        } else {
            progress("Step 6/7: Naming categories with LLM...");
        }
    } else {
        progress("Step 6/7: Naming categories with keyword extraction...");
    }

    // Collect adaptive categories by depth (to process bottom-up)
    let mut categories_by_depth: Vec<Vec<Node>> = Vec::new();
    let mut max_cat_depth: i32 = 1;
    for depth in 1i32..=50 {
        let cats_at_depth: Vec<Node> = db.get_nodes_at_depth(depth)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|n| n.id.starts_with("adaptive-"))
            .collect();
        if cats_at_depth.is_empty() && depth > 1 {
            break;
        }
        if !cats_at_depth.is_empty() {
            max_cat_depth = depth;
        }
        categories_by_depth.push(cats_at_depth);
    }

    // Get all existing category names to avoid duplicates
    let mut forbidden_names: HashSet<String> = db.get_all_category_names()
        .unwrap_or_default()
        .into_iter()
        .collect();

    // Process bottom-up: deepest categories first (they have item children)
    let mut named_count = 0;
    for cats_at_depth in categories_by_depth.iter().rev() {
        for category in cats_at_depth {
            let children = db.get_children(&category.id).map_err(|e| e.to_string())?;

            // Collect titles from children (items or already-named subcategories)
            let titles: Vec<String> = children.iter()
                .filter(|c| !c.title.is_empty() && c.title != "Category")
                .take(15)
                .map(|c| c.title.clone())
                .collect();

            if !titles.is_empty() {
                // Check if we should use LLM for this category
                let use_llm_for_this = llm_available &&
                    (!is_local_llm || is_predominantly_english(&titles));

                let name = if use_llm_for_this {
                    // Get parent name for context
                    let parent_name: Option<String> = if let Some(parent_id) = &category.parent_id {
                        db.get_node(parent_id)
                            .ok()
                            .flatten()
                            .and_then(|p| if p.title != "Universe" && !p.title.is_empty() { Some(p.title.clone()) } else { None })
                    } else {
                        None
                    };

                    // Use LLM with parent context if available
                    let forbidden_vec: Vec<String> = forbidden_names.iter().cloned().collect();
                    let llm_result = if let Some(parent) = parent_name {
                        ai_client::name_cluster_with_parent(&titles, &parent, &forbidden_vec).await
                    } else {
                        ai_client::name_cluster_from_samples(&titles, &forbidden_vec).await
                    };

                    // Fall back to keyword extraction if LLM fails
                    match llm_result {
                        Ok(n) if !n.is_empty() => n,
                        _ => {
                            if titles.len() > 5 {
                                ai_client::extract_top_keywords(&titles, 4)
                            } else {
                                ai_client::extract_top_keywords(&titles, 3)
                            }
                        }
                    }
                } else {
                    // Keyword extraction only
                    if titles.len() > 5 {
                        ai_client::extract_top_keywords(&titles, 4)
                    } else {
                        ai_client::extract_top_keywords(&titles, 3)
                    }
                };

                if !name.is_empty() && name != "Category" {
                    db.update_node_title(&category.id, &name).map_err(|e| e.to_string())?;
                    forbidden_names.insert(name.clone());
                    named_count += 1;
                }
            }
        }
    }

    progress(&format!("  {} categories named", named_count));

    // Step 7.5: Merge binary cascades where AI couldn't differentiate siblings
    progress("Step 7.5/7: Merging similar binary siblings...");
    let merged = crate::dendrogram::merge_similar_binary_siblings(db, 0.75)
        .map_err(|e| e.to_string())?;
    if merged > 0 {
        progress(&format!("  Merged {} redundant binary splits", merged));
        // Update parent child counts after merging
        let all_parents: Vec<String> = db.get_all_nodes(false)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|n| !n.is_item)
            .map(|n| n.id)
            .collect();
        for parent_id in &all_parents {
            let child_count = db.get_children(parent_id).map_err(|e| e.to_string())?.len();
            let _ = db.update_child_count(parent_id, child_count as i32);
        }
    }

    // Step 7.6: Collapse binary cascade routing nodes
    progress("Step 7.6/7: Collapsing binary cascades...");
    let collapsed = crate::dendrogram::collapse_binary_cascades(db)
        .map_err(|e| e.to_string())?;
    if collapsed > 0 {
        progress(&format!("  Collapsed {} binary routing nodes", collapsed));
    }

    // Step 7: Final-pass orphan assignment using nearest-neighbor
    progress("Step 7/7: Final-pass orphan assignment...");

    // Get orphans (items at depth=1 under Universe)
    let orphan_nodes: Vec<Node> = db.get_children(&universe_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|n| n.is_item)
        .collect();

    if !orphan_nodes.is_empty() {
        // Build edge index for O(1) weight lookups
        let edge_index = EdgeIndex::new(&edges);

        // Get all leaf categories
        let mut all_categories: Vec<Node> = Vec::new();
        for depth in 1..=max_cat_depth {
            let cats_at_depth: Vec<Node> = db.get_nodes_at_depth(depth)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|n| !n.is_item && n.id.starts_with("adaptive-"))
                .collect();
            if cats_at_depth.is_empty() && depth > 1 {
                break;
            }
            all_categories.extend(cats_at_depth);
        }

        // For each category, get its item children
        let mut leaf_categories: Vec<(String, Vec<String>, i32)> = Vec::new();
        for cat in &all_categories {
            let children = db.get_children(&cat.id).map_err(|e| e.to_string())?;
            let item_children: Vec<String> = children.iter()
                .filter(|c| c.is_item)
                .map(|c| c.id.clone())
                .collect();
            if !item_children.is_empty() {
                leaf_categories.push((cat.id.clone(), item_children, cat.depth));
            }
        }

        progress(&format!("  {} orphans, {} leaf categories", orphan_nodes.len(), leaf_categories.len()));

        let mut rescued = 0;
        let mut truly_isolated = 0;

        for orphan in &orphan_nodes {
            let mut best_cat: Option<&str> = None;
            let mut best_avg: f64 = 0.0;
            let mut best_depth: i32 = 0;

            for (cat_id, members, depth) in &leaf_categories {
                let mut sum = 0.0;
                let mut count = 0;
                for member in members {
                    if let Some(w) = edge_index.weight(&orphan.id, member) {
                        sum += w;
                        count += 1;
                    }
                }
                if count > 0 {
                    let avg = sum / count as f64;
                    if avg > best_avg {
                        best_avg = avg;
                        best_cat = Some(cat_id);
                        best_depth = *depth;
                    }
                }
            }

            if let Some(cat_id) = best_cat {
                db.update_node_hierarchy(&orphan.id, Some(cat_id), best_depth + 1)
                    .map_err(|e| e.to_string())?;
                papers_assigned.insert(orphan.id.clone());
                rescued += 1;
            } else {
                truly_isolated += 1;
            }
        }

        // Update child counts for affected categories
        for (cat_id, _, _) in &leaf_categories {
            let children = db.get_children(cat_id).map_err(|e| e.to_string())?;
            db.update_child_count(cat_id, children.len() as i32).map_err(|e| e.to_string())?;
        }

        progress(&format!("  {} orphans rescued via NN assignment", rescued));
        if truly_isolated > 0 {
            progress(&format!("  {} truly isolated (no edges to any category)", truly_isolated));
        }
    }

    // Final step: Recalculate child_count for all categories
    let updated = db.recalculate_all_child_counts().map_err(|e| e.to_string())?;
    progress(&format!("  Updated child_count for {} categories", updated));

    // Update edge parent IDs so edges appear in the correct views
    let edge_updates = db.update_edge_parents().map_err(|e| e.to_string())?;
    progress(&format!("  Updated parent IDs for {} edges", edge_updates));

    let elapsed = start.elapsed();
    progress(&format!("Completed in {:.1}s", elapsed.as_secs_f64()));

    Ok(RebuildResult {
        categories: categories_created,
        papers_assigned: papers_assigned.len(),
        sibling_edges: sibling_edges_created,
        bridges: bridge_count,
    })
}
