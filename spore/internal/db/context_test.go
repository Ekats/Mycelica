package db

import (
	"database/sql"
	"fmt"
	"math"
	"testing"

	_ "modernc.org/sqlite"
)

// setupTestDB creates an in-memory SQLite database with minimal schema for testing.
func setupTestDB(t *testing.T) *DB {
	t.Helper()
	conn, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}

	// Minimal schema matching the real database
	_, err = conn.Exec(`
		CREATE TABLE nodes (
			id TEXT PRIMARY KEY,
			type TEXT NOT NULL DEFAULT 'page',
			title TEXT NOT NULL,
			url TEXT,
			content TEXT,
			created_at INTEGER NOT NULL,
			updated_at INTEGER NOT NULL,
			depth INTEGER NOT NULL DEFAULT 0,
			is_item INTEGER NOT NULL DEFAULT 1,
			is_universe INTEGER NOT NULL DEFAULT 0,
			parent_id TEXT,
			child_count INTEGER NOT NULL DEFAULT 0,
			ai_title TEXT,
			summary TEXT,
			tags TEXT,
			emoji TEXT,
			is_processed INTEGER NOT NULL DEFAULT 0,
			agent_id TEXT,
			node_class TEXT,
			meta_type TEXT,
			content_type TEXT,
			source TEXT,
			author TEXT,
			embedding BLOB
		);
		CREATE TABLE edges (
			id TEXT PRIMARY KEY,
			source_id TEXT NOT NULL,
			target_id TEXT NOT NULL,
			type TEXT NOT NULL,
			label TEXT,
			weight REAL,
			confidence REAL,
			agent_id TEXT,
			reason TEXT,
			content TEXT,
			created_at INTEGER NOT NULL,
			updated_at INTEGER,
			superseded_by TEXT,
			metadata TEXT
		);
	`)
	if err != nil {
		t.Fatal(err)
	}

	return &DB{conn: conn, Path: ":memory:"}
}

func insertNode(t *testing.T, d *DB, id, title string, isItem bool) {
	t.Helper()
	item := 0
	if isItem {
		item = 1
	}
	_, err := d.conn.Exec(
		`INSERT INTO nodes (id, type, title, created_at, updated_at, is_item) VALUES (?, 'page', ?, 1000, 1000, ?)`,
		id, title, item,
	)
	if err != nil {
		t.Fatal(err)
	}
}

func insertEdge(t *testing.T, d *DB, id, source, target, edgeType string, confidence *float64) {
	t.Helper()
	_, err := d.conn.Exec(
		`INSERT INTO edges (id, source_id, target_id, type, created_at, confidence) VALUES (?, ?, ?, ?, 1000, ?)`,
		id, source, target, edgeType, confidence,
	)
	if err != nil {
		t.Fatal(err)
	}
}

func f64(v float64) *float64 { return &v }

func TestEdgeTypePriority(t *testing.T) {
	tests := []struct {
		edgeType string
		want     float64
	}{
		{"contradicts", 1.0},
		{"flags", 1.0},
		{"summarizes", 0.7},
		{"derives_from", 0.7},
		{"supports", 0.5},
		{"questions", 0.5},
		{"related", 0.3},
		{"calls", 0.3},
		{"reference", 0.3},
	}
	for _, tt := range tests {
		got := EdgeTypePriority(tt.edgeType)
		if got != tt.want {
			t.Errorf("EdgeTypePriority(%q) = %f, want %f", tt.edgeType, got, tt.want)
		}
	}
}

