//! Dendrogram-based hierarchy building from edge weights.
//!
//! The core insight: edges between papers already encode similarity as weights (0.5-1.0).
//! Hierarchy emerges naturally by finding connected components at different weight thresholds.
//! No separate clustering needed. The dendrogram IS the hierarchy.

use std::collections::{HashMap, HashSet};

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
}
