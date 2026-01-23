//! Dendrogram-based hierarchy building from edge weights.
//!
//! The core insight: edges between papers already encode similarity as weights (0.5-1.0).
//! Hierarchy emerges naturally by finding connected components at different weight thresholds.
//! No separate clustering needed. The dendrogram IS the hierarchy.
//!
//! ## V2 Adaptive Tree Algorithm
//!
//! The adaptive tree algorithm builds hierarchy recursively with per-subtree thresholds:
//! - Each split is validated (gap, balance, cohesion checks)
//! - Bridge papers near cut thresholds get multi-parent membership
//! - Sibling edges include bridge metadata

use std::collections::{HashMap, HashSet};

/// Algorithm constants for v2 adaptive tree building.
pub mod constants {
    /// Minimum similarity to store as edge (edges below this lack sufficient signal).
    pub const EDGE_FLOOR: f64 = 0.3;

    /// Minimum papers to form a named group.
    pub const MIN_SIZE: usize = 5;

    /// Variance threshold below which group is already cohesive (no split needed).
    /// Note: Set very low because variance alone doesn't detect two homogeneous subgroups.
    /// The real check is whether find_valid_splits returns any valid splits.
    pub const TIGHT_THRESHOLD: f64 = 0.001;

    /// Minimum intra/inter ratio for valid split.
    pub const COHESION_THRESHOLD: f64 = 1.2;

    /// Minimum gap between parent and child cut thresholds.
    pub const DELTA_MIN: f64 = 0.03;
}

/// A merge event in the dendrogram - records when two components joined.
#[derive(Debug, Clone)]
pub struct MergeEvent {
    /// Unique ID for this merge (format: "merge-{index}")
    pub id: String,
    /// Left child - paper ID or previous merge ID
    pub left: String,
    /// Right child - paper ID or previous merge ID
    pub right: String,
    /// Edge weight at which merge occurred
    pub weight: f64,
    /// Total papers in the merged component
    pub size: usize,
}

/// Complete dendrogram built from edge weights.
#[derive(Debug)]
pub struct Dendrogram {
    /// Merge events ordered by weight descending (tightest clusters first)
    pub merges: Vec<MergeEvent>,
    /// Map from paper ID to its index in the leaf set
    pub paper_to_leaf: HashMap<String, usize>,
    /// Root merge ID (or None if only one paper)
    pub root: Option<String>,
    /// All paper IDs
    pub papers: Vec<String>,
}

/// A component at a particular threshold level.
#[derive(Debug, Clone)]
pub struct Component {
    /// Unique ID for this component
    pub id: String,
    /// Paper IDs in this component
    pub papers: Vec<String>,
    /// Parent component ID (at lower threshold)
    pub parent: Option<String>,
    /// Child component IDs (at higher threshold)
    pub children: Vec<String>,
    /// Weight at which this component formed (merge weight)
    pub merge_weight: Option<f64>,
}

/// Hierarchy levels extracted from dendrogram.
#[derive(Debug)]
pub struct HierarchyLevels {
    /// Thresholds used to cut the dendrogram
    pub thresholds: Vec<f64>,
    /// Levels from root (index 0) to leaves (last index)
    /// Each level is a vec of components at that threshold
    pub levels: Vec<Vec<Component>>,
}

/// Configuration for dendrogram hierarchy building.
#[derive(Debug, Clone)]
pub struct DendrogramConfig {
    /// Target number of hierarchy levels (default: 4)
    pub target_levels: usize,
    /// Maximum papers in a component before recursive subdivision (default: 500)
    pub max_component_size: usize,
    /// Minimum papers to create a named category (default: 5)
    pub min_component_size: usize,
}

impl Default for DendrogramConfig {
    fn default() -> Self {
        Self {
            target_levels: 4,
            max_component_size: 500,
            min_component_size: 5,
        }
    }
}

// ============================================================================
// V2 Adaptive Tree Algorithm Types
// ============================================================================

/// Configuration for v2 adaptive tree building.
#[derive(Debug, Clone)]
pub struct AdaptiveTreeConfig {
    /// Minimum papers to form a named group.
    pub min_size: usize,
    /// Variance threshold below which group is already cohesive.
    pub tight_threshold: f64,
    /// Minimum intra/inter ratio for valid split.
    pub cohesion_threshold: f64,
    /// Minimum gap between parent and child cut thresholds.
    pub delta_min: f64,
}

impl Default for AdaptiveTreeConfig {
    fn default() -> Self {
        Self {
            min_size: constants::MIN_SIZE,
            tight_threshold: constants::TIGHT_THRESHOLD,
            cohesion_threshold: constants::COHESION_THRESHOLD,
            delta_min: constants::DELTA_MIN,
        }
    }
}

/// A similarity range for a subtree.
#[derive(Debug, Clone, Copy)]
pub struct SimRange {
    pub min: f64,
    pub max: f64,
}

impl SimRange {
    pub fn new(min: f64, max: f64) -> Self {
        Self { min, max }
    }

    /// Width of the similarity range.
    pub fn width(&self) -> f64 {
        self.max - self.min
    }
}

/// A candidate split at a threshold.
#[derive(Debug, Clone)]
pub struct Split {
    /// The threshold at which this split occurs.
    pub threshold: f64,
    /// Paper IDs per child component.
    pub children: Vec<Vec<String>>,
    /// Quality metric: n_children * balance.
    pub quality: f64,
}

/// Result of build_tree: either a leaf or an internal node.
#[derive(Debug, Clone)]
pub enum TreeNode {
    /// A leaf node containing papers that can't be split further.
    Leaf {
        id: String,
        papers: Vec<String>,
        range: SimRange,
    },
    /// An internal node with children created by splitting at a threshold.
    Internal {
        id: String,
        papers: Vec<String>,
        children: Vec<TreeNode>,
        /// The cut threshold used for this split.
        threshold: f64,
        range: SimRange,
        /// Bridge paper IDs (papers near the cut that belong to multiple children).
        bridges: Vec<String>,
    },
}

impl TreeNode {
    /// Get the node ID.
    pub fn id(&self) -> &str {
        match self {
            TreeNode::Leaf { id, .. } => id,
            TreeNode::Internal { id, .. } => id,
        }
    }

    /// Get the papers in this node.
    pub fn papers(&self) -> &[String] {
        match self {
            TreeNode::Leaf { papers, .. } => papers,
            TreeNode::Internal { papers, .. } => papers,
        }
    }

    /// Check if this is a leaf node.
    pub fn is_leaf(&self) -> bool {
        matches!(self, TreeNode::Leaf { .. })
    }

    /// Get children if internal node, empty slice if leaf.
    pub fn children(&self) -> &[TreeNode] {
        match self {
            TreeNode::Leaf { .. } => &[],
            TreeNode::Internal { children, .. } => children,
        }
    }
}

/// Metadata for sibling edges with bridge information.
#[derive(Debug, Clone)]
pub struct SiblingEdgeMeta {
    /// Source category ID.
    pub source_id: String,
    /// Target category ID.
    pub target_id: String,
    /// Mean similarity between the two groups (inter similarity).
    pub weight: f64,
    /// Cut threshold where the split occurred.
    pub threshold: f64,
    /// Paper IDs that bridge both siblings.
    pub bridges: Vec<String>,
}