func TestDijkstra_SimpleChain(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	// A -> B -> C with known confidence
	insertNode(t, d, "A", "Node A", true)
	insertNode(t, d, "B", "Node B", true)
	insertNode(t, d, "C", "Node C", true)
	insertEdge(t, d, "e1", "A", "B", "supports", f64(0.8))
	insertEdge(t, d, "e2", "B", "C", "supports", f64(0.8))

	results, err := d.ContextForTask("A", DefaultContextConfig())
	if err != nil {
		t.Fatal(err)
	}

	if len(results) != 2 {
		t.Fatalf("expected 2 results, got %d", len(results))
	}

	// B should be first (closer)
	if results[0].NodeID != "B" {
		t.Errorf("expected B first, got %s", results[0].NodeID)
	}
	if results[1].NodeID != "C" {
		t.Errorf("expected C second, got %s", results[1].NodeID)
	}

	// Distance to B should be less than distance to C
	if results[0].Distance >= results[1].Distance {
		t.Errorf("B distance (%f) should be less than C (%f)", results[0].Distance, results[1].Distance)
	}

	// Ranks
	if results[0].Rank != 1 || results[1].Rank != 2 {
		t.Errorf("ranks should be 1,2, got %d,%d", results[0].Rank, results[1].Rank)
	}

	// Path reconstruction for C should have 2 hops
	if results[1].Hops != 2 {
		t.Errorf("C should be 2 hops, got %d", results[1].Hops)
	}
	if len(results[1].Path) != 2 {
		t.Errorf("C path should have 2 entries, got %d", len(results[1].Path))
	}
}

func TestDijkstra_BudgetCutoff(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	insertNode(t, d, "center", "Center", true)
	for i := 0; i < 10; i++ {
		id := fmt.Sprintf("s%d", i)
		insertNode(t, d, id, "Spoke "+id, true)
		insertEdge(t, d, fmt.Sprintf("e%d", i), "center", id, "related", f64(0.5))
	}

	config := &ContextConfig{Budget: 3, MaxHops: 6, MaxCost: 3.0}
	results, err := d.ContextForTask("center", config)
	if err != nil {
		t.Fatal(err)
	}
	if len(results) != 3 {
		t.Errorf("expected 3 results (budget), got %d", len(results))
	}
}

func TestDijkstra_MaxHopsCutoff(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	// Chain: A -> B -> C -> D -> E
	for _, id := range []string{"A", "B", "C", "D", "E"} {
		insertNode(t, d, id, "Node "+id, true)
	}
	insertEdge(t, d, "e1", "A", "B", "related", f64(0.9))
	insertEdge(t, d, "e2", "B", "C", "related", f64(0.9))
	insertEdge(t, d, "e3", "C", "D", "related", f64(0.9))
	insertEdge(t, d, "e4", "D", "E", "related", f64(0.9))

	config := &ContextConfig{Budget: 20, MaxHops: 2, MaxCost: 3.0}
	results, err := d.ContextForTask("A", config)
	if err != nil {
		t.Fatal(err)
	}
	if len(results) != 2 {
		t.Errorf("expected 2 results (maxHops=2), got %d", len(results))
	}
}

func TestDijkstra_MaxCostCutoff(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	insertNode(t, d, "A", "Node A", true)
	insertNode(t, d, "B", "Node B", true)
	insertNode(t, d, "C", "Node C", true)
	// Low confidence = high cost
	insertEdge(t, d, "e1", "A", "B", "related", f64(0.1))
	insertEdge(t, d, "e2", "B", "C", "related", f64(0.1))

	config := &ContextConfig{Budget: 20, MaxHops: 6, MaxCost: 1.0}
	results, err := d.ContextForTask("A", config)
	if err != nil {
		t.Fatal(err)
	}

	// cost per edge = (1-0.1)*(1-0.5*0.3) = 0.9*0.85 = 0.765
	// B at 0.765 (within 1.0), C at 1.53 (over 1.0)
	if len(results) != 1 {
		t.Errorf("expected 1 result (C over cost limit), got %d", len(results))
	}
	if len(results) > 0 && results[0].NodeID != "B" {
		t.Errorf("expected B, got %s", results[0].NodeID)
	}
}

