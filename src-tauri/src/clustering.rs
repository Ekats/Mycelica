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
use rand::seq::SliceRandom;
use crate::utils::safe_truncate;

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
/// NOTE: Only clusters items where content_type = 'idea' OR content_type IS NULL
/// Items with content_type = 'code', 'debug', or 'paste' are skipped (supporting items)
pub async fn run_clustering(db: &Database, use_ai: bool) -> Result<ClusteringResult, String> {
    // Reset cancel flag at start
    reset_rebuild_cancel();

    // Get items needing clustering
    let all_items = db.get_items_needing_clustering().map_err(|e| e.to_string())?;

    // Filter out protected items (Recent Notes and descendants)
    let protected_ids = db.get_protected_node_ids();
    let after_protected: Vec<Node> = all_items
        .into_iter()
        .filter(|item| !protected_ids.contains(&item.id))
        .collect();

    let skipped_protected = protected_ids.len();
    if skipped_protected > 0 {
        println!("[Clustering] Skipping {} protected items (Recent Notes)", skipped_protected);
    }

    // Filter out non-visible items - only cluster VISIBLE tier (insight, exploration, synthesis, question, planning)
    // SUPPORTING tier (investigation, discussion, reference, creative) and HIDDEN tier (debug, code, paste, trivial) are excluded
    let total_after_protected = after_protected.len();
    let after_content_filter: Vec<Node> = after_protected
        .into_iter()
        .filter(|item| {
            match item.content_type.as_deref() {
                // VISIBLE tier - cluster these
                None | Some("insight") | Some("idea") | Some("exploration") |
                Some("synthesis") | Some("question") | Some("planning") => true,
                // SUPPORTING tier - exclude from clustering
                Some("investigation") | Some("discussion") | Some("reference") | Some("creative") => false,
                // HIDDEN tier - exclude from clustering
                Some("code") | Some("debug") | Some("paste") | Some("trivial") => false,
                _ => true,  // Include any unknown types (backwards compat)
            }
        })
        .collect();

    let skipped_supporting_count = total_after_protected - after_content_filter.len();
    if skipped_supporting_count > 0 {
        println!("[Clustering] Skipping {} non-visible items (supporting/hidden)", skipped_supporting_count);
    }

    // Filter out private items (privacy < threshold) - they go to Personal category
    let privacy_threshold = crate::settings::get_privacy_threshold() as f64;
    let (private_items, items): (Vec<Node>, Vec<Node>) = after_content_filter
        .into_iter()
        .partition(|item| item.privacy.map(|p| p < privacy_threshold).unwrap_or(false));

    if !private_items.is_empty() {
        println!("[Clustering] Excluding {} private items (privacy < {}) - will go to Personal category", private_items.len(), privacy_threshold);
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

    println!("[Clustering] Starting embedding-based clustering for {} items (use_ai={})", items.len(), use_ai);

    // Use embedding-based clustering (generates embeddings on-demand if missing)
    cluster_with_embeddings_impl(db, &items, use_ai).await
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

/// Get clustering thresholds from settings, with 0.75/0.60 defaults for accurate clustering
fn get_adaptive_thresholds(_item_count: usize) -> (f32, f32) {
    // Get thresholds from settings (defaults to 0.75/0.60)
    let (primary_opt, secondary_opt) = crate::settings::get_clustering_thresholds();

    let primary = primary_opt.unwrap_or(0.75);
    let secondary = secondary_opt.unwrap_or(0.60);

    println!("[Clustering] Using thresholds: primary={:.2}, secondary={:.2}", primary, secondary);
    (primary, secondary)
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
                // Generate embedding from TITLE + CONTENT for better semantic differentiation
                // Title helps distinguish items with similar/identical content (e.g., book chapters)
                let title = item.ai_title.as_ref().unwrap_or(&item.title);
                let text = if let Some(content) = &item.content {
                    // Prepend title to content (reserve ~100 chars for title, rest for content)
                    format!("{}\n\n{}", title, safe_truncate(content, 900))
                } else {
                    // Fallback for items without content (shouldn't happen for items)
                    format!("{} {}", title, item.summary.as_deref().unwrap_or(""))
                };

                println!("[Clustering] Generating embedding for item: {} ({}chars)", item.id, text.len());
                let emb = ai_client::generate_embedding(&text).await?;
                db.update_node_embedding(&item.id, &emb).map_err(|e| e.to_string())?;
                emb
            }
        };

        result.push((item.clone(), embedding));
    }

    Ok(result)
}

/// Conversation bonus for items from the same conversation
/// Same conversation = same session of thought, provides cheap structural signal
const CONVERSATION_BONUS: f32 = 0.1;

/// Tag bonus per shared tag between items
/// Persistent tags provide stable similarity anchors across rebuilds
const TAG_BONUS_PER_TAG: f32 = 0.08;

