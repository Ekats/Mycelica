//! Clustering module with AI + TF-IDF fallback
//!
//! Primary: Claude AI for semantic clustering (when API key available)
//! Fallback: Pure Rust TF-IDF keyword clustering (offline mode)
//!
//! Multi-path associations: Items connect to multiple topics with varying strengths.
//! - Primary association: stored in cluster_id (for hierarchy navigation)
//! - All associations: stored as BelongsTo edges (for graph traversal)

use std::collections::{HashMap, HashSet};
use crate::db::{Database, Node, Edge, EdgeType};
use crate::ai_client::{self, ClusterItem, ExistingCluster, MultiClusterAssignment};
use crate::commands::{is_rebuild_cancelled, reset_rebuild_cancel};
use serde::Serialize;

/// Result of clustering operation
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusteringResult {
    pub items_processed: usize,
    pub clusters_created: usize,
    pub items_assigned: usize,
    pub edges_created: usize,  // Multi-path BelongsTo edges created
    pub method: String, // "ai" or "tfidf"
}

/// Stop words to filter out
const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with",
    "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do", "does",
    "did", "will", "would", "could", "should", "may", "might", "must", "shall", "can",
    "this", "that", "these", "those", "i", "you", "he", "she", "it", "we", "they", "me",
    "him", "her", "us", "them", "my", "your", "his", "its", "our", "their", "what", "which",
    "who", "whom", "when", "where", "why", "how", "all", "each", "every", "both", "few",
    "more", "most", "other", "some", "such", "no", "nor", "not", "only", "own", "same",
    "so", "than", "too", "very", "just", "also", "now", "here", "there", "then", "once",
    "if", "because", "as", "until", "while", "about", "against", "between", "into",
    "through", "during", "before", "after", "above", "below", "from", "up", "down", "out",
    "off", "over", "under", "again", "further", "any", "like", "get", "got", "getting",
    "make", "made", "making", "use", "using", "used", "need", "want", "know", "think",
    "see", "look", "find", "give", "tell", "say", "said", "go", "going", "come", "take",
    "yes", "yeah", "okay", "ok", "sure", "right", "well", "actually", "really", "just",
    "thing", "things", "something", "anything", "everything", "way", "ways", "time",
    "new", "old", "good", "bad", "first", "last", "long", "great", "little", "own",
    "able", "trying", "try", "let", "example", "untitled", "help", "please", "thanks",
    "claude", "assistant", "human", "user", "message", "conversation", "chat",
];

/// Extract keywords from text using TF-IDF-like scoring
pub fn extract_keywords(text: &str, top_n: usize) -> Vec<(String, f64)> {
    let words = tokenize(text);
    let stop_set: HashSet<&str> = STOP_WORDS.iter().copied().collect();

    // Count word frequencies
    let mut word_counts: HashMap<String, usize> = HashMap::new();
    let mut total_words = 0;

    for word in words {
        if word.len() < 3 || word.len() > 25 {
            continue;
        }
        if stop_set.contains(word.as_str()) {
            continue;
        }
        // Skip numbers
        if word.chars().all(|c| c.is_numeric()) {
            continue;
        }
        *word_counts.entry(word).or_insert(0) += 1;
        total_words += 1;
    }

    if total_words == 0 {
        return vec![];
    }

    // Calculate TF scores (term frequency)
    let mut scored: Vec<(String, f64)> = word_counts
        .into_iter()
        .map(|(word, count)| {
            let tf = count as f64 / total_words as f64;
            // Boost multi-occurrence words
            let boost = if count > 1 { 1.0 + (count as f64).ln() } else { 1.0 };
            (word, tf * boost)
        })
        .collect();

    // Sort by score descending
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    scored.truncate(top_n);
    scored
}

/// Tokenize text into lowercase words
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_matches('\'').to_string())
        .collect()
}

/// Node data for clustering
#[derive(Debug, Clone)]
pub struct NodeData {
    pub id: String,
    pub title: String,
    pub content: String,
}

/// Cluster result
#[derive(Debug, Clone)]
pub struct ClusterResult {
    pub cluster_id: i32,
    pub label: String,
    pub node_ids: Vec<String>,
}

