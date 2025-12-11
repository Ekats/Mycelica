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
}