/// Pre-indexed edge data for efficient lookups during tree building.
pub struct EdgeIndex {
    /// paper_id -> Vec<(other_paper_id, weight)> sorted by weight DESC.
    edges_by_paper: HashMap<String, Vec<(String, f64)>>,
    /// (paper_a, paper_b) -> weight where a < b lexicographically.
    edge_weights: HashMap<(String, String), f64>,
    /// All paper IDs.
    papers: HashSet<String>,
}

impl EdgeIndex {
    /// Create a new EdgeIndex from edges.
    pub fn new(edges: &[(String, String, f64)]) -> Self {
        let mut edges_by_paper: HashMap<String, Vec<(String, f64)>> = HashMap::new();
        let mut edge_weights: HashMap<(String, String), f64> = HashMap::new();
        let mut papers: HashSet<String> = HashSet::new();

        for (a, b, weight) in edges {
            papers.insert(a.clone());
            papers.insert(b.clone());

            // Store in both directions for edges_by_paper
            edges_by_paper
                .entry(a.clone())
                .or_default()
                .push((b.clone(), *weight));
            edges_by_paper
                .entry(b.clone())
                .or_default()
                .push((a.clone(), *weight));

            // Store with canonical key (smaller string first)
            let key = if a < b {
                (a.clone(), b.clone())
            } else {
                (b.clone(), a.clone())
            };
            edge_weights.insert(key, *weight);
        }

        // Sort edges by weight descending for each paper
        for edges in edges_by_paper.values_mut() {
            edges.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        }

        Self {
            edges_by_paper,
            edge_weights,
            papers,
        }
    }

    /// Get weight between two papers (None if no edge).
    pub fn weight(&self, a: &str, b: &str) -> Option<f64> {
        let key = if a < b {
            (a.to_string(), b.to_string())
        } else {
            (b.to_string(), a.to_string())
        };
        self.edge_weights.get(&key).copied()
    }

