package db

import (
	"container/heap"
	"math"
)

// ContextNode represents a node reached by Dijkstra traversal from a source.
type ContextNode struct {
	Rank      int       `json:"rank"`
	NodeID    string    `json:"nodeId"`
	NodeTitle string    `json:"nodeTitle"`
	Distance  float64   `json:"distance"`
	Relevance float64   `json:"relevance"`
	Hops      int       `json:"hops"`
	Path      []PathHop `json:"path"`
	NodeClass *string   `json:"nodeClass"`
	IsItem    bool      `json:"isItem"`
}

// PathHop represents one hop in a path from source to destination.
type PathHop struct {
	EdgeID    string `json:"edgeId"`
	EdgeType  string `json:"edgeType"`
	NodeID    string `json:"nodeId"`
	NodeTitle string `json:"nodeTitle"`
}

// ContextConfig holds parameters for the Dijkstra context expansion.
type ContextConfig struct {
	Budget           int
	MaxHops          int
	MaxCost          float64
	EdgeTypes        []string // allowlist; nil means all
	ExcludeEdgeTypes []string // blocklist
	NotSuperseded    bool
	ItemsOnly        bool
}

// DefaultContextConfig returns sensible defaults matching the CLI.
func DefaultContextConfig() *ContextConfig {
	return &ContextConfig{
		Budget:  20,
		MaxHops: 6,
		MaxCost: 3.0,
	}
}

// prevEntry tracks how we reached a node (for path reconstruction).
type prevEntry struct {
	prevNodeID string
	edgeID     string
	edgeType   string
}

// dijkstraEntry is a min-heap entry.
type dijkstraEntry struct {
	distance float64
	nodeID   string
	hops     int
}

// dijkstraHeap implements container/heap.Interface as a min-heap.
// Ties broken by nodeID (lexicographic) for deterministic output.
type dijkstraHeap []dijkstraEntry

func (h dijkstraHeap) Len() int { return len(h) }
func (h dijkstraHeap) Less(i, j int) bool {
	if h[i].distance != h[j].distance {
		return h[i].distance < h[j].distance
	}
	return h[i].nodeID < h[j].nodeID
}
func (h dijkstraHeap) Swap(i, j int)       { h[i], h[j] = h[j], h[i] }
func (h *dijkstraHeap) Push(x interface{}) { *h = append(*h, x.(dijkstraEntry)) }
func (h *dijkstraHeap) Pop() interface{} {
	old := *h
	n := len(old)
	item := old[n-1]
	*h = old[:n-1]
	return item
}