/// Agglomerative clustering using cosine similarity on embeddings
/// Uses Union-Find for O(n²) complexity instead of O(n³)
///
/// Items from the same conversation get a small similarity bonus (0.1)
/// Items with shared tags get additional bonus (+0.08 per shared tag)
fn agglomerative_cluster_embeddings(
    embeddings: &[(String, Vec<f32>, Option<String>)],  // (id, embedding, conversation_id)
    threshold: f32,
    item_tags: &HashMap<String, Vec<String>>,  // Pre-loaded item -> tags mapping
) -> Vec<Vec<usize>> {
    let n = embeddings.len();
    if n == 0 {
        return vec![];
    }

    // Use Union-Find for efficient clustering
    let mut uf = UnionFind::new(n);

    // Single pass: union all pairs above threshold - O(n²)
    for i in 0..n {
        for j in (i + 1)..n {
            let base_sim = cosine_similarity(&embeddings[i].1, &embeddings[j].1);

            // Add conversation bonus if both items are from the same conversation
            let same_conversation = embeddings[i].2.is_some()
                && embeddings[i].2 == embeddings[j].2;
            let conversation_bonus = if same_conversation { CONVERSATION_BONUS } else { 0.0 };

            // Add tag bonus for shared persistent tags
            let tag_bonus = compute_tag_bonus(&embeddings[i].0, &embeddings[j].0, item_tags);

            let sim = (base_sim + conversation_bonus + tag_bonus).min(1.0);

            if sim >= threshold {
                uf.union(i, j);
            }
        }
    }

    // Group indices by their root - O(n)
    let mut groups: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for i in 0..n {
        let root = uf.find(i);
        groups.entry(root).or_default().push(i);
    }

    groups.into_values().collect()
}

/// Compute similarity bonus based on shared tags between two items
fn compute_tag_bonus(item_a_id: &str, item_b_id: &str, item_tags: &HashMap<String, Vec<String>>) -> f32 {
    let tags_a = item_tags.get(item_a_id);
    let tags_b = item_tags.get(item_b_id);

    match (tags_a, tags_b) {
        (Some(a), Some(b)) => {
            // Count shared tags
            let shared_count = a.iter().filter(|t| b.contains(t)).count();
            shared_count as f32 * TAG_BONUS_PER_TAG
        }
        _ => 0.0,
    }
}

/// Cluster a single batch of items using agglomerative clustering
/// Returns: Vec<(centroid, Vec<item_indices>)>
fn cluster_batch(
    embeddings: &[(String, Vec<f32>, Option<String>)],  // (id, embedding, conversation_id)
    threshold: f32,
    item_tags: &HashMap<String, Vec<String>>,  // Pre-loaded item -> tags mapping
) -> Vec<(Vec<f32>, Vec<usize>)> {
    if embeddings.is_empty() {
        return vec![];
    }

    // Use existing agglomerative clustering for small batch
    let cluster_groups = agglomerative_cluster_embeddings(embeddings, threshold, item_tags);

    // Compute centroid for each cluster
    cluster_groups.iter().map(|indices| {
        let member_embeddings: Vec<&[f32]> = indices.iter()
            .map(|&i| embeddings[i].1.as_slice())
            .collect();
        let centroid = compute_centroid(&member_embeddings)
            .unwrap_or_else(|| vec![0.0; embeddings[0].1.len()]);
        (centroid, indices.clone())
    }).collect()
}

/// Union-Find data structure for efficient cluster merging
struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]); // Path compression
        }
        self.parent[x]
    }

    fn union(&mut self, x: usize, y: usize) {
        let px = self.find(x);
        let py = self.find(y);
        if px == py { return; }

        // Union by rank
        if self.rank[px] < self.rank[py] {
            self.parent[px] = py;
        } else if self.rank[px] > self.rank[py] {
            self.parent[py] = px;
        } else {
            self.parent[py] = px;
            self.rank[px] += 1;
        }
    }
}

