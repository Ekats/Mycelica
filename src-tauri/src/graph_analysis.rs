//! Pure computation engine for graph structural analysis.
//!
//! Provides topology metrics, staleness detection, bridge/articulation-point analysis,
//! and a composite health score. All functions are pure: they take a `GraphSnapshot`
//! and return report structs. No I/O, no println.
//!
//! ## Usage
//!
//! ```ignore
//! let snapshot = GraphSnapshot::from_db(&db)?;
//! let report = analyze(&snapshot, &AnalyzerConfig::default());
//! ```

use std::collections::HashMap;
use serde::Serialize;

use crate::db::{Database, Node, Edge};
use crate::dendrogram::UnionFind;

// ---------------------------------------------------------------------------
// Data structures: lightweight, DB-decoupled snapshot
// ---------------------------------------------------------------------------

/// Lightweight node info decoupled from DB types.
#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub id: String,
    pub title: String,
    pub node_type: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub parent_id: Option<String>,
    pub depth: i32,
    pub is_item: bool,
}

/// Lightweight edge info decoupled from DB types.
#[derive(Debug, Clone)]
pub struct EdgeInfo {
    pub id: String,
    pub source: String,
    pub target: String,
    pub edge_type: String,
    pub created_at: i64,
    pub updated_at: Option<i64>,
}

/// Immutable snapshot of the graph with precomputed adjacency and region maps.
pub struct GraphSnapshot {
    pub nodes: HashMap<String, NodeInfo>,
    pub edges: Vec<EdgeInfo>,
    /// Undirected adjacency: node_id -> list of neighbor node_ids
    pub adj: HashMap<String, Vec<String>>,
    /// Directed outgoing: source -> list of targets
    pub out_adj: HashMap<String, Vec<String>>,
    /// Directed incoming: target -> list of sources
    pub in_adj: HashMap<String, Vec<String>>,
    /// Region map: node_id -> depth-1 ancestor id
    pub regions: HashMap<String, String>,
}

impl GraphSnapshot {
    /// Build a snapshot from raw node/edge data. Computes adjacency lists and regions.
    pub fn new(nodes: Vec<NodeInfo>, edges: Vec<EdgeInfo>) -> Self {
        let node_map: HashMap<String, NodeInfo> = nodes
            .into_iter()
            .map(|n| (n.id.clone(), n))
            .collect();

        let mut adj: HashMap<String, Vec<String>> = HashMap::new();
        let mut out_adj: HashMap<String, Vec<String>> = HashMap::new();
        let mut in_adj: HashMap<String, Vec<String>> = HashMap::new();

        // Ensure every node has an entry in adj even if it has no edges
        for id in node_map.keys() {
            adj.entry(id.clone()).or_default();
            out_adj.entry(id.clone()).or_default();
            in_adj.entry(id.clone()).or_default();
        }

        for e in &edges {
            // Only add adjacency for nodes that exist in the snapshot
            if node_map.contains_key(&e.source) && node_map.contains_key(&e.target) {
                adj.entry(e.source.clone()).or_default().push(e.target.clone());
                adj.entry(e.target.clone()).or_default().push(e.source.clone());
                out_adj.entry(e.source.clone()).or_default().push(e.target.clone());
                in_adj.entry(e.target.clone()).or_default().push(e.source.clone());
            }
        }

        // Compute regions: each node's depth-1 ancestor
        let regions = compute_regions(&node_map);

        GraphSnapshot {
            nodes: node_map,
            edges,
            adj,
            out_adj,
            in_adj,
            regions,
        }
    }

    /// Load a snapshot directly from the database.
    pub fn from_db(db: &Database) -> Result<Self, String> {
        let db_nodes = db.get_all_nodes(false).map_err(|e| e.to_string())?;
        let db_edges = db.get_all_edges().map_err(|e| e.to_string())?;

        let nodes: Vec<NodeInfo> = db_nodes
            .into_iter()
            .map(|n: Node| NodeInfo {
                id: n.id,
                title: n.title,
                node_type: n.node_type.as_str().to_string(),
                created_at: n.created_at,
                updated_at: n.updated_at,
                parent_id: n.parent_id,
                depth: n.depth,
                is_item: n.is_item,
            })
            .collect();

        let edges: Vec<EdgeInfo> = db_edges
            .into_iter()
            .map(|e: Edge| EdgeInfo {
                id: e.id,
                source: e.source,
                target: e.target,
                edge_type: e.edge_type.as_str().to_string(),
                created_at: e.created_at,
                updated_at: e.updated_at,
            })
            .collect();

        Ok(Self::new(nodes, edges))
    }

