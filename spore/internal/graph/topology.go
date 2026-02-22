package graph

import "sort"

// HubNode is a node with high connectivity
type HubNode struct {
	ID        string `json:"id"`
	Title     string `json:"title"`
	Degree    int    `json:"degree"`
	InDegree  int    `json:"in_degree"`
	OutDegree int    `json:"out_degree"`
}

// DegreeBucket is one bucket in the degree histogram
type DegreeBucket struct {
	Label string `json:"label"`
	Count int    `json:"count"`
}

// TopologyReport contains topology analysis results
type TopologyReport struct {
	TotalNodes        int            `json:"total_nodes"`
	TotalEdges        int            `json:"total_edges"`
	NumComponents     int            `json:"num_components"`
	LargestComponent  int            `json:"largest_component"`
	SmallestComponent int            `json:"smallest_component"`
	OrphanCount       int            `json:"orphan_count"`
	OrphanIDs         []string       `json:"orphan_ids"`
	DegreeHistogram   []DegreeBucket `json:"degree_histogram"`
	Hubs              []HubNode      `json:"hubs"`
}

// ComputeTopology analyzes graph topology: components, orphans, degree distribution, hubs
func ComputeTopology(snap *GraphSnapshot, hubThreshold, topN int) *TopologyReport {
	totalNodes := len(snap.Nodes)
	totalEdges := len(snap.Edges)

	if totalNodes == 0 {
		return &TopologyReport{
			DegreeHistogram: defaultHistogram(),
		}
	}

	// Connected components via UnionFind
	nodeIDs := snap.NodeIDs()
	uf := NewUnionFind(nodeIDs)
	for _, e := range snap.Edges {
		if _, ok := snap.Nodes[e.Source]; !ok {
			continue
		}
		if _, ok := snap.Nodes[e.Target]; !ok {
			continue
		}
		uf.Union(e.Source, e.Target)
	}

	components := uf.Components()
	numComponents := len(components)
	largest, smallest := 0, totalNodes
	for _, c := range components {
		if len(c) > largest {
			largest = len(c)
		}
		if len(c) < smallest {
			smallest = len(c)
		}
	}

	// Orphans: degree == 0
	var orphans []string
	for _, id := range nodeIDs {
		if len(snap.Adj[id]) == 0 {
			orphans = append(orphans, id)
		}
	}
	orphanCount := len(orphans)
	sort.Strings(orphans)
	if len(orphans) > topN {
		orphans = orphans[:topN]
	}

	// Degree histogram (log-scale buckets)
	buckets := [7]int{}
	for _, id := range nodeIDs {
		degree := len(snap.Adj[id])
		buckets[degreeBucket(degree)]++
	}
	histogram := defaultHistogram()
	for i := range histogram {
		histogram[i].Count = buckets[i]
	}

	// Hubs: degree > threshold
	var hubs []HubNode
	for _, id := range nodeIDs {
		degree := len(snap.Adj[id])
		if degree > hubThreshold {
			hubs = append(hubs, HubNode{
				ID:        id,
				Title:     snap.Nodes[id].Title,
				Degree:    degree,
				InDegree:  len(snap.InAdj[id]),
				OutDegree: len(snap.OutAdj[id]),
			})
		}
	}
	sort.Slice(hubs, func(i, j int) bool { return hubs[i].Degree > hubs[j].Degree })
	if len(hubs) > topN {
		hubs = hubs[:topN]
	}

	return &TopologyReport{
		TotalNodes:        totalNodes,
		TotalEdges:        totalEdges,
		NumComponents:     numComponents,
		LargestComponent:  largest,
		SmallestComponent: smallest,
		OrphanCount:       orphanCount,
		OrphanIDs:         orphans,
		DegreeHistogram:   histogram,
		Hubs:              hubs,
	}
}

func defaultHistogram() []DegreeBucket {
	return []DegreeBucket{
		{Label: "0"}, {Label: "1"}, {Label: "2-3"},
		{Label: "4-7"}, {Label: "8-15"}, {Label: "16-31"}, {Label: "32+"},
	}
}

func degreeBucket(degree int) int {
	switch {
	case degree == 0:
		return 0
	case degree == 1:
		return 1
	case degree <= 3:
		return 2
	case degree <= 7:
		return 3
	case degree <= 15:
		return 4
	case degree <= 31:
		return 5
	default:
		return 6
	}
}
