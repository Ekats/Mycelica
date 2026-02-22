package cmd

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/spf13/cobra"
	"mycelica/spore/internal/db"
)

var (
	ctxBudget        int
	ctxMaxHops       int
	ctxMaxCost       float64
	ctxNotSuperseded bool
	ctxItemsOnly     bool
	ctxJSON          bool
	ctxEdgeTypes     string
)

var contextCmd = &cobra.Command{
	Use:   "context-for-task <id>",
	Short: "Dijkstra context expansion from a source node",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		d, err := OpenDatabase()
		if err != nil {
			return err
		}
		defer d.Close()

		source, err := ResolveNode(d, args[0])
		if err != nil {
			return err
		}

		config := &db.ContextConfig{
			Budget:        ctxBudget,
			MaxHops:       ctxMaxHops,
			MaxCost:       ctxMaxCost,
			NotSuperseded: ctxNotSuperseded,
			ItemsOnly:     ctxItemsOnly,
		}

		if ctxEdgeTypes != "" {
			config.EdgeTypes = strings.Split(ctxEdgeTypes, ",")
			for i := range config.EdgeTypes {
				config.EdgeTypes[i] = strings.TrimSpace(config.EdgeTypes[i])
			}
		}

		results, err := d.ContextForTask(source.ID, config)
		if err != nil {
			return fmt.Errorf("context expansion: %w", err)
		}

		if ctxJSON {
			srcTitle := source.Title
			if source.AITitle != nil {
				srcTitle = *source.AITitle
			}
			output := struct {
				Source  interface{}      `json:"source"`
				Budget int              `json:"budget"`
				Results []db.ContextNode `json:"results"`
				Count  int              `json:"count"`
			}{
				Source: struct {
					ID    string `json:"id"`
					Title string `json:"title"`
				}{source.ID, srcTitle},
				Budget:  ctxBudget,
				Results: results,
				Count:   len(results),
			}
			enc := json.NewEncoder(os.Stdout)
			enc.SetIndent("", "  ")
			return enc.Encode(output)
		}

		printContextHumanReadable(source, results)
		return nil
	},
}

func init() {
	contextCmd.Flags().IntVar(&ctxBudget, "budget", 20, "Max nodes to return")
	contextCmd.Flags().IntVar(&ctxMaxHops, "max-hops", 6, "Max graph depth")
	contextCmd.Flags().Float64Var(&ctxMaxCost, "max-cost", 3.0, "Cost ceiling")
	contextCmd.Flags().BoolVar(&ctxNotSuperseded, "not-superseded", false, "Filter superseded edges")
	contextCmd.Flags().BoolVar(&ctxItemsOnly, "items-only", false, "Skip categories from results")
	contextCmd.Flags().BoolVar(&ctxJSON, "json", false, "JSON output")
	contextCmd.Flags().StringVar(&ctxEdgeTypes, "edge-types", "", "Comma-separated edge type allowlist")
	rootCmd.AddCommand(contextCmd)
}

func printContextHumanReadable(source *db.Node, results []db.ContextNode) {
	srcTitle := source.Title
	if source.AITitle != nil {
		srcTitle = *source.AITitle
	}
	srcID := source.ID
	if len(srcID) > 8 {
		srcID = srcID[:8]
	}

	if len(results) == 0 {
		fmt.Printf("No context nodes found for: %s\n", srcTitle)
		return
	}

	fmt.Printf("Context for: %s (%s)  budget=%d\n\n", srcTitle, srcID, ctxBudget)

	for _, r := range results {
		marker := "[I]"
		if !r.IsItem {
			marker = "[C]"
		}
		classLabel := ""
		if r.NodeClass != nil {
			classLabel = fmt.Sprintf(" [%s]", *r.NodeClass)
		}
		fmt.Printf("  %2d. %s %s%s — dist=%.3f rel=%.0f%% hops=%d\n",
			r.Rank, marker, r.NodeTitle, classLabel,
			r.Distance, r.Relevance*100, r.Hops)

		if len(r.Path) > 0 {
			hops := make([]string, len(r.Path))
			for i, hop := range r.Path {
				title := hop.NodeTitle
				if len(title) > 40 {
					title = title[:40]
				}
				hops[i] = fmt.Sprintf("→[%s]→ %s", hop.EdgeType, title)
			}
			fmt.Printf("      %s\n", strings.Join(hops, " "))
		}
	}

	fmt.Printf("\n%d node(s) within budget\n", len(results))
}
