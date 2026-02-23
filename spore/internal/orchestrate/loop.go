package orchestrate

import (
	"bufio"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"mycelica/spore/internal/db"
)

// LoopConfig controls the loop execution.
type LoopConfig struct {
	Source            string  // file path or "-" for stdin
	Budget            float64 // max total spend in USD
	MaxRuns           int     // max tasks to run
	StopOnEscalation  int     // consecutive escalations before abort (default 3)
	Reset             bool    // clear persisted loop state
	AutoCommit        bool    // git add+commit between verified tasks
	PauseOnEscalation bool    // stop loop on first escalation
	OrchConfig        OrchestrationConfig
}

// LoopResult summarizes the full loop execution.
type LoopResult struct {
	Tasks     []LoopTaskResult `json:"tasks"`
	TotalCost float64          `json:"total_cost_usd"`
	Duration  time.Duration    `json:"duration"`
}

// LoopTaskResult is the outcome of one task.
type LoopTaskResult struct {
	Task       string        `json:"task"`
	Status     string        `json:"status"` // "verified", "escalated", "failed"
	Cost       float64       `json:"cost_usd"`
	Duration   time.Duration `json:"duration"`
	TaskNodeID string        `json:"task_node_id,omitempty"`
}

// RunLoop dispatches multiple tasks from a file with budget tracking and resume support.
// Port of handle_spore_loop (spore.rs:4042-4267).
func RunLoop(d *db.DB, config LoopConfig) (*LoopResult, error) {
	tasks, err := ReadTasks(config.Source)
	if err != nil {
		return nil, err
	}

	stopOnEsc := config.StopOnEscalation
	if stopOnEsc <= 0 {
		stopOnEsc = 3
	}

	// Load persisted loop state
	statePath := loopStatePath(config.Source)
	if config.Reset {
		if err := os.Remove(statePath); err != nil && !os.IsNotExist(err) {
			return nil, fmt.Errorf("failed to delete loop state: %w", err)
		}
		if !os.IsNotExist(err) {
			fmt.Fprintf(os.Stderr, "[loop] Loop state reset: deleted %s\n", statePath)
		}
	}
	state := loadLoopState(statePath, config.Source)
	alreadyVerified := len(state.VerifiedTasks)

	// Header
	fmt.Fprintf(os.Stderr, "[loop] Starting: %d tasks, $%.2f budget, max %d runs\n",
		len(tasks), config.Budget, config.MaxRuns)
	fmt.Fprintf(os.Stderr, "[loop] Config: max_bounces=%d, max_turns=%d, auto_commit=%v\n",
		config.OrchConfig.MaxBounces, config.OrchConfig.MaxTurns, config.AutoCommit)
	if alreadyVerified > 0 {
		fmt.Fprintf(os.Stderr, "[loop] Resuming: %d task(s) already verified, will skip.\n", alreadyVerified)
	}

	// Dry run: list tasks with complexity estimates, no agents
	if config.OrchConfig.DryRun {
		fmt.Fprintf(os.Stderr, "\n[loop] === DRY RUN ===\n")
		shown := 0
		for i, task := range tasks {
			if shown >= config.MaxRuns {
				break
			}
			complexity := EstimateComplexity(task)
			taskShort := TruncateMiddle(task, 70)
			fmt.Fprintf(os.Stderr, "  %d. [complexity %d/10] %s\n", i+1, complexity, taskShort)
			shown++
		}
		if len(tasks) > config.MaxRuns {
			fmt.Fprintf(os.Stderr, "  ... and %d more tasks (limited by --max-runs %d)\n",
				len(tasks)-config.MaxRuns, config.MaxRuns)
		}
		fmt.Fprintf(os.Stderr, "\n[loop] Would dispatch %d task(s). No agents spawned.\n", shown)
		return &LoopResult{
			Tasks:     nil,
			TotalCost: 0,
			Duration:  0,
		}, nil
	}

	// Resume total_cost from persisted state so budget is accurate across restarts
	totalCost := state.TotalCost
	var results []LoopTaskResult
	consecutiveEscalations := 0
	loopStart := time.Now()

	workDir := findProjectRoot(filepath.Dir(d.Path))

	for i, task := range tasks {
		// Budget check
		if totalCost >= config.Budget {
			fmt.Fprintf(os.Stderr, "\n[loop] Budget exhausted ($%.2f/$%.2f). Stopping.\n", totalCost, config.Budget)
			break
		}

		// Max runs check
		if len(results) >= config.MaxRuns {
			fmt.Fprintf(os.Stderr, "\n[loop] Max runs reached (%d/%d). Stopping.\n", len(results), config.MaxRuns)
			break
		}

		// Consecutive escalation check
		if consecutiveEscalations >= stopOnEsc {
			fmt.Fprintf(os.Stderr, "\n[loop] %d consecutive escalations. Stopping -- likely systemic issue.\n", stopOnEsc)
			break
		}

		// Skip already-verified tasks
		if state.isVerified(task) {
			fmt.Fprintf(os.Stderr, "[loop] Skipping task %d (already verified)\n", i+1)
			continue
		}

		remainingBudget := config.Budget - totalCost
		fmt.Fprintf(os.Stderr, "\n[loop] === Task %d/%d: %s ===\n",
			i+1, len(tasks), TruncateMiddle(task, 60))
		fmt.Fprintf(os.Stderr, "[loop] Budget remaining: $%.2f\n", remainingBudget)

		taskStart := time.Now()

		if config.OrchConfig.Verbose {
			complexity := EstimateComplexity(task)
			fmt.Fprintf(os.Stderr, "[loop] Complexity %d/10 (informational only)\n", complexity)
		}

		// Dispatch via orchestration pipeline
		orchResult, orchErr := RunOrchestration(d, task, config.OrchConfig)

		taskDuration := time.Since(taskStart)

		// Determine status
		var status string
		var runCost float64
		var taskNodeID string

		if orchResult != nil {
			runCost = orchResult.TotalCost
			taskNodeID = orchResult.TaskNodeID
		}

		if orchErr == nil {
			status = "verified"
		} else {
			errMsg := orchErr.Error()
			if strings.Contains(errMsg, "Escalation") ||
				strings.Contains(errMsg, "bounce") ||
				strings.Contains(errMsg, "exhausted") {
				status = "escalated"
			} else {
				status = "failed"
			}
		}

		taskResult := LoopTaskResult{
			Task:       task,
			Status:     status,
			Cost:       runCost,
			Duration:   taskDuration,
			TaskNodeID: taskNodeID,
		}

		totalCost += runCost

		// Persist state immediately
		state.recordResult(&taskResult)
		if err := state.save(); err != nil {
			fmt.Fprintf(os.Stderr, "[loop] Warning: failed to persist loop state: %v\n", err)
		}

		// Print status
		switch status {
		case "verified":
			consecutiveEscalations = 0
			fmt.Fprintf(os.Stderr, "[loop] VERIFIED: $%.2f, %s\n",
				runCost, FormatDurationShort(taskDuration.Milliseconds()))

			// Auto-commit between tasks
			if config.AutoCommit && i+1 < len(tasks) {
				autoCommit(task, workDir)
			}

		case "escalated":
			consecutiveEscalations++
			fmt.Fprintf(os.Stderr, "[loop] ESCALATED: #%d consecutive -- %s\n",
				consecutiveEscalations, TruncateMiddle(task, 50))
			if config.PauseOnEscalation {
				fmt.Fprintf(os.Stderr, "[loop] --pause-on-escalation: stopping loop\n")
				results = append(results, taskResult)
				break
			}

		case "failed":
			errMsg := "unknown"
			if orchErr != nil {
				errMsg = orchErr.Error()
			}
			fmt.Fprintf(os.Stderr, "[loop] FAILED: %s -- %s\n",
				TruncateMiddle(task, 50), TruncateMiddle(errMsg, 60))
		}

		// Cost anomaly detection: warn if current task cost > 3x running average
		if len(results) >= 3 && runCost > 0.0 {
			previousTotal := totalCost - runCost
			avg := previousTotal / float64(len(results))
			if avg > 0.0 {
				ratio := runCost / avg
				if ratio > 3.0 {
					fmt.Fprintf(os.Stderr, "[loop] Cost anomaly: $%.2f is %.1fx the average $%.2f\n",
						runCost, ratio, avg)
				}
			}
		}

		results = append(results, taskResult)

		// If we broke out of the switch due to pause-on-escalation, stop the loop
		if status == "escalated" && config.PauseOnEscalation {
			break
		}

		// Brief pause between dispatches
		if i+1 < len(tasks) && len(results) < config.MaxRuns {
			time.Sleep(5 * time.Second)
		}
	}

	totalDuration := time.Since(loopStart)

	printLoopSummary(results, totalCost, config.Budget, totalDuration, config.OrchConfig.JSON)

	return &LoopResult{
		Tasks:     results,
		TotalCost: totalCost,
		Duration:  totalDuration,
	}, nil
}

