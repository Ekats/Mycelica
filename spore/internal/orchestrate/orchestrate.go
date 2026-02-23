package orchestrate

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/google/uuid"
	"mycelica/spore/internal/db"
)

// RunOrchestration runs the full coder -> verifier -> summarizer pipeline.
// It creates a task node, then bounces between coder and verifier until
// verification passes or max bounces is exhausted.
func RunOrchestration(d *db.DB, task string, config OrchestrationConfig) (*OrchestrationResult, error) {
	// Fail fast: check claude binary
	if _, err := exec.LookPath("claude"); err != nil {
		return nil, fmt.Errorf("claude binary not found in PATH: %w", err)
	}

	runID := uuid.New().String()

	cliBinary, err := db.FindCLIBinary()
	if err != nil {
		return nil, fmt.Errorf("finding CLI binary: %w", err)
	}

	workDir := findProjectRoot(filepath.Dir(d.Path))

	// Create task node
	taskNodeID, err := d.CreateNode(truncateTitle(task, 100), db.CreateNodeOpts{
		AgentID:   "spore:orchestrator",
		NodeClass: "operational",
		MetaType:  "task",
		Source:    "spore-go",
		Content:   task,
	})
	if err != nil {
		return nil, fmt.Errorf("creating task node: %w", err)
	}

	result := &OrchestrationResult{
		TaskNodeID: taskNodeID,
		RunID:      runID,
		Status:     StatusFailed,
	}

	// DryRun: generate task file and return
	if config.DryRun {
		taskFilePath, ctxCount, err := GenerateTaskFile(
			d, task, RoleCoder, runID, taskNodeID,
			0, config.MaxBounces,
			"", VerdictUnknown,
			config.TaskFile, config.OutputDir,
		)
		if err != nil {
			return result, fmt.Errorf("generating task file: %w", err)
		}
		if !config.Quiet {
			fmt.Fprintf(os.Stderr, "[orchestrate] Dry run: task file at %s (%d context nodes)\n", taskFilePath, ctxCount)
		}
		result.Status = StatusSuccess
		return result, nil
	}

	maxBounces := config.MaxBounces
	if maxBounces <= 0 {
		maxBounces = 3
	}

	var lastImplID string
	var lastVerdict Verdict

	for bounce := 0; bounce < maxBounces; bounce++ {
		result.Bounces = bounce + 1

		if !config.Quiet {
			fmt.Fprintf(os.Stderr, "\n[orchestrate] Bounce %d/%d\n", bounce+1, maxBounces)
		}

		// --- Coder ---
		coderResult, err := runCoder(d, task, runID, taskNodeID, bounce, maxBounces,
			lastImplID, lastVerdict, cliBinary, workDir, config)
		if err != nil {
			recordRunStatus(d, taskNodeID, runID, "coder", "failed", coderResult.Claude, config.Experiment)
			result.Phases = append(result.Phases, *coderResult)
			result.TotalCost += coderResult.Claude.CostUSD
			return result, fmt.Errorf("coder failed on bounce %d: %w", bounce+1, err)
		}
		result.Phases = append(result.Phases, *coderResult)
		result.TotalCost += coderResult.Claude.CostUSD

		if !config.Quiet {
			fmt.Fprintf(os.Stderr, "  Coder: %s %d turns, %s, $%.4f\n",
				selectCoderModel(config),
				coderResult.Claude.NumTurns,
				FormatDurationShort(coderResult.Claude.Duration.Milliseconds()),
				coderResult.Claude.CostUSD)
			if len(coderResult.ChangedFiles) > 0 {
				fmt.Fprintf(os.Stderr, "  Changed: %s\n", strings.Join(coderResult.ChangedFiles, ", "))
			}
		}

		// Post-coder cleanup: re-index changed files
		if len(coderResult.ChangedFiles) > 0 {
			postCoderCleanup(d, cliBinary, workDir, coderResult.ChangedFiles)
		}

		lastImplID = coderResult.ImplNodeID

		// --- Verifier ---
		verifierResult, err := runVerifier(d, task, runID, taskNodeID, coderResult.ImplNodeID,
			bounce, cliBinary, workDir, config)
		if err != nil {
			recordRunStatus(d, taskNodeID, runID, "verifier", "failed", verifierResult.Claude, config.Experiment)
			result.Phases = append(result.Phases, *verifierResult)
			result.TotalCost += verifierResult.Claude.CostUSD
			return result, fmt.Errorf("verifier failed on bounce %d: %w", bounce+1, err)
		}
		result.Phases = append(result.Phases, *verifierResult)
		result.TotalCost += verifierResult.Claude.CostUSD

		verdict := verifierResult.Verdict
		if verdict == nil {
			verdict = &VerifierVerdict{Verdict: VerdictUnknown, Confidence: 0}
		}
		result.Verdict = verdict.Verdict

		if !config.Quiet {
			fmt.Fprintf(os.Stderr, "  Verifier: %s (%.0f%% confidence)\n",
				verdict.Verdict.String(), verdict.Confidence*100)
		}

		lastVerdict = verdict.Verdict

		if verdict.Verdict == VerdictSupports {
			// Success -- run summarizer if enabled
			if !config.NoSummarize {
				sumResult, err := runSummarizer(d, task, runID, taskNodeID, coderResult.ImplNodeID,
					cliBinary, workDir, config)
				if err != nil {
					// Summarizer failure is non-fatal
					fmt.Fprintf(os.Stderr, "[orchestrate] Summarizer failed (non-fatal): %v\n", err)
				} else {
					result.Phases = append(result.Phases, *sumResult)
					result.TotalCost += sumResult.Claude.CostUSD
				}
			}

			recordRunStatus(d, taskNodeID, runID, "orchestrator", "success", nil, config.Experiment)
			result.Status = StatusSuccess
			return result, nil
		}

		// Verdict was contradicts or unknown -- continue to next bounce
	}

	// Max bounces exhausted
	createEscalation(d, taskNodeID, lastImplID, maxBounces, task)
	recordRunStatus(d, taskNodeID, runID, "orchestrator", "failed", nil, config.Experiment)
	return result, fmt.Errorf("max bounces (%d) exhausted without verification", maxBounces)
}

