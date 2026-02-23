package cmd

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/spf13/cobra"
	"mycelica/spore/internal/orchestrate"
)

var (
	loopFile             string
	loopStdin            bool
	loopBudget           float64
	loopMaxRuns          int
	loopMaxBounces       int
	loopMaxTurns         int
	loopDryRun           bool
	loopReset            bool
	loopNoCommit         bool
	loopPauseOnEscalation bool
	loopExperiment       string
	loopCoderModel       string
	loopVerbose          bool
	loopJSON             bool
)

var loopCmd = &cobra.Command{
	Use:   "loop [flags]",
	Short: "Run multiple tasks from a file or stdin with budget tracking",
	Long: `Dispatches tasks from a file (one per line, or --- delimited) through the
coder -> verifier -> summarizer pipeline. Tracks budget, persists state for
resume across restarts, and auto-commits between verified tasks.

Lines starting with # and blank lines are ignored.`,
	RunE: func(cmd *cobra.Command, args []string) error {
		// Determine source
		source := loopFile
		if loopStdin {
			source = "-"
		}
		if source == "" {
			return fmt.Errorf("specify --file <path> or --stdin")
		}

		d, err := OpenDatabase()
		if err != nil {
			return err
		}
		defer d.Close()

		orchConfig := orchestrate.OrchestrationConfig{
			TaskFile:    orchestrate.DefaultTaskFileConfig(),
			MaxBounces:  loopMaxBounces,
			MaxTurns:    loopMaxTurns,
			CoderModel:  loopCoderModel,
			OutputDir:   "/tmp/spore/",
			Experiment:  loopExperiment,
			DryRun:      loopDryRun,
			Verbose:     loopVerbose,
			JSON:        loopJSON,
		}

		config := orchestrate.LoopConfig{
			Source:            source,
			Budget:            loopBudget,
			MaxRuns:           loopMaxRuns,
			StopOnEscalation:  3,
			Reset:             loopReset,
			AutoCommit:        !loopNoCommit,
			PauseOnEscalation: loopPauseOnEscalation,
			OrchConfig:        orchConfig,
		}

		result, err := orchestrate.RunLoop(d, config)
		if err != nil {
			return err
		}

		if loopJSON && result != nil {
			enc := json.NewEncoder(os.Stdout)
			enc.SetIndent("", "  ")
			_ = enc.Encode(result)
		}

		return nil
	},
}

func init() {
	loopCmd.Flags().StringVar(&loopFile, "file", "", "Read tasks from file (one per line, or --- delimited)")
	loopCmd.Flags().BoolVar(&loopStdin, "stdin", false, "Read tasks from stdin")
	loopCmd.Flags().Float64Var(&loopBudget, "budget", 10.0, "Maximum total spend in USD")
	loopCmd.Flags().IntVar(&loopMaxRuns, "max-runs", 50, "Maximum tasks to run")
	loopCmd.Flags().IntVar(&loopMaxBounces, "max-bounces", 3, "Maximum coder->verifier bounces per task")
	loopCmd.Flags().IntVar(&loopMaxTurns, "max-turns", 50, "Maximum Claude turns per agent")
	loopCmd.Flags().BoolVar(&loopDryRun, "dry-run", false, "List tasks with complexity estimates, don't spawn agents")
	loopCmd.Flags().BoolVar(&loopReset, "reset", false, "Clear persisted loop state before starting")
	loopCmd.Flags().BoolVar(&loopNoCommit, "no-commit", false, "Disable auto-commit between verified tasks")
	loopCmd.Flags().BoolVar(&loopPauseOnEscalation, "pause-on-escalation", false, "Stop loop on first escalation")
	loopCmd.Flags().StringVar(&loopExperiment, "experiment", "", "A/B experiment label")
	loopCmd.Flags().StringVar(&loopCoderModel, "coder-model", "", "Override coder model")
	loopCmd.Flags().BoolVar(&loopVerbose, "verbose", false, "Verbose output")
	loopCmd.Flags().BoolVar(&loopJSON, "json", false, "Output as JSON")
	rootCmd.AddCommand(loopCmd)
}
