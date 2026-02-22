package cmd

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"strings"

	"github.com/spf13/cobra"
	"mycelica/spore/internal/graph"
)

var (
	analyzeJSON         bool
	analyzeRegion       string
	analyzeTopN         int
	analyzeStaleDays    int64
	analyzeHubThreshold int
)

var analyzeCmd = &cobra.Command{
	Use:   "analyze",
	Short: "Analyze graph structure: topology, staleness, bridges, health score",
	RunE: func(cmd *cobra.Command, args []string) error {
		db, err := OpenDatabase()
		if err != nil {
			return err
		}
		defer db.Close()

		snap, err := graph.SnapshotFromDB(db)
		if err != nil {
			return fmt.Errorf("loading graph: %w", err)
		}

		if analyzeRegion != "" {
			snap = snap.FilterToRegion(analyzeRegion)
		}

		config := &graph.AnalyzerConfig{
			HubThreshold: analyzeHubThreshold,
			TopN:         analyzeTopN,
			StaleDays:    analyzeStaleDays,
		}

		report := graph.Analyze(snap, config)

		if analyzeJSON {
			enc := json.NewEncoder(os.Stdout)
			enc.SetIndent("", "  ")
			return enc.Encode(report)
		}

		printHumanReadable(report, snap)
		return nil
	},
}

func init() {
	analyzeCmd.Flags().BoolVar(&analyzeJSON, "json", false, "Output as JSON")
	analyzeCmd.Flags().StringVar(&analyzeRegion, "region", "", "Scope analysis to descendants of this node ID")
	analyzeCmd.Flags().IntVar(&analyzeTopN, "top-n", 10, "Number of top items to show per section")
	analyzeCmd.Flags().Int64Var(&analyzeStaleDays, "stale-days", 60, "Days since update to consider a node stale")
	analyzeCmd.Flags().IntVar(&analyzeHubThreshold, "hub-threshold", 15, "Minimum degree to consider a node a hub")
	rootCmd.AddCommand(analyzeCmd)
}