func runCoder(
	d *db.DB, task, runID, taskNodeID string,
	bounce, maxBounces int,
	lastImplID string, lastVerdict Verdict,
	cliBinary, workDir string,
	config OrchestrationConfig,
) (*PhaseResult, error) {
	// Capture git state before
	gitBefore, err := CaptureGitState(workDir)
	if err != nil {
		fmt.Fprintf(os.Stderr, "[orchestrate] Warning: failed to capture git state before coder: %v\n", err)
	}

	// Generate task file
	taskFilePath, _, err := GenerateTaskFile(
		d, task, RoleCoder, runID, taskNodeID,
		bounce, maxBounces,
		lastImplID, lastVerdict,
		config.TaskFile, config.OutputDir,
	)
	if err != nil {
		return &PhaseResult{Role: RoleCoder, Claude: &ClaudeResult{}},
			fmt.Errorf("generating coder task file: %w", err)
	}

	taskFileContent, err := os.ReadFile(taskFilePath)
	if err != nil {
		return &PhaseResult{Role: RoleCoder, Claude: &ClaudeResult{}},
			fmt.Errorf("reading task file %s: %w", taskFilePath, err)
	}

	// Select model
	model := selectCoderModel(config)

	// Resolve agent name
	agentName := ResolveAgentName(RoleCoder, workDir)

	// Write MCP config
	mcpConfigPath, err := WriteMCPConfig(cliBinary, "coder", AgentID(RoleCoder), runID, d.Path)
	if err != nil {
		return &PhaseResult{Role: RoleCoder, Claude: &ClaudeResult{}},
			fmt.Errorf("writing MCP config: %w", err)
	}

	allowed, disallowed := toolPermissions(RoleCoder)

	claudeCfg := ClaudeConfig{
		Role:          RoleCoder,
		Prompt:        string(taskFileContent),
		Model:         model,
		MaxTurns:      config.MaxTurns,
		AllowedTools:  allowed,
		DisallowTools: disallowed,
		MCPConfig:     mcpConfigPath,
		AgentName:     agentName,
		WorkDir:       workDir,
		Verbose:       config.Verbose,
	}

	// Spawn Claude
	claudeResult, err := SpawnClaude(claudeCfg)
	if err != nil {
		return &PhaseResult{Role: RoleCoder, Claude: &ClaudeResult{}},
			fmt.Errorf("spawning coder: %w", err)
	}

	// Capture git state after
	var changedFiles []string
	if gitBefore != nil {
		gitAfter, err := CaptureGitState(workDir)
		if err == nil {
			changedFiles = DiffChangedFiles(gitBefore, gitAfter)
		}
	}

	// Check for hard failure: non-zero exit and no changes
	if claudeResult.ExitCode != 0 && len(changedFiles) == 0 {
		return &PhaseResult{Role: RoleCoder, Claude: claudeResult, ChangedFiles: changedFiles},
			fmt.Errorf("coder exited with code %d and no files changed", claudeResult.ExitCode)
	}

	// Create implementation node
	implNodeID, err := d.CreateNode("Implementation: "+truncateTitle(task, 80), db.CreateNodeOpts{
		AgentID:   AgentID(RoleCoder),
		NodeClass: "operational",
		MetaType:  "implementation",
		Source:    "spore-go",
		Content:   fmt.Sprintf("Changed files: %s", strings.Join(changedFiles, ", ")),
	})
	if err != nil {
		return &PhaseResult{Role: RoleCoder, Claude: claudeResult, ChangedFiles: changedFiles},
			fmt.Errorf("creating implementation node: %w", err)
	}

	// Create DerivesFrom edge: impl -> task
	_, err = d.CreateEdge(implNodeID, taskNodeID, "derives_from", db.CreateEdgeOpts{
		Agent:  "spore:orchestrator",
		Reason: fmt.Sprintf("coder output bounce %d", bounce+1),
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "[orchestrate] Warning: failed to create derives_from edge: %v\n", err)
	}

	return &PhaseResult{
		Role:         RoleCoder,
		Claude:       claudeResult,
		ImplNodeID:   implNodeID,
		ChangedFiles: changedFiles,
	}, nil
}

