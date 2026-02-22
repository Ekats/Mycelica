package graph

import (
	"math"
	"sort"

	"mycelica/spore/internal/db"
)

// SimilarNode is a node with its similarity score to a target embedding.
type SimilarNode struct {
	ID         string
	Title      string
	Similarity float32
}

// CosineSimilarity computes cosine similarity between two vectors.
// Returns 0.0 for zero-norm vectors or mismatched lengths.
func CosineSimilarity(a, b []float32) float32 {
	if len(a) != len(b) || len(a) == 0 {
		return 0.0
	}

	var dot, normA, normB float32
	for i := range a {
		dot += a[i] * b[i]
		normA += a[i] * a[i]
		normB += b[i] * b[i]
	}

	na := float32(math.Sqrt(float64(normA)))
	nb := float32(math.Sqrt(float64(normB)))

	if na == 0 || nb == 0 {
		return 0.0
	}

	return dot / (na * nb)
}

// FindSimilar finds the top-N most similar nodes to a target embedding.
// Excludes the node with excludeID. Only returns nodes with similarity >= minSimilarity.
// Results are sorted by descending similarity.
func FindSimilar(target []float32, candidates []db.NodeEmbedding, excludeID string, topN int, minSimilarity float32) []SimilarNode {
	var results []SimilarNode
	for _, c := range candidates {
		if c.ID == excludeID {
			continue
		}
		sim := CosineSimilarity(target, c.Embedding)
		if sim >= minSimilarity {
			results = append(results, SimilarNode{
				ID:         c.ID,
				Similarity: sim,
			})
		}
	}

	sort.Slice(results, func(i, j int) bool {
		return results[i].Similarity > results[j].Similarity
	})

	if len(results) > topN {
		results = results[:topN]
	}
	return results
}