// ReadTasks reads task descriptions from a file or stdin.
// Supports two formats:
//  1. One task per line
//  2. Multi-line tasks separated by "---" on its own line
//
// In both formats, blank lines and lines starting with '#' are skipped.
func ReadTasks(source string) ([]string, error) {
	var reader io.Reader
	if source == "-" {
		reader = os.Stdin
	} else {
		f, err := os.Open(source)
		if err != nil {
			return nil, fmt.Errorf("failed to read task source '%s': %w", source, err)
		}
		defer f.Close()
		reader = f
	}

	content, err := io.ReadAll(reader)
	if err != nil {
		return nil, fmt.Errorf("reading tasks: %w", err)
	}

	tasks := parseTaskContent(string(content))
	if len(tasks) == 0 {
		return nil, fmt.Errorf("no tasks found in '%s' (blank lines and # comments ignored)", source)
	}
	return tasks, nil
}

// parseTaskContent parses task descriptions from text content.
// If "---" delimiters are present, lines between them are joined into multi-line tasks.
// Otherwise, each non-blank, non-comment line is a separate task.
func parseTaskContent(content string) []string {
	lines := strings.Split(content, "\n")

	// Check for multi-line delimiter format
	hasDelimiter := false
	for _, line := range lines {
		if strings.TrimSpace(line) == "---" {
			hasDelimiter = true
			break
		}
	}

	if hasDelimiter {
		var tasks []string
		var currentLines []string

		for _, line := range lines {
			if strings.TrimSpace(line) == "---" {
				task := flushTaskSection(currentLines)
				if task != "" {
					tasks = append(tasks, task)
				}
				currentLines = nil
			} else {
				currentLines = append(currentLines, line)
			}
		}
		// Flush final section
		task := flushTaskSection(currentLines)
		if task != "" {
			tasks = append(tasks, task)
		}
		return tasks
	}

	// Simple format: one task per line
	var tasks []string
	scanner := bufio.NewScanner(strings.NewReader(content))
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		tasks = append(tasks, line)
	}
	return tasks
}

