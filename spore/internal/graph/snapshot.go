package graph

import "sort"

// NodeInfo is a lightweight node representation decoupled from DB types
type NodeInfo struct {
	ID        string
	Title     string
	NodeType  string
	CreatedAt int64
	UpdatedAt int64
	ParentID  *string
	Depth     int
	IsItem    bool
}

// EdgeInfo is a lightweight edge representation
type EdgeInfo struct {
	ID        string
	Source    string
	Target   string
	EdgeType string // lowercase
	CreatedAt int64
	UpdatedAt *int64
}

// GraphSnapshot holds a graph with precomputed adjacency lists and region map
type GraphSnapshot struct {
	Nodes   map[string]*NodeInfo
	Edges   []EdgeInfo
	Adj     map[string][]string // undirected
	OutAdj  map[string][]string // directed: source -> targets
	InAdj   map[string][]string // directed: target -> sources
	Regions map[string]string   // node_id -> depth-1 ancestor
}

// NewSnapshot builds a GraphSnapshot from raw nodes and edges
func NewSnapshot(nodes []*NodeInfo, edges []EdgeInfo) *GraphSnapshot {
	nodeMap := make(map[string]*NodeInfo, len(nodes))
	adj := make(map[string][]string)
	outAdj := make(map[string][]string)
	inAdj := make(map[string][]string)

	for _, n := range nodes {
		nodeMap[n.ID] = n
		adj[n.ID] = nil    // ensure entry exists
		outAdj[n.ID] = nil
		inAdj[n.ID] = nil
	}

	for _, e := range edges {
		if _, ok := nodeMap[e.Source]; !ok {
			continue
		}
		if _, ok := nodeMap[e.Target]; !ok {
			continue
		}
		adj[e.Source] = append(adj[e.Source], e.Target)
		adj[e.Target] = append(adj[e.Target], e.Source)
		outAdj[e.Source] = append(outAdj[e.Source], e.Target)
		inAdj[e.Target] = append(inAdj[e.Target], e.Source)
	}

	regions := computeRegions(nodeMap)

	return &GraphSnapshot{
		Nodes:   nodeMap,
		Edges:   edges,
		Adj:     adj,
		OutAdj:  outAdj,
		InAdj:   inAdj,
		Regions: regions,
	}
}

// FilterToRegion returns a new snapshot containing only descendants of regionNodeID
func (s *GraphSnapshot) FilterToRegion(regionNodeID string) *GraphSnapshot {
	included := make(map[string]bool)
	for id := range s.Nodes {
		isDescendantOf(id, regionNodeID, s.Nodes, included)
	}

	var filteredNodes []*NodeInfo
	filteredSet := make(map[string]bool)
	for id, isDesc := range included {
		if isDesc {
			filteredNodes = append(filteredNodes, s.Nodes[id])
			filteredSet[id] = true
		}
	}

	var filteredEdges []EdgeInfo
	for _, e := range s.Edges {
		if filteredSet[e.Source] && filteredSet[e.Target] {
			filteredEdges = append(filteredEdges, e)
		}
	}

	return NewSnapshot(filteredNodes, filteredEdges)
}

// NodeIDs returns a sorted list of all node IDs (for deterministic output)
func (s *GraphSnapshot) NodeIDs() []string {
	ids := make([]string, 0, len(s.Nodes))
	for id := range s.Nodes {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	return ids
}

func isDescendantOf(nodeID, ancestorID string, nodes map[string]*NodeInfo, cache map[string]bool) bool {
	if nodeID == ancestorID {
		cache[nodeID] = true
		return true
	}
	if cached, ok := cache[nodeID]; ok {
		return cached
	}
	node, ok := nodes[nodeID]
	if !ok || node.ParentID == nil {
		cache[nodeID] = false
		return false
	}
	result := isDescendantOf(*node.ParentID, ancestorID, nodes, cache)
	cache[nodeID] = result
	return result
}

func computeRegions(nodes map[string]*NodeInfo) map[string]string {
	regions := make(map[string]string, len(nodes))
	for id, node := range nodes {
		if node.Depth <= 1 {
			regions[id] = id
		} else {
			regions[id] = findDepth1Ancestor(id, nodes)
		}
	}
	return regions
}

func findDepth1Ancestor(nodeID string, nodes map[string]*NodeInfo) string {
	current := nodeID
	visited := make(map[string]bool)
	for {
		if visited[current] {
			return "unassigned" // cycle
		}
		visited[current] = true
		node, ok := nodes[current]
		if !ok {
			return "unassigned"
		}
		if node.Depth <= 1 {
			return current
		}
		if node.ParentID == nil {
			return "unassigned"
		}
		current = *node.ParentID
	}
}