/// Cluster nodes based on keyword similarity
pub fn cluster_nodes(nodes: &[NodeData], min_cluster_size: usize) -> Vec<ClusterResult> {
    if nodes.is_empty() {
        return vec![];
    }

    // Extract keywords for each node
    let node_keywords: Vec<(String, HashSet<String>)> = nodes
        .iter()
        .map(|n| {
            let text = format!("{} {}", n.title, n.content);
            let keywords: HashSet<String> = extract_keywords(&text, 20)
                .into_iter()
                .map(|(w, _)| w)
                .collect();
            (n.id.clone(), keywords)
        })
        .collect();

    // Build similarity matrix using Jaccard similarity
    let n = nodes.len();
    let mut similarities: Vec<Vec<f64>> = vec![vec![0.0; n]; n];

    for i in 0..n {
        for j in i..n {
            if i == j {
                similarities[i][j] = 1.0;
            } else {
                let sim = jaccard_similarity(&node_keywords[i].1, &node_keywords[j].1);
                similarities[i][j] = sim;
                similarities[j][i] = sim;
            }
        }
    }

    // Simple agglomerative clustering
    let labels = agglomerative_cluster(&similarities, 0.15); // threshold for similarity

    // Group nodes by cluster
    let mut cluster_nodes_map: HashMap<i32, Vec<usize>> = HashMap::new();
    for (idx, &label) in labels.iter().enumerate() {
        cluster_nodes_map.entry(label).or_default().push(idx);
    }

    // Generate cluster results with labels
    let mut results: Vec<ClusterResult> = cluster_nodes_map
        .into_iter()
        .filter(|(_, indices)| indices.len() >= min_cluster_size)
        .map(|(cluster_id, indices)| {
            // Collect all keywords from nodes in this cluster
            let mut keyword_counts: HashMap<String, usize> = HashMap::new();
            for &idx in &indices {
                for kw in &node_keywords[idx].1 {
                    *keyword_counts.entry(kw.clone()).or_insert(0) += 1;
                }
            }

            // Get top keywords that appear in multiple nodes
            let mut top_keywords: Vec<(String, usize)> = keyword_counts
                .into_iter()
                .filter(|(_, count)| *count > 1 || indices.len() == 1)
                .collect();
            top_keywords.sort_by(|a, b| b.1.cmp(&a.1));

            // Generate label from top 3 keywords
            let label = top_keywords
                .iter()
                .take(3)
                .map(|(kw, _)| capitalize(kw))
                .collect::<Vec<_>>()
                .join(", ");

            let label = if label.is_empty() {
                format!("Cluster {}", cluster_id)
            } else {
                label
            };

            ClusterResult {
                cluster_id,
                label,
                node_ids: indices.iter().map(|&i| nodes[i].id.clone()).collect(),
            }
        })
        .collect();

    // Handle unclustered nodes (put in "Other" cluster)
    let clustered_ids: HashSet<String> = results
        .iter()
        .flat_map(|c| c.node_ids.iter().cloned())
        .collect();

    let unclustered: Vec<String> = nodes
        .iter()
        .filter(|n| !clustered_ids.contains(&n.id))
        .map(|n| n.id.clone())
        .collect();

    if !unclustered.is_empty() {
        let max_id = results.iter().map(|c| c.cluster_id).max().unwrap_or(-1);
        results.push(ClusterResult {
            cluster_id: max_id + 1,
            label: "Miscellaneous".to_string(),
            node_ids: unclustered,
        });
    }

    // Renumber clusters from 0
    results.sort_by_key(|c| std::cmp::Reverse(c.node_ids.len()));
    for (i, cluster) in results.iter_mut().enumerate() {
        cluster.cluster_id = i as i32;
    }

    results
}

