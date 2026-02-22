package graph

import "sort"

// ArticulationPoint is a node whose removal disconnects the graph
type ArticulationPoint struct {
	ID                  string `json:"id"`
	Title               string `json:"title"`
	ComponentsIfRemoved int    `json:"components_if_removed"`
}

// BridgeEdge is an edge whose removal disconnects the graph
type BridgeEdge struct {
	SourceID    string `json:"source_id"`
	TargetID    string `json:"target_id"`
	SourceTitle string `json:"source_title"`
	TargetTitle string `json:"target_title"`
}

// FragileConnection represents two regions with very few cross-edges
type FragileConnection struct {
	RegionA    string `json:"region_a"`
	RegionB    string `json:"region_b"`
	CrossEdges int    `json:"cross_edges"`
}

// BridgeReport contains bridge analysis results
type BridgeReport struct {
	ArticulationPoints []ArticulationPoint `json:"articulation_points"`
	BridgeEdges        []BridgeEdge        `json:"bridge_edges"`
	FragileConnections []FragileConnection  `json:"fragile_connections"`
	APCount            int                 `json:"ap_count"`
	BridgeCount        int                 `json:"bridge_count"`
}

// ComputeBridges finds articulation points, bridge edges, and fragile inter-region connections
func ComputeBridges(snap *GraphSnapshot) *BridgeReport {
	if len(snap.Nodes) == 0 {
		return &BridgeReport{}
	}

	// Map node IDs to indices
	nodeIDs := snap.NodeIDs()
	idToIdx := make(map[string]int, len(nodeIDs))
	for i, id := range nodeIDs {
		idToIdx[id] = i
	}
	n := len(nodeIDs)

	// Build deduplicated undirected adjacency (as indices)
	adjIdx := make([][]int, n)
	type edgePair struct{ u, v int }
	seen := make(map[edgePair]bool)

	for _, e := range snap.Edges {
		u, okU := idToIdx[e.Source]
		v, okV := idToIdx[e.Target]
		if !okU || !okV || u == v {
			continue
		}
		key := edgePair{u, v}
		if u > v {
			key = edgePair{v, u}
		}
		if !seen[key] {
			seen[key] = true
			adjIdx[u] = append(adjIdx[u], v)
			adjIdx[v] = append(adjIdx[v], u)
		}
	}

	disc := make([]int, n)
	low := make([]int, n)
	visited := make([]bool, n)
	isAP := make([]bool, n)
	var bridgePairs [][2]int
	counter := 1

	const noParent = -1

	// Iterative Tarjan for each connected component
	type frame struct {
		node, parent, ni int
	}

	for start := 0; start < n; start++ {
		if visited[start] {
			continue
		}

		visited[start] = true
		disc[start] = counter
		low[start] = counter
		counter++

		stack := []frame{{start, noParent, 0}}
		rootChildren := 0

		for len(stack) > 0 {
			top := &stack[len(stack)-1]
			node := top.node
			parent := top.parent

			if top.ni < len(adjIdx[node]) {
				child := adjIdx[node][top.ni]
				top.ni++

				if child == parent {
					continue
				}

				if visited[child] {
					// Back edge
					if disc[child] < low[node] {
						low[node] = disc[child]
					}
				} else {
					// Tree edge
					visited[child] = true
					disc[child] = counter
					low[child] = counter
					counter++

					if node == start {
						rootChildren++
					}

					stack = append(stack, frame{child, node, 0})
				}
			} else {
				// Done with this node, pop and propagate
				stack = stack[:len(stack)-1]

				if len(stack) > 0 {
					parentFrame := &stack[len(stack)-1]
					pn := parentFrame.node

					if low[node] < low[pn] {
						low[pn] = low[node]
					}

					// Bridge check
					if low[node] > disc[pn] {
						bridgePairs = append(bridgePairs, [2]int{pn, node})
					}

					// AP check (non-root)
					if pn != start && low[node] >= disc[pn] {
						isAP[pn] = true
					}
				}
			}
		}

		// Root is AP if 2+ tree children
		if rootChildren >= 2 {
			isAP[start] = true
		}
	}

	// Convert results
	var aps []ArticulationPoint
	for i := 0; i < n; i++ {
		if isAP[i] {
			id := nodeIDs[i]
			aps = append(aps, ArticulationPoint{
				ID:                  id,
				Title:               snap.Nodes[id].Title,
				ComponentsIfRemoved: len(adjIdx[i]),
			})
		}
	}

	var bridges []BridgeEdge
	for _, pair := range bridgePairs {
		uid := nodeIDs[pair[0]]
		vid := nodeIDs[pair[1]]
		bridges = append(bridges, BridgeEdge{
			SourceID:    uid,
			TargetID:    vid,
			SourceTitle: snap.Nodes[uid].Title,
			TargetTitle: snap.Nodes[vid].Title,
		})
	}

	// Fragile connections: cross-region edge counts
	type regionPair struct{ a, b string }
	pairCounts := make(map[regionPair]int)
	for _, e := range snap.Edges {
		ra := snap.Regions[e.Source]
		rb := snap.Regions[e.Target]
		if ra == "" {
			ra = "unassigned"
		}
		if rb == "" {
			rb = "unassigned"
		}
		if ra == rb {
			continue
		}
		key := regionPair{ra, rb}
		if ra > rb {
			key = regionPair{rb, ra}
		}
		pairCounts[key]++
	}

	var fragile []FragileConnection
	for pair, count := range pairCounts {
		if count <= 2 {
			fragile = append(fragile, FragileConnection{
				RegionA:    pair.a,
				RegionB:    pair.b,
				CrossEdges: count,
			})
		}
	}
	sort.Slice(fragile, func(i, j int) bool { return fragile[i].CrossEdges < fragile[j].CrossEdges })

	return &BridgeReport{
		ArticulationPoints: aps,
		BridgeEdges:        bridges,
		FragileConnections: fragile,
		APCount:            len(aps),
		BridgeCount:        len(bridges),
	}
}
