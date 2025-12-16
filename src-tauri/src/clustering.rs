//! Clustering module - embedding-based with TF-IDF fallback
//!
//! Primary: Embedding cosine similarity clustering (deterministic, local compute)
//! AI: Used only for naming clusters (single call for all clusters)
//! Fallback: TF-IDF keyword clustering for items without embeddings
//!
//! Multi-path associations: Items connect to multiple topics with varying strengths.
//! - Primary association: stored in cluster_id (for hierarchy navigation)
//! - All associations: stored as BelongsTo edges (for graph traversal)

use std::collections::{HashMap, HashSet};
use crate::db::{Database, Node, Edge, EdgeType};
use crate::ai_client::{self, MultiClusterAssignment};
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

// NOTE: Old TF-IDF clustering structs (NodeData, ClusterResult) and functions
// (cluster_nodes, jaccard_similarity, agglomerative_cluster) have been removed.
// Embedding-based clustering now handles all clustering logic.

/// Capitalize first letter
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

// NOTE: build_clustering_content() removed - embedding generation uses title + summary directly

// ==================== Main Clustering Orchestration ====================

/// Run clustering on items that need it
/// Uses embedding-based clustering (primary), with TF-IDF fallback for items without embeddings
pub async fn run_clustering(db: &Database, _use_ai: bool) -> Result<ClusteringResult, String> {
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

    println!("[Clustering] Starting embedding-based clustering for {} items", items.len());

    // Use embedding-based clustering (generates embeddings on-demand if missing)
    cluster_with_embeddings(db, &items).await
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

// NOTE: TF-IDF clustering (cluster_with_tfidf) has been removed.
// We now use embedding-based clustering exclusively.
// TF-IDF keyword extraction is still used for fallback cluster naming.

// NOTE: TF-IDF batch clustering functions have been removed.
// The embedding-based approach handles all clustering now.

// ==================== Embedding-Based Clustering ====================

use crate::similarity::{cosine_similarity, compute_centroid};

/// Get adaptive thresholds based on collection size
fn get_adaptive_thresholds(item_count: usize) -> (f32, f32) {
    match item_count {
        0..=50 => (0.50, 0.35),
        51..=300 => (0.55, 0.40),
        301..=1000 => (0.60, 0.45),
        _ => (0.65, 0.50),
    }
}

/// Ensure all items have embeddings, generating on-demand if needed
async fn ensure_embeddings(
    db: &Database,
    items: &[Node],
) -> Result<Vec<(Node, Vec<f32>)>, String> {
    let mut result = Vec::with_capacity(items.len());

    for item in items {
        // Check for cancellation
        if is_rebuild_cancelled() {
            return Err("Embedding generation cancelled".to_string());
        }

        let embedding = match db.get_node_embedding(&item.id).map_err(|e| e.to_string())? {
            Some(emb) => emb,
            None => {
                // Generate on-demand using title + summary
                let text = format!(
                    "{} {}",
                    item.ai_title.as_ref().unwrap_or(&item.title),
                    item.summary.as_deref().unwrap_or("")
                );

                println!("[Clustering] Generating embedding for item: {}", item.id);
                let emb = ai_client::generate_embedding(&text).await?;
                db.update_node_embedding(&item.id, &emb).map_err(|e| e.to_string())?;
                emb
            }
        };

        result.push((item.clone(), embedding));
    }

    Ok(result)
}

/// Agglomerative clustering using cosine similarity on embeddings
/// Uses average linkage (UPGMA) for cluster merging
// TODO: Future optimization - use priority queue for merge candidates
// instead of scanning all pairs each iteration (would improve from O(n³) to O(n² log n))
fn agglomerative_cluster_embeddings(
    embeddings: &[(String, Vec<f32>)],
    threshold: f32,
) -> Vec<Vec<usize>> {
    let n = embeddings.len();
    if n == 0 {
        return vec![];
    }

    // Start with each item in its own cluster
    let mut labels: Vec<i32> = (0..n as i32).collect();

    // Precompute similarity matrix
    let mut similarities: Vec<Vec<f32>> = vec![vec![0.0; n]; n];
    for i in 0..n {
        similarities[i][i] = 1.0;
        for j in (i + 1)..n {
            let sim = cosine_similarity(&embeddings[i].1, &embeddings[j].1);
            similarities[i][j] = sim;
            similarities[j][i] = sim;
        }
    }

    // Merge clusters iteratively
    loop {
        let mut best_merge: Option<(i32, i32, f32)> = None;

        // Find best merge (highest average linkage above threshold)
        for i in 0..n {
            for j in (i + 1)..n {
                if labels[i] != labels[j] {
                    // Calculate average linkage between clusters
                    let cluster_i: Vec<usize> = labels.iter()
                        .enumerate()
                        .filter(|(_, &l)| l == labels[i])
                        .map(|(idx, _)| idx)
                        .collect();
                    let cluster_j: Vec<usize> = labels.iter()
                        .enumerate()
                        .filter(|(_, &l)| l == labels[j])
                        .map(|(idx, _)| idx)
                        .collect();

                    let mut total_sim = 0.0;
                    let mut count = 0;
                    for &ci in &cluster_i {
                        for &cj in &cluster_j {
                            total_sim += similarities[ci][cj];
                            count += 1;
                        }
                    }
                    let avg_sim = if count > 0 { total_sim / count as f32 } else { 0.0 };

                    if avg_sim > threshold {
                        match &best_merge {
                            None => best_merge = Some((labels[i], labels[j], avg_sim)),
                            Some((_, _, best_sim)) if avg_sim > *best_sim => {
                                best_merge = Some((labels[i], labels[j], avg_sim));
                            }
                            _ => {}
                        }
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
            None => break,
        }
    }

    // Group indices by cluster label
    let unique_labels: HashSet<i32> = labels.iter().copied().collect();
    unique_labels
        .into_iter()
        .map(|cluster_label| {
            labels.iter()
                .enumerate()
                .filter(|(_, &l)| l == cluster_label)
                .map(|(idx, _)| idx)
                .collect()
        })
        .collect()
}

/// Cluster items using embedding similarity
/// Returns cluster assignments in MultiClusterAssignment format
pub async fn cluster_with_embeddings(
    db: &Database,
    items: &[Node],
) -> Result<ClusteringResult, String> {
    println!("[Clustering] Using embedding-based clustering for {} items", items.len());

    // Step 1: Ensure all items have embeddings
    let items_with_embeddings = ensure_embeddings(db, items).await?;

    if items_with_embeddings.is_empty() {
        return Ok(ClusteringResult {
            items_processed: 0,
            clusters_created: 0,
            items_assigned: 0,
            edges_created: 0,
            method: "embedding".to_string(),
        });
    }

    // Step 2: Get adaptive thresholds
    let (primary_threshold, secondary_threshold) = get_adaptive_thresholds(items.len());
    println!("[Clustering] Using thresholds: primary={}, secondary={}", primary_threshold, secondary_threshold);

    // Step 3: Run agglomerative clustering
    let embeddings: Vec<(String, Vec<f32>)> = items_with_embeddings
        .iter()
        .map(|(node, emb)| (node.id.clone(), emb.clone()))
        .collect();

    let cluster_groups = agglomerative_cluster_embeddings(&embeddings, primary_threshold);
    println!("[Clustering] Formed {} clusters", cluster_groups.len());

    // Step 4: Compute centroids for each cluster
    let mut cluster_data: Vec<(i32, Vec<usize>, Vec<f32>)> = Vec::new(); // (cluster_id, member_indices, centroid)
    let next_cluster_id = db.get_next_cluster_id().map_err(|e| e.to_string())?;

    for (idx, indices) in cluster_groups.iter().enumerate() {
        let cluster_id = next_cluster_id + idx as i32;
        let member_embeddings: Vec<&[f32]> = indices
            .iter()
            .map(|&i| embeddings[i].1.as_slice())
            .collect();

        if let Some(centroid) = compute_centroid(&member_embeddings) {
            cluster_data.push((cluster_id, indices.clone(), centroid));
        }
    }

    // Step 5: Name clusters with AI
    let cluster_names = name_clusters_with_ai(db, &cluster_data, &items_with_embeddings).await?;

    // Step 6: Build multi-path assignments
    let assignments = build_multipath_assignments(
        &items_with_embeddings,
        &cluster_data,
        &cluster_names,
        secondary_threshold,
    );

    // Step 7: Apply assignments using existing function
    let (assigned, new_clusters, edges) = apply_multipath_assignments(db, &assignments, next_cluster_id)?;

    println!("[Clustering] Embedding clustering complete: {} items assigned, {} clusters, {} edges",
             assigned, new_clusters, edges);

    Ok(ClusteringResult {
        items_processed: items.len(),
        clusters_created: new_clusters,
        items_assigned: assigned,
        edges_created: edges,
        method: "embedding".to_string(),
    })
}

/// Name clusters using AI (single call for all clusters)
async fn name_clusters_with_ai(
    _db: &Database,
    cluster_data: &[(i32, Vec<usize>, Vec<f32>)],
    items_with_embeddings: &[(Node, Vec<f32>)],
) -> Result<HashMap<i32, String>, String> {
    let mut names: HashMap<i32, String> = HashMap::new();

    if cluster_data.is_empty() {
        return Ok(names);
    }

    // Build cluster info for AI
    let mut clusters_info: Vec<(i32, Vec<String>)> = Vec::new();
    for (cluster_id, indices, _) in cluster_data {
        let titles: Vec<String> = indices
            .iter()
            .take(15) // Sample up to 15 titles per cluster
            .filter_map(|&i| {
                items_with_embeddings.get(i).map(|(node, _)| {
                    node.ai_title.clone().unwrap_or_else(|| node.title.clone())
                })
            })
            .collect();
        clusters_info.push((*cluster_id, titles));
    }

    // Call AI to name clusters
    match ai_client::name_clusters(&clusters_info).await {
        Ok(ai_names) => {
            for (cluster_id, name) in ai_names {
                names.insert(cluster_id, name);
            }
        }
        Err(e) => {
            eprintln!("[Clustering] AI naming failed, using keyword fallback: {}", e);
            // Fallback: use keywords from titles
            for (cluster_id, titles) in &clusters_info {
                let combined = titles.join(" ");
                let keywords = extract_keywords(&combined, 3);
                let name = if keywords.is_empty() {
                    format!("Cluster {}", cluster_id)
                } else {
                    keywords.iter()
                        .map(|(w, _)| capitalize(w))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                names.insert(*cluster_id, name);
            }
        }
    }

    // Ensure all clusters have names
    for (cluster_id, _, _) in cluster_data {
        names.entry(*cluster_id).or_insert_with(|| format!("Cluster {}", cluster_id));
    }

    Ok(names)
}

/// Build multi-path assignments: each item gets assigned to all clusters above secondary threshold
fn build_multipath_assignments(
    items_with_embeddings: &[(Node, Vec<f32>)],
    cluster_data: &[(i32, Vec<usize>, Vec<f32>)],
    cluster_names: &HashMap<i32, String>,
    secondary_threshold: f32,
) -> Vec<MultiClusterAssignment> {
    use crate::ai_client::ClusterWithStrength;

    items_with_embeddings
        .iter()
        .enumerate()
        .map(|(item_idx, (node, item_embedding))| {
            // Calculate similarity to each cluster centroid
            let mut cluster_similarities: Vec<(i32, f32)> = cluster_data
                .iter()
                .map(|(cluster_id, _, centroid)| {
                    let sim = cosine_similarity(item_embedding, centroid);
                    (*cluster_id, sim)
                })
                .filter(|(_, sim)| *sim >= secondary_threshold)
                .collect();

            // Sort by similarity descending
            cluster_similarities.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            // Keep top 4
            cluster_similarities.truncate(4);

            // Find primary cluster (the one this item was assigned to during agglomerative clustering)
            let primary_cluster = cluster_data
                .iter()
                .find(|(_, indices, _)| indices.contains(&item_idx))
                .map(|(id, _, _)| *id);

            // Ensure primary is first with highest weight
            if let Some(primary_id) = primary_cluster {
                if cluster_similarities.first().map(|(id, _)| *id) != Some(primary_id) {
                    cluster_similarities.retain(|(id, _)| *id != primary_id);
                    let primary_strength = cluster_similarities.first()
                        .map(|(_, s)| s.max(0.8))
                        .unwrap_or(0.8);
                    cluster_similarities.insert(0, (primary_id, primary_strength));
                    cluster_similarities.truncate(4);
                }
            }

            // If no clusters above threshold, use primary with default strength
            if cluster_similarities.is_empty() {
                if let Some(primary_id) = primary_cluster {
                    cluster_similarities.push((primary_id, 0.7));
                }
            }

            // Convert to ClusterWithStrength
            let clusters: Vec<ClusterWithStrength> = cluster_similarities
                .iter()
                .map(|(cluster_id, strength)| ClusterWithStrength {
                    id: *cluster_id,
                    label: cluster_names.get(cluster_id)
                        .cloned()
                        .unwrap_or_else(|| format!("Cluster {}", cluster_id)),
                    strength: *strength as f64,
                    is_new: true, // All clusters from embedding clustering are new
                })
                .collect();

            MultiClusterAssignment {
                item_id: node.id.clone(),
                clusters,
            }
        })
        .collect()
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
}