func TestDijkstra_SupersededFilter(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	insertNode(t, d, "A", "Node A", true)
	insertNode(t, d, "B", "Node B", true)
	// Insert edge with superseded_by set
	_, err := d.conn.Exec(
		`INSERT INTO edges (id, source_id, target_id, type, created_at, confidence, superseded_by) VALUES (?, ?, ?, ?, 1000, ?, ?)`,
		"e1", "A", "B", "related", 0.8, "e2",
	)
	if err != nil {
		t.Fatal(err)
	}

	config := &ContextConfig{Budget: 20, MaxHops: 6, MaxCost: 3.0, NotSuperseded: true}
	results, err := d.ContextForTask("A", config)
	if err != nil {
		t.Fatal(err)
	}
	if len(results) != 0 {
		t.Errorf("expected 0 results (edge superseded), got %d", len(results))
	}
}

func TestDijkstra_ItemsOnlyFilter(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	insertNode(t, d, "A", "Node A", true)
	insertNode(t, d, "CAT", "Category", false) // is_item = false
	insertNode(t, d, "C", "Node C", true)
	insertEdge(t, d, "e1", "A", "CAT", "belongs_to", f64(0.9))
	insertEdge(t, d, "e2", "CAT", "C", "related", f64(0.9))

	config := &ContextConfig{Budget: 20, MaxHops: 6, MaxCost: 3.0, ItemsOnly: true}
	results, err := d.ContextForTask("A", config)
	if err != nil {
		t.Fatal(err)
	}

	// CAT should be traversed but not in results (is_item=false)
	// C should be reachable through CAT
	for _, r := range results {
		if r.NodeID == "CAT" {
			t.Error("Category should not appear in results with ItemsOnly")
		}
	}
	found := false
	for _, r := range results {
		if r.NodeID == "C" {
			found = true
		}
	}
	if !found {
		t.Error("C should be reachable through category")
	}
}

func TestDijkstra_DeterministicTieBreaking(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	// Star: center -> {X, Y, Z} all same edge type + confidence
	insertNode(t, d, "center", "Center", true)
	insertNode(t, d, "X", "Node X", true)
	insertNode(t, d, "Y", "Node Y", true)
	insertNode(t, d, "Z", "Node Z", true)
	insertEdge(t, d, "e1", "center", "X", "related", f64(0.5))
	insertEdge(t, d, "e2", "center", "Y", "related", f64(0.5))
	insertEdge(t, d, "e3", "center", "Z", "related", f64(0.5))

	// Run twice, should get same order
	for run := 0; run < 3; run++ {
		results, err := d.ContextForTask("center", DefaultContextConfig())
		if err != nil {
			t.Fatal(err)
		}
		if len(results) != 3 {
			t.Fatalf("run %d: expected 3 results, got %d", run, len(results))
		}
		// Equal distance -> alphabetical: X, Y, Z
		if results[0].NodeID != "X" || results[1].NodeID != "Y" || results[2].NodeID != "Z" {
			t.Errorf("run %d: expected X,Y,Z order, got %s,%s,%s",
				run, results[0].NodeID, results[1].NodeID, results[2].NodeID)
		}
	}
}

func TestDijkstra_StructuralPenalty(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	// A has two paths to B1 (via defined_in) and B2 (via supports)
	// Both have confidence=0.9
	insertNode(t, d, "A", "Node A", true)
	insertNode(t, d, "B1", "Structural", true)
	insertNode(t, d, "B2", "Semantic", true)
	insertEdge(t, d, "e1", "A", "B1", "defined_in", f64(0.9))
	insertEdge(t, d, "e2", "A", "B2", "supports", f64(0.9))

	results, err := d.ContextForTask("A", DefaultContextConfig())
	if err != nil {
		t.Fatal(err)
	}

	if len(results) < 2 {
		t.Fatalf("expected 2 results, got %d", len(results))
	}

	// B2 (supports) should be closer than B1 (defined_in)
	// supports: cost = (1-0.9)*(1-0.5*0.5) = 0.1*0.75 = 0.075
	// defined_in: cost = max((1-0.9)*(1-0.5*0.3), 0.4) = max(0.085, 0.4) = 0.4 (structural floor!)
	var b1Dist, b2Dist float64
	for _, r := range results {
		if r.NodeID == "B1" {
			b1Dist = r.Distance
		}
		if r.NodeID == "B2" {
			b2Dist = r.Distance
		}
	}

	if b2Dist >= b1Dist {
		t.Errorf("supports edge (%.4f) should be cheaper than defined_in (%.4f) due to structural penalty",
			b2Dist, b1Dist)
	}

	// Verify the actual values
	expectedB2 := 0.075 // (1-0.9)*(1-0.5*0.5) = 0.1*0.75
	expectedB1 := 0.4   // floored at 0.4
	if math.Abs(b2Dist-expectedB2) > 0.001 {
		t.Errorf("B2 distance = %.4f, expected ~%.4f", b2Dist, expectedB2)
	}
	if math.Abs(b1Dist-expectedB1) > 0.001 {
		t.Errorf("B1 distance = %.4f, expected ~%.4f", b1Dist, expectedB1)
	}
}

