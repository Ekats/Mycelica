package cmd

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/spf13/cobra"
	"mycelica/spore/internal/db"
	"mycelica/spore/internal/orchestrate"
)

var (
	retryMaxBounces  int
	retryMaxTurns    int
	retryNoSummarize bool
	retryVerbose     bool
	retryQuiet       bool
	retryJSON        bool
	retryCoderModel  string
	retryExperiment  string
)

var retryCmd = &cobra.Command{
	Use:   "retry <run-id>",
	Short: "Re-run a failed or escalated task",
	Long:  "Resolves the original task from a run node ID (prefix or full), extracts the task description, and runs a fresh orchestration.",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		d, err := OpenDatabase()
		if err != nil {
			return err
		}
		defer d.Close()

		// Resolve the task node
		node, err := ResolveNode(d, args[0])
		if err != nil {
			return fmt.Errorf("cannot find run: %w", err)
		}

		// Extract task description from node content ("## Task" section) or title
		task := extractTaskFromNode(node)
		if task == "" {
			return fmt.Errorf("node %s doesn't look like an orchestration task (title: %s)", args[0], node.Title)
		}

		shortID := node.ID
		if len(shortID) > 8 {
			shortID = shortID[:8]
		}

		if !retryQuiet && !retryJSON {
			fmt.Printf("[retry] Original run: %s (%s)\n", shortID, node.Title)
			fmt.Printf("[retry] Task: %s\n", task)
			fmt.Printf("[retry] Retrying with max_bounces=%d, max_turns=%d\n", retryMaxBounces, retryMaxTurns)
		}

		config := orchestrate.OrchestrationConfig{
			TaskFile:    orchestrate.DefaultTaskFileConfig(),
			MaxBounces:  retryMaxBounces,
			MaxTurns:    retryMaxTurns,
			CoderModel:  retryCoderModel,
			OutputDir:   "/tmp/spore/",
			Experiment:  retryExperiment,
			NoSummarize: retryNoSummarize,
			Verbose:     retryVerbose,
			Quiet:       retryQuiet,
			JSON:        retryJSON,
		}

		result, err := orchestrate.RunOrchestration(d, task, config)

		if retryJSON && result != nil {
			enc := json.NewEncoder(os.Stdout)
			enc.SetIndent("", "  ")
			_ = enc.Encode(result)
			if err != nil {
				return err
			}
			return nil
		}

		if result != nil && !retryQuiet {
			shortRunID := result.RunID
			if len(shortRunID) > 8 {
				shortRunID = shortRunID[:8]
			}
			fmt.Printf("Run ID: %s\n", shortRunID)
			fmt.Printf("\nResult: %s\n", result.Status)
			fmt.Printf("Total cost: $%.4f\n", result.TotalCost)
		}

		if err != nil {
			return err
		}
		return nil
	},
}

// extractTaskFromNode extracts the task description from a node.
// Port of spore.rs:799-812.
//  1. If Content is non-nil, look for "## Task\n" section
//  2. Take all lines after "## Task\n" until the next "## " heading
//  3. Trim whitespace
//  4. If no content or no "## Task" section: fall back to title with "Orchestration:" prefix stripped
//  5. Return empty string if nothing works
func extractTaskFromNode(node *db.Node) string {
	if node.Content != nil {
		content := *node.Content
		marker := "## Task\n"
		idx := strings.Index(content, marker)
		if idx >= 0 {
			after := content[idx+len(marker):]
			var lines []string
			for _, line := range strings.Split(after, "\n") {
				if strings.HasPrefix(line, "## ") {
					break
				}
				lines = append(lines, line)
			}
			task := strings.TrimSpace(strings.Join(lines, "\n"))
			if task != "" {
				return task
			}
		}
	}

	// Fallback: strip "Orchestration:" prefix from title
	title := node.Title
	title = strings.TrimPrefix(title, "Orchestration:")
	title = strings.TrimSpace(title)
	return title
}

func init() {
	retryCmd.Flags().IntVar(&retryMaxBounces, "max-bounces", 3, "Maximum coder->verifier bounces")
	retryCmd.Flags().IntVar(&retryMaxTurns, "max-turns", 50, "Maximum Claude turns per agent")
	retryCmd.Flags().BoolVar(&retryNoSummarize, "no-summarize", false, "Skip summarizer after verification")
	retryCmd.Flags().BoolVar(&retryVerbose, "verbose", false, "Verbose output")
	retryCmd.Flags().BoolVar(&retryQuiet, "quiet", false, "Suppress non-essential output")
	retryCmd.Flags().BoolVar(&retryJSON, "json", false, "Output as JSON")
	retryCmd.Flags().StringVar(&retryCoderModel, "coder-model", "", "Override coder model")
	retryCmd.Flags().StringVar(&retryExperiment, "experiment", "", "A/B experiment label")
	rootCmd.AddCommand(retryCmd)
}