/// Jaccard similarity between two sets
fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Simple agglomerative clustering
fn agglomerative_cluster(similarities: &[Vec<f64>], threshold: f64) -> Vec<i32> {
    let n = similarities.len();
    if n == 0 {
        return vec![];
    }

    // Start with each node in its own cluster
    let mut labels: Vec<i32> = (0..n as i32).collect();

    // Merge similar clusters
    loop {
        let mut best_merge: Option<(i32, i32, f64)> = None;

        // Find the best merge (highest similarity above threshold)
        for i in 0..n {
            for j in (i + 1)..n {
                if labels[i] != labels[j] && similarities[i][j] > threshold {
                    match &best_merge {
                        None => best_merge = Some((labels[i], labels[j], similarities[i][j])),
                        Some((_, _, best_sim)) if similarities[i][j] > *best_sim => {
                            best_merge = Some((labels[i], labels[j], similarities[i][j]));
                        }
                        _ => {}
                    }
                }
            }
        }

        match best_merge {
            Some((cluster_a, cluster_b, _)) => {
                // Merge cluster_b into cluster_a
                for label in labels.iter_mut() {
                    if *label == cluster_b {
                        *label = cluster_a;
                    }
                }
            }
            None => break, // No more merges possible
        }
    }

    // Renumber clusters to be consecutive
    let unique_labels: HashSet<i32> = labels.iter().copied().collect();
    let mut label_map: HashMap<i32, i32> = HashMap::new();
    for (new_id, old_id) in unique_labels.into_iter().enumerate() {
        label_map.insert(old_id, new_id as i32);
    }

    labels.iter().map(|l| *label_map.get(l).unwrap_or(l)).collect()
}

/// Capitalize first letter
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

/// Build content string for clustering, preferring AI-generated fields
/// Priority: summary > content, with tags appended for extra semantic signal
fn build_clustering_content(node: &Node) -> String {
    // Prefer summary (AI-generated, concise) over raw content
    let main_content = node.summary.clone()
        .or_else(|| node.content.clone())
        .unwrap_or_default();

    // Append tags if available - they're high-quality semantic keywords
    if let Some(ref tags_json) = node.tags {
        // Tags are stored as JSON array: ["tag1", "tag2", ...]
        if let Ok(tags) = serde_json::from_str::<Vec<String>>(tags_json) {
            if !tags.is_empty() {
                let tags_str = tags.join(" ");
                return format!("{} {}", main_content, tags_str);
            }
        }
    }

    main_content
}

// ==================== Main Clustering Orchestration ====================

/// Batch size for AI clustering (balance token limits vs API calls)
const AI_BATCH_SIZE: usize = 15;

/// Run clustering on items that need it
/// Uses AI when available, falls back to TF-IDF
pub async fn run_clustering(db: &Database, use_ai: bool) -> Result<ClusteringResult, String> {
    // Reset cancel flag at start
    reset_rebuild_cancel();

    // Get items needing clustering
    let all_items = db.get_items_needing_clustering().map_err(|e| e.to_string())?;

    // Filter out protected items (Recent Notes and descendants)
    let protected_ids = db.get_protected_node_ids();
    let items: Vec<Node> = all_items
        .into_iter()
        .filter(|item| !protected_ids.contains(&item.id))
        .collect();

    let skipped = protected_ids.len();
    if skipped > 0 {
        println!("[Clustering] Skipping {} protected items (Recent Notes)", skipped);
    }

    if items.is_empty() {
        return Ok(ClusteringResult {
            items_processed: 0,
            clusters_created: 0,
            items_assigned: 0,
            edges_created: 0,
            method: "none".to_string(),
        });
    }

    println!("[Clustering] Starting for {} items", items.len());

    // Check if AI is available and requested
    let ai_available = use_ai && ai_client::is_available();

    if ai_available {
        cluster_with_ai(db, &items).await
    } else {
        if use_ai && !ai_client::is_available() {
            println!("AI clustering requested but no API key - falling back to TF-IDF");
        }
        cluster_with_tfidf(db, &items)
    }
}