// ContextForTask performs Dijkstra context expansion from a source node.
// Returns up to config.Budget context nodes sorted by distance (ascending).
// Port of schema.rs:5714-5867.
func (d *DB) ContextForTask(sourceID string, config *ContextConfig) ([]ContextNode, error) {
	if config == nil {
		config = DefaultContextConfig()
	}
	budget := config.Budget
	if budget <= 0 {
		budget = 20
	}
	maxHops := config.MaxHops
	if maxHops <= 0 {
		maxHops = 6
	}
	maxCost := config.MaxCost
	if maxCost <= 0 {
		maxCost = 3.0
	}

	// Build allow/exclude sets for fast lookup
	var allowSet map[string]bool
	if config.EdgeTypes != nil {
		allowSet = make(map[string]bool, len(config.EdgeTypes))
		for _, t := range config.EdgeTypes {
			allowSet[t] = true
		}
	}
	var excludeSet map[string]bool
	if config.ExcludeEdgeTypes != nil {
		excludeSet = make(map[string]bool, len(config.ExcludeEdgeTypes))
		for _, t := range config.ExcludeEdgeTypes {
			excludeSet[t] = true
		}
	}

	dist := map[string]float64{sourceID: 0.0}
	prev := map[string]prevEntry{}
	visited := map[string]bool{}

	h := &dijkstraHeap{{distance: 0.0, nodeID: sourceID, hops: 0}}
	heap.Init(h)

	var results []ContextNode

	for h.Len() > 0 {
		entry := heap.Pop(h).(dijkstraEntry)
		current := entry.nodeID
		currentDist := entry.distance
		currentHops := entry.hops

		if visited[current] {
			continue
		}
		visited[current] = true

		// Collect this node (skip source)
		if current != sourceID {
			node, err := d.GetNode(current)
			if err == nil && node != nil {
				// items_only: skip categories from results but traverse through them
				if !config.ItemsOnly || node.IsItem {
					path, _ := d.reconstructPath(prev, sourceID, current)
					title := node.Title
					if node.AITitle != nil {
						title = *node.AITitle
					}
					results = append(results, ContextNode{
						Rank:      0,
						NodeID:    current,
						NodeTitle: title,
						Distance:  currentDist,
						Relevance: 1.0 / (1.0 + currentDist),
						Hops:      currentHops,
						Path:      path,
						NodeClass: node.NodeClass,
						IsItem:    node.IsItem,
					})
					if len(results) >= budget {
						break
					}
				}
			}
		}

		// Stop expanding if max hops reached
		if currentHops >= maxHops {
			continue
		}

		// Expand neighbors
		edges, err := d.GetEdgesForNode(current)
		if err != nil {
			continue
		}

		for _, edge := range edges {
			// Superseded filter
			if config.NotSuperseded && edge.SupersededBy != nil {
				continue
			}

			// Edge type allowlist
			if allowSet != nil && !allowSet[edge.EdgeType] {
				continue
			}

			// Edge type blocklist
			if excludeSet != nil && excludeSet[edge.EdgeType] {
				continue
			}

			// Get neighbor (bidirectional traversal)
			neighbor := edge.TargetID
			if edge.SourceID != current {
				neighbor = edge.SourceID
			}

			if visited[neighbor] {
				continue
			}

			// Compute cost
			confidence := 0.5
			if edge.Confidence != nil {
				confidence = *edge.Confidence
			}
			typePriority := EdgeTypePriority(edge.EdgeType)
			baseCost := math.Max((1.0-confidence)*(1.0-0.5*typePriority), 0.001)

			// Structural edge penalty: high-confidence but low-information edges
			// (same file, hierarchy) get a cost floor so they don't flood the budget
			// before semantic edges.
			if IsStructuralEdge(edge.EdgeType) {
				baseCost = math.Max(baseCost, 0.4)
			}

			newDist := currentDist + baseCost

			// Cost ceiling
			if newDist > maxCost {
				continue
			}

			// Relax if better path found
			prevDist, exists := dist[neighbor]
			if !exists || newDist < prevDist {
				dist[neighbor] = newDist
				prev[neighbor] = prevEntry{
					prevNodeID: current,
					edgeID:     edge.ID,
					edgeType:   edge.EdgeType,
				}
				heap.Push(h, dijkstraEntry{
					distance: newDist,
					nodeID:   neighbor,
					hops:     currentHops + 1,
				})
			}
		}
	}

	// Assign ranks (1-indexed, in distance order)
	for i := range results {
		results[i].Rank = i + 1
	}

	return results, nil
}

// reconstructPath walks the prev map backwards from target to source
// and resolves node titles.
func (d *DB) reconstructPath(prev map[string]prevEntry, source, target string) ([]PathHop, error) {
	var path []PathHop
	current := target
	for current != source {
		entry, ok := prev[current]
		if !ok {
			break
		}
		title := current
		if len(title) > 8 {
			title = title[:8]
		}
		node, err := d.GetNode(current)
		if err == nil && node != nil {
			if node.AITitle != nil {
				title = *node.AITitle
			} else {
				title = node.Title
			}
		}
		path = append(path, PathHop{
			EdgeID:   entry.edgeID,
			EdgeType: entry.edgeType,
			NodeID:   current,
			NodeTitle: title,
		})
		current = entry.prevNodeID
	}
	// Reverse to get source-to-target order
	for i, j := 0, len(path)-1; i < j; i, j = i+1, j-1 {
		path[i], path[j] = path[j], path[i]
	}
	return path, nil
}