func runVerifier(
	d *db.DB, task, runID, taskNodeID, implNodeID string,
	bounce int,
	cliBinary, workDir string,
	config OrchestrationConfig,
) (*PhaseResult, error) {
	// Generate verifier task file
	taskFilePath, _, err := GenerateTaskFile(
		d, task, RoleVerifier, runID, taskNodeID,
		0, 1,
		implNodeID, VerdictUnknown,
		config.TaskFile, config.OutputDir,
	)
	if err != nil {
		return &PhaseResult{Role: RoleVerifier, Claude: &ClaudeResult{}},
			fmt.Errorf("generating verifier task file: %w", err)
	}

	taskFileContent, err := os.ReadFile(taskFilePath)
	if err != nil {
		return &PhaseResult{Role: RoleVerifier, Claude: &ClaudeResult{}},
			fmt.Errorf("reading verifier task file: %w", err)
	}

	model := SelectModelForRole(RoleVerifier)
	agentName := ResolveAgentName(RoleVerifier, workDir)

	mcpConfigPath, err := WriteMCPConfig(cliBinary, "verifier", AgentID(RoleVerifier), runID, d.Path)
	if err != nil {
		return &PhaseResult{Role: RoleVerifier, Claude: &ClaudeResult{}},
			fmt.Errorf("writing verifier MCP config: %w", err)
	}

	allowed, disallowed := toolPermissions(RoleVerifier)

	claudeCfg := ClaudeConfig{
		Role:          RoleVerifier,
		Prompt:        string(taskFileContent),
		Model:         model,
		MaxTurns:      config.MaxTurns,
		AllowedTools:  allowed,
		DisallowTools: disallowed,
		MCPConfig:     mcpConfigPath,
		AgentName:     agentName,
		WorkDir:       workDir,
		Verbose:       config.Verbose,
	}

	claudeResult, err := SpawnClaude(claudeCfg)
	if err != nil {
		return &PhaseResult{Role: RoleVerifier, Claude: &ClaudeResult{}},
			fmt.Errorf("spawning verifier: %w", err)
	}

	// Determine verdict: check thinking first, then stderr as fallback
	verifierOutput := claudeResult.Thinking
	if verifierOutput == "" {
		verifierOutput = claudeResult.Stderr
	}
	verdict := DetermineVerdict(d, implNodeID, verifierOutput)

	return &PhaseResult{
		Role:    RoleVerifier,
		Claude:  claudeResult,
		Verdict: verdict,
	}, nil
}

