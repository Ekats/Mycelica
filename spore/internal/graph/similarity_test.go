package graph

import (
	"math"
	"testing"

	"mycelica/spore/internal/db"
)

func TestCosineSimilarity_Identical(t *testing.T) {
	a := []float32{1, 2, 3}
	b := []float32{1, 2, 3}
	sim := CosineSimilarity(a, b)
	if math.Abs(float64(sim)-1.0) > 0.0001 {
		t.Errorf("expected ~1.0, got %f", sim)
	}
}

func TestCosineSimilarity_Orthogonal(t *testing.T) {
	a := []float32{1, 0, 0}
	b := []float32{0, 1, 0}
	sim := CosineSimilarity(a, b)
	if math.Abs(float64(sim)) > 0.0001 {
		t.Errorf("expected ~0.0, got %f", sim)
	}
}

func TestCosineSimilarity_Opposite(t *testing.T) {
	a := []float32{1, 0}
	b := []float32{-1, 0}
	sim := CosineSimilarity(a, b)
	if math.Abs(float64(sim)+1.0) > 0.0001 {
		t.Errorf("expected ~-1.0, got %f", sim)
	}
}

func TestCosineSimilarity_ZeroNorm(t *testing.T) {
	a := []float32{0, 0, 0}
	b := []float32{1, 0, 0}
	if sim := CosineSimilarity(a, b); sim != 0.0 {
		t.Errorf("expected 0.0, got %f", sim)
	}
}

func TestCosineSimilarity_MismatchedLength(t *testing.T) {
	a := []float32{1, 0}
	b := []float32{1, 0, 0}
	if sim := CosineSimilarity(a, b); sim != 0.0 {
		t.Errorf("expected 0.0 for mismatched lengths, got %f", sim)
	}
}

func TestCosineSimilarity_Empty(t *testing.T) {
	if sim := CosineSimilarity(nil, nil); sim != 0.0 {
		t.Errorf("expected 0.0, got %f", sim)
	}
}

func TestFindSimilar_Basic(t *testing.T) {
	target := []float32{1, 0, 0}
	candidates := []db.NodeEmbedding{
		{ID: "a", Embedding: []float32{1, 0, 0}},
		{ID: "b", Embedding: []float32{0.9, 0.1, 0}},
		{ID: "c", Embedding: []float32{0, 1, 0}},
		{ID: "d", Embedding: []float32{-1, 0, 0}},
	}
	results := FindSimilar(target, candidates, "", 2, 0.0)
	if len(results) != 2 {
		t.Fatalf("expected 2 results, got %d", len(results))
	}
	if results[0].ID != "a" {
		t.Errorf("expected 'a' first, got '%s'", results[0].ID)
	}
	if results[1].ID != "b" {
		t.Errorf("expected 'b' second, got '%s'", results[1].ID)
	}
}

func TestFindSimilar_ExcludesSelf(t *testing.T) {
	target := []float32{1, 0, 0}
	candidates := []db.NodeEmbedding{
		{ID: "self", Embedding: []float32{1, 0, 0}},
		{ID: "other", Embedding: []float32{0.5, 0.5, 0}},
	}
	results := FindSimilar(target, candidates, "self", 5, 0.0)
	if len(results) != 1 {
		t.Fatalf("expected 1 result (self excluded), got %d", len(results))
	}
	if results[0].ID != "other" {
		t.Errorf("expected 'other', got '%s'", results[0].ID)
	}
}

func TestFindSimilar_MinThreshold(t *testing.T) {
	target := []float32{1, 0, 0}
	candidates := []db.NodeEmbedding{
		{ID: "similar", Embedding: []float32{0.9, 0.1, 0}},
		{ID: "orthogonal", Embedding: []float32{0, 1, 0}},
	}
	results := FindSimilar(target, candidates, "", 5, 0.5)
	if len(results) != 1 {
		t.Fatalf("expected 1 result above threshold, got %d", len(results))
	}
	if results[0].ID != "similar" {
		t.Errorf("expected 'similar', got '%s'", results[0].ID)
	}
}