/// Cluster items using Claude AI with multi-path associations
async fn cluster_with_ai(db: &Database, items: &[Node]) -> Result<ClusteringResult, String> {
    // Get existing clusters for context
    let existing_clusters_raw = db.get_existing_clusters().map_err(|e| e.to_string())?;
    let existing_clusters: Vec<ExistingCluster> = existing_clusters_raw
        .into_iter()
        .map(|(id, label, count)| ExistingCluster { id, label, count })
        .collect();

    let mut next_cluster_id = db.get_next_cluster_id().map_err(|e| e.to_string())?;
    let mut total_assigned = 0;
    let mut new_clusters_created = 0;
    let mut total_edges_created = 0;

    // Process in batches
    for (batch_idx, batch) in items.chunks(AI_BATCH_SIZE).enumerate() {
        // Check for cancellation
        if is_rebuild_cancelled() {
            println!("[Clustering] Cancelled by user after {} batches", batch_idx);
            return Err("Clustering cancelled by user".to_string());
        }

        println!("[Clustering] Processing batch {} ({} items)", batch_idx + 1, batch.len());

        // Convert to ClusterItem format (include AI-processed fields if available)
        let cluster_items: Vec<ClusterItem> = batch
            .iter()
            .map(|node| ClusterItem {
                id: node.id.clone(),
                title: node.title.clone(),
                content: node.content.clone().unwrap_or_default(),
                ai_title: node.ai_title.clone(),
                summary: node.summary.clone(),
                tags: node.tags.clone(),
            })
            .collect();

        // Call AI multi-path clustering
        match ai_client::cluster_items_with_ai_multipath(&cluster_items, &existing_clusters, next_cluster_id).await {
            Ok(assignments) => {
                // Apply multi-path assignments
                let (assigned, new_clusters, edges) = apply_multipath_assignments(db, &assignments, next_cluster_id)?;
                total_assigned += assigned;
                new_clusters_created += new_clusters;
                total_edges_created += edges;

                // Update next_cluster_id based on highest new cluster seen
                for assignment in &assignments {
                    for cluster in &assignment.clusters {
                        if cluster.is_new && cluster.id >= next_cluster_id {
                            next_cluster_id = cluster.id + 1;
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("AI clustering failed for batch {}: {}", batch_idx + 1, e);
                // Fall back to TF-IDF for this batch
                let tfidf_result = cluster_batch_with_tfidf_multipath(db, batch, next_cluster_id)?;
                total_assigned += tfidf_result.0;
                new_clusters_created += tfidf_result.1;
                total_edges_created += tfidf_result.2;
                next_cluster_id += tfidf_result.1 as i32;
            }
        }
    }

    println!("AI clustering complete: {} items assigned, {} new clusters, {} edges created",
             total_assigned, new_clusters_created, total_edges_created);

    Ok(ClusteringResult {
        items_processed: items.len(),
        clusters_created: new_clusters_created,
        items_assigned: total_assigned,
        edges_created: total_edges_created,
        method: "ai".to_string(),
    })
}

/// Apply multi-path assignments: primary → cluster_id, all → BelongsTo edges
/// Returns (items_assigned, new_clusters_created, edges_created)
fn apply_multipath_assignments(
    db: &Database,
    assignments: &[MultiClusterAssignment],
    next_cluster_id: i32,
) -> Result<(usize, usize, usize), String> {
    let mut items_assigned = 0;
    let mut new_clusters = 0;
    let mut edges_created = 0;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    for assignment in assignments {
        if assignment.clusters.is_empty() {
            continue;
        }

        // Primary = highest strength (clusters are pre-sorted)
        let primary = &assignment.clusters[0];

        // Update cluster_id for hierarchy navigation
        db.update_node_clustering(
            &assignment.item_id,
            primary.id,
            &primary.label,
        ).map_err(|e| e.to_string())?;
        items_assigned += 1;

        // Track new clusters
        if primary.is_new && primary.id >= next_cluster_id {
            new_clusters += 1;
        }

        // Clear old BelongsTo edges for this item
        db.delete_belongs_to_edges(&assignment.item_id).map_err(|e| e.to_string())?;

        // Create BelongsTo edges for ALL associations
        for cluster in &assignment.clusters {
            // Find target node: existing topic node or placeholder
            let target_id = db.find_topic_node_for_cluster(cluster.id)
                .map_err(|e| e.to_string())?
                .unwrap_or_else(|| format!("cluster-{}", cluster.id));

            let edge = Edge {
                id: format!("{}-belongs-to-{}", assignment.item_id, cluster.id),
                source: assignment.item_id.clone(),
                target: target_id,
                edge_type: EdgeType::BelongsTo,
                label: Some(cluster.label.clone()),
                weight: Some(cluster.strength),
                edge_source: Some("ai".to_string()),  // Mark as AI-generated (can be re-clustered)
                evidence_id: None,
                confidence: None,
                created_at: now,
            };

            // Insert edge (ignore errors for duplicate IDs on re-clustering)
            if db.insert_edge(&edge).is_ok() {
                edges_created += 1;
            }
        }
    }

    Ok((items_assigned, new_clusters, edges_created))
}

/// Cluster items using TF-IDF (fallback) with multi-path edges
fn cluster_with_tfidf(db: &Database, items: &[Node]) -> Result<ClusteringResult, String> {
    let next_cluster_id = db.get_next_cluster_id().map_err(|e| e.to_string())?;
    let (assigned, new_clusters, edges) = cluster_batch_with_tfidf_multipath(db, items, next_cluster_id)?;

    println!("TF-IDF clustering complete: {} items assigned, {} new clusters, {} edges",
             assigned, new_clusters, edges);

    Ok(ClusteringResult {
        items_processed: items.len(),
        clusters_created: new_clusters,
        items_assigned: assigned,
        edges_created: edges,
        method: "tfidf".to_string(),
    })
}

/// Cluster a batch of items using TF-IDF
/// Returns (items_assigned, new_clusters_created)
#[allow(dead_code)]
fn cluster_batch_with_tfidf(db: &Database, items: &[Node], start_cluster_id: i32) -> Result<(usize, usize), String> {
    // Convert to NodeData format - prefer AI-generated content when available
    let node_data: Vec<NodeData> = items
        .iter()
        .map(|n| NodeData {
            id: n.id.clone(),
            title: n.ai_title.clone().unwrap_or_else(|| n.title.clone()),
            content: build_clustering_content(n),
        })
        .collect();

    // Run TF-IDF clustering
    let clusters = cluster_nodes(&node_data, 2);

    let mut assigned = 0;
    let mut new_clusters = 0;

    for cluster in &clusters {
        let cluster_id = start_cluster_id + cluster.cluster_id;

        for node_id in &cluster.node_ids {
            db.update_node_clustering(node_id, cluster_id, &cluster.label)
                .map_err(|e| e.to_string())?;
            assigned += 1;
        }

        if cluster.label != "Miscellaneous" {
            new_clusters += 1;
        }
    }

    Ok((assigned, new_clusters))
}

/// Cluster a batch of items using TF-IDF with multi-path edges
/// Returns (items_assigned, new_clusters_created, edges_created)
fn cluster_batch_with_tfidf_multipath(db: &Database, items: &[Node], start_cluster_id: i32) -> Result<(usize, usize, usize), String> {
    // Convert to NodeData format - prefer AI-generated content when available
    let node_data: Vec<NodeData> = items
        .iter()
        .map(|n| NodeData {
            id: n.id.clone(),
            title: n.ai_title.clone().unwrap_or_else(|| n.title.clone()),
            content: build_clustering_content(n),
        })
        .collect();

    // Extract keywords for each node (needed for secondary associations)
    // Uses NodeData which already has AI-preferred content
    let node_keywords: Vec<(String, HashSet<String>)> = node_data
        .iter()
        .map(|n| {
            let text = format!("{} {}", n.title, n.content);
            let keywords: HashSet<String> = extract_keywords(&text, 20)
                .into_iter()
                .map(|(w, _)| w)
                .collect();
            (n.id.clone(), keywords)
        })
        .collect();

    // Run TF-IDF clustering
    let clusters = cluster_nodes(&node_data, 2);

    // Build cluster keyword profiles for similarity
    let mut cluster_keywords: HashMap<i32, HashSet<String>> = HashMap::new();
    for cluster in &clusters {
        let cluster_id = start_cluster_id + cluster.cluster_id;
        let mut combined_keywords: HashSet<String> = HashSet::new();
        for node_id in &cluster.node_ids {
            if let Some((_, kws)) = node_keywords.iter().find(|(id, _)| id == node_id) {
                combined_keywords.extend(kws.iter().cloned());
            }
        }
        cluster_keywords.insert(cluster_id, combined_keywords);
    }

    let mut assigned = 0;
    let mut new_clusters = 0;
    let mut edges_created = 0;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    // Build map of node_id -> primary cluster
    let mut node_primary: HashMap<String, (i32, String)> = HashMap::new();
    for cluster in &clusters {
        let cluster_id = start_cluster_id + cluster.cluster_id;
        for node_id in &cluster.node_ids {
            node_primary.insert(node_id.clone(), (cluster_id, cluster.label.clone()));
        }
    }

    // Assign and create edges
    for (node_id, keywords) in &node_keywords {
        let Some((primary_cluster_id, primary_label)) = node_primary.get(node_id) else {
            continue;
        };

        // Update cluster_id for hierarchy
        db.update_node_clustering(node_id, *primary_cluster_id, primary_label)
            .map_err(|e| e.to_string())?;
        assigned += 1;

        // Clear old BelongsTo edges
        db.delete_belongs_to_edges(node_id).map_err(|e| e.to_string())?;

        // Calculate similarity to all clusters and create edges
        let mut cluster_similarities: Vec<(i32, String, f64)> = clusters
            .iter()
            .map(|c| {
                let cluster_id = start_cluster_id + c.cluster_id;
                let similarity = cluster_keywords
                    .get(&cluster_id)
                    .map(|ckw| jaccard_similarity(keywords, ckw))
                    .unwrap_or(0.0);
                (cluster_id, c.label.clone(), similarity)
            })
            .filter(|(_, _, sim)| *sim > 0.05) // Only include non-trivial associations
            .collect();

        // Sort by similarity descending
        cluster_similarities.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

        // Keep top 4 associations
        cluster_similarities.truncate(4);

        // Ensure primary is always first with highest weight
        if !cluster_similarities.is_empty() && cluster_similarities[0].0 != *primary_cluster_id {
            // Primary wasn't highest similarity - adjust weights
            let primary_strength = cluster_similarities.first().map(|c| c.2).unwrap_or(0.8).max(0.8);
            cluster_similarities.retain(|(id, _, _)| *id != *primary_cluster_id);
            cluster_similarities.insert(0, (*primary_cluster_id, primary_label.clone(), primary_strength));
            cluster_similarities.truncate(4);
        }

        // Create edges for all associations
        for (cluster_id, label, strength) in &cluster_similarities {
            let target_id = db.find_topic_node_for_cluster(*cluster_id)
                .map_err(|e| e.to_string())?
                .unwrap_or_else(|| format!("cluster-{}", cluster_id));

            let edge = Edge {
                id: format!("{}-belongs-to-{}", node_id, cluster_id),
                source: node_id.clone(),
                target: target_id,
                edge_type: EdgeType::BelongsTo,
                label: Some(label.clone()),
                weight: Some(*strength),
                edge_source: Some("ai".to_string()),  // Mark as AI-generated (TF-IDF fallback still auto-generated)
                evidence_id: None,
                confidence: None,
                created_at: now,
            };

            if db.insert_edge(&edge).is_ok() {
                edges_created += 1;
            }
        }
    }

    // Count new clusters (excluding Miscellaneous)
    for cluster in &clusters {
        if cluster.label != "Miscellaneous" {
            new_clusters += 1;
        }
    }

    Ok((assigned, new_clusters, edges_created))
}

/// Force re-clustering of all items (clears existing assignments)
pub async fn recluster_all(db: &Database, use_ai: bool) -> Result<ClusteringResult, String> {
    // Mark all items as needing clustering
    let count = db.mark_all_items_need_clustering().map_err(|e| e.to_string())?;
    println!("Marked {} items for re-clustering", count);

    // Run clustering
    run_clustering(db, use_ai).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_keywords() {
        let text = "Rust programming is great for systems programming. Rust is memory safe.";
        let keywords = extract_keywords(text, 5);
        assert!(!keywords.is_empty());
        // "rust" and "programming" should be top keywords
        let words: Vec<&str> = keywords.iter().map(|(w, _)| w.as_str()).collect();
        assert!(words.contains(&"rust") || words.contains(&"programming"));
    }

    #[test]
    fn test_jaccard_similarity() {
        let a: HashSet<String> = ["rust", "code", "fast"].iter().map(|s| s.to_string()).collect();
        let b: HashSet<String> = ["rust", "code", "safe"].iter().map(|s| s.to_string()).collect();
        let sim = jaccard_similarity(&a, &b);
        assert!(sim > 0.4 && sim < 0.6); // 2/4 = 0.5
    }
}
