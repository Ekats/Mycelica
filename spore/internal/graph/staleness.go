package graph

import (
	"sort"
	"time"
)

// StaleNode is a node that's old but still being referenced
type StaleNode struct {
	ID              string `json:"id"`
	Title           string `json:"title"`
	DaysSinceUpdate int64  `json:"days_since_update"`
	RecentRefCount  int    `json:"recent_reference_count"`
}

// StaleSummary is a summary whose target has been updated after the summary
type StaleSummary struct {
	SummaryNodeID string `json:"summary_node_id"`
	SummaryTitle  string `json:"summary_title"`
	TargetNodeID  string `json:"target_node_id"`
	TargetTitle   string `json:"target_title"`
	DriftDays     int64  `json:"drift_days"`
}

// StalenessReport contains staleness analysis results
type StalenessReport struct {
	StaleNodes        []StaleNode    `json:"stale_nodes"`
	StaleSummaries    []StaleSummary `json:"stale_summaries"`
	StaleNodeCount    int            `json:"stale_node_count"`
	StaleSummaryCount int            `json:"stale_summary_count"`
}

// ComputeStaleness finds stale nodes and outdated summaries
func ComputeStaleness(snap *GraphSnapshot, staleDays int64) *StalenessReport {
	nowMs := time.Now().UnixMilli()
	staleThresholdMs := staleDays * 86_400_000
	recentWindowMs := int64(7 * 86_400_000)

	// Stale nodes: old but recently referenced
	var staleNodes []StaleNode
	for _, node := range snap.Nodes {
		ageMs := nowMs - node.UpdatedAt
		if ageMs <= staleThresholdMs {
			continue
		}

		// Count recent incoming edges
		recentCount := 0
		for _, e := range snap.Edges {
			if e.Target == node.ID && e.Source != node.ID {
				if (nowMs - e.CreatedAt) < recentWindowMs {
					recentCount++
				}
			}
		}

		if recentCount > 0 {
			staleNodes = append(staleNodes, StaleNode{
				ID:              node.ID,
				Title:           node.Title,
				DaysSinceUpdate: ageMs / 86_400_000,
				RecentRefCount:  recentCount,
			})
		}
	}
	sort.Slice(staleNodes, func(i, j int) bool {
		return staleNodes[i].RecentRefCount > staleNodes[j].RecentRefCount
	})

	// Stale summaries: "summarizes" edges where target updated after source
	var staleSummaries []StaleSummary
	for _, e := range snap.Edges {
		if e.EdgeType != "summarizes" {
			continue
		}
		sourceNode := snap.Nodes[e.Source]
		targetNode := snap.Nodes[e.Target]
		if sourceNode == nil || targetNode == nil {
			continue
		}
		if targetNode.UpdatedAt > sourceNode.UpdatedAt {
			driftMs := targetNode.UpdatedAt - sourceNode.UpdatedAt
			staleSummaries = append(staleSummaries, StaleSummary{
				SummaryNodeID: e.Source,
				SummaryTitle:  sourceNode.Title,
				TargetNodeID:  e.Target,
				TargetTitle:   targetNode.Title,
				DriftDays:     driftMs / 86_400_000,
			})
		}
	}
	sort.Slice(staleSummaries, func(i, j int) bool {
		return staleSummaries[i].DriftDays > staleSummaries[j].DriftDays
	})

	return &StalenessReport{
		StaleNodes:        staleNodes,
		StaleSummaries:    staleSummaries,
		StaleNodeCount:    len(staleNodes),
		StaleSummaryCount: len(staleSummaries),
	}
}