// flushTaskSection joins non-blank, non-comment lines from a section into a single task.
func flushTaskSection(lines []string) string {
	var parts []string
	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" || strings.HasPrefix(trimmed, "#") {
			continue
		}
		parts = append(parts, trimmed)
	}
	return strings.Join(parts, " ")
}

// ---------------------------------------------------------------------------
// Loop State Persistence
// ---------------------------------------------------------------------------

// loopState tracks verified tasks and cumulative cost across loop restarts.
type loopState struct {
	Source        string            `json:"source"`
	VerifiedTasks map[string]bool  `json:"verified_tasks"`
	TotalCost     float64          `json:"total_cost"`
	Runs          []loopStateRun   `json:"runs"`
	CreatedAt     string           `json:"created_at"`
	UpdatedAt     string           `json:"updated_at"`
	path          string           // filesystem path (not serialized)
}

type loopStateRun struct {
	Task        string  `json:"task"`
	Status      string  `json:"status"`
	Cost        float64 `json:"cost"`
	DurationMS  int64   `json:"duration_ms"`
	TaskNodeID  string  `json:"task_node_id,omitempty"`
	CompletedAt string  `json:"completed_at"`
}

func newLoopState(path, source string) *loopState {
	now := time.Now().UTC().Format(time.RFC3339)
	return &loopState{
		Source:        source,
		VerifiedTasks: make(map[string]bool),
		Runs:          nil,
		CreatedAt:     now,
		UpdatedAt:     now,
		path:          path,
	}
}