func printHumanReadable(report *graph.AnalysisReport, snap *graph.GraphSnapshot) {
	// Health bar
	barLen := int(report.HealthScore * 20)
	if barLen > 20 {
		barLen = 20
	}
	bar := strings.Repeat("█", barLen) + strings.Repeat("░", 20-barLen)
	fmt.Printf("\n  Graph Health: %.0f%%  [%s]\n", report.HealthScore*100, bar)
	fmt.Printf("  breakdown: connectivity=%.2f components=%.2f staleness=%.2f fragility=%.2f\n\n",
		report.HealthBreakdown.Connectivity,
		report.HealthBreakdown.Components,
		report.HealthBreakdown.Staleness,
		report.HealthBreakdown.Fragility)

	// Topology
	t := report.Topology
	fmt.Println("  TOPOLOGY")
	fmt.Println("  ────────────────────────────────────────")
	fmt.Printf("  Nodes: %d  Edges: %d  Components: %d\n", t.TotalNodes, t.TotalEdges, t.NumComponents)
	fmt.Printf("  Largest component: %d  Smallest: %d\n", t.LargestComponent, t.SmallestComponent)

	if t.OrphanCount > 0 {
		fmt.Printf("  Orphans: %d disconnected nodes\n", t.OrphanCount)
		limit := 5
		if len(t.OrphanIDs) < limit {
			limit = len(t.OrphanIDs)
		}
		for _, id := range t.OrphanIDs[:limit] {
			node := snap.Nodes[id]
			title := "?"
			if node != nil {
				title = truncTitle(node.Title, 50)
			}
			fmt.Printf("    - %s (%s)\n", truncID(id), title)
		}
		if t.OrphanCount > 5 {
			fmt.Printf("    ... and %d more\n", t.OrphanCount-5)
		}
	}

	// Degree distribution
	fmt.Println("\n  Degree distribution:")
	for _, b := range t.DegreeHistogram {
		if b.Count > 0 {
			barWidth := int(math.Log2(float64(b.Count))) + 2
			if barWidth < 1 {
				barWidth = 1
			}
			fmt.Printf("    %5s: %4d  %s\n", b.Label, b.Count, strings.Repeat("=", barWidth))
		}
	}

	// Hubs
	if len(t.Hubs) > 0 {
		fmt.Println("\n  Top hubs (degree > threshold):")
		for _, hub := range t.Hubs {
			fmt.Printf("    %s degree=%d (in=%d, out=%d)  %s\n",
				truncID(hub.ID), hub.Degree, hub.InDegree, hub.OutDegree, truncTitle(hub.Title, 40))
		}
	}

	// Staleness
	s := report.Staleness
	if s.StaleNodeCount > 0 || s.StaleSummaryCount > 0 {
		fmt.Println("\n  STALENESS")
		fmt.Println("  ────────────────────────────────────────")
		if s.StaleNodeCount > 0 {
			fmt.Printf("  %d stale nodes (old but recently referenced):\n", s.StaleNodeCount)
			limit := 10
			if len(s.StaleNodes) < limit {
				limit = len(s.StaleNodes)
			}
			for _, n := range s.StaleNodes[:limit] {
				fmt.Printf("    %s %dd old, %d recent refs  %s\n",
					truncID(n.ID), n.DaysSinceUpdate, n.RecentRefCount, truncTitle(n.Title, 40))
			}
		}
		if s.StaleSummaryCount > 0 {
			fmt.Printf("  %d stale summaries (target updated after summary):\n", s.StaleSummaryCount)
			limit := 10
			if len(s.StaleSummaries) < limit {
				limit = len(s.StaleSummaries)
			}
			for _, ss := range s.StaleSummaries[:limit] {
				fmt.Printf("    %s -> %s (%dd drift)\n",
					truncTitle(ss.SummaryTitle, 25), truncTitle(ss.TargetTitle, 25), ss.DriftDays)
			}
		}
	}

	// Bridges
	br := report.Bridges
	if br.APCount > 0 || br.BridgeCount > 0 || len(br.FragileConnections) > 0 {
		fmt.Println("\n  STRUCTURAL FRAGILITY")
		fmt.Println("  ────────────────────────────────────────")
		if br.APCount > 0 {
			fmt.Printf("  %d articulation points (removal disconnects graph):\n", br.APCount)
			limit := 10
			if len(br.ArticulationPoints) < limit {
				limit = len(br.ArticulationPoints)
			}
			for _, ap := range br.ArticulationPoints[:limit] {
				fmt.Printf("    %s (degree ~%d)  %s\n",
					truncID(ap.ID), ap.ComponentsIfRemoved, truncTitle(ap.Title, 40))
			}
		}
		if br.BridgeCount > 0 {
			fmt.Printf("  %d bridge edges (removal disconnects graph):\n", br.BridgeCount)
			limit := 10
			if len(br.BridgeEdges) < limit {
				limit = len(br.BridgeEdges)
			}
			for _, be := range br.BridgeEdges[:limit] {
				fmt.Printf("    %s -> %s\n", truncTitle(be.SourceTitle, 30), truncTitle(be.TargetTitle, 30))
			}
		}
		if len(br.FragileConnections) > 0 {
			fmt.Printf("  %d fragile inter-region connections (<=2 edges):\n", len(br.FragileConnections))
			limit := 10
			if len(br.FragileConnections) < limit {
				limit = len(br.FragileConnections)
			}
			for _, fc := range br.FragileConnections[:limit] {
				raTitle := fc.RegionA
				rbTitle := fc.RegionB
				if n := snap.Nodes[fc.RegionA]; n != nil {
					raTitle = n.Title
				}
				if n := snap.Nodes[fc.RegionB]; n != nil {
					rbTitle = n.Title
				}
				s := ""
				if fc.CrossEdges != 1 {
					s = "s"
				}
				fmt.Printf("    %s <-> %s (%d edge%s)\n",
					truncTitle(raTitle, 25), truncTitle(rbTitle, 25), fc.CrossEdges, s)
			}
		}
	}

	fmt.Println()
}

func truncID(id string) string {
	if len(id) > 8 {
		return id[:8]
	}
	return id
}

func truncTitle(s string, max int) string {
	if len(s) <= max {
		return s
	}
	// Find a safe UTF-8 boundary
	truncated := s[:max]
	for len(truncated) > 0 && truncated[len(truncated)-1]>>6 == 2 {
		truncated = truncated[:len(truncated)-1]
	}
	return truncated + "..."
}