/// Merge clusters with centroids above threshold similarity
/// Uses Union-Find for O(c²) instead of O(c³) complexity
fn merge_similar_clusters(
    clusters: &mut Vec<(Vec<f32>, Vec<usize>)>,
    threshold: f32,
) {
    let n = clusters.len();
    if n <= 1 { return; }

    // Step 1: Find ALL pairs above threshold (single O(c²) pass)
    let mut uf = UnionFind::new(n);
    let mut merge_count = 0;

    for i in 0..n {
        for j in (i + 1)..n {
            let sim = cosine_similarity(&clusters[i].0, &clusters[j].0);
            if sim >= threshold {
                uf.union(i, j);
                merge_count += 1;
            }
        }
    }

    if merge_count == 0 { return; }

    // Step 2: Group clusters by their root
    let mut groups: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for i in 0..n {
        let root = uf.find(i);
        groups.entry(root).or_default().push(i);
    }

    // Step 3: Merge each group into single cluster
    let mut new_clusters: Vec<(Vec<f32>, Vec<usize>)> = Vec::new();

    for (_root, group) in groups {
        if group.len() == 1 {
            // Single cluster, keep as-is
            new_clusters.push(clusters[group[0]].clone());
        } else {
            // Merge all clusters in group
            let mut merged_indices: Vec<usize> = Vec::new();
            let mut weighted_centroid: Vec<f32> = vec![0.0; clusters[group[0]].0.len()];
            let mut total_size: f32 = 0.0;

            for &idx in &group {
                let (centroid, indices) = &clusters[idx];
                let size = indices.len() as f32;

                // Weighted centroid accumulation
                for (i, &val) in centroid.iter().enumerate() {
                    weighted_centroid[i] += val * size;
                }
                total_size += size;
                merged_indices.extend(indices.iter().cloned());
            }

            // Normalize centroid
            if total_size > 0.0 {
                for val in &mut weighted_centroid {
                    *val /= total_size;
                }
            }

            new_clusters.push((weighted_centroid, merged_indices));
        }
    }

    *clusters = new_clusters;
}

/// Cluster items using embedding similarity
/// Returns cluster assignments in MultiClusterAssignment format
///
/// `use_ai_naming`: true = use AI for cluster names (costs $), false = use keywords (FREE)
pub async fn cluster_with_embeddings(
    db: &Database,
    items: &[Node],
) -> Result<ClusteringResult, String> {
    cluster_with_embeddings_impl(db, items, true).await
}

/// Internal implementation with optional AI naming flag
async fn cluster_with_embeddings_impl(
    db: &Database,
    items: &[Node],
    use_ai_naming: bool,
) -> Result<ClusteringResult, String> {
    // Group items by parent_id to preserve existing hierarchy (e.g., FOS categories)
    // Items with same parent get clustered together; items without parent cluster together
    let items_by_parent: HashMap<Option<String>, Vec<Node>> = items
        .iter()
        .cloned()
        .fold(HashMap::new(), |mut acc, item| {
            acc.entry(item.parent_id.clone()).or_default().push(item);
            acc
        });

    let num_groups = items_by_parent.len();
    if num_groups > 1 {
        println!("[Clustering] Respecting {} parent groups (FOS/hierarchy preserved)", num_groups);
    }

    let mut total_result = ClusteringResult {
        items_processed: 0,
        clusters_created: 0,
        items_assigned: 0,
        edges_created: 0,
        method: "embedding".to_string(),
    };

    // Cluster each parent group separately
    for (parent_id, group_items) in items_by_parent {
        let group_name = parent_id.as_deref().unwrap_or("(no parent)");
        if num_groups > 1 {
            println!("[Clustering] Processing group '{}' with {} items", group_name, group_items.len());
        }

        let result = cluster_single_group(db, &group_items, use_ai_naming).await?;

        total_result.items_processed += result.items_processed;
        total_result.clusters_created += result.clusters_created;
        total_result.items_assigned += result.items_assigned;
        total_result.edges_created += result.edges_created;
    }

    Ok(total_result)
}

