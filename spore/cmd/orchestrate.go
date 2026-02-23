package cmd

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/spf13/cobra"
	"mycelica/spore/internal/orchestrate"
)

var (
	orchMaxBounces  int
	orchMaxTurns    int
	orchNoSummarize bool
	orchDryRun      bool
	orchVerbose     bool
	orchQuiet       bool
	orchOutputDir   string
	orchExperiment  string
	orchCoderModel  string
	orchJSON        bool
)

var orchestrateCmd = &cobra.Command{
	Use:   "orchestrate <task>",
	Short: "Run the coder -> verifier -> summarizer pipeline on a task",
	Long:  "Orchestrates a multi-agent pipeline: coder writes code, verifier checks it, summarizer records the outcome. Bounces on verification failure.",
	Args:  cobra.MinimumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		task := strings.Join(args, " ")

		d, err := OpenDatabase()
		if err != nil {
			return err
		}
		defer d.Close()

		config := orchestrate.OrchestrationConfig{
			TaskFile:    orchestrate.DefaultTaskFileConfig(),
			MaxBounces:  orchMaxBounces,
			MaxTurns:    orchMaxTurns,
			CoderModel:  orchCoderModel,
			OutputDir:   orchOutputDir,
			Experiment:  orchExperiment,
			DryRun:      orchDryRun,
			NoSummarize: orchNoSummarize,
			Verbose:     orchVerbose,
			Quiet:       orchQuiet,
			JSON:        orchJSON,
		}

		if !orchQuiet && !orchJSON {
			taskShort := task
			if len(taskShort) > 60 {
				taskShort = taskShort[:60] + "..."
			}
			fmt.Printf("Orchestrating: %s\n", taskShort)
		}

		result, err := orchestrate.RunOrchestration(d, task, config)

		if orchJSON && result != nil {
			enc := json.NewEncoder(os.Stdout)
			enc.SetIndent("", "  ")
			_ = enc.Encode(result)
			if err != nil {
				return err
			}
			return nil
		}

		if result != nil && !orchQuiet {
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

func init() {
	orchestrateCmd.Flags().IntVar(&orchMaxBounces, "max-bounces", 3, "Maximum coder->verifier bounces")
	orchestrateCmd.Flags().IntVar(&orchMaxTurns, "max-turns", 50, "Maximum Claude turns per agent")
	orchestrateCmd.Flags().BoolVar(&orchNoSummarize, "no-summarize", false, "Skip summarizer after verification")
	orchestrateCmd.Flags().BoolVar(&orchDryRun, "dry-run", false, "Generate task file only, don't spawn agents")
	orchestrateCmd.Flags().BoolVar(&orchVerbose, "verbose", false, "Verbose output")
	orchestrateCmd.Flags().BoolVar(&orchQuiet, "quiet", false, "Suppress non-essential output")
	orchestrateCmd.Flags().StringVar(&orchOutputDir, "output-dir", "/tmp/spore/", "Task file output directory")
	orchestrateCmd.Flags().StringVar(&orchExperiment, "experiment", "", "A/B experiment label")
	orchestrateCmd.Flags().StringVar(&orchCoderModel, "coder-model", "", "Override coder model")
	orchestrateCmd.Flags().BoolVar(&orchJSON, "json", false, "Output as JSON")
	rootCmd.AddCommand(orchestrateCmd)
}