func loadLoopState(path, source string) *loopState {
	data, err := os.ReadFile(path)
	if err != nil {
		return newLoopState(path, source)
	}
	var state loopState
	if err := json.Unmarshal(data, &state); err != nil {
		return newLoopState(path, source)
	}
	state.path = path
	if state.VerifiedTasks == nil {
		state.VerifiedTasks = make(map[string]bool)
	}
	return &state
}

func (s *loopState) save() error {
	data, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		return fmt.Errorf("serializing loop state: %w", err)
	}
	// Ensure parent directory exists
	dir := filepath.Dir(s.path)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return fmt.Errorf("creating state dir: %w", err)
	}
	if err := os.WriteFile(s.path, data, 0644); err != nil {
		return fmt.Errorf("writing loop state to %s: %w", s.path, err)
	}
	return nil
}

func (s *loopState) isVerified(task string) bool {
	return s.VerifiedTasks[task]
}

func (s *loopState) recordResult(r *LoopTaskResult) {
	if r.Status == "verified" {
		s.VerifiedTasks[r.Task] = true
	}
	s.TotalCost += r.Cost
	s.Runs = append(s.Runs, loopStateRun{
		Task:        r.Task,
		Status:      r.Status,
		Cost:        r.Cost,
		DurationMS:  r.Duration.Milliseconds(),
		TaskNodeID:  r.TaskNodeID,
		CompletedAt: time.Now().UTC().Format(time.RFC3339),
	})
	s.UpdatedAt = time.Now().UTC().Format(time.RFC3339)
}

// loopStatePath computes the state file path: same directory as source, with .loop-state.json suffix.
func loopStatePath(source string) string {
	if source == "-" {
		return filepath.Join(os.TempDir(), "spore-loop-stdin.loop-state.json")
	}
	// Strip optional "file:" prefix
	path := strings.TrimPrefix(source, "file:")
	dir := filepath.Dir(path)
	stem := strings.TrimSuffix(filepath.Base(path), filepath.Ext(path))
	if stem == "" {
		stem = "tasks"
	}
	return filepath.Join(dir, stem+".loop-state.json")
}

// ---------------------------------------------------------------------------
// Auto-commit
// ---------------------------------------------------------------------------

// autoCommit stages and commits changes between loop tasks, excluding internal artifacts.
func autoCommit(task, workDir string) {
	staged := selectiveGitAdd(workDir)
	if !staged {
		return
	}

	shortDesc := TruncateMiddle(task, 50)
	msg := fmt.Sprintf("feat(loop): %s", shortDesc)

	cmd := exec.Command("git", "commit", "-m", msg, "--allow-empty")
	cmd.Dir = workDir
	out, err := cmd.CombinedOutput()
	if err != nil {
		fmt.Fprintf(os.Stderr, "[loop] No changes to commit (or commit failed): %s\n",
			strings.TrimSpace(string(out)))
		return
	}
	fmt.Fprintf(os.Stderr, "[loop] Auto-committed changes before next task\n")
}

// selectiveGitAdd stages tracked modifications and selectively adds new untracked files,
// excluding internal artifacts. Returns true if staging succeeded.
func selectiveGitAdd(workDir string) bool {
	// Stage modifications/deletions to already-tracked files
	cmd := exec.Command("git", "add", "-u")
	cmd.Dir = workDir
	if err := cmd.Run(); err != nil {
		return false
	}

	// List new untracked files (respects .gitignore)
	lsCmd := exec.Command("git", "ls-files", "--others", "--exclude-standard")
	lsCmd.Dir = workDir
	out, err := lsCmd.Output()
	if err != nil {
		return true // tracked file staging succeeded, untracked listing failed -- ok
	}

	var toAdd []string
	for _, line := range strings.Split(string(out), "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		if !ShouldExcludeFile(line) {
			toAdd = append(toAdd, line)
		}
	}

	if len(toAdd) > 0 {
		addArgs := append([]string{"add"}, toAdd...)
		addCmd := exec.Command("git", addArgs...)
		addCmd.Dir = workDir
		_ = addCmd.Run()
	}

	return true
}