/// Cluster a single group of items (all with same parent_id)
async fn cluster_single_group(
    db: &Database,
    items: &[Node],
    use_ai_naming: bool,
) -> Result<ClusteringResult, String> {
    use std::time::Instant;

    if items.is_empty() {
        return Ok(ClusteringResult {
            items_processed: 0,
            clusters_created: 0,
            items_assigned: 0,
            edges_created: 0,
            method: "embedding".to_string(),
        });
    }

    let total_start = Instant::now();
    println!("[Clustering] Using embedding-based clustering for {} items", items.len());

    // Step 1: Ensure all items have embeddings
    let embed_start = Instant::now();
    let items_with_embeddings = ensure_embeddings(db, items).await?;
    println!("[Timing] Embeddings loaded/generated: {:.2}s", embed_start.elapsed().as_secs_f64());

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

    // Step 2.5: Load persistent tag associations for similarity bonus
    let item_tags = db.get_all_item_tags_map().unwrap_or_default();
    if !item_tags.is_empty() {
        println!("[Clustering] Loaded {} items with persistent tag associations", item_tags.len());
    }

    // Step 3: Prepare embeddings for clustering (with conversation_id for bonus)
    let embeddings: Vec<(String, Vec<f32>, Option<String>)> = items_with_embeddings
        .iter()
        .map(|(node, emb)| (node.id.clone(), emb.clone(), node.conversation_id.clone()))
        .collect();

    // Step 4: Global vs Batch clustering based on dataset size
    // Global: Better cluster quality (no fragmentation/contamination) for small datasets
    // Batch + refinement: Required for large datasets (papers) to avoid O(n²) explosion
    const GLOBAL_THRESHOLD: usize = 10_000;
    const BATCH_SIZE: usize = 10_000;

    // Time estimate for large datasets
    let comparisons = embeddings.len() * (embeddings.len() - 1) / 2;
    let estimate_secs = comparisons as f64 / 1_000_000.0;
    if estimate_secs > 60.0 {
        println!("[Clustering] Estimated time: {:.1} minutes (one-time cost for quality)",
                 estimate_secs / 60.0);
    }

    let mut all_clusters: Vec<(Vec<f32>, Vec<usize>)> = if embeddings.len() < GLOBAL_THRESHOLD {
        // GLOBAL CLUSTERING: Compare all items to all items
        // O(n²) but fast for small datasets: 5000 items = 12.5M comparisons ≈ 12.5s at 1M cmp/s
        println!("[Clustering] Using GLOBAL clustering for {} items (< {} threshold)",
                 embeddings.len(), GLOBAL_THRESHOLD);

        let cluster_start = Instant::now();

        let batch_clusters = cluster_batch(&embeddings, primary_threshold, &item_tags);

        let elapsed = cluster_start.elapsed().as_secs_f64();
        println!("[Timing] Global clustering: {:.2}s for {} comparisons ({:.0} cmp/s)",
                 elapsed, comparisons, comparisons as f64 / elapsed.max(0.001));

        batch_clusters
    } else {
        // BATCH CLUSTERING + K-MEANS REFINEMENT
        // For large datasets: batch → merge → iterative reassignment
        println!("[Clustering] Using BATCH clustering for {} items (>= {} threshold)",
                 embeddings.len(), GLOBAL_THRESHOLD);

        // Shuffle to spread similar items across batches
        let mut shuffled = embeddings.clone();
        shuffled.shuffle(&mut rand::thread_rng());

        let mut batch_clusters: Vec<(Vec<f32>, Vec<usize>)> = Vec::new();
        let mut global_offset = 0;
        let num_batches = (shuffled.len() + BATCH_SIZE - 1) / BATCH_SIZE;
        let mut total_similarity_time = 0.0f64;

        for batch_start in (0..shuffled.len()).step_by(BATCH_SIZE) {
            let batch_end = (batch_start + BATCH_SIZE).min(shuffled.len());
            let batch: Vec<(String, Vec<f32>, Option<String>)> = shuffled[batch_start..batch_end]
                .iter()
                .cloned()
                .collect();

            let batch_start_time = Instant::now();
            println!("[Clustering] Processing batch {}-{} of {} ({}/{})",
                     batch_start, batch_end, shuffled.len(),
                     batch_start / BATCH_SIZE + 1, num_batches);

            let clusters = cluster_batch(&batch, primary_threshold, &item_tags);
            let batch_elapsed = batch_start_time.elapsed().as_secs_f64();
            total_similarity_time += batch_elapsed;

            let batch_size_actual = batch_end - batch_start;
            let batch_comparisons = batch_size_actual * (batch_size_actual - 1) / 2;
            println!("[Timing] Batch {}/{}: {:.3}s for {} comparisons ({:.0} cmp/s)",
                     batch_start / BATCH_SIZE + 1, num_batches,
                     batch_elapsed, batch_comparisons,
                     batch_comparisons as f64 / batch_elapsed.max(0.001));

            // Adjust indices to global offset
            for (centroid, local_indices) in clusters {
                let global_indices: Vec<usize> = local_indices.iter()
                    .map(|&i| i + global_offset)
                    .collect();
                batch_clusters.push((centroid, global_indices));
            }
            global_offset = batch_end;
        }

        println!("[Timing] All batches complete: {:.2}s total similarity computation", total_similarity_time);
        println!("[Clustering] {} clusters from {} batches, merging similar...",
                 batch_clusters.len(), num_batches);

        // Merge similar clusters across batches
        let merge_start = Instant::now();
        let merge_threshold = primary_threshold * 0.95;
        merge_similar_clusters(&mut batch_clusters, merge_threshold);
        println!("[Timing] Cluster merge: {:.3}s for {} clusters",
                 merge_start.elapsed().as_secs_f64(), batch_clusters.len());

        // K-MEANS REFINEMENT: Iteratively reassign items to nearest centroid globally
        // Fixes fragmentation from batch boundaries (sock leaves MBTI, joins household)
        println!("[Clustering] Running k-means refinement to fix batch fragmentation...");
        let refine_start = Instant::now();
        let mut iteration = 0;
        const MAX_ITERATIONS: usize = 10;

        loop {
            iteration += 1;
            let mut changed = 0usize;

            // Build item → cluster assignment map
            let mut item_cluster: Vec<usize> = vec![0; shuffled.len()];
            for (cluster_idx, (_, indices)) in batch_clusters.iter().enumerate() {
                for &item_idx in indices {
                    item_cluster[item_idx] = cluster_idx;
                }
            }

            // For each item, find nearest centroid globally
            for item_idx in 0..shuffled.len() {
                let item_embedding = &shuffled[item_idx].1;
                let current_cluster = item_cluster[item_idx];

                // Find best cluster by similarity to centroid
                let mut best_cluster = current_cluster;
                let mut best_sim = cosine_similarity(item_embedding, &batch_clusters[current_cluster].0);

                for (cluster_idx, (centroid, _)) in batch_clusters.iter().enumerate() {
                    if cluster_idx == current_cluster {
                        continue;
                    }
                    let sim = cosine_similarity(item_embedding, centroid);
                    if sim > best_sim {
                        best_sim = sim;
                        best_cluster = cluster_idx;
                    }
                }

                if best_cluster != current_cluster {
                    // Reassign: remove from old, add to new
                    batch_clusters[current_cluster].1.retain(|&i| i != item_idx);
                    batch_clusters[best_cluster].1.push(item_idx);
                    changed += 1;
                }
            }

            // Recompute centroids after reassignments
            for (centroid, indices) in &mut batch_clusters {
                if indices.is_empty() {
                    continue;
                }
                let member_embeddings: Vec<&[f32]> = indices.iter()
                    .map(|&i| shuffled[i].1.as_slice())
                    .collect();
                if let Some(new_centroid) = compute_centroid(&member_embeddings) {
                    *centroid = new_centroid;
                }
            }

            // Remove empty clusters
            batch_clusters.retain(|(_, indices)| !indices.is_empty());

            println!("[Refinement] Iteration {}: {} items reassigned, {} clusters remaining",
                     iteration, changed, batch_clusters.len());

            if changed == 0 || iteration >= MAX_ITERATIONS {
                break;
            }
        }

        println!("[Timing] K-means refinement: {:.2}s ({} iterations)",
                 refine_start.elapsed().as_secs_f64(), iteration);

        batch_clusters
    };

    println!("[Clustering] After clustering: {} clusters", all_clusters.len());

    // Convert to cluster_groups format
    let cluster_groups: Vec<Vec<usize>> = all_clusters.iter()
        .map(|(_, indices)| indices.clone())
        .collect();

    // Step 5: Build cluster_data with IDs
    let mut cluster_data: Vec<(i32, Vec<usize>, Vec<f32>)> = Vec::new();
    let next_cluster_id = db.get_next_cluster_id().map_err(|e| e.to_string())?;

    for (idx, (centroid, indices)) in all_clusters.iter().enumerate() {
        let cluster_id = next_cluster_id + idx as i32;
        cluster_data.push((cluster_id, indices.clone(), centroid.clone()));
    }

    println!("[Clustering] Formed {} clusters", cluster_groups.len());

    // Step 5: Name clusters (AI or reuse existing names)
    let naming_start = Instant::now();
    let cluster_names = if use_ai_naming {
        name_clusters_with_ai(db, &cluster_data, &items_with_embeddings).await?
    } else {
        // LITE mode: reuse existing cluster_label from items, fallback to keywords
        reuse_existing_cluster_names(&cluster_data, &items_with_embeddings)
    };
    println!("[Timing] Cluster naming: {:.2}s (ai={})", naming_start.elapsed().as_secs_f64(), use_ai_naming);

    // Step 6: Build multi-path assignments
    let assign_start = Instant::now();
    let assignments = build_multipath_assignments(
        &items_with_embeddings,
        &cluster_data,
        &cluster_names,
        secondary_threshold,
    );
    println!("[Timing] Build assignments: {:.3}s for {} items", assign_start.elapsed().as_secs_f64(), assignments.len());

    // Step 7: Apply assignments using existing function
    let db_write_start = Instant::now();
    let (assigned, _new_clusters, edges) = apply_multipath_assignments(db, &assignments, next_cluster_id)?;
    println!("[Timing] DB writes: {:.2}s for {} assignments + {} edges", db_write_start.elapsed().as_secs_f64(), assigned, edges);

    let actual_cluster_count = cluster_data.len();

    println!("[Clustering] Embedding clustering complete: {} items assigned to {} clusters, {} edges",
             assigned, actual_cluster_count, edges);
    println!("[Timing] TOTAL cluster_single_group: {:.2}s", total_start.elapsed().as_secs_f64());

    Ok(ClusteringResult {
        items_processed: items.len(),
        clusters_created: actual_cluster_count,
        items_assigned: assigned,
        edges_created: edges,
        method: "embedding".to_string(),
    })
}