    /// Get all edges for a paper (sorted by weight descending).
    pub fn edges_for(&self, paper_id: &str) -> &[(String, f64)] {
        self.edges_by_paper
            .get(paper_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Get max edge weight for a paper.
    pub fn max_edge(&self, paper_id: &str) -> Option<f64> {
        self.edges_for(paper_id).first().map(|(_, w)| *w)
    }

    /// Get internal edges for a group (both endpoints in the group).
    pub fn internal_edges(&self, papers: &[String]) -> Vec<(String, String, f64)> {
        let paper_set: HashSet<&String> = papers.iter().collect();
        let mut result = Vec::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();

        for paper in papers {
            for (other, weight) in self.edges_for(paper) {
                if paper_set.contains(other) {
                    let key = if paper < other {
                        (paper.clone(), other.clone())
                    } else {
                        (other.clone(), paper.clone())
                    };
                    if seen.insert(key.clone()) {
                        result.push((key.0, key.1, *weight));
                    }
                }
            }
        }

        result
    }

    /// Compute intra-group similarity (mean internal edge weight).
    pub fn intra(&self, papers: &[String]) -> f64 {
        let edges = self.internal_edges(papers);
        if edges.is_empty() {
            return 0.0;
        }
        let sum: f64 = edges.iter().map(|(_, _, w)| w).sum();
        sum / edges.len() as f64
    }

    /// Compute inter-group similarity (mean edge weight crossing boundary).
    pub fn inter(&self, group_a: &[String], group_b: &[String]) -> f64 {
        let set_b: HashSet<&String> = group_b.iter().collect();

        let mut sum = 0.0;
        let mut count = 0;

        for paper_a in group_a {
            for (other, weight) in self.edges_for(paper_a) {
                if set_b.contains(other) {
                    sum += weight;
                    count += 1;
                }
            }
        }

        if count == 0 {
            0.0
        } else {
            sum / count as f64
        }
    }

    /// Check if a paper exists in the index.
    pub fn contains(&self, paper_id: &str) -> bool {
        self.papers.contains(paper_id)
    }

    /// Get all paper IDs.
    pub fn all_papers(&self) -> Vec<String> {
        self.papers.iter().cloned().collect()
    }
}

/// Union-Find data structure with merge tracking.
pub struct UnionFind {
    /// Parent pointers (paper_id -> parent_id, or self if root)
    parent: HashMap<String, String>,
    /// Rank for union by rank
    rank: HashMap<String, usize>,
    /// Component sizes
    size: HashMap<String, usize>,
}

impl UnionFind {
    /// Create a new UnionFind with each paper as its own component.
    pub fn new(papers: &[String]) -> Self {
        let mut parent = HashMap::new();
        let mut rank = HashMap::new();
        let mut size = HashMap::new();

        for paper in papers {
            parent.insert(paper.clone(), paper.clone());
            rank.insert(paper.clone(), 0);
            size.insert(paper.clone(), 1);
        }

        Self { parent, rank, size }
    }

    /// Find the root of the component containing `id`, with path compression.
    pub fn find(&mut self, id: &str) -> String {
        let parent = self.parent.get(id).cloned().unwrap_or_else(|| id.to_string());
        if parent != id {
            let root = self.find(&parent);
            self.parent.insert(id.to_string(), root.clone());
            root
        } else {
            id.to_string()
        }
    }

    /// Union two components by rank. Returns (root_a, root_b, new_root) if they were separate.
    pub fn union(&mut self, a: &str, b: &str) -> Option<(String, String, String)> {
        let root_a = self.find(a);
        let root_b = self.find(b);

        if root_a == root_b {
            return None; // Already in same component
        }

        let rank_a = *self.rank.get(&root_a).unwrap_or(&0);
        let rank_b = *self.rank.get(&root_b).unwrap_or(&0);
        let size_a = *self.size.get(&root_a).unwrap_or(&1);
        let size_b = *self.size.get(&root_b).unwrap_or(&1);

        let new_root = if rank_a < rank_b {
            self.parent.insert(root_a.clone(), root_b.clone());
            self.size.insert(root_b.clone(), size_a + size_b);
            root_b.clone()
        } else if rank_a > rank_b {
            self.parent.insert(root_b.clone(), root_a.clone());
            self.size.insert(root_a.clone(), size_a + size_b);
            root_a.clone()
        } else {
            self.parent.insert(root_b.clone(), root_a.clone());
            self.rank.insert(root_a.clone(), rank_a + 1);
            self.size.insert(root_a.clone(), size_a + size_b);
            root_a.clone()
        };

        Some((root_a, root_b, new_root))
    }

    /// Get the size of the component containing `id`.
    pub fn get_size(&mut self, id: &str) -> usize {
        let root = self.find(id);
        *self.size.get(&root).unwrap_or(&1)
    }

    /// Get all components (sets of paper IDs).
    pub fn get_components(&mut self) -> Vec<Vec<String>> {
        let mut components: HashMap<String, Vec<String>> = HashMap::new();

        let ids: Vec<String> = self.parent.keys().cloned().collect();
        for id in ids {
            let root = self.find(&id);
            components.entry(root).or_default().push(id);
        }

        components.into_values().collect()
    }
}

/// Build a dendrogram from sorted edges using union-find.
///
/// # Arguments
/// * `papers` - All paper IDs
/// * `edges` - Edges sorted by weight descending: (source, target, weight)
///
/// # Returns
/// Complete dendrogram with merge events
pub fn build_dendrogram(papers: Vec<String>, edges: Vec<(String, String, f64)>) -> Dendrogram {
    let mut uf = UnionFind::new(&papers);
    let mut merges = Vec::new();
    let paper_to_leaf: HashMap<String, usize> = papers
        .iter()
        .enumerate()
        .map(|(i, p)| (p.clone(), i))
        .collect();

    // Track which papers/merges form each component's "representative"
    // Maps root -> current representative (paper ID or merge ID)
    let mut root_to_repr: HashMap<String, String> = papers
        .iter()
        .map(|p| (p.clone(), p.clone()))
        .collect();

    for (i, (source, target, weight)) in edges.into_iter().enumerate() {
        // Get roots before union
        let root_a = uf.find(&source);
        let root_b = uf.find(&target);

        if root_a == root_b {
            continue; // Already in same component
        }

        let size_a = uf.get_size(&root_a);
        let size_b = uf.get_size(&root_b);

        // Get representatives
        let repr_a = root_to_repr.get(&root_a).cloned().unwrap_or(root_a.clone());
        let repr_b = root_to_repr.get(&root_b).cloned().unwrap_or(root_b.clone());

        // Perform union
        if let Some((_, _, new_root)) = uf.union(&source, &target) {
            let merge_id = format!("merge-{}", i);

            merges.push(MergeEvent {
                id: merge_id.clone(),
                left: repr_a,
                right: repr_b,
                weight,
                size: size_a + size_b,
            });

            // Update representative for new root
            root_to_repr.insert(new_root, merge_id);
        }
    }

    let root = merges.last().map(|m| m.id.clone());

    Dendrogram {
        merges,
        paper_to_leaf,
        root,
        papers,
    }
}

/// Find natural thresholds using gap detection in the weight distribution.
///
/// Analyzes where merge rate changes significantly - these are natural
/// "boundaries" in the similarity space.
pub fn find_natural_thresholds(dendrogram: &Dendrogram, target_levels: usize) -> Vec<f64> {
    if dendrogram.merges.is_empty() {
        return vec![];
    }

    let weights: Vec<f64> = dendrogram.merges.iter().map(|m| m.weight).collect();

    if weights.len() < target_levels {
        // Not enough merges, return all unique weights
        let mut unique: Vec<f64> = weights.clone();
        unique.dedup();
        return unique;
    }

    // Method: Find largest gaps in sorted weight sequence
    // Gaps indicate natural boundaries between conceptual clusters

    let mut gaps: Vec<(usize, f64)> = Vec::new();
    for i in 1..weights.len() {
        let gap = weights[i - 1] - weights[i]; // weights are descending
        gaps.push((i, gap));
    }

    // Sort gaps by size descending
    gaps.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Take top (target_levels - 1) gaps to create target_levels regions
    let mut threshold_indices: Vec<usize> = gaps
        .iter()
        .take(target_levels.saturating_sub(1))
        .map(|(i, _)| *i)
        .collect();

    threshold_indices.sort();

    // Convert indices to threshold values (midpoint of gap)
    let mut thresholds = Vec::new();
    for idx in threshold_indices {
        if idx < weights.len() {
            let threshold = (weights[idx - 1] + weights[idx]) / 2.0;
            thresholds.push(threshold);
        }
    }

    // Sort thresholds descending (tightest first)
    thresholds.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    thresholds
}

/// Find thresholds using percentiles of the weight distribution.
///
/// More robust than gap detection when weights are evenly distributed.
/// Creates levels that each contain roughly equal numbers of merges.
pub fn find_percentile_thresholds(dendrogram: &Dendrogram, target_levels: usize) -> Vec<f64> {
    if dendrogram.merges.is_empty() || target_levels < 2 {
        return vec![];
    }

    let weights: Vec<f64> = dendrogram.merges.iter().map(|m| m.weight).collect();

    // Calculate percentile positions for target_levels regions
    // E.g., for 4 levels: 25th, 50th, 75th percentiles
    let mut thresholds = Vec::new();
    for i in 1..target_levels {
        let percentile = (i as f64) / (target_levels as f64);
        let index = ((weights.len() as f64) * percentile) as usize;
        if index < weights.len() {
            thresholds.push(weights[index]);
        }
    }

    // Sort descending (highest weight = tightest clusters first)
    thresholds.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    thresholds.dedup_by(|a, b| (*a - *b).abs() < 0.001); // Remove near-duplicates

    thresholds
}

/// Find thresholds using fixed weight values.
///
/// Simple and predictable. Good when you know the weight distribution.
pub fn fixed_thresholds(levels: &[f64]) -> Vec<f64> {
    let mut thresholds = levels.to_vec();
    thresholds.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    thresholds
}

/// Find dynamic thresholds based on where component count changes meaningfully.
/// Walks from low threshold (broad) to high threshold (tight), creating a level
/// where the component count increases by 30% or >=5.
///
/// # Arguments
/// * `dendrogram` - The complete dendrogram
/// * `max_levels` - Optional maximum number of levels (None = unlimited)
/// * `step` - Threshold increment (default 0.02 for fine granularity)
/// * `min_threshold` - Lowest threshold to consider (default 0.5)
/// * `max_threshold` - Highest threshold to consider (default 0.95)
///
/// # Returns
/// Thresholds in descending order (tightest first, for consistency with other methods)
pub fn find_dynamic_thresholds(
    dendrogram: &Dendrogram,
    max_levels: Option<usize>,
    step: Option<f64>,
    min_threshold: Option<f64>,
    max_threshold: Option<f64>,
) -> Vec<f64> {
    let step = step.unwrap_or(0.02); // Fine granularity for natural level detection
    let min_t = min_threshold.unwrap_or(0.5);
    let max_t = max_threshold.unwrap_or(0.95);
    let max_levels = max_levels.unwrap_or(20); // Sanity limit

    if dendrogram.merges.is_empty() {
        return vec![];
    }

    // Pre-sort merges by weight descending for efficient counting
    let weights: Vec<f64> = dendrogram.merges.iter().map(|m| m.weight).collect();

    // Helper: count components at a given threshold
    // Components = papers + (merges above threshold) - (papers merged above threshold)
    // Simpler: at threshold T, components = papers that aren't yet merged above T
    let count_components = |threshold: f64| -> usize {
        // Use union-find simulation
        let mut uf = UnionFind::new(&dendrogram.papers);
        for merge in &dendrogram.merges {
            if merge.weight >= threshold {
                // This merge happens at or above threshold
                let left_root = uf.find(&merge.left);
                let right_root = uf.find(&merge.right);
                if left_root != right_root {
                    uf.union(&merge.left, &merge.right);
                }
            }
        }
        // Count unique roots
        let mut roots = HashSet::new();
        for paper in &dendrogram.papers {
            roots.insert(uf.find(paper));
        }
        roots.len()
    };

    // Walk thresholds from min to max, detect meaningful jumps
    let mut thresholds = Vec::new();
    let mut prev_count = count_components(min_t);
    thresholds.push(min_t); // Always include the floor

    let mut current = min_t + step;
    while current <= max_t && thresholds.len() < max_levels {
        let count = count_components(current);

        // Create level if component count meaningfully increases (30% or +5)
        let should_create = (count as f64) >= (prev_count as f64 * 1.3) || count >= prev_count + 5;

        if should_create {
            thresholds.push(current);
            prev_count = count;
        }

        current += step;
    }

    // Return in descending order (tightest/highest first) for consistency
    thresholds.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    thresholds
}

/// Subdivide a large component into smaller sub-components.
/// Uses internal edges (both endpoints in component) to split at median weight.
/// Recursively subdivides until all components are <= max_size.
///
/// # Arguments
/// * `component` - The component to subdivide
/// * `all_edges` - All edges (source, target, weight) sorted by weight descending
/// * `max_size` - Maximum papers per sub-component
///
/// # Returns
/// Vec of sub-components, each <= max_size (or unsplittable)
pub fn subdivide_component(
    component: &Component,
    all_edges: &[(String, String, f64)],
    max_size: usize,
) -> Vec<Component> {
    // If already small enough, return as-is
    if component.papers.len() <= max_size {
        return vec![component.clone()];
    }

    // Filter to internal edges only (both endpoints in this component)
    let paper_set: HashSet<&String> = component.papers.iter().collect();
    let internal_edges: Vec<_> = all_edges.iter()
        .filter(|(s, t, _)| paper_set.contains(s) && paper_set.contains(t))
        .cloned()
        .collect();

    // If no internal edges, can't subdivide
    if internal_edges.is_empty() {
        return vec![component.clone()];
    }

    // Find median weight of internal edges
    let mut weights: Vec<f64> = internal_edges.iter().map(|(_, _, w)| *w).collect();
    weights.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let median_weight = weights[weights.len() / 2];

    // Use union-find to split at median threshold
    let papers: Vec<String> = component.papers.clone();
    let mut uf = UnionFind::new(&papers);

    for (source, target, weight) in &internal_edges {
        if *weight >= median_weight {
            uf.union(source, target);
        }
    }

    // Extract sub-components
    let mut root_to_papers: HashMap<String, Vec<String>> = HashMap::new();
    for paper in &papers {
        let root = uf.find(paper);
        root_to_papers.entry(root).or_default().push(paper.clone());
    }

    // If didn't actually split (all in one component), try higher threshold
    if root_to_papers.len() == 1 {
        // Try 75th percentile instead
        let higher_threshold = weights[weights.len() / 4];
        if (higher_threshold - median_weight).abs() > 0.01 {
            let mut uf2 = UnionFind::new(&papers);
            for (source, target, weight) in &internal_edges {
                if *weight >= higher_threshold {
                    uf2.union(source, target);
                }
            }
            root_to_papers.clear();
            for paper in &papers {
                let root = uf2.find(paper);
                root_to_papers.entry(root).or_default().push(paper.clone());
            }
        }
    }

    // Still couldn't split? Return as-is
    if root_to_papers.len() == 1 {
        return vec![component.clone()];
    }

    // Create sub-components and recursively subdivide if needed
    let mut result = Vec::new();
    for (idx, (_root, papers)) in root_to_papers.into_iter().enumerate() {
        let sub = Component {
            id: format!("{}-sub{}", component.id, idx),
            papers,
            parent: component.parent.clone(),
            children: vec![],
            merge_weight: Some(median_weight),
        };

        // Recurse if still too large
        if sub.papers.len() > max_size {
            result.extend(subdivide_component(&sub, all_edges, max_size));
        } else {
            result.push(sub);
        }
    }

    result
}

/// Extract hierarchy levels by cutting the dendrogram at thresholds.
/// Optimized version: pre-computes paper membership, processes all levels in one pass.
///
/// # Arguments
/// * `dendrogram` - The complete dendrogram
/// * `thresholds` - Thresholds in descending order (tightest first)
///
/// # Returns
/// HierarchyLevels with components at each threshold
pub fn extract_levels(dendrogram: &Dendrogram, thresholds: &[f64]) -> HierarchyLevels {
    if thresholds.is_empty() {
        return HierarchyLevels {
            thresholds: vec![],
            levels: vec![],
        };
    }

    // Pre-compute: for each merge, which paper can represent it
    // This avoids repeated tree traversal
    let mut merge_to_paper: HashMap<String, String> = HashMap::new();
    for paper in &dendrogram.papers {
        merge_to_paper.insert(paper.clone(), paper.clone());
    }
    for merge in &dendrogram.merges {
        // Find a representative paper for this merge
        let left_paper = merge_to_paper.get(&merge.left).cloned();
        if let Some(paper) = left_paper {
            merge_to_paper.insert(merge.id.clone(), paper);
        }
    }

    // Process all levels efficiently
    let mut levels = Vec::with_capacity(thresholds.len());

    for (level_idx, &threshold) in thresholds.iter().enumerate() {
        // Build union-find up to this threshold
        let mut uf = UnionFind::new(&dendrogram.papers);

        // Apply merges at or above threshold
        for merge in &dendrogram.merges {
            if merge.weight >= threshold {
                if let (Some(left_paper), Some(right_paper)) = (
                    merge_to_paper.get(&merge.left),
                    merge_to_paper.get(&merge.right),
                ) {
                    uf.union(left_paper, right_paper);
                }
            }
        }

        // Extract components
        let component_groups = uf.get_components();
        let mut components = Vec::with_capacity(component_groups.len());

        for (comp_idx, papers) in component_groups.into_iter().enumerate() {
            let comp_id = format!("level-{}-comp-{}", level_idx, comp_idx);

            components.push(Component {
                id: comp_id,
                papers,
                parent: None,
                children: Vec::new(),
                merge_weight: Some(threshold), // Use threshold as merge weight
            });
        }

        levels.push(components);
    }

    // Link parent-child relationships between levels
    link_levels(&mut levels);

    HierarchyLevels {
        thresholds: thresholds.to_vec(),
        levels,
    }
}

/// Find a paper ID in a subtree (either a paper itself or dig into a merge).
fn find_paper_in_subtree(id: &str, dendrogram: &Dendrogram) -> Option<String> {
    if dendrogram.paper_to_leaf.contains_key(id) {
        return Some(id.to_string());
    }

    // It's a merge ID, find the merge and recurse
    if let Some(merge) = dendrogram.merges.iter().find(|m| m.id == id) {
        find_paper_in_subtree(&merge.left, dendrogram)
            .or_else(|| find_paper_in_subtree(&merge.right, dendrogram))
    } else {
        None
    }
}

/// Link parent-child relationships between adjacent levels.
fn link_levels(levels: &mut [Vec<Component>]) {
    if levels.len() < 2 {
        return;
    }

    for i in 0..levels.len() - 1 {
        let parent_level_idx = i;
        let child_level_idx = i + 1;

        // Build a map: paper -> parent component ID
        let mut paper_to_parent: HashMap<String, String> = HashMap::new();
        for comp in &levels[parent_level_idx] {
            for paper in &comp.papers {
                paper_to_parent.insert(paper.clone(), comp.id.clone());
            }
        }

        // For each child component, find its parent
        // (the parent component that contains all its papers)
        let child_parents: Vec<(usize, Option<String>)> = levels[child_level_idx]
            .iter()
            .enumerate()
            .map(|(idx, child)| {
                let parent_id = child.papers.first()
                    .and_then(|p| paper_to_parent.get(p).cloned());
                (idx, parent_id)
            })
            .collect();

        // Group children by parent
        let mut parent_children: HashMap<String, Vec<String>> = HashMap::new();
        for (child_idx, parent_id) in &child_parents {
            if let Some(pid) = parent_id {
                parent_children
                    .entry(pid.clone())
                    .or_default()
                    .push(levels[child_level_idx][*child_idx].id.clone());
            }
        }

        // Update parent references in children
        for (child_idx, parent_id) in child_parents {
            if let Some(pid) = parent_id {
                levels[child_level_idx][child_idx].parent = Some(pid);
            }
        }

        // Update children references in parents
        for comp in &mut levels[parent_level_idx] {
            if let Some(children) = parent_children.get(&comp.id) {
                comp.children = children.clone();
            }
        }
    }
}

/// Recursively subdivide components that exceed max_size.
///
/// Re-runs dendrogram logic on internal edges to create sub-hierarchy.
pub fn subdivide_large_component(
    papers: &[String],
    edges: &[(String, String, f64)],
    max_size: usize,
    min_size: usize,
    config: &DendrogramConfig,
) -> Vec<Component> {
    if papers.len() <= max_size {
        // Small enough, return as single component
        return vec![Component {
            id: format!("subdiv-{}", papers.first().unwrap_or(&"unknown".to_string())),
            papers: papers.to_vec(),
            parent: None,
            children: Vec::new(),
            merge_weight: None,
        }];
    }

    // Filter edges to only those within this component
    let paper_set: HashSet<&String> = papers.iter().collect();
    let internal_edges: Vec<(String, String, f64)> = edges
        .iter()
        .filter(|(s, t, _)| paper_set.contains(s) && paper_set.contains(t))
        .cloned()
        .collect();

    if internal_edges.is_empty() {
        // No internal edges, can't subdivide further
        return vec![Component {
            id: format!("subdiv-{}", papers.first().unwrap_or(&"unknown".to_string())),
            papers: papers.to_vec(),
            parent: None,
            children: Vec::new(),
            merge_weight: None,
        }];
    }

    // Build sub-dendrogram
    let sub_dendrogram = build_dendrogram(papers.to_vec(), internal_edges.clone());

    // Find thresholds for subdivision
    let sub_thresholds = find_natural_thresholds(&sub_dendrogram, config.target_levels);

    if sub_thresholds.is_empty() {
        return vec![Component {
            id: format!("subdiv-{}", papers.first().unwrap_or(&"unknown".to_string())),
            papers: papers.to_vec(),
            parent: None,
            children: Vec::new(),
            merge_weight: None,
        }];
    }

    // Extract levels and take the first level below root that has multiple components
    let levels = extract_levels(&sub_dendrogram, &sub_thresholds);

    // Find first level with multiple components of reasonable size
    for level in &levels.levels {
        if level.len() > 1 {
            let mut result = Vec::new();
            for comp in level {
                if comp.papers.len() >= min_size {
                    // Recursively subdivide if still too large
                    let sub_comps = subdivide_large_component(
                        &comp.papers,
                        &internal_edges,
                        max_size,
                        min_size,
                        config,
                    );
                    result.extend(sub_comps);
                } else if comp.papers.len() > 0 {
                    result.push(comp.clone());
                }
            }
            if !result.is_empty() {
                return result;
            }
        }
    }

    // Couldn't subdivide meaningfully
    vec![Component {
        id: format!("subdiv-{}", papers.first().unwrap_or(&"unknown".to_string())),
        papers: papers.to_vec(),
        parent: None,
        children: Vec::new(),
        merge_weight: None,
    }]
}

// ============================================================================
// V2 Adaptive Tree Algorithm Functions
// ============================================================================

/// Adaptive minimum balance ratio based on group size.
///
/// Larger groups can have more imbalanced splits (small outlier clusters).
/// Smaller groups need more balanced splits.
pub fn min_ratio(group_size: usize) -> f64 {
    // Adaptive min ratio - more permissive for larger groups
    // Large datasets often have a dense core with sparse outliers
    if group_size > 500 {
        0.01  // Allow 1% balance for very large groups
    } else if group_size > 200 {
        0.03  // 3% for large groups
    } else if group_size > 50 {
        0.08  // 8% for medium groups
    } else {
        0.15  // 15% for small groups
    }
}

/// Compute split quality: n_children * balance.
///
/// Higher quality = more structure found with better balance.
pub fn split_quality(children: &[Vec<String>]) -> f64 {
    if children.is_empty() {
        return 0.0;
    }

    let sizes: Vec<usize> = children.iter().map(|c| c.len()).collect();
    let n = children.len() as f64;
    let min_size = *sizes.iter().min().unwrap_or(&0) as f64;
    let mean_size = sizes.iter().sum::<usize>() as f64 / n;

    if mean_size == 0.0 {
        return 0.0;
    }

    let balance = min_size / mean_size;
    n * balance
}

/// Compute variance of edge weights using Welford's algorithm.
///
/// Used to detect tight groups (low variance = already cohesive).
pub fn edge_weight_variance(papers: &[String], edge_index: &EdgeIndex) -> f64 {
    let edges = edge_index.internal_edges(papers);
    if edges.len() < 2 {
        return 0.0;
    }

    let mut mean = 0.0;
    let mut m2 = 0.0;

    for (i, (_, _, w)) in edges.iter().enumerate() {
        let delta = w - mean;
        mean += delta / (i + 1) as f64;
        m2 += delta * (w - mean);
    }

    m2 / (edges.len() - 1) as f64
}

/// Check if a split has valid cohesion (all sibling pairs well-separated).
///
/// For each pair (A, B):
///   ratio = (intra(A) + intra(B)) / (2 * inter(A, B))
///   Valid if ratio >= COHESION_THRESHOLD
pub fn valid_cohesion(
    components: &[Vec<String>],
    edge_index: &EdgeIndex,
    config: &AdaptiveTreeConfig,
) -> bool {
    for i in 0..components.len() {
        for j in (i + 1)..components.len() {
            let intra_a = edge_index.intra(&components[i]);
            let intra_b = edge_index.intra(&components[j]);
            let inter = edge_index.inter(&components[i], &components[j]);

            // If no inter-edges, they're definitely separated
            if inter == 0.0 {
                continue;
            }

            let ratio = (intra_a + intra_b) / (2.0 * inter);
            if ratio < config.cohesion_threshold {
                return false;
            }
        }
    }
    true
}

/// Find all valid splits within similarity range.
///
/// Checks: gap from parent, balance ratio, cohesion ratio.
pub fn find_valid_splits(
    papers: &[String],
    range: SimRange,
    parent_threshold: Option<f64>,
    edge_index: &EdgeIndex,
    config: &AdaptiveTreeConfig,
) -> Vec<Split> {
    // Get internal edges within range
    let all_edges = edge_index.internal_edges(papers);
    let edges_in_range: Vec<_> = all_edges
        .iter()
        .filter(|(_, _, w)| *w >= range.min && *w <= range.max)
        .collect();

    if edges_in_range.is_empty() {
        return vec![];
    }

    // Get unique thresholds sorted descending
    let mut thresholds: Vec<f64> = edges_in_range.iter().map(|(_, _, w)| *w).collect();
    thresholds.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    thresholds.dedup_by(|a, b| (*a - *b).abs() < 0.001);

    let mut valid = Vec::new();

    for threshold in thresholds {
        // Gap check: must be far enough from parent threshold
        if let Some(parent_t) = parent_threshold {
            if (threshold - parent_t).abs() < config.delta_min {
                continue;
            }
        }

        // Cut at threshold using union-find
        let mut uf = UnionFind::new(papers);
        for (a, b, w) in &all_edges {
            if *w >= threshold {
                uf.union(a, b);
            }
        }

        // Get components and filter to >= MIN_SIZE
        let components: Vec<Vec<String>> = uf.get_components()
            .into_iter()
            .filter(|c| c.len() >= config.min_size)
            .collect();

        // Need at least 2 components
        if components.len() < 2 {
            continue;
        }

        // Balance check
        let sizes: Vec<usize> = components.iter().map(|c| c.len()).collect();
        let min_size = *sizes.iter().min().unwrap_or(&0);
        let max_size = *sizes.iter().max().unwrap_or(&1);
        let ratio = min_size as f64 / max_size as f64;

        if ratio < min_ratio(papers.len()) {
            continue;
        }

        // Cohesion check
        if !valid_cohesion(&components, edge_index, config) {
            continue;
        }

        // Compute quality
        let quality = split_quality(&components);

        valid.push(Split {
            threshold,
            children: components,
            quality,
        });
    }

    valid
}

/// Find bridge papers near the cut threshold.
///
/// A paper is a bridge if: |max_edge(p) - threshold| < delta
pub fn find_bridges(
    papers: &[String],
    threshold: f64,
    delta: f64,
    edge_index: &EdgeIndex,
) -> Vec<String> {
    let mut bridges = Vec::new();

    for paper in papers {
        if let Some(max_edge) = edge_index.max_edge(paper) {
            if (max_edge - threshold).abs() < delta {
                bridges.push(paper.clone());
            }
        }
    }

    bridges
}

/// Assign bridge papers to children they connect to.
///
/// Returns map: child_idx -> Vec<bridge_paper_ids>
pub fn assign_bridges_to_children(
    bridges: &[String],
    children: &[Vec<String>],
    edge_index: &EdgeIndex,
) -> HashMap<usize, Vec<String>> {
    let mut result: HashMap<usize, Vec<String>> = HashMap::new();

    // Build child sets for quick lookup
    let child_sets: Vec<HashSet<&String>> = children
        .iter()
        .map(|c| c.iter().collect())
        .collect();

    for bridge in bridges {
        // Find which children this bridge connects to
        for (other, _weight) in edge_index.edges_for(bridge) {
            for (idx, child_set) in child_sets.iter().enumerate() {
                if child_set.contains(other) {
                    result.entry(idx).or_default().push(bridge.clone());
                    break; // Only add once per child
                }
            }
        }
    }

    // Deduplicate
    for bridges in result.values_mut() {
        bridges.sort();
        bridges.dedup();
    }

    result
}

/// Find bridges shared between two children (connected to both).
pub fn find_shared_bridges(
    bridges: &[String],
    papers_a: &[String],
    papers_b: &[String],
    edge_index: &EdgeIndex,
) -> Vec<String> {
    let set_a: HashSet<&String> = papers_a.iter().collect();
    let set_b: HashSet<&String> = papers_b.iter().collect();

    let mut shared = Vec::new();

    for bridge in bridges {
        let mut connects_a = false;
        let mut connects_b = false;

        for (other, _) in edge_index.edges_for(bridge) {
            if set_a.contains(other) {
                connects_a = true;
            }
            if set_b.contains(other) {
                connects_b = true;
            }
            if connects_a && connects_b {
                shared.push(bridge.clone());
                break;
            }
        }
    }

    shared
}

/// Create sibling edge metadata between two child subtrees.
pub fn create_sibling_edge_meta(
    child_a: &TreeNode,
    child_b: &TreeNode,
    threshold: f64,
    all_bridges: &[String],
    edge_index: &EdgeIndex,
) -> SiblingEdgeMeta {
    let papers_a = child_a.papers();
    let papers_b = child_b.papers();

    let shared_bridges = find_shared_bridges(all_bridges, papers_a, papers_b, edge_index);
    let weight = edge_index.inter(papers_a, papers_b);

    SiblingEdgeMeta {
        source_id: child_a.id().to_string(),
        target_id: child_b.id().to_string(),
        weight,
        threshold,
        bridges: shared_bridges,
    }
}

/// Build adaptive tree recursively with per-subtree thresholds.
///
/// # Arguments
/// * `papers` - Paper IDs in this subtree
/// * `range` - Similarity range for this subtree
/// * `parent_threshold` - Parent's cut threshold (None for root)
/// * `edge_index` - Pre-indexed edges for efficient lookups
/// * `config` - Algorithm configuration
/// * `depth` - Current recursion depth (for ID generation)
/// * `id_counter` - Mutable counter for unique IDs
///
/// # Returns
/// TreeNode (Leaf or Internal)
pub fn build_tree(
    papers: Vec<String>,
    range: SimRange,
    parent_threshold: Option<f64>,
    edge_index: &EdgeIndex,
    config: &AdaptiveTreeConfig,
    depth: usize,
    id_counter: &mut usize,
) -> TreeNode {
    let node_id = format!("node-{}", *id_counter);
    *id_counter += 1;

    // Stopping condition 1: too small
    if papers.len() < config.min_size {
        return TreeNode::Leaf {
            id: node_id,
            papers,
            range,
        };
    }

    // Stopping condition 2: already tight (low variance)
    let variance = edge_weight_variance(&papers, edge_index);
    if variance < config.tight_threshold {
        return TreeNode::Leaf {
            id: node_id,
            papers,
            range,
        };
    }

    // Stopping condition 3: range too narrow
    if range.width() < config.delta_min {
        return TreeNode::Leaf {
            id: node_id,
            papers,
            range,
        };
    }

    // Find valid splits
    let splits = find_valid_splits(&papers, range, parent_threshold, edge_index, config);

    // Stopping condition 4: no valid splits
    if splits.is_empty() {
        return TreeNode::Leaf {
            id: node_id,
            papers,
            range,
        };
    }

    // Select best split by quality
    let mut best = splits
        .into_iter()
        .max_by(|a, b| a.quality.partial_cmp(&b.quality).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap();

    // Nearest-neighbor assignment for orphan papers
    // Papers in small components (< MIN_SIZE) didn't make it into best.children
    // Assign them to the child with highest average edge weight
    let child_paper_set: HashSet<&String> = best.children.iter().flatten().collect();
    let orphans: Vec<&String> = papers.iter().filter(|p| !child_paper_set.contains(p)).collect();

    if !orphans.is_empty() {
        for orphan in &orphans {
            // Compute average edge weight to each child
            let mut best_child_idx: Option<usize> = None;
            let mut best_avg: f64 = 0.0;

            for (idx, child) in best.children.iter().enumerate() {
                let mut sum = 0.0;
                let mut count = 0;
                for child_paper in child {
                    if let Some(w) = edge_index.weight(orphan, child_paper) {
                        sum += w;
                        count += 1;
                    }
                }
                if count > 0 {
                    let avg = sum / child.len() as f64;
                    if avg > best_avg {
                        best_avg = avg;
                        best_child_idx = Some(idx);
                    }
                }
            }

            // Assign orphan to best child if any connection exists
            if let Some(idx) = best_child_idx {
                best.children[idx].push((*orphan).clone());
            }
            // Otherwise orphan stays in parent's papers field (handled by flatten_tree)
        }
    }

    // Find bridges
    let bridges = find_bridges(&papers, best.threshold, config.delta_min, edge_index);

    // Assign bridges to children
    let bridge_assignments = assign_bridges_to_children(&bridges, &best.children, edge_index);

    // Recurse into children
    let mut children = Vec::new();
    for (idx, mut child_papers) in best.children.into_iter().enumerate() {
        // Add bridges assigned to this child
        if let Some(child_bridges) = bridge_assignments.get(&idx) {
            for bridge in child_bridges {
                if !child_papers.contains(bridge) {
                    child_papers.push(bridge.clone());
                }
            }
        }

        // Compute child range from internal edges
        let child_edges = edge_index.internal_edges(&child_papers);
        let child_range = if child_edges.is_empty() {
            SimRange::new(range.min, range.max)
        } else {
            let min_w = child_edges
                .iter()
                .map(|(_, _, w)| *w)
                .fold(f64::INFINITY, f64::min);
            let max_w = child_edges
                .iter()
                .map(|(_, _, w)| *w)
                .fold(f64::NEG_INFINITY, f64::max);
            SimRange::new(min_w, max_w)
        };

        let child_node = build_tree(
            child_papers,
            child_range,
            Some(best.threshold),
            edge_index,
            config,
            depth + 1,
            id_counter,
        );
        children.push(child_node);
    }

    TreeNode::Internal {
        id: node_id,
        papers,
        children,
        threshold: best.threshold,
        range,
        bridges,
    }
}

/// Collect all sibling edge metadata from a tree (post-order traversal).
fn collect_sibling_edges(
    node: &TreeNode,
    edge_index: &EdgeIndex,
    result: &mut Vec<SiblingEdgeMeta>,
) {
    if let TreeNode::Internal {
        children,
        threshold,
        bridges,
        ..
    } = node
    {
        // Create sibling edges for all pairs of children
        for i in 0..children.len() {
            for j in (i + 1)..children.len() {
                let meta = create_sibling_edge_meta(
                    &children[i],
                    &children[j],
                    *threshold,
                    bridges,
                    edge_index,
                );
                result.push(meta);
            }
        }

        // Recurse into children
        for child in children {
            collect_sibling_edges(child, edge_index, result);
        }
    }
}

/// Main entry point for v2 adaptive tree building.
///
/// # Arguments
/// * `papers` - All paper IDs
/// * `edges` - All edges (source, target, weight)
/// * `config` - Optional configuration (uses defaults if None)
///
/// # Returns
/// (root TreeNode, Vec<SiblingEdgeMeta>)
pub fn build_adaptive_tree(
    papers: Vec<String>,
    edges: Vec<(String, String, f64)>,
    config: Option<AdaptiveTreeConfig>,
) -> (TreeNode, Vec<SiblingEdgeMeta>) {
    let config = config.unwrap_or_default();

    // Build edge index for efficient lookups
    let edge_index = EdgeIndex::new(&edges);

    // Compute initial range from all edges
    let range = if edges.is_empty() {
        SimRange::new(constants::EDGE_FLOOR, 1.0)
    } else {
        let min_w = edges
            .iter()
            .map(|(_, _, w)| *w)
            .fold(f64::INFINITY, f64::min);
        let max_w = edges
            .iter()
            .map(|(_, _, w)| *w)
            .fold(f64::NEG_INFINITY, f64::max);
        SimRange::new(min_w, max_w)
    };

    // Build tree recursively
    let mut id_counter = 0;
    let root = build_tree(
        papers,
        range,
        None,
        &edge_index,
        &config,
        0,
        &mut id_counter,
    );

    // Collect sibling edges
    let mut sibling_edges = Vec::new();
    collect_sibling_edges(&root, &edge_index, &mut sibling_edges);

    (root, sibling_edges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_union_find_basic() {
        let papers = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut uf = UnionFind::new(&papers);

        assert_eq!(uf.find("a"), "a");
        assert_eq!(uf.find("b"), "b");

        uf.union("a", "b");
        assert_eq!(uf.find("a"), uf.find("b"));
        assert_eq!(uf.get_size("a"), 2);

        uf.union("b", "c");
        assert_eq!(uf.find("a"), uf.find("c"));
        assert_eq!(uf.get_size("a"), 3);
    }

    #[test]
    fn test_build_dendrogram_simple() {
        let papers = vec!["p1".to_string(), "p2".to_string(), "p3".to_string()];
        let edges = vec![
            ("p1".to_string(), "p2".to_string(), 0.9),
            ("p2".to_string(), "p3".to_string(), 0.7),
            ("p1".to_string(), "p3".to_string(), 0.6),
        ];

        let dendrogram = build_dendrogram(papers, edges);

        assert_eq!(dendrogram.merges.len(), 2); // Two merges to connect 3 papers
        assert_eq!(dendrogram.merges[0].weight, 0.9); // Highest weight first
        assert_eq!(dendrogram.merges[1].weight, 0.7); // Then next
    }

    #[test]
    fn test_find_natural_thresholds() {
        let papers = vec!["p1".to_string(), "p2".to_string(), "p3".to_string(), "p4".to_string()];
        let edges = vec![
            ("p1".to_string(), "p2".to_string(), 0.9),
            ("p3".to_string(), "p4".to_string(), 0.85),
            ("p1".to_string(), "p3".to_string(), 0.5), // Big gap here
        ];

        let dendrogram = build_dendrogram(papers, edges);
        let thresholds = find_natural_thresholds(&dendrogram, 2);

        // Should find the gap between 0.85 and 0.5
        assert!(!thresholds.is_empty());
        if let Some(&t) = thresholds.first() {
            assert!(t > 0.5 && t < 0.85);
        }
    }

    #[test]
    fn test_extract_levels() {
        let papers = vec!["p1".to_string(), "p2".to_string(), "p3".to_string(), "p4".to_string()];
        let edges = vec![
            ("p1".to_string(), "p2".to_string(), 0.9),
            ("p3".to_string(), "p4".to_string(), 0.85),
            ("p1".to_string(), "p3".to_string(), 0.5),
        ];

        let dendrogram = build_dendrogram(papers, edges);
        let thresholds = vec![0.8, 0.4]; // Two levels

        let levels = extract_levels(&dendrogram, &thresholds);

        assert_eq!(levels.levels.len(), 2);
        // At threshold 0.8: two components (p1-p2) and (p3-p4)
        assert_eq!(levels.levels[0].len(), 2);
        // At threshold 0.4: one component (all papers)
        assert_eq!(levels.levels[1].len(), 1);
    }

    // ========================================================================
    // V2 Adaptive Tree Algorithm Tests
    // ========================================================================

    #[test]
    fn test_edge_index_basic() {
        let edges = vec![
            ("a".to_string(), "b".to_string(), 0.9),
            ("b".to_string(), "c".to_string(), 0.7),
            ("a".to_string(), "c".to_string(), 0.5),
        ];

        let index = EdgeIndex::new(&edges);

        assert_eq!(index.weight("a", "b"), Some(0.9));
        assert_eq!(index.weight("b", "a"), Some(0.9)); // Symmetric
        assert_eq!(index.weight("a", "c"), Some(0.5));
        assert_eq!(index.weight("a", "d"), None); // No edge
        assert_eq!(index.max_edge("a"), Some(0.9));
        assert_eq!(index.max_edge("c"), Some(0.7));
    }

    #[test]
    fn test_edge_index_intra_inter() {
        let edges = vec![
            // Cluster A: a1, a2, a3 with high internal similarity
            ("a1".to_string(), "a2".to_string(), 0.9),
            ("a2".to_string(), "a3".to_string(), 0.85),
            ("a1".to_string(), "a3".to_string(), 0.8),
            // Cluster B: b1, b2 with high internal similarity
            ("b1".to_string(), "b2".to_string(), 0.88),
            // Cross-cluster edges (lower similarity)
            ("a1".to_string(), "b1".to_string(), 0.4),
            ("a2".to_string(), "b2".to_string(), 0.35),
        ];

        let index = EdgeIndex::new(&edges);

        let cluster_a = vec!["a1".to_string(), "a2".to_string(), "a3".to_string()];
        let cluster_b = vec!["b1".to_string(), "b2".to_string()];

        let intra_a = index.intra(&cluster_a);
        let intra_b = index.intra(&cluster_b);
        let inter = index.inter(&cluster_a, &cluster_b);

        // Intra should be high (around 0.85)
        assert!(intra_a > 0.8, "intra_a should be > 0.8, got {}", intra_a);
        assert!(intra_b > 0.8, "intra_b should be > 0.8, got {}", intra_b);
        // Inter should be low (around 0.375)
        assert!(inter < 0.5, "inter should be < 0.5, got {}", inter);
    }

    #[test]
    fn test_min_ratio_scaling() {
        assert_eq!(min_ratio(600), 0.05);
        assert_eq!(min_ratio(300), 0.08);
        assert_eq!(min_ratio(100), 0.12);
        assert_eq!(min_ratio(20), 0.25);
    }

    #[test]
    fn test_split_quality() {
        // Balanced split: 2 children of 50 each
        let balanced = vec![
            (0..50).map(|i| format!("p{}", i)).collect(),
            (50..100).map(|i| format!("p{}", i)).collect(),
        ];
        let q_balanced = split_quality(&balanced);
        assert!(q_balanced > 1.9, "balanced quality should be ~2.0, got {}", q_balanced);

        // Imbalanced split: 90 vs 10
        let imbalanced = vec![
            (0..90).map(|i| format!("p{}", i)).collect(),
            (90..100).map(|i| format!("p{}", i)).collect(),
        ];
        let q_imbalanced = split_quality(&imbalanced);
        assert!(q_imbalanced < q_balanced, "imbalanced should have lower quality");
    }

    #[test]
    fn test_valid_cohesion() {
        // Two well-separated clusters
        let edges = vec![
            ("a1".to_string(), "a2".to_string(), 0.9),
            ("b1".to_string(), "b2".to_string(), 0.9),
            ("a1".to_string(), "b1".to_string(), 0.3), // Weak cross-edge
        ];
        let index = EdgeIndex::new(&edges);
        let config = AdaptiveTreeConfig::default();

        let components = vec![
            vec!["a1".to_string(), "a2".to_string()],
            vec!["b1".to_string(), "b2".to_string()],
        ];

        assert!(valid_cohesion(&components, &index, &config));
    }

    #[test]
    fn test_find_bridges() {
        let edges = vec![
            ("a".to_string(), "b".to_string(), 0.72),
            ("b".to_string(), "c".to_string(), 0.68),
            ("c".to_string(), "d".to_string(), 0.65),
        ];
        let index = EdgeIndex::new(&edges);

        let papers = vec!["a".to_string(), "b".to_string(), "c".to_string(), "d".to_string()];
        let threshold = 0.70;
        let delta = 0.03;

        let bridges = find_bridges(&papers, threshold, delta, &index);

        // b has max_edge 0.72, which is within 0.03 of 0.70
        // c has max_edge 0.68, which is within 0.03 of 0.70
        assert!(bridges.contains(&"b".to_string()) || bridges.contains(&"c".to_string()));
    }

    #[test]
    fn test_build_adaptive_tree_small_becomes_leaf() {
        // Small group should become leaf immediately
        let papers = vec!["p1".to_string(), "p2".to_string()];
        let edges = vec![("p1".to_string(), "p2".to_string(), 0.9)];

        let config = AdaptiveTreeConfig {
            min_size: 5, // Requires 5 papers minimum
            ..Default::default()
        };

        let (root, _) = build_adaptive_tree(papers, edges, Some(config));

        assert!(root.is_leaf());
    }

    #[test]
    fn test_build_adaptive_tree_two_clusters() {
        // Two clear clusters that should split
        let mut papers = Vec::new();
        let mut edges = Vec::new();

        // Cluster A: papers 0-9 with high internal similarity
        for i in 0..10 {
            papers.push(format!("a{}", i));
            for j in (i + 1)..10 {
                edges.push((format!("a{}", i), format!("a{}", j), 0.85 + (i as f64 * 0.001)));
            }
        }

        // Cluster B: papers 0-9 with high internal similarity
        for i in 0..10 {
            papers.push(format!("b{}", i));
            for j in (i + 1)..10 {
                edges.push((format!("b{}", i), format!("b{}", j), 0.82 + (i as f64 * 0.001)));
            }
        }

        // Weak cross-cluster edges
        for i in 0..3 {
            edges.push((format!("a{}", i), format!("b{}", i), 0.4));
        }

        let (root, sibling_edges) = build_adaptive_tree(papers, edges, None);

        // Two distinct clusters should split
        assert!(!root.is_leaf(), "Two distinct clusters should split, got leaf");

        // Should have at least 2 children
        if let TreeNode::Internal { children, .. } = &root {
            assert!(children.len() >= 2, "Should have at least 2 children, got {}", children.len());
        }

        // Split should produce sibling edges
        assert!(!sibling_edges.is_empty(), "Split should produce sibling edges");
    }
}