    /// Return a new snapshot filtered to nodes that are descendants of `region_node_id`
    /// (or the region node itself). Only edges where both endpoints are in the filtered
    /// set are included.
    pub fn filter_to_region(&self, region_node_id: &str) -> GraphSnapshot {
        // Collect all descendant node IDs by scanning parent chains
        let mut included: HashMap<String, bool> = HashMap::new();

        for (id, node) in &self.nodes {
            if is_descendant_of(id, region_node_id, &self.nodes, &mut included) {
                // marked in cache
            }
            let _ = node; // suppress unused
        }

        let filtered_ids: std::collections::HashSet<String> = included
            .into_iter()
            .filter(|(_, is_desc)| *is_desc)
            .map(|(id, _)| id)
            .collect();

        let nodes: Vec<NodeInfo> = self.nodes.values()
            .filter(|n| filtered_ids.contains(&n.id))
            .cloned()
            .collect();

        let edges: Vec<EdgeInfo> = self.edges.iter()
            .filter(|e| filtered_ids.contains(&e.source) && filtered_ids.contains(&e.target))
            .cloned()
            .collect();

        GraphSnapshot::new(nodes, edges)
    }
}

/// Recursively check if `node_id` is a descendant of `ancestor_id` (or is the ancestor itself).
fn is_descendant_of(
    node_id: &str,
    ancestor_id: &str,
    nodes: &HashMap<String, NodeInfo>,
    cache: &mut HashMap<String, bool>,
) -> bool {
    if node_id == ancestor_id {
        cache.insert(node_id.to_string(), true);
        return true;
    }
    if let Some(&cached) = cache.get(node_id) {
        return cached;
    }
    let result = if let Some(node) = nodes.get(node_id) {
        if let Some(ref parent_id) = node.parent_id {
            is_descendant_of(parent_id, ancestor_id, nodes, cache)
        } else {
            false
        }
    } else {
        false
    };
    cache.insert(node_id.to_string(), result);
    result
}

/// Compute the depth-1 ancestor for each node. Nodes at depth 0 or 1 are their own region.
/// Nodes with no parent get region "unassigned".
fn compute_regions(nodes: &HashMap<String, NodeInfo>) -> HashMap<String, String> {
    let mut regions: HashMap<String, String> = HashMap::new();

    for (id, node) in nodes {
        if node.depth <= 1 {
            regions.insert(id.clone(), id.clone());
        } else {
            // Walk parent chain to find depth-1 ancestor
            let region = find_depth1_ancestor(id, nodes);
            regions.insert(id.clone(), region);
        }
    }

    regions
}