func TestDijkstra_BidirectionalTraversal(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	// Edge from A to B, start at B -> should reach A
	insertNode(t, d, "A", "Node A", true)
	insertNode(t, d, "B", "Node B", true)
	insertEdge(t, d, "e1", "A", "B", "related", f64(0.5))

	results, err := d.ContextForTask("B", DefaultContextConfig())
	if err != nil {
		t.Fatal(err)
	}
	if len(results) != 1 {
		t.Fatalf("expected 1 result, got %d", len(results))
	}
	if results[0].NodeID != "A" {
		t.Errorf("expected A, got %s", results[0].NodeID)
	}
}

func TestDijkstra_PathReconstruction(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	insertNode(t, d, "A", "Node A", true)
	insertNode(t, d, "B", "Node B", true)
	insertNode(t, d, "C", "Node C", true)
	insertEdge(t, d, "e1", "A", "B", "supports", f64(0.8))
	insertEdge(t, d, "e2", "B", "C", "summarizes", f64(0.7))

	results, err := d.ContextForTask("A", DefaultContextConfig())
	if err != nil {
		t.Fatal(err)
	}

	// Find C in results
	var cResult *ContextNode
	for i := range results {
		if results[i].NodeID == "C" {
			cResult = &results[i]
			break
		}
	}
	if cResult == nil {
		t.Fatal("C not found in results")
	}

	// Path should be: B -> C
	if len(cResult.Path) != 2 {
		t.Fatalf("expected 2 path hops, got %d", len(cResult.Path))
	}
	if cResult.Path[0].NodeID != "B" || cResult.Path[0].EdgeType != "supports" {
		t.Errorf("first hop should be B via supports, got %s via %s", cResult.Path[0].NodeID, cResult.Path[0].EdgeType)
	}
	if cResult.Path[1].NodeID != "C" || cResult.Path[1].EdgeType != "summarizes" {
		t.Errorf("second hop should be C via summarizes, got %s via %s", cResult.Path[1].NodeID, cResult.Path[1].EdgeType)
	}
}

func TestDijkstra_EdgeTypeAllowlist(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	insertNode(t, d, "A", "Node A", true)
	insertNode(t, d, "B", "Node B", true)
	insertNode(t, d, "C", "Node C", true)
	insertEdge(t, d, "e1", "A", "B", "supports", f64(0.8))
	insertEdge(t, d, "e2", "A", "C", "related", f64(0.8))

	// Only allow "supports" edges
	config := &ContextConfig{
		Budget:    20,
		MaxHops:   6,
		MaxCost:   3.0,
		EdgeTypes: []string{"supports"},
	}
	results, err := d.ContextForTask("A", config)
	if err != nil {
		t.Fatal(err)
	}
	if len(results) != 1 {
		t.Fatalf("expected 1 result (only supports), got %d", len(results))
	}
	if results[0].NodeID != "B" {
		t.Errorf("expected B, got %s", results[0].NodeID)
	}
}

