//! Semantic similarity calculations for embeddings
//!
//! Provides cosine similarity and k-nearest-neighbor search for node embeddings.

/// Cosine similarity between two embedding vectors
/// Returns a value between -1.0 and 1.0 (1.0 = identical, 0.0 = orthogonal)
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Compute the centroid (average) of multiple embeddings
/// Returns a normalized centroid vector
pub fn compute_centroid(embeddings: &[&[f32]]) -> Option<Vec<f32>> {
    if embeddings.is_empty() {
        return None;
    }

    let dim = embeddings[0].len();
    if dim == 0 {
        return None;
    }

    // Sum all embeddings
    let mut centroid = vec![0.0f32; dim];
    for emb in embeddings {
        if emb.len() != dim {
            continue; // Skip mismatched dimensions
        }
        for (i, &val) in emb.iter().enumerate() {
            centroid[i] += val;
        }
    }

    // Average
    let n = embeddings.len() as f32;
    for val in &mut centroid {
        *val /= n;
    }

    // Normalize (L2 norm)
    let norm: f32 = centroid.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-10 {
        for val in &mut centroid {
            *val /= norm;
        }
    }

    Some(centroid)
}

/// Find the top N most similar nodes to a target embedding
/// Returns (node_id, similarity_score) pairs, sorted by similarity descending
pub fn find_similar(
    target_embedding: &[f32],
    all_embeddings: &[(String, Vec<f32>)],
    exclude_id: &str,
    top_n: usize,
    min_similarity: f32,
) -> Vec<(String, f32)> {
    let mut similarities: Vec<(String, f32)> = all_embeddings
        .iter()
        .filter(|(id, _)| id != exclude_id)
        .map(|(id, emb)| {
            let sim = cosine_similarity(target_embedding, emb);
            (id.clone(), sim)
        })
        .filter(|(_, sim)| *sim >= min_similarity)
        .collect();

    // Sort by similarity descending
    similarities.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Take top N
    similarities.truncate(top_n);
    similarities
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 0.0001);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_find_similar() {
        let target = vec![1.0, 0.0, 0.0];
        let embeddings = vec![
            ("a".to_string(), vec![1.0, 0.0, 0.0]),    // identical
            ("b".to_string(), vec![0.9, 0.1, 0.0]),    // very similar
            ("c".to_string(), vec![0.0, 1.0, 0.0]),    // orthogonal
            ("d".to_string(), vec![-1.0, 0.0, 0.0]),   // opposite
        ];

        let similar = find_similar(&target, &embeddings, "a", 2, 0.0);
        assert_eq!(similar.len(), 2);
        assert_eq!(similar[0].0, "b"); // Most similar after excluding "a"
    }

    #[test]
    fn test_find_similar_empty() {
        let target = vec![1.0, 0.0, 0.0];
        let embeddings: Vec<(String, Vec<f32>)> = vec![];
        let similar = find_similar(&target, &embeddings, "x", 5, 0.0);
        assert!(similar.is_empty());
    }

    // Tests for the 0.15 lesson similarity threshold introduced in generate_task_file()

    /// Lessons with similarity below 0.15 should be excluded entirely.
    #[test]
    fn test_find_similar_below_threshold_excluded() {
        let target = vec![1.0, 0.0, 0.0];
        // sim(target, a) ≈ 0.10 — below 0.15 threshold
        let a = vec![0.10_f32, 0.99498_f32, 0.0];
        // sim(target, b) ≈ 0.05 — below 0.15 threshold
        let b = vec![0.05_f32, 0.99875_f32, 0.0];
        let embeddings = vec![
            ("a".to_string(), a),
            ("b".to_string(), b),
        ];
        let similar = find_similar(&target, &embeddings, "x", 5, 0.15);
        assert!(similar.is_empty(), "Expected no results below 0.15, got {:?}", similar);
    }

    /// Lesson at exactly the 0.15 boundary should be included.
    #[test]
    fn test_find_similar_at_threshold_boundary_included() {
        let target = vec![1.0, 0.0, 0.0];
        // Construct a vector with cosine similarity exactly 0.15 to [1,0,0]:
        // [0.15, sqrt(1 - 0.15^2), 0] is already unit-length
        let at_threshold = vec![0.15_f32, (1.0_f32 - 0.15_f32 * 0.15_f32).sqrt(), 0.0];
        let embeddings = vec![("a".to_string(), at_threshold)];
        let similar = find_similar(&target, &embeddings, "x", 5, 0.15);
        assert_eq!(similar.len(), 1, "Lesson at exactly 0.15 should be included");
        assert!((similar[0].1 - 0.15).abs() < 0.001, "Similarity should be ~0.15");
    }

    /// Mix of lessons: above threshold kept, below discarded.
    #[test]
    fn test_find_similar_mixed_threshold_filters_low() {
        let target = vec![1.0, 0.0, 0.0];
        // sim ≈ 0.20 — above threshold
        let high = vec![0.20_f32, (1.0_f32 - 0.04_f32).sqrt(), 0.0];
        // sim ≈ 0.05 — below threshold
        let low = vec![0.05_f32, (1.0_f32 - 0.0025_f32).sqrt(), 0.0];
        let embeddings = vec![
            ("high".to_string(), high),
            ("low".to_string(), low),
        ];
        let similar = find_similar(&target, &embeddings, "x", 5, 0.15);
        assert_eq!(similar.len(), 1);
        assert_eq!(similar[0].0, "high");
    }

    /// When all lessons pass the threshold, top_n cap still applies.
    #[test]
    fn test_find_similar_lesson_threshold_respects_top_n() {
        let target = vec![1.0, 0.0, 0.0];
        // All embeddings have similarity > 0.15 (identical = 1.0)
        let embeddings: Vec<(String, Vec<f32>)> = (0..10)
            .map(|i| (format!("lesson-{}", i), vec![1.0, 0.0, 0.0]))
            .collect();
        let similar = find_similar(&target, &embeddings, "x", 5, 0.15);
        assert_eq!(similar.len(), 5, "top_n cap should still apply");
    }
}