/// Walk parent_id chain to find the depth-1 ancestor. Returns "unassigned" if chain breaks.
fn find_depth1_ancestor(node_id: &str, nodes: &HashMap<String, NodeInfo>) -> String {
    let mut current = node_id.to_string();
    let mut visited = std::collections::HashSet::new();

    loop {
        if !visited.insert(current.clone()) {
            // Cycle detected
            return "unassigned".to_string();
        }
        match nodes.get(&current) {
            Some(n) if n.depth <= 1 => return current,
            Some(n) => {
                match &n.parent_id {
                    Some(pid) => current = pid.clone(),
                    None => return "unassigned".to_string(),
                }
            }
            None => return "unassigned".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Topology analysis
// ---------------------------------------------------------------------------

/// A node with high connectivity.
#[derive(Debug, Clone, Serialize)]
pub struct HubNode {
    pub id: String,
    pub title: String,
    pub degree: usize,
    pub in_degree: usize,
    pub out_degree: usize,
}

/// Full topology report: components, orphans, degree distribution, hubs.
#[derive(Debug, Clone, Serialize)]
pub struct TopologyReport {
    pub total_nodes: usize,
    pub total_edges: usize,
    pub num_components: usize,
    pub largest_component: usize,
    pub smallest_component: usize,
    pub orphan_count: usize,
    pub orphan_ids: Vec<String>,
    /// Degree histogram: bucket label -> count. Buckets: "0","1","2-3","4-7","8-15","16-31","32+"
    pub degree_histogram: Vec<(String, usize)>,
    pub hubs: Vec<HubNode>,
}

/// Compute topology metrics for the graph.
///
/// - `hub_threshold`: minimum degree to be considered a hub
/// - `top_n`: max number of orphans/hubs to return
pub fn compute_topology(snapshot: &GraphSnapshot, hub_threshold: usize, top_n: usize) -> TopologyReport {
    let total_nodes = snapshot.nodes.len();
    let total_edges = snapshot.edges.len();

    if total_nodes == 0 {
        return TopologyReport {
            total_nodes: 0,
            total_edges: 0,
            num_components: 0,
            largest_component: 0,
            smallest_component: 0,
            orphan_count: 0,
            orphan_ids: vec![],
            degree_histogram: default_histogram(),
            hubs: vec![],
        };
    }

    // Connected components via UnionFind
    let node_ids: Vec<String> = snapshot.nodes.keys().cloned().collect();
    let mut uf = UnionFind::new(&node_ids);

    for edge in &snapshot.edges {
        if snapshot.nodes.contains_key(&edge.source) && snapshot.nodes.contains_key(&edge.target) {
            uf.union(&edge.source, &edge.target);
        }
    }

    let components = uf.get_components();
    let num_components = components.len();
    let largest_component = components.iter().map(|c| c.len()).max().unwrap_or(0);
    let smallest_component = components.iter().map(|c| c.len()).min().unwrap_or(0);

    // Orphans: nodes with no edges
    let mut orphans: Vec<String> = Vec::new();
    for id in snapshot.nodes.keys() {
        let degree = snapshot.adj.get(id).map(|v| v.len()).unwrap_or(0);
        if degree == 0 {
            orphans.push(id.clone());
        }
    }
    orphans.sort(); // deterministic order
    let orphan_count = orphans.len();
    orphans.truncate(top_n);

    // Degree histogram with log-scale buckets
    let mut buckets: [usize; 7] = [0; 7]; // 0, 1, 2-3, 4-7, 8-15, 16-31, 32+
    for id in snapshot.nodes.keys() {
        let degree = snapshot.adj.get(id).map(|v| v.len()).unwrap_or(0);
        let bucket = degree_bucket(degree);
        buckets[bucket] += 1;
    }

    let bucket_labels = ["0", "1", "2-3", "4-7", "8-15", "16-31", "32+"];
    let degree_histogram: Vec<(String, usize)> = bucket_labels
        .iter()
        .zip(buckets.iter())
        .map(|(label, count)| (label.to_string(), *count))
        .collect();

    // Hubs: degree > threshold, sorted desc
    let mut hub_candidates: Vec<HubNode> = Vec::new();
    for (id, node) in &snapshot.nodes {
        let degree = snapshot.adj.get(id).map(|v| v.len()).unwrap_or(0);
        if degree > hub_threshold {
            let in_degree = snapshot.in_adj.get(id).map(|v| v.len()).unwrap_or(0);
            let out_degree = snapshot.out_adj.get(id).map(|v| v.len()).unwrap_or(0);
            hub_candidates.push(HubNode {
                id: id.clone(),
                title: node.title.clone(),
                degree,
                in_degree,
                out_degree,
            });
        }
    }
    hub_candidates.sort_by(|a, b| b.degree.cmp(&a.degree));
    hub_candidates.truncate(top_n);

    TopologyReport {
        total_nodes,
        total_edges,
        num_components,
        largest_component,
        smallest_component,
        orphan_count,
        orphan_ids: orphans,
        degree_histogram,
        hubs: hub_candidates,
    }
}

fn default_histogram() -> Vec<(String, usize)> {
    ["0", "1", "2-3", "4-7", "8-15", "16-31", "32+"]
        .iter()
        .map(|l| (l.to_string(), 0))
        .collect()
}

fn degree_bucket(degree: usize) -> usize {
    match degree {
        0 => 0,
        1 => 1,
        2..=3 => 2,
        4..=7 => 3,
        8..=15 => 4,
        16..=31 => 5,
        _ => 6,
    }
}

// ---------------------------------------------------------------------------
// Staleness analysis
// ---------------------------------------------------------------------------

/// A node that is old but still being referenced by recent edges.
#[derive(Debug, Clone, Serialize)]
pub struct StaleNode {
    pub id: String,
    pub title: String,
    pub days_since_update: i64,
    pub recent_reference_count: usize,
}

/// A summary node whose target has been updated more recently than the summary itself.
#[derive(Debug, Clone, Serialize)]
pub struct StaleSummary {
    pub summary_node_id: String,
    pub summary_title: String,
    pub target_node_id: String,
    pub target_title: String,
    pub drift_days: i64,
}

/// Staleness report: old-but-referenced nodes and outdated summaries.
#[derive(Debug, Clone, Serialize)]
pub struct StalenessReport {
    pub stale_nodes: Vec<StaleNode>,
    pub stale_summaries: Vec<StaleSummary>,
    pub stale_node_count: usize,
    pub stale_summary_count: usize,
}

/// Compute staleness metrics.
///
/// - `stale_days`: how many days since update for a node to be considered stale
pub fn compute_staleness(snapshot: &GraphSnapshot, stale_days: i64) -> StalenessReport {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let stale_threshold_ms = stale_days * 86_400_000;
    let recent_window_ms: i64 = 7 * 86_400_000; // 7 days

    // Build edge lookup: target_id -> list of edges pointing at it
    let mut edges_by_target: HashMap<&str, Vec<&EdgeInfo>> = HashMap::new();
    for e in &snapshot.edges {
        edges_by_target.entry(e.target.as_str()).or_default().push(e);
        // Also consider source for undirected references
        edges_by_target.entry(e.source.as_str()).or_default().push(e);
    }

    // Stale nodes: old update + recent incoming references
    let mut stale_nodes: Vec<StaleNode> = Vec::new();
    for (id, node) in &snapshot.nodes {
        let age_ms = now - node.updated_at;
        if age_ms <= stale_threshold_ms {
            continue; // Not old enough
        }

        // Count recent references (edges created in last 7 days that touch this node)
        let mut recent_count = 0usize;
        if let Some(incoming) = snapshot.in_adj.get(id) {
            for source_id in incoming {
                // Find the edge(s) from source_id to id
                for e in &snapshot.edges {
                    if e.source == *source_id && e.target == *id {
                        if (now - e.created_at) < recent_window_ms {
                            recent_count += 1;
                        }
                    }
                }
            }
        }

        if recent_count > 0 {
            stale_nodes.push(StaleNode {
                id: id.clone(),
                title: node.title.clone(),
                days_since_update: age_ms / 86_400_000,
                recent_reference_count: recent_count,
            });
        }
    }

    stale_nodes.sort_by(|a, b| b.recent_reference_count.cmp(&a.recent_reference_count));

    // Stale summaries: "summarizes" edges where target was updated after source
    let mut stale_summaries: Vec<StaleSummary> = Vec::new();
    for e in &snapshot.edges {
        if e.edge_type != "summarizes" {
            continue;
        }
        let source_node = match snapshot.nodes.get(&e.source) {
            Some(n) => n,
            None => continue,
        };
        let target_node = match snapshot.nodes.get(&e.target) {
            Some(n) => n,
            None => continue,
        };

        if target_node.updated_at > source_node.updated_at {
            let drift_ms = target_node.updated_at - source_node.updated_at;
            stale_summaries.push(StaleSummary {
                summary_node_id: e.source.clone(),
                summary_title: source_node.title.clone(),
                target_node_id: e.target.clone(),
                target_title: target_node.title.clone(),
                drift_days: drift_ms / 86_400_000,
            });
        }
    }

    stale_summaries.sort_by(|a, b| b.drift_days.cmp(&a.drift_days));

    let stale_node_count = stale_nodes.len();
    let stale_summary_count = stale_summaries.len();

    StalenessReport {
        stale_nodes,
        stale_summaries,
        stale_node_count,
        stale_summary_count,
    }
}

// ---------------------------------------------------------------------------
// Bridge / articulation point analysis (iterative Tarjan's)
// ---------------------------------------------------------------------------

/// A node whose removal would disconnect the graph.
#[derive(Debug, Clone, Serialize)]
pub struct ArticulationPoint {
    pub id: String,
    pub title: String,
    /// Estimate of components if removed (degree for APs).
    pub components_if_removed: usize,
}

/// An edge whose removal would disconnect the graph.
#[derive(Debug, Clone, Serialize)]
pub struct BridgeEdge {
    pub source_id: String,
    pub target_id: String,
    pub source_title: String,
    pub target_title: String,
}

/// Two regions connected by very few edges.
#[derive(Debug, Clone, Serialize)]
pub struct FragileConnection {
    pub region_a: String,
    pub region_b: String,
    pub cross_edges: usize,
}

/// Bridge analysis report.
#[derive(Debug, Clone, Serialize)]
pub struct BridgeReport {
    pub articulation_points: Vec<ArticulationPoint>,
    pub bridge_edges: Vec<BridgeEdge>,
    pub fragile_connections: Vec<FragileConnection>,
    pub ap_count: usize,
    pub bridge_count: usize,
}

/// Compute bridges, articulation points, and fragile inter-region connections.
///
/// Uses iterative Tarjan's algorithm to avoid stack overflow on large graphs.
pub fn compute_bridges(snapshot: &GraphSnapshot) -> BridgeReport {
    if snapshot.nodes.is_empty() {
        return BridgeReport {
            articulation_points: vec![],
            bridge_edges: vec![],
            fragile_connections: vec![],
            ap_count: 0,
            bridge_count: 0,
        };
    }

    let node_ids: Vec<&String> = snapshot.nodes.keys().collect();

    // Map node IDs to indices for array-based Tarjan
    let id_to_idx: HashMap<&str, usize> = node_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();

    let n = node_ids.len();

    // Build deduplicated undirected adjacency (as indices)
    let mut adj_idx: Vec<Vec<usize>> = vec![vec![]; n];
    let mut seen_edges: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();

    for e in &snapshot.edges {
        if let (Some(&u), Some(&v)) = (id_to_idx.get(e.source.as_str()), id_to_idx.get(e.target.as_str())) {
            if u != v {
                let key = if u < v { (u, v) } else { (v, u) };
                if seen_edges.insert(key) {
                    adj_idx[u].push(v);
                    adj_idx[v].push(u);
                }
            }
        }
    }

    let mut disc = vec![0u32; n];
    let mut low = vec![0u32; n];
    let mut visited = vec![false; n];
    let mut counter: u32 = 1;

    let mut is_ap = vec![false; n];
    let mut bridge_pairs: Vec<(usize, usize)> = Vec::new();

    // Iterative Tarjan for each connected component
    // Stack frame: (node_idx, parent_idx, neighbor_list_position)
    // We use usize::MAX as "no parent" sentinel.
    for start in 0..n {
        if visited[start] {
            continue;
        }

        // Process this component
        let mut stack: Vec<(usize, usize, usize)> = Vec::new();
        visited[start] = true;
        disc[start] = counter;
        low[start] = counter;
        counter += 1;

        stack.push((start, usize::MAX, 0));

        // Track root children count for AP detection
        let mut root_children: usize = 0;

        while let Some((node, parent, ni)) = stack.last_mut() {
            let node = *node;
            let parent = *parent;

            if *ni < adj_idx[node].len() {
                let child = adj_idx[node][*ni];
                *ni += 1;

                if child == parent {
                    // Skip the edge back to parent
                    continue;
                }

                if visited[child] {
                    // Back edge: update low
                    low[node] = low[node].min(disc[child]);
                } else {
                    // Tree edge: visit child
                    visited[child] = true;
                    disc[child] = counter;
                    low[child] = counter;
                    counter += 1;

                    if node == start {
                        root_children += 1;
                    }

                    stack.push((child, node, 0));
                }
            } else {
                // Done processing all neighbors of `node`
                stack.pop();

                // Propagate low value to parent
                if let Some((parent_node, _, _)) = stack.last() {
                    let pn = *parent_node;
                    low[pn] = low[pn].min(low[node]);

                    // Bridge check
                    if low[node] > disc[pn] {
                        bridge_pairs.push((pn, node));
                    }

                    // AP check for non-root
                    if pn != start && low[node] >= disc[pn] {
                        is_ap[pn] = true;
                    }
                }
            }
        }

        // Root is AP if it has 2+ tree children
        if root_children >= 2 {
            is_ap[start] = true;
        }
    }

    // Convert results back to node IDs
    let articulation_points: Vec<ArticulationPoint> = (0..n)
        .filter(|&i| is_ap[i])
        .map(|i| {
            let id = node_ids[i];
            let degree = adj_idx[i].len();
            ArticulationPoint {
                id: id.clone(),
                title: snapshot.nodes.get(id).map(|n| n.title.clone()).unwrap_or_default(),
                components_if_removed: degree,
            }
        })
        .collect();

    let bridge_edges: Vec<BridgeEdge> = bridge_pairs
        .iter()
        .map(|&(u, v)| {
            let uid = node_ids[u];
            let vid = node_ids[v];
            BridgeEdge {
                source_id: uid.clone(),
                target_id: vid.clone(),
                source_title: snapshot.nodes.get(uid).map(|n| n.title.clone()).unwrap_or_default(),
                target_title: snapshot.nodes.get(vid).map(|n| n.title.clone()).unwrap_or_default(),
            }
        })
        .collect();

    // Fragile connections: cross-region edge counts
    let mut region_pairs: HashMap<(String, String), usize> = HashMap::new();
    for e in &snapshot.edges {
        let ra = snapshot.regions.get(&e.source).cloned().unwrap_or_else(|| "unassigned".to_string());
        let rb = snapshot.regions.get(&e.target).cloned().unwrap_or_else(|| "unassigned".to_string());
        if ra != rb {
            // Canonical order so (A,B) == (B,A)
            let key = if ra < rb { (ra, rb) } else { (rb, ra) };
            *region_pairs.entry(key).or_insert(0) += 1;
        }
    }

    let mut fragile_connections: Vec<FragileConnection> = region_pairs
        .into_iter()
        .filter(|(_, count)| *count <= 2)
        .map(|((a, b), count)| FragileConnection {
            region_a: a,
            region_b: b,
            cross_edges: count,
        })
        .collect();
    fragile_connections.sort_by(|a, b| a.cross_edges.cmp(&b.cross_edges));

    let ap_count = articulation_points.len();
    let bridge_count = bridge_edges.len();

    BridgeReport {
        articulation_points,
        bridge_edges,
        fragile_connections,
        ap_count,
        bridge_count,
    }
}

// ---------------------------------------------------------------------------
// Health score
// ---------------------------------------------------------------------------

/// Breakdown of the health score into sub-components.
#[derive(Debug, Clone, Serialize)]
pub struct HealthBreakdown {
    pub connectivity: f64,
    pub components: f64,
    pub staleness: f64,
    pub fragility: f64,
}

/// Full analysis report combining topology, staleness, bridges, and health.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisReport {
    pub health_score: f64,
    pub health_breakdown: HealthBreakdown,
    pub topology: TopologyReport,
    pub staleness: StalenessReport,
    pub bridges: BridgeReport,
}

/// Configuration for the analyzer.
pub struct AnalyzerConfig {
    pub hub_threshold: usize,
    pub top_n: usize,
    pub stale_days: i64,
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        AnalyzerConfig {
            hub_threshold: 10,
            top_n: 50,
            stale_days: 30,
        }
    }
}

/// Run all analyses and produce a combined report with health score.
pub fn analyze(snapshot: &GraphSnapshot, config: &AnalyzerConfig) -> AnalysisReport {
    let topology = compute_topology(snapshot, config.hub_threshold, config.top_n);
    let staleness = compute_staleness(snapshot, config.stale_days);
    let bridges = compute_bridges(snapshot);

    let total = topology.total_nodes as f64;

    // Connectivity: penalize orphan ratio, capped at 20%
    let connectivity = if total > 0.0 {
        (1.0 - (topology.orphan_count as f64 / total).min(0.2) * 5.0).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // Components: ideal is 1 component
    let components = if topology.num_components > 0 {
        (1.0 / topology.num_components as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // Staleness: penalize stale ratio, capped at 10%
    let staleness_score = if total > 0.0 {
        (1.0 - (staleness.stale_node_count as f64 / total).min(0.1) * 10.0).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // Fragility: penalize articulation point ratio, capped at 5%
    let fragility = if total > 0.0 {
        (1.0 - (bridges.ap_count as f64 / total).min(0.05) * 20.0).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let health_score = 0.30 * connectivity + 0.25 * components + 0.25 * staleness_score + 0.20 * fragility;

    AnalysisReport {
        health_score,
        health_breakdown: HealthBreakdown {
            connectivity,
            components: components,
            staleness: staleness_score,
            fragility,
        },
        topology,
        staleness,
        bridges,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to build a test snapshot from compact descriptions.
    fn make_test_snapshot(
        nodes: Vec<(&str, i64, i64, Option<&str>, i32)>,
        edges: Vec<(&str, &str, &str, i64)>,
    ) -> GraphSnapshot {
        let node_infos: Vec<NodeInfo> = nodes
            .into_iter()
            .map(|(id, created, updated, parent, depth)| NodeInfo {
                id: id.to_string(),
                title: format!("Node {}", id),
                node_type: "page".to_string(),
                created_at: created,
                updated_at: updated,
                parent_id: parent.map(|p| p.to_string()),
                depth,
                is_item: depth > 1,
            })
            .collect();

        let edge_infos: Vec<EdgeInfo> = edges
            .into_iter()
            .enumerate()
            .map(|(i, (src, tgt, etype, created))| EdgeInfo {
                id: format!("e{}", i),
                source: src.to_string(),
                target: tgt.to_string(),
                edge_type: etype.to_string(),
                created_at: created,
                updated_at: None,
            })
            .collect();

        GraphSnapshot::new(node_infos, edge_infos)
    }

    // Shorthand: current time in millis (approx)
    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }

    fn days_ago(days: i64) -> i64 {
        now_ms() - days * 86_400_000
    }

    // -----------------------------------------------------------------------
    // Topology tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_topology_empty_graph() {
        let snap = make_test_snapshot(vec![], vec![]);
        let report = compute_topology(&snap, 4, 10);
        assert_eq!(report.total_nodes, 0);
        assert_eq!(report.total_edges, 0);
        assert_eq!(report.num_components, 0);
        assert_eq!(report.orphan_count, 0);
    }

    #[test]
    fn test_topology_single_component() {
        // Chain: A-B-C-D-E
        let now = now_ms();
        let snap = make_test_snapshot(
            vec![
                ("A", now, now, None, 0),
                ("B", now, now, None, 0),
                ("C", now, now, None, 0),
                ("D", now, now, None, 0),
                ("E", now, now, None, 0),
            ],
            vec![
                ("A", "B", "related", now),
                ("B", "C", "related", now),
                ("C", "D", "related", now),
                ("D", "E", "related", now),
            ],
        );
        let report = compute_topology(&snap, 4, 10);
        assert_eq!(report.total_nodes, 5);
        assert_eq!(report.num_components, 1);
        assert_eq!(report.largest_component, 5);
        assert_eq!(report.orphan_count, 0);
    }

    #[test]
    fn test_topology_two_components() {
        // Component 1: A-B-C, Component 2: D-E
        let now = now_ms();
        let snap = make_test_snapshot(
            vec![
                ("A", now, now, None, 0),
                ("B", now, now, None, 0),
                ("C", now, now, None, 0),
                ("D", now, now, None, 0),
                ("E", now, now, None, 0),
            ],
            vec![
                ("A", "B", "related", now),
                ("B", "C", "related", now),
                ("D", "E", "related", now),
            ],
        );
        let report = compute_topology(&snap, 4, 10);
        assert_eq!(report.num_components, 2);
        assert_eq!(report.largest_component, 3);
        assert_eq!(report.smallest_component, 2);
    }

    #[test]
    fn test_orphan_detection() {
        // A-B connected, C disconnected
        let now = now_ms();
        let snap = make_test_snapshot(
            vec![
                ("A", now, now, None, 0),
                ("B", now, now, None, 0),
                ("C", now, now, None, 0),
            ],
            vec![
                ("A", "B", "related", now),
            ],
        );
        let report = compute_topology(&snap, 4, 10);
        assert_eq!(report.orphan_count, 1);
        assert!(report.orphan_ids.contains(&"C".to_string()));
    }

    #[test]
    fn test_hub_detection() {
        // Star: center connected to 5 spokes
        let now = now_ms();
        let snap = make_test_snapshot(
            vec![
                ("center", now, now, None, 0),
                ("s1", now, now, None, 0),
                ("s2", now, now, None, 0),
                ("s3", now, now, None, 0),
                ("s4", now, now, None, 0),
                ("s5", now, now, None, 0),
            ],
            vec![
                ("center", "s1", "related", now),
                ("center", "s2", "related", now),
                ("center", "s3", "related", now),
                ("center", "s4", "related", now),
                ("center", "s5", "related", now),
            ],
        );
        let report = compute_topology(&snap, 4, 10);
        assert_eq!(report.hubs.len(), 1);
        assert_eq!(report.hubs[0].id, "center");
        // Undirected degree = 5 (out) + 5 (in reflected) = 10 in adj list
        // Actually: adj has both directions, so center has 5 neighbors in adj
        assert!(report.hubs[0].degree > 4);
    }

    // -----------------------------------------------------------------------
    // Bridge / Tarjan tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_tarjan_bridge() {
        // A-B-C chain: A-B and B-C are bridges, B is AP
        let now = now_ms();
        let snap = make_test_snapshot(
            vec![
                ("A", now, now, None, 0),
                ("B", now, now, None, 0),
                ("C", now, now, None, 0),
            ],
            vec![
                ("A", "B", "related", now),
                ("B", "C", "related", now),
            ],
        );
        let report = compute_bridges(&snap);
        assert_eq!(report.bridge_count, 2, "A-B and B-C are both bridges");
        assert!(report.ap_count >= 1, "B should be an articulation point");
        let ap_ids: Vec<&str> = report.articulation_points.iter().map(|a| a.id.as_str()).collect();
        assert!(ap_ids.contains(&"B"), "B is the articulation point");
    }

    #[test]
    fn test_tarjan_cycle_no_bridges() {
        // Triangle: A-B, B-C, C-A -- no bridges, no APs
        let now = now_ms();
        let snap = make_test_snapshot(
            vec![
                ("A", now, now, None, 0),
                ("B", now, now, None, 0),
                ("C", now, now, None, 0),
            ],
            vec![
                ("A", "B", "related", now),
                ("B", "C", "related", now),
                ("C", "A", "related", now),
            ],
        );
        let report = compute_bridges(&snap);
        assert_eq!(report.bridge_count, 0);
        assert_eq!(report.ap_count, 0);
    }

    #[test]
    fn test_tarjan_two_cycles_joined() {
        // Two triangles connected by a bridge:
        // Triangle 1: A-B-C-A
        // Triangle 2: D-E-F-D
        // Bridge: C-D
        let now = now_ms();
        let snap = make_test_snapshot(
            vec![
                ("A", now, now, None, 0),
                ("B", now, now, None, 0),
                ("C", now, now, None, 0),
                ("D", now, now, None, 0),
                ("E", now, now, None, 0),
                ("F", now, now, None, 0),
            ],
            vec![
                ("A", "B", "related", now),
                ("B", "C", "related", now),
                ("C", "A", "related", now),
                ("D", "E", "related", now),
                ("E", "F", "related", now),
                ("F", "D", "related", now),
                ("C", "D", "related", now), // bridge
            ],
        );
        let report = compute_bridges(&snap);
        assert_eq!(report.bridge_count, 1, "C-D is the only bridge");
        assert!(report.ap_count >= 2, "C and D are articulation points");
        let ap_ids: Vec<&str> = report.articulation_points.iter().map(|a| a.id.as_str()).collect();
        assert!(ap_ids.contains(&"C"));
        assert!(ap_ids.contains(&"D"));
    }

    // -----------------------------------------------------------------------
    // Staleness tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_staleness_detected() {
        // Node A: updated 90 days ago, referenced by a 1-day-old edge
        let now = now_ms();
        let snap = make_test_snapshot(
            vec![
                ("A", days_ago(100), days_ago(90), None, 0),
                ("B", now, now, None, 0),
            ],
            vec![
                ("B", "A", "reference", days_ago(1)),
            ],
        );
        let report = compute_staleness(&snap, 30);
        assert_eq!(report.stale_node_count, 1);
        assert_eq!(report.stale_nodes[0].id, "A");
        assert!(report.stale_nodes[0].days_since_update >= 89);
    }

    #[test]
    fn test_staleness_no_false_positive() {
        // Node A: updated 90 days ago, but only old edges (60 days old)
        let snap = make_test_snapshot(
            vec![
                ("A", days_ago(100), days_ago(90), None, 0),
                ("B", days_ago(100), days_ago(60), None, 0),
            ],
            vec![
                ("B", "A", "reference", days_ago(60)),
            ],
        );
        let report = compute_staleness(&snap, 30);
        assert_eq!(report.stale_node_count, 0, "Old node with only old edges should not be stale");
    }

    #[test]
    fn test_stale_summary() {
        // Summary S summarizes target T. T was updated after S.
        let snap = make_test_snapshot(
            vec![
                ("S", days_ago(10), days_ago(10), None, 0), // summary, 10 days old
                ("T", days_ago(20), days_ago(2), None, 0),  // target, updated 2 days ago
            ],
            vec![
                ("S", "T", "summarizes", days_ago(10)),
            ],
        );
        let report = compute_staleness(&snap, 30);
        assert_eq!(report.stale_summary_count, 1);
        assert_eq!(report.stale_summaries[0].summary_node_id, "S");
        assert_eq!(report.stale_summaries[0].target_node_id, "T");
        assert!(report.stale_summaries[0].drift_days >= 7, "drift should be ~8 days");
    }

    // -----------------------------------------------------------------------
    // Region tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_region_computation() {
        // Root (depth 0) -> Cat (depth 1) -> Item (depth 2)
        let now = now_ms();
        let snap = make_test_snapshot(
            vec![
                ("root", now, now, None, 0),
                ("cat", now, now, Some("root"), 1),
                ("item", now, now, Some("cat"), 2),
            ],
            vec![],
        );

        assert_eq!(snap.regions.get("root").unwrap(), "root");
        assert_eq!(snap.regions.get("cat").unwrap(), "cat");
        assert_eq!(snap.regions.get("item").unwrap(), "cat", "item's depth-1 ancestor is cat");
    }

    #[test]
    fn test_fragile_connections() {
        // Two regions (cat1, cat2) connected by 1 cross-edge
        let now = now_ms();
        let snap = make_test_snapshot(
            vec![
                ("root", now, now, None, 0),
                ("cat1", now, now, Some("root"), 1),
                ("cat2", now, now, Some("root"), 1),
                ("item1", now, now, Some("cat1"), 2),
                ("item2", now, now, Some("cat2"), 2),
            ],
            vec![
                ("item1", "item2", "related", now), // single cross-region edge
            ],
        );
        let report = compute_bridges(&snap);
        assert!(!report.fragile_connections.is_empty(), "Should detect fragile connection");
        let fragile = &report.fragile_connections[0];
        assert_eq!(fragile.cross_edges, 1);
    }

    // -----------------------------------------------------------------------
    // Health score tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_health_score_range() {
        // Various graph shapes should always produce health in [0, 1]
        let now = now_ms();

        // Disconnected graph with orphans
        let snap = make_test_snapshot(
            vec![
                ("A", now, now, None, 0),
                ("B", now, now, None, 0),
                ("C", now, now, None, 0),
            ],
            vec![],
        );
        let config = AnalyzerConfig::default();
        let report = analyze(&snap, &config);
        assert!(report.health_score >= 0.0 && report.health_score <= 1.0,
            "Health {:.3} out of range", report.health_score);

        // Connected graph
        let snap2 = make_test_snapshot(
            vec![
                ("A", now, now, None, 0),
                ("B", now, now, None, 0),
            ],
            vec![("A", "B", "related", now)],
        );
        let report2 = analyze(&snap2, &config);
        assert!(report2.health_score >= 0.0 && report2.health_score <= 1.0,
            "Health {:.3} out of range", report2.health_score);
    }

    #[test]
    fn test_health_score_perfect() {
        // Single connected component, no orphans, no stale, no APs
        // Triangle avoids bridges/APs
        let now = now_ms();
        let snap = make_test_snapshot(
            vec![
                ("A", now, now, None, 0),
                ("B", now, now, None, 0),
                ("C", now, now, None, 0),
            ],
            vec![
                ("A", "B", "related", now),
                ("B", "C", "related", now),
                ("C", "A", "related", now),
            ],
        );
        let config = AnalyzerConfig { hub_threshold: 10, top_n: 50, stale_days: 30 };
        let report = analyze(&snap, &config);

        // Should be very close to 1.0:
        // connectivity = 1.0 (no orphans)
        // components = 1.0 (1 component)
        // staleness = 1.0 (nothing stale)
        // fragility = 1.0 (no APs)
        // health = 0.30 + 0.25 + 0.25 + 0.20 = 1.0
        assert!(report.health_score > 0.95,
            "Perfect graph should have health ~1.0, got {:.3}", report.health_score);
        assert!((report.health_score - 1.0).abs() < 0.01,
            "Expected ~1.0, got {:.3}", report.health_score);
    }
}