func runSummarizer(
	d *db.DB, task, runID, taskNodeID, implNodeID string,
	cliBinary, workDir string,
	config OrchestrationConfig,
) (*PhaseResult, error) {
	taskFilePath, _, err := GenerateTaskFile(
		d, task, RoleSummarizer, runID, taskNodeID,
		0, 1,
		implNodeID, VerdictSupports,
		config.TaskFile, config.OutputDir,
	)
	if err != nil {
		return &PhaseResult{Role: RoleSummarizer, Claude: &ClaudeResult{}},
			fmt.Errorf("generating summarizer task file: %w", err)
	}

	taskFileContent, err := os.ReadFile(taskFilePath)
	if err != nil {
		return &PhaseResult{Role: RoleSummarizer, Claude: &ClaudeResult{}},
			fmt.Errorf("reading summarizer task file: %w", err)
	}

	model := SelectModelForRole(RoleSummarizer)
	agentName := ResolveAgentName(RoleSummarizer, workDir)

	mcpConfigPath, err := WriteMCPConfig(cliBinary, "summarizer", AgentID(RoleSummarizer), runID, d.Path)
	if err != nil {
		return &PhaseResult{Role: RoleSummarizer, Claude: &ClaudeResult{}},
			fmt.Errorf("writing summarizer MCP config: %w", err)
	}

	allowed, disallowed := toolPermissions(RoleSummarizer)

	claudeCfg := ClaudeConfig{
		Role:          RoleSummarizer,
		Prompt:        string(taskFileContent),
		Model:         model,
		MaxTurns:      15, // summarizer needs fewer turns
		AllowedTools:  allowed,
		DisallowTools: disallowed,
		MCPConfig:     mcpConfigPath,
		AgentName:     agentName,
		WorkDir:       workDir,
		Verbose:       config.Verbose,
	}

	claudeResult, err := SpawnClaude(claudeCfg)
	if err != nil {
		return &PhaseResult{Role: RoleSummarizer, Claude: &ClaudeResult{}},
			fmt.Errorf("spawning summarizer: %w", err)
	}

	// Create summary node
	summaryNodeID, err := d.CreateNode("Summary: "+truncateTitle(task, 80), db.CreateNodeOpts{
		AgentID:   AgentID(RoleSummarizer),
		NodeClass: "operational",
		MetaType:  "summary",
		Source:    "spore-go",
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "[orchestrate] Warning: failed to create summary node: %v\n", err)
	} else {
		// Create Summarizes edge: summary -> impl
		_, err = d.CreateEdge(summaryNodeID, implNodeID, "summarizes", db.CreateEdgeOpts{
			Agent:  "spore:orchestrator",
			Reason: "summarizer output",
		})
		if err != nil {
			fmt.Fprintf(os.Stderr, "[orchestrate] Warning: failed to create summarizes edge: %v\n", err)
		}
	}

	return &PhaseResult{
		Role:   RoleSummarizer,
		Claude: claudeResult,
	}, nil
}

// postCoderCleanup re-indexes changed files via the CLI.
func postCoderCleanup(d *db.DB, cliBinary, workDir string, changedFiles []string) {
	for _, file := range changedFiles {
		absPath := file
		if !filepath.IsAbs(file) {
			absPath = filepath.Join(workDir, file)
		}
		// Check file exists (may have been deleted)
		if _, err := os.Stat(absPath); os.IsNotExist(err) {
			continue
		}
		cmd := exec.Command(cliBinary, "import", "code", absPath, "--update", "--db", d.Path)
		cmd.Dir = workDir
		if out, err := cmd.CombinedOutput(); err != nil {
			fmt.Fprintf(os.Stderr, "[orchestrate] Warning: re-index %s failed: %v (%s)\n",
				file, err, strings.TrimSpace(string(out)))
		}
	}
}

// recordRunStatus creates a self-referential Tracks edge on the task node with
// run metadata. Non-fatal: logs warnings on failure.
func recordRunStatus(d *db.DB, taskNodeID, runID, agent, status string, result *ClaudeResult, experiment string) {
	var exitCode int
	var costUSD float64
	var numTurns int
	var durationMS int64
	var model string

	if result != nil {
		exitCode = result.ExitCode
		costUSD = result.CostUSD
		numTurns = result.NumTurns
		durationMS = result.Duration.Milliseconds()
	}

	metadata := fmt.Sprintf(
		`{"run_id":"%s","status":"%s","agent":"%s","exit_code":%d,"cost_usd":%.4f,"num_turns":%d,"duration_ms":%d,"experiment":"%s","model":"%s"}`,
		runID, status, agent, exitCode, costUSD, numTurns, durationMS, experiment, model,
	)

	shortRunID := runID
	if len(shortRunID) > 8 {
		shortRunID = shortRunID[:8]
	}

	_, err := d.CreateEdge(taskNodeID, taskNodeID, "tracks", db.CreateEdgeOpts{
		Agent:    "spore:orchestrator",
		Metadata: metadata,
		Reason:   fmt.Sprintf("run %s status: %s", shortRunID, status),
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "[orchestrate] Warning: failed to record run status: %v\n", err)
	}
}