func TestDijkstra_EdgeTypeExclude(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	insertNode(t, d, "A", "Node A", true)
	insertNode(t, d, "B", "Node B", true)
	insertNode(t, d, "C", "Node C", true)
	insertEdge(t, d, "e1", "A", "B", "supports", f64(0.8))
	insertEdge(t, d, "e2", "A", "C", "related", f64(0.8))

	// Exclude "supports" edges
	config := &ContextConfig{
		Budget:           20,
		MaxHops:          6,
		MaxCost:          3.0,
		ExcludeEdgeTypes: []string{"supports"},
	}
	results, err := d.ContextForTask("A", config)
	if err != nil {
		t.Fatal(err)
	}
	if len(results) != 1 {
		t.Fatalf("expected 1 result (supports excluded), got %d", len(results))
	}
	if results[0].NodeID != "C" {
		t.Errorf("expected C, got %s", results[0].NodeID)
	}
}

func TestDijkstra_EmptyGraph(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	insertNode(t, d, "A", "Node A", true)

	results, err := d.ContextForTask("A", DefaultContextConfig())
	if err != nil {
		t.Fatal(err)
	}
	if len(results) != 0 {
		t.Errorf("expected 0 results for isolated node, got %d", len(results))
	}
}

func TestDijkstra_RelevanceCalculation(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	insertNode(t, d, "A", "Node A", true)
	insertNode(t, d, "B", "Node B", true)
	insertEdge(t, d, "e1", "A", "B", "supports", f64(0.8))

	results, err := d.ContextForTask("A", DefaultContextConfig())
	if err != nil {
		t.Fatal(err)
	}
	if len(results) != 1 {
		t.Fatalf("expected 1 result, got %d", len(results))
	}

	// relevance = 1 / (1 + distance)
	expected := 1.0 / (1.0 + results[0].Distance)
	if math.Abs(results[0].Relevance-expected) > 0.0001 {
		t.Errorf("relevance = %f, expected %f", results[0].Relevance, expected)
	}
}

func TestDijkstra_NilConfig(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	insertNode(t, d, "A", "Node A", true)
	insertNode(t, d, "B", "Node B", true)
	insertEdge(t, d, "e1", "A", "B", "related", f64(0.5))

	// nil config should use defaults
	results, err := d.ContextForTask("A", nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(results) != 1 {
		t.Errorf("expected 1 result with nil config, got %d", len(results))
	}
}

func TestDijkstra_ShortestPathWins(t *testing.T) {
	d := setupTestDB(t)
	defer d.Close()

	// A -> B via two paths:
	//   direct: A --(low confidence)--> B  cost = high
	//   indirect: A --(high conf)--> M --(high conf)--> B  cost = low+low
	insertNode(t, d, "A", "Node A", true)
	insertNode(t, d, "B", "Node B", true)
	insertNode(t, d, "M", "Node M", true)
	insertEdge(t, d, "e_direct", "A", "B", "related", f64(0.1))   // cost = (0.9)*(0.85) = 0.765
	insertEdge(t, d, "e_am", "A", "M", "supports", f64(0.95))     // cost = (0.05)*(0.75) = 0.0375
	insertEdge(t, d, "e_mb", "M", "B", "supports", f64(0.95))     // cost = 0.0375

	results, err := d.ContextForTask("A", DefaultContextConfig())
	if err != nil {
		t.Fatal(err)
	}

	// Find B
	var bResult *ContextNode
	for i := range results {
		if results[i].NodeID == "B" {
			bResult = &results[i]
			break
		}
	}
	if bResult == nil {
		t.Fatal("B not found")
	}

	// B should be reached via M (shorter total distance: 0.075 vs 0.765)
	if bResult.Hops != 2 {
		t.Errorf("B should be reached in 2 hops via M, got %d hops", bResult.Hops)
	}
	if bResult.Distance > 0.1 {
		t.Errorf("B distance should be ~0.075, got %f", bResult.Distance)
	}
}
