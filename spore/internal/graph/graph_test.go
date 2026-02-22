package graph

import (
	"fmt"
	"testing"
	"time"
)

func nowMs() int64          { return time.Now().UnixMilli() }
func daysAgo(d int64) int64 { return nowMs() - d*86_400_000 }

func strPtr(s string) *string { return &s }

func makeTestSnapshot(
	nodes []struct {
		id        string
		createdAt int64
		updatedAt int64
		parentID  *string
		depth     int
	},
	edges []struct {
		source, target, edgeType string
		createdAt                int64
	},
) *GraphSnapshot {
	var nodeInfos []*NodeInfo
	for _, n := range nodes {
		nodeInfos = append(nodeInfos, &NodeInfo{
			ID:        n.id,
			Title:     "Node " + n.id,
			NodeType:  "page",
			CreatedAt: n.createdAt,
			UpdatedAt: n.updatedAt,
			ParentID:  n.parentID,
			Depth:     n.depth,
			IsItem:    n.depth > 1,
		})
	}
	var edgeInfos []EdgeInfo
	for i, e := range edges {
		edgeInfos = append(edgeInfos, EdgeInfo{
			ID:        fmt.Sprintf("e%d", i),
			Source:    e.source,
			Target:   e.target,
			EdgeType: e.edgeType,
			CreatedAt: e.createdAt,
		})
	}
	return NewSnapshot(nodeInfos, edgeInfos)
}

// Use a simpler helper for most tests
func quickSnapshot(nodeIDs []string, edges [][2]string) *GraphSnapshot {
	now := nowMs()
	var nodes []*NodeInfo
	for _, id := range nodeIDs {
		nodes = append(nodes, &NodeInfo{
			ID: id, Title: "Node " + id, NodeType: "page",
			CreatedAt: now, UpdatedAt: now, Depth: 0,
		})
	}
	var edgeInfos []EdgeInfo
	for i, e := range edges {
		edgeInfos = append(edgeInfos, EdgeInfo{
			ID: fmt.Sprintf("e%d", i), Source: e[0], Target: e[1],
			EdgeType: "related", CreatedAt: now,
		})
	}
	return NewSnapshot(nodes, edgeInfos)
}

// --- Topology Tests ---

func TestTopology_EmptyGraph(t *testing.T) {
	snap := NewSnapshot(nil, nil)
	r := ComputeTopology(snap, 4, 10)
	if r.TotalNodes != 0 || r.TotalEdges != 0 || r.NumComponents != 0 {
		t.Errorf("empty graph should have all zeros, got nodes=%d edges=%d components=%d",
			r.TotalNodes, r.TotalEdges, r.NumComponents)
	}
}

func TestTopology_SingleComponent(t *testing.T) {
	snap := quickSnapshot(
		[]string{"A", "B", "C", "D", "E"},
		[][2]string{{"A", "B"}, {"B", "C"}, {"C", "D"}, {"D", "E"}},
	)
	r := ComputeTopology(snap, 4, 10)
	if r.NumComponents != 1 {
		t.Errorf("expected 1 component, got %d", r.NumComponents)
	}
	if r.LargestComponent != 5 {
		t.Errorf("expected largest=5, got %d", r.LargestComponent)
	}
	if r.OrphanCount != 0 {
		t.Errorf("expected 0 orphans, got %d", r.OrphanCount)
	}
}

func TestTopology_TwoComponents(t *testing.T) {
	snap := quickSnapshot(
		[]string{"A", "B", "C", "D", "E"},
		[][2]string{{"A", "B"}, {"B", "C"}, {"D", "E"}},
	)
	r := ComputeTopology(snap, 4, 10)
	if r.NumComponents != 2 {
		t.Errorf("expected 2 components, got %d", r.NumComponents)
	}
	if r.LargestComponent != 3 {
		t.Errorf("expected largest=3, got %d", r.LargestComponent)
	}
	if r.SmallestComponent != 2 {
		t.Errorf("expected smallest=2, got %d", r.SmallestComponent)
	}
}

func TestOrphan_Detection(t *testing.T) {
	snap := quickSnapshot(
		[]string{"A", "B", "C"},
		[][2]string{{"A", "B"}},
	)
	r := ComputeTopology(snap, 4, 10)
	if r.OrphanCount != 1 {
		t.Errorf("expected 1 orphan, got %d", r.OrphanCount)
	}
	found := false
	for _, id := range r.OrphanIDs {
		if id == "C" {
			found = true
		}
	}
	if !found {
		t.Errorf("C should be an orphan, got %v", r.OrphanIDs)
	}
}