// createEscalation creates an escalation node and links it to the task node.
func createEscalation(d *db.DB, taskNodeID, lastImplID string, bounceCount int, task string) {
	content := fmt.Sprintf("Task exceeded %d bounces without verification.", bounceCount)
	if lastImplID != "" {
		content += fmt.Sprintf(" Last implementation: %s", lastImplID)
	}

	escID, err := d.CreateNode("Escalation: "+truncateTitle(task, 80), db.CreateNodeOpts{
		AgentID:   "spore:orchestrator",
		NodeClass: "operational",
		MetaType:  "escalation",
		Content:   content,
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "[orchestrate] Warning: failed to create escalation node: %v\n", err)
		return
	}

	_, err = d.CreateEdge(escID, taskNodeID, "tracks", db.CreateEdgeOpts{
		Agent:  "spore:orchestrator",
		Reason: "escalation after max bounces",
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "[orchestrate] Warning: failed to create escalation edge: %v\n", err)
	}
}

// toolPermissions returns the allowed and disallowed tool strings for a role.
func toolPermissions(role AgentRole) (allowed, disallowed string) {
	switch role {
	case RoleCoder:
		return "Read,Write,Edit,Bash,mcp__mycelica__*", "Grep,Glob"
	case RoleVerifier:
		return "Read,Grep,Glob,Bash,mcp__mycelica__*", ""
	case RoleSummarizer:
		return "mcp__mycelica__*", "Bash,Edit,Write"
	default:
		return "", ""
	}
}

// truncateTitle shortens a string to max bytes.
func truncateTitle(s string, max int) string {
	if len(s) <= max {
		return s
	}
	return s[:max]
}

// findProjectRoot walks up from startPath looking for a .git directory.
// Returns startPath if no .git is found.
func findProjectRoot(startPath string) string {
	dir := startPath
	for {
		if _, err := os.Stat(filepath.Join(dir, ".git")); err == nil {
			return dir
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return startPath
		}
		dir = parent
	}
}

// selectCoderModel returns the coder model, preferring config override.
func selectCoderModel(config OrchestrationConfig) string {
	if config.CoderModel != "" {
		return config.CoderModel
	}
	return SelectModelForRole(RoleCoder)
}

// RecordRunStatus is the exported wrapper for tests.
func RecordRunStatus(d *db.DB, taskNodeID, runID, agent, status string, result *ClaudeResult, experiment string) {
	recordRunStatus(d, taskNodeID, runID, agent, status, result, experiment)
}

// CreateEscalation is the exported wrapper for tests.
func CreateEscalation(d *db.DB, taskNodeID, lastImplID string, bounceCount int, task string) {
	createEscalation(d, taskNodeID, lastImplID, bounceCount, task)
}

// FindProjectRoot is the exported wrapper for tests.
func FindProjectRoot(startPath string) string {
	return findProjectRoot(startPath)
}

// ToolPermissions is the exported wrapper for tests.
func ToolPermissions(role AgentRole) (allowed, disallowed string) {
	return toolPermissions(role)
}

// TruncateTitle is the exported wrapper for tests.
func TruncateTitle(s string, max int) string {
	return truncateTitle(s, max)
}

// mergeStartTime returns the total cost across all phases.
func totalPhaseCost(phases []PhaseResult) float64 {
	var total float64
	for _, p := range phases {
		if p.Claude != nil {
			total += p.Claude.CostUSD
		}
	}
	return total
}

// totalPhaseDuration returns the wall-clock duration across all phases.
func totalPhaseDuration(phases []PhaseResult) time.Duration {
	var total time.Duration
	for _, p := range phases {
		if p.Claude != nil {
			total += p.Claude.Duration
		}
	}
	return total
}