/// Maximum clusters per AI naming batch to prevent response truncation
const NAMING_BATCH_SIZE: usize = 30;

/// Name clusters using AI with batching to prevent response truncation
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
            .take(10) // Sample up to 10 titles per cluster (reduced from 15 for batching)
            .filter_map(|&i| {
                items_with_embeddings.get(i).map(|(node, _)| {
                    node.ai_title.clone().unwrap_or_else(|| node.title.clone())
                })
            })
            .collect();
        clusters_info.push((*cluster_id, titles));
    }

    // Batch clusters for AI naming to prevent response truncation
    let num_batches = (clusters_info.len() + NAMING_BATCH_SIZE - 1) / NAMING_BATCH_SIZE;
    if num_batches > 1 {
        println!("[Clustering] Naming {} clusters in {} batches of ~{}",
                 clusters_info.len(), num_batches, NAMING_BATCH_SIZE);
    }

    for (batch_idx, batch) in clusters_info.chunks(NAMING_BATCH_SIZE).enumerate() {
        if num_batches > 1 {
            println!("[Clustering] Naming batch {}/{} ({} clusters)",
                     batch_idx + 1, num_batches, batch.len());
        }

        // Call AI to name this batch of clusters
        match ai_client::name_clusters(batch).await {
            Ok(ai_names) => {
                for (cluster_id, name) in ai_names {
                    names.insert(cluster_id, name);
                }
            }
            Err(e) => {
                eprintln!("[Clustering] AI naming failed for batch {}, using keyword fallback: {}",
                         batch_idx + 1, e);
                // Fallback: use keywords from titles for this batch
                for (cluster_id, titles) in batch {
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
    }

    // Ensure all clusters have names (fallback for any missed)
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

// ==================== Lite Clustering (No AI) ====================

/// Reuse existing cluster labels from items using majority voting (FREE)
///
/// - If >50% of members share the same cluster_label → cluster is stable, use that label
/// - If no majority → cluster merged/split, generate keyword name
pub fn reuse_existing_cluster_names(
    cluster_data: &[(i32, Vec<usize>, Vec<f32>)],
    items_with_embeddings: &[(Node, Vec<f32>)],
) -> HashMap<i32, String> {
    let mut names: HashMap<i32, String> = HashMap::new();

    for (cluster_id, indices, _) in cluster_data {
        // Count label occurrences
        let mut label_counts: HashMap<String, usize> = HashMap::new();
        for &i in indices {
            if let Some((node, _)) = items_with_embeddings.get(i) {
                if let Some(label) = &node.cluster_label {
                    if !label.is_empty() {
                        *label_counts.entry(label.clone()).or_insert(0) += 1;
                    }
                }
            }
        }

        // Find majority label (>50% of cluster members)
        let majority_threshold = indices.len() / 2;
        let majority_label = label_counts.iter()
            .filter(|(_, count)| **count > majority_threshold)
            .max_by_key(|(_, count)| *count)
            .map(|(label, _)| label.clone());

        let name = if let Some(label) = majority_label {
            // Cluster is stable - keep existing AI name
            label
        } else {
            // Cluster merged/split - generate keyword name
            let titles: Vec<String> = indices
                .iter()
                .take(15)
                .filter_map(|&i| {
                    items_with_embeddings.get(i).map(|(node, _)| {
                        node.ai_title.clone().unwrap_or_else(|| node.title.clone())
                    })
                })
                .collect();

            let combined = titles.join(" ");
            let keywords = extract_keywords(&combined, 3);

            if keywords.is_empty() {
                titles.first()
                    .and_then(|t| t.split_whitespace().find(|w| w.len() > 3))
                    .map(|w| capitalize(w))
                    .unwrap_or_else(|| format!("Cluster {}", cluster_id))
            } else {
                keywords.iter()
                    .map(|(w, _)| capitalize(w))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        };

        names.insert(*cluster_id, name);
    }

    names
}

/// Name clusters using keywords only (no AI) - FREE
/// Extracts top keywords from member titles to generate cluster names
pub fn name_clusters_from_keywords(
    cluster_data: &[(i32, Vec<usize>, Vec<f32>)],
    items_with_embeddings: &[(Node, Vec<f32>)],
) -> HashMap<i32, String> {
    let mut names: HashMap<i32, String> = HashMap::new();

    for (cluster_id, indices, _) in cluster_data {
        // Collect titles from cluster members
        let titles: Vec<String> = indices
            .iter()
            .take(15) // Sample up to 15 titles
            .filter_map(|&i| {
                items_with_embeddings.get(i).map(|(node, _)| {
                    node.ai_title.clone().unwrap_or_else(|| node.title.clone())
                })
            })
            .collect();

        // Combine titles and extract keywords
        let combined = titles.join(" ");
        let keywords = extract_keywords(&combined, 3);

        let name = if keywords.is_empty() {
            // Fallback to first significant word from first title
            titles.first()
                .and_then(|t| t.split_whitespace().find(|w| w.len() > 3))
                .map(|w| capitalize(w))
                .unwrap_or_else(|| format!("Cluster {}", cluster_id))
        } else {
            keywords.iter()
                .map(|(w, _)| capitalize(w))
                .collect::<Vec<_>>()
                .join(", ")
        };

        names.insert(*cluster_id, name);
    }

    names
}

/// Cluster with embeddings using keyword-based naming (NO AI) - FREE
///
/// Same as cluster_with_embeddings but uses keyword extraction instead of AI for naming.
/// Use this for Rebuild Lite mode.
pub async fn cluster_with_embeddings_lite(db: &Database) -> Result<ClusteringResult, String> {
    // Reset cancel flag
    reset_rebuild_cancel();

    // Step 1: Get items that need clustering (VISIBLE tier only)
    let items = db.get_items_needing_clustering().map_err(|e| e.to_string())?;

    if items.is_empty() {
        return Ok(ClusteringResult {
            items_processed: 0,
            clusters_created: 0,
            items_assigned: 0,
            edges_created: 0,
            method: "embedding-lite".to_string(),
        });
    }

    println!("[Clustering LITE] Processing {} items with embedding similarity (no AI)...", items.len());

    // Use the regular embedding clustering flow
    let result = cluster_with_embeddings_impl(db, &items, false).await?;

    println!("[Clustering LITE] Complete: {} items → {} clusters, {} edges (keyword naming)",
             result.items_assigned, result.clusters_created, result.edges_created);

    Ok(ClusteringResult {
        items_processed: result.items_processed,
        clusters_created: result.clusters_created,
        items_assigned: result.items_assigned,
        edges_created: result.edges_created,
        method: "embedding-lite".to_string(),
    })
}

/// Cluster papers by FOS (Field of Science) - creates actual hierarchy nodes
/// No embeddings needed - just reads FOS from papers.subjects JSON.
/// Creates FOS parent nodes under universe, assigns papers as children.
/// Fast O(n) operation.
pub async fn cluster_with_fos_pregrouping(db: &Database) -> Result<ClusteringResult, String> {
    use crate::db::{Node, NodeType, Position};

    // Get papers grouped by FOS directly from database
    let papers_by_fos = db.get_papers_by_fos().map_err(|e| e.to_string())?;

    if papers_by_fos.is_empty() {
        println!("[Clustering FOS] No papers with FOS data found");
        return Ok(ClusteringResult {
            items_processed: 0,
            clusters_created: 0,
            items_assigned: 0,
            edges_created: 0,
            method: "fos".to_string(),
        });
    }

    let total_papers: usize = papers_by_fos.values().map(|v| v.len()).sum();
    println!("[Clustering FOS] {} papers across {} FOS categories", total_papers, papers_by_fos.len());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    // Get or create universe node
    let universe = match db.get_universe().map_err(|e| e.to_string())? {
        Some(u) => u,
        None => {
            // Create Universe node
            println!("[Clustering FOS] Creating Universe node");
            let universe_node = Node {
                id: "universe-root".to_string(),
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
                parent_id: None,
                child_count: 0,
                emoji: Some("🌌".to_string()),
                summary: None,
                ai_title: None,
                tags: None,
                is_processed: false,
                is_pinned: false,
                is_private: None,
                last_accessed_at: None,
                conversation_id: None,
                sequence_index: None,
                latest_child_date: None,
                privacy_reason: None,
                source: None,
                pdf_available: None,
                content_type: None,
                associated_idea_id: None,
                privacy: None,
            };
            db.insert_node(&universe_node).map_err(|e| e.to_string())?;
            universe_node
        }
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let mut items_assigned = 0;
    let mut clusters_created = 0;

    // Create FOS parent nodes and assign papers
    for (fos_name, paper_ids) in &papers_by_fos {
        if paper_ids.is_empty() {
            continue;
        }

        // Create FOS category node
        let fos_id = format!("fos-{}", fos_name.to_lowercase().replace(' ', "-"));

        // Check if FOS node already exists
        let fos_node = if let Ok(Some(existing)) = db.get_node(&fos_id) {
            println!("[Clustering FOS] '{}': using existing node", fos_name);
            existing
        } else {
            // Create new FOS parent node
            let node = Node {
                id: fos_id.clone(),
                node_type: NodeType::Cluster,
                title: fos_name.clone(),
                url: None,
                content: Some(format!("{} papers", paper_ids.len())),
                position: Position { x: 0.0, y: 0.0 },
                created_at: now,
                updated_at: now,
                cluster_id: None,
                cluster_label: Some(fos_name.clone()),
                depth: 1, // Direct child of universe
                is_item: false,
                is_universe: false,
                parent_id: Some(universe.id.clone()),
                child_count: paper_ids.len() as i32,
                emoji: Some("📚".to_string()),
                summary: None,
                ai_title: None,
                tags: None,
                is_processed: true,
                is_pinned: false,
                is_private: None,
                last_accessed_at: None,
                conversation_id: None,
                sequence_index: None,
                latest_child_date: None,
                privacy_reason: None,
                source: None,
                pdf_available: None,
                content_type: None,
                associated_idea_id: None,
                privacy: None,
            };

            db.insert_node(&node).map_err(|e| e.to_string())?;
            clusters_created += 1;
            println!("[Clustering FOS] '{}': created node with {} papers", fos_name, paper_ids.len());
            node
        };

        // Assign all papers in this FOS as children of the FOS node
        for paper_id in paper_ids {
            if let Err(e) = db.update_node_parent(paper_id, &fos_node.id) {
                eprintln!("[Clustering FOS] Failed to assign {}: {}", paper_id, e);
            } else {
                items_assigned += 1;
            }
        }
    }

    println!("[Clustering FOS] Complete: {} papers → {} FOS categories", items_assigned, clusters_created);

    Ok(ClusteringResult {
        items_processed: total_papers,
        clusters_created,
        items_assigned,
        edges_created: 0,
        method: "fos".to_string(),
    })
}

/// Force re-clustering of all items (clears existing assignments)
pub async fn recluster_all(db: &Database, use_ai: bool) -> Result<ClusteringResult, String> {
    // Mark all items as needing clustering
    let count = db.mark_all_items_need_clustering().map_err(|e| e.to_string())?;
    println!("Marked {} items for re-clustering", count);

    // Run clustering
    run_clustering(db, use_ai).await
}

// ==================== Cluster Naming (Standalone) ====================

/// Result of cluster naming operation
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamingResult {
    pub clusters_named: usize,
    pub clusters_skipped: usize,
}

/// Name clusters that don't have AI-generated names yet
/// Finds clusters with keyword-only names (contain commas) or generic names
/// and runs AI naming on them
pub async fn name_unnamed_clusters(db: &Database) -> Result<NamingResult, String> {
    use std::time::Instant;

    let start = Instant::now();

    // Get all unique cluster_ids with their labels
    let clusters = db.get_clusters_needing_names().map_err(|e| e.to_string())?;

    if clusters.is_empty() {
        println!("[Naming] No clusters need naming");
        return Ok(NamingResult {
            clusters_named: 0,
            clusters_skipped: 0,
        });
    }

    println!("[Naming] Found {} clusters needing AI names", clusters.len());

    // Build cluster info for AI naming: (cluster_id, sample_titles)
    let mut clusters_info: Vec<(i32, Vec<String>)> = Vec::new();

    for (cluster_id, _current_label) in &clusters {
        // Get sample items from this cluster
        let sample_items = db.get_cluster_sample_items(*cluster_id, 10)
            .map_err(|e| e.to_string())?;

        let titles: Vec<String> = sample_items
            .iter()
            .map(|node| node.ai_title.clone().unwrap_or_else(|| node.title.clone()))
            .collect();

        if !titles.is_empty() {
            clusters_info.push((*cluster_id, titles));
        }
    }

    if clusters_info.is_empty() {
        return Ok(NamingResult {
            clusters_named: 0,
            clusters_skipped: clusters.len(),
        });
    }

    // Batch AI naming (same logic as name_clusters_with_ai)
    let num_batches = (clusters_info.len() + NAMING_BATCH_SIZE - 1) / NAMING_BATCH_SIZE;
    println!("[Naming] Naming {} clusters in {} batches", clusters_info.len(), num_batches);

    let mut named_count = 0;

    for (batch_idx, batch) in clusters_info.chunks(NAMING_BATCH_SIZE).enumerate() {
        println!("[Naming] Batch {}/{} ({} clusters)", batch_idx + 1, num_batches, batch.len());

        match ai_client::name_clusters(batch).await {
            Ok(ai_names) => {
                for (cluster_id, name) in ai_names {
                    // Update all items in this cluster with the new label
                    if let Err(e) = db.update_cluster_label(cluster_id, &name) {
                        eprintln!("[Naming] Failed to update cluster {}: {}", cluster_id, e);
                    } else {
                        named_count += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("[Naming] AI naming failed for batch {}: {}", batch_idx + 1, e);
            }
        }
    }

    println!("[Naming] Complete: {} clusters named in {:.1}s",
             named_count, start.elapsed().as_secs_f64());

    Ok(NamingResult {
        clusters_named: named_count,
        clusters_skipped: clusters.len() - named_count,
    })
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