func TestHub_Detection(t *testing.T) {
	snap := quickSnapshot(
		[]string{"center", "s1", "s2", "s3", "s4", "s5"},
		[][2]string{{"center", "s1"}, {"center", "s2"}, {"center", "s3"}, {"center", "s4"}, {"center", "s5"}},
	)
	r := ComputeTopology(snap, 4, 10)
	if len(r.Hubs) != 1 {
		t.Fatalf("expected 1 hub, got %d", len(r.Hubs))
	}
	if r.Hubs[0].ID != "center" {
		t.Errorf("expected center as hub, got %s", r.Hubs[0].ID)
	}
	if r.Hubs[0].Degree <= 4 {
		t.Errorf("center degree should be > 4, got %d", r.Hubs[0].Degree)
	}
}

// --- Tarjan Tests ---

func TestTarjan_Bridge(t *testing.T) {
	snap := quickSnapshot(
		[]string{"A", "B", "C"},
		[][2]string{{"A", "B"}, {"B", "C"}},
	)
	r := ComputeBridges(snap)
	if r.BridgeCount != 2 {
		t.Errorf("expected 2 bridges, got %d", r.BridgeCount)
	}
	if r.APCount < 1 {
		t.Errorf("expected at least 1 AP, got %d", r.APCount)
	}
	foundB := false
	for _, ap := range r.ArticulationPoints {
		if ap.ID == "B" {
			foundB = true
		}
	}
	if !foundB {
		t.Errorf("B should be AP")
	}
}

func TestTarjan_CycleNoBridges(t *testing.T) {
	snap := quickSnapshot(
		[]string{"A", "B", "C"},
		[][2]string{{"A", "B"}, {"B", "C"}, {"C", "A"}},
	)
	r := ComputeBridges(snap)
	if r.BridgeCount != 0 {
		t.Errorf("triangle should have 0 bridges, got %d", r.BridgeCount)
	}
	if r.APCount != 0 {
		t.Errorf("triangle should have 0 APs, got %d", r.APCount)
	}
}

func TestTarjan_TwoCyclesJoined(t *testing.T) {
	snap := quickSnapshot(
		[]string{"A", "B", "C", "D", "E", "F"},
		[][2]string{
			{"A", "B"}, {"B", "C"}, {"C", "A"}, // triangle 1
			{"D", "E"}, {"E", "F"}, {"F", "D"}, // triangle 2
			{"C", "D"}, // bridge
		},
	)
	r := ComputeBridges(snap)
	if r.BridgeCount != 1 {
		t.Errorf("expected 1 bridge (C-D), got %d", r.BridgeCount)
	}
	if r.APCount < 2 {
		t.Errorf("expected at least 2 APs (C and D), got %d", r.APCount)
	}
	apIDs := make(map[string]bool)
	for _, ap := range r.ArticulationPoints {
		apIDs[ap.ID] = true
	}
	if !apIDs["C"] || !apIDs["D"] {
		t.Errorf("C and D should be APs, got %v", apIDs)
	}
}

// --- Staleness Tests ---

func TestStaleness_Detected(t *testing.T) {
	now := nowMs()
	snap := makeTestSnapshot(
		[]struct {
			id        string
			createdAt int64
			updatedAt int64
			parentID  *string
			depth     int
		}{
			{"A", daysAgo(100), daysAgo(90), nil, 0},
			{"B", now, now, nil, 0},
		},
		[]struct {
			source, target, edgeType string
			createdAt                int64
		}{
			{"B", "A", "reference", daysAgo(1)},
		},
	)
	r := ComputeStaleness(snap, 30)
	if r.StaleNodeCount != 1 {
		t.Fatalf("expected 1 stale node, got %d", r.StaleNodeCount)
	}
	if r.StaleNodes[0].ID != "A" {
		t.Errorf("expected A to be stale, got %s", r.StaleNodes[0].ID)
	}
	if r.StaleNodes[0].DaysSinceUpdate < 89 {
		t.Errorf("expected ~90 days, got %d", r.StaleNodes[0].DaysSinceUpdate)
	}
}