// ShouldExcludeFile returns true for files that should not be auto-committed by the loop.
// Matches is_spore_excluded from spore.rs.
func ShouldExcludeFile(path string) bool {
	basename := filepath.Base(path)

	if strings.HasSuffix(path, ".loop-state.json") {
		return true
	}
	if strings.HasPrefix(basename, ".env") {
		return true
	}
	if strings.HasPrefix(path, "target/") || strings.HasPrefix(path, "node_modules/") {
		return true
	}
	if strings.HasSuffix(path, ".db") || strings.HasSuffix(path, ".db-journal") ||
		strings.HasSuffix(path, ".db-wal") || strings.HasSuffix(path, ".db-shm") {
		return true
	}
	if strings.HasSuffix(path, ".log") {
		return true
	}
	if strings.HasSuffix(path, ".lock") {
		return true
	}
	return false
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

func printLoopSummary(results []LoopTaskResult, totalCost, budget float64, duration time.Duration, asJSON bool) {
	total := len(results)
	verified := 0
	escalated := 0
	failed := 0
	for _, r := range results {
		switch r.Status {
		case "verified":
			verified++
		case "escalated":
			escalated++
		case "failed":
			failed++
		}
	}
	avgCost := 0.0
	if total > 0 {
		avgCost = totalCost / float64(total)
	}

	if asJSON {
		type jsonTask struct {
			Description string  `json:"description"`
			Status      string  `json:"status"`
			Cost        float64 `json:"cost"`
			DurationMS  int64   `json:"duration_ms"`
		}
		tasksJSON := make([]jsonTask, len(results))
		for i, r := range results {
			tasksJSON[i] = jsonTask{
				Description: r.Task,
				Status:      r.Status,
				Cost:        r.Cost,
				DurationMS:  r.Duration.Milliseconds(),
			}
		}

		output := struct {
			TasksDispatched int       `json:"tasks_dispatched"`
			Verified        int       `json:"verified"`
			Escalated       int       `json:"escalated"`
			Failed          int       `json:"failed"`
			TotalCost       float64   `json:"total_cost"`
			Budget          float64   `json:"budget"`
			AvgCostPerTask  float64   `json:"avg_cost_per_task"`
			TotalDurationMS int64     `json:"total_duration_ms"`
			Tasks           []jsonTask `json:"tasks"`
		}{
			TasksDispatched: total,
			Verified:        verified,
			Escalated:       escalated,
			Failed:          failed,
			TotalCost:       totalCost,
			Budget:          budget,
			AvgCostPerTask:  avgCost,
			TotalDurationMS: duration.Milliseconds(),
			Tasks:           tasksJSON,
		}

		data, _ := json.MarshalIndent(output, "", "  ")
		fmt.Println(string(data))
		return
	}

	rate := 0.0
	if total > 0 {
		rate = float64(verified) / float64(total) * 100.0
	}

	fmt.Fprintf(os.Stderr, "\n[loop] === Summary ===\n")
	fmt.Fprintf(os.Stderr, "  Tasks dispatched: %d\n", total)
	fmt.Fprintf(os.Stderr, "  Verified:         %d (%.0f%%)\n", verified, rate)
	fmt.Fprintf(os.Stderr, "  Escalated:        %d\n", escalated)
	fmt.Fprintf(os.Stderr, "  Failed:           %d\n", failed)
	fmt.Fprintf(os.Stderr, "  Total cost:       $%.2f / $%.2f budget\n", totalCost, budget)
	fmt.Fprintf(os.Stderr, "  Avg cost/task:    $%.2f\n", avgCost)
	fmt.Fprintf(os.Stderr, "  Total duration:   %s\n", FormatDurationShort(duration.Milliseconds()))

	// Per-task breakdown
	if total > 0 {
		fmt.Fprintf(os.Stderr, "\n  Task details:\n")
		for i, r := range results {
			statusStr := strings.ToUpper(r.Status)
			taskShort := TruncateMiddle(r.Task, 50)
			fmt.Fprintf(os.Stderr, "    %d. [%s] $%.2f %s -- %s\n",
				i+1, statusStr, r.Cost,
				FormatDurationShort(r.Duration.Milliseconds()),
				taskShort)
		}
	}
}

// taskHash returns a short hex hash of a task string for state file naming.
func taskHash(s string) string {
	h := sha256.Sum256([]byte(s))
	return fmt.Sprintf("%x", h[:8])
}