func TestStaleness_NoFalsePositive(t *testing.T) {
	snap := makeTestSnapshot(
		[]struct {
			id        string
			createdAt int64
			updatedAt int64
			parentID  *string
			depth     int
		}{
			{"A", daysAgo(100), daysAgo(90), nil, 0},
			{"B", daysAgo(100), daysAgo(60), nil, 0},
		},
		[]struct {
			source, target, edgeType string
			createdAt                int64
		}{
			{"B", "A", "reference", daysAgo(60)},
		},
	)
	r := ComputeStaleness(snap, 30)
	if r.StaleNodeCount != 0 {
		t.Errorf("old node with only old edges should not be stale, got %d", r.StaleNodeCount)
	}
}

func TestStale_Summary(t *testing.T) {
	snap := makeTestSnapshot(
		[]struct {
			id        string
			createdAt int64
			updatedAt int64
			parentID  *string
			depth     int
		}{
			{"S", daysAgo(10), daysAgo(10), nil, 0},
			{"T", daysAgo(20), daysAgo(2), nil, 0},
		},
		[]struct {
			source, target, edgeType string
			createdAt                int64
		}{
			{"S", "T", "summarizes", daysAgo(10)},
		},
	)
	r := ComputeStaleness(snap, 30)
	if r.StaleSummaryCount != 1 {
		t.Fatalf("expected 1 stale summary, got %d", r.StaleSummaryCount)
	}
	if r.StaleSummaries[0].SummaryNodeID != "S" {
		t.Errorf("expected S as summary, got %s", r.StaleSummaries[0].SummaryNodeID)
	}
	if r.StaleSummaries[0].DriftDays < 7 {
		t.Errorf("expected drift ~8 days, got %d", r.StaleSummaries[0].DriftDays)
	}
}

// --- Region Tests ---

func TestRegion_Computation(t *testing.T) {
	now := nowMs()
	snap := makeTestSnapshot(
		[]struct {
			id        string
			createdAt int64
			updatedAt int64
			parentID  *string
			depth     int
		}{
			{"root", now, now, nil, 0},
			{"cat", now, now, strPtr("root"), 1},
			{"item", now, now, strPtr("cat"), 2},
		},
		nil,
	)
	if snap.Regions["root"] != "root" {
		t.Errorf("root region should be root, got %s", snap.Regions["root"])
	}
	if snap.Regions["cat"] != "cat" {
		t.Errorf("cat region should be cat, got %s", snap.Regions["cat"])
	}
	if snap.Regions["item"] != "cat" {
		t.Errorf("item region should be cat, got %s", snap.Regions["item"])
	}
}

func TestFragile_Connections(t *testing.T) {
	now := nowMs()
	snap := makeTestSnapshot(
		[]struct {
			id        string
			createdAt int64
			updatedAt int64
			parentID  *string
			depth     int
		}{
			{"root", now, now, nil, 0},
			{"cat1", now, now, strPtr("root"), 1},
			{"cat2", now, now, strPtr("root"), 1},
			{"item1", now, now, strPtr("cat1"), 2},
			{"item2", now, now, strPtr("cat2"), 2},
		},
		[]struct {
			source, target, edgeType string
			createdAt                int64
		}{
			{"item1", "item2", "related", now},
		},
	)
	r := ComputeBridges(snap)
	if len(r.FragileConnections) == 0 {
		t.Error("should detect fragile connection")
	}
	if r.FragileConnections[0].CrossEdges != 1 {
		t.Errorf("expected 1 cross-edge, got %d", r.FragileConnections[0].CrossEdges)
	}
}

// --- Health Tests ---

func TestHealthScore_Range(t *testing.T) {
	// All orphans
	snap := quickSnapshot([]string{"A", "B", "C"}, nil)
	r := Analyze(snap, DefaultConfig())
	if r.HealthScore < 0 || r.HealthScore > 1 {
		t.Errorf("health out of range: %f", r.HealthScore)
	}

	// Connected
	snap2 := quickSnapshot([]string{"A", "B"}, [][2]string{{"A", "B"}})
	r2 := Analyze(snap2, DefaultConfig())
	if r2.HealthScore < 0 || r2.HealthScore > 1 {
		t.Errorf("health out of range: %f", r2.HealthScore)
	}
}

func TestHealthScore_Perfect(t *testing.T) {
	snap := quickSnapshot(
		[]string{"A", "B", "C"},
		[][2]string{{"A", "B"}, {"B", "C"}, {"C", "A"}},
	)
	r := Analyze(snap, &AnalyzerConfig{HubThreshold: 10, TopN: 50, StaleDays: 30})
	if r.HealthScore < 0.95 {
		t.Errorf("perfect graph should have health ~1.0, got %f", r.HealthScore)
	}
}
