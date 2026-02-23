package orchestrate

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"
)

// SpawnClaude starts a Claude Code subprocess with the given configuration,
// reads stream-json output, and returns the captured result.
func SpawnClaude(config ClaudeConfig) (*ClaudeResult, error) {
	// Build args
	args := []string{
		"-p", config.Prompt,
		"--model", config.Model,
		"--output-format", "stream-json",
		"--dangerously-skip-permissions",
		"--verbose",
		"--max-turns", strconv.Itoa(config.MaxTurns),
	}

	if config.AllowedTools != "" {
		args = append(args, "--allowedTools", config.AllowedTools)
	}
	if config.DisallowTools != "" {
		args = append(args, "--disallowedTools", config.DisallowTools)
	}
	if config.MCPConfig != "" {
		args = append(args, "--mcp-config", config.MCPConfig)
	}
	if config.AgentName != "" {
		args = append(args, "--agent-name", config.AgentName)
	}
	if config.ResumeID != "" {
		args = append(args, "--resume", config.ResumeID)
	}

	cmd := exec.Command("claude", args...)

	// Set working directory
	if config.WorkDir != "" {
		cmd.Dir = config.WorkDir
	}

	// Filter environment: remove CLAUDE_CODE_* and CLAUDECODE vars to prevent
	// recursive agent detection when the orchestrator runs inside Claude Code.
	cmd.Env = filterClaudeEnv(os.Environ())

	// Get stdout pipe for stream-json parsing
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return nil, fmt.Errorf("creating stdout pipe: %w", err)
	}

	// Capture stderr to a capped buffer
	var stderrBuf cappedBuffer
	stderrBuf.limit = 10 * 1024 // 10KB
	cmd.Stderr = &stderrBuf

	// Start the process
	if err := cmd.Start(); err != nil {
		return nil, fmt.Errorf("starting claude: %w", err)
	}

	// Channel to signal first output received
	firstOutput := make(chan struct{}, 1)
	// Channel to signal parsing is done
	parseDone := make(chan struct{})

	var result *ClaudeResult

	// Parse stream-json output in a goroutine
	go func() {
		defer close(parseDone)
		result = parseStreamJSON(stdout, firstOutput)
	}()

	// Watchdog goroutine for timeouts
	startupTimeout := 90 * time.Second
	normalSeconds := config.MaxTurns * 120
	if normalSeconds < 600 {
		normalSeconds = 600
	}
	normalTimeout := time.Duration(normalSeconds) * time.Second
	if config.Timeout > 0 {
		normalTimeout = config.Timeout
	}

	watchdogDone := make(chan struct{})
	go func() {
		defer close(watchdogDone)
		watchdog(cmd.Process, firstOutput, startupTimeout, normalTimeout)
	}()

	// Wait for parsing to complete (stdout EOF)
	<-parseDone

	// Wait for process exit
	waitErr := cmd.Wait()

	// Signal watchdog to stop (process already exited)
	// The watchdog checks Process state, but we close its channel path
	// by having the process already exited.
	<-watchdogDone

	if result == nil {
		result = &ClaudeResult{}
	}

	// Capture exit code
	if waitErr != nil {
		if exitErr, ok := waitErr.(*exec.ExitError); ok {
			result.ExitCode = exitErr.ExitCode()
		} else {
			result.ExitCode = -1
		}
	}

	result.Stderr = stderrBuf.String()

	return result, nil
}

// parseStreamJSON reads stream-json lines from r and extracts a ClaudeResult.
// Signals firstOutput on the first successfully parsed line.
// Exported-friendly via the ParseStreamJSON wrapper for testing.
func parseStreamJSON(r io.Reader, firstOutput chan<- struct{}) *ClaudeResult {
	result := &ClaudeResult{}
	scanner := bufio.NewScanner(r)
	// Allow large lines (some assistant messages can be huge)
	scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)

	signaled := false
	var lastThinking string

	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" {
			continue
		}

		// Signal first output
		if !signaled {
			select {
			case firstOutput <- struct{}{}:
			default:
			}
			signaled = true
		}

		var event streamEvent
		if err := json.Unmarshal([]byte(line), &event); err != nil {
			continue // skip malformed lines
		}

		switch event.Type {
		case "system":
			// Check for MCP connection status
			if event.MCPServers != nil {
				for _, srv := range event.MCPServers {
					status := srv.Status
					if status == "connected" {
						result.MCPStatus = "connected"
					} else if result.MCPStatus != "connected" {
						// Only set failed if we haven't seen a connected status
						result.MCPStatus = "failed"
					}
				}
			}

		case "assistant":
			// Look for thinking blocks in message content
			if event.Message != nil {
				for _, block := range event.Message.Content {
					if block.Type == "thinking" && block.Thinking != "" {
						lastThinking = strings.TrimSpace(block.Thinking)
					}
				}
			}

		case "result":
			result.SessionID = event.SessionID
			result.CostUSD = event.TotalCostUSD
			result.NumTurns = event.NumTurns
			if event.DurationMS > 0 {
				result.Duration = time.Duration(event.DurationMS) * time.Millisecond
			}
		}
	}

	// Keep only the last thinking block
	result.Thinking = lastThinking

	return result
}

// ParseStreamJSON is the exported test wrapper for parseStreamJSON.
func ParseStreamJSON(r io.Reader) *ClaudeResult {
	ch := make(chan struct{}, 1)
	return parseStreamJSON(r, ch)
}

// streamEvent represents a single line from Claude Code's stream-json output.
type streamEvent struct {
	Type string `json:"type"`

	// For "system" events
	MCPServers []mcpServerStatus `json:"mcp_servers,omitempty"`

	// For "assistant" events
	Message *assistantMessage `json:"message,omitempty"`

	// For "result" events — field names match actual Claude CLI output
	SessionID    string  `json:"session_id,omitempty"`
	TotalCostUSD float64 `json:"total_cost_usd,omitempty"`
	NumTurns     int     `json:"num_turns,omitempty"`
	DurationMS   int64   `json:"duration_ms,omitempty"`
}

type mcpServerStatus struct {
	Name   string `json:"name"`
	Status string `json:"status"`
}

type assistantMessage struct {
	Content []contentBlock `json:"content"`
}

type contentBlock struct {
	Type     string `json:"type"`
	Text     string `json:"text,omitempty"`
	Thinking string `json:"thinking,omitempty"`
}

// watchdog monitors a Claude process and kills it on timeout.
// startupTimeout applies until firstOutput is signaled.
// normalTimeout applies after first output.
func watchdog(proc *os.Process, firstOutput <-chan struct{}, startupTimeout, normalTimeout time.Duration) {
	startupTimer := time.NewTimer(startupTimeout)
	defer startupTimer.Stop()

	// Wait for first output or startup timeout
	select {
	case <-firstOutput:
		// Good — got output. Switch to normal timeout.
		startupTimer.Stop()
	case <-startupTimer.C:
		// Startup timeout — kill the process
		killProcess(proc)
		return
	}

	// Normal timeout
	normalTimer := time.NewTimer(normalTimeout)
	defer normalTimer.Stop()

	// We need to detect process exit. Poll periodically.
	ticker := time.NewTicker(1 * time.Second)
	defer ticker.Stop()

	for {
		select {
		case <-normalTimer.C:
			killProcess(proc)
			return
		case <-ticker.C:
			// Check if process still exists by sending signal 0
			if err := proc.Signal(syscall.Signal(0)); err != nil {
				// Process has exited
				return
			}
		}
	}
}

// killProcess sends SIGTERM, waits 3 seconds, then SIGKILL.
func killProcess(proc *os.Process) {
	_ = proc.Signal(syscall.SIGTERM)
	done := make(chan struct{})
	go func() {
		// Wait for process exit
		_, _ = proc.Wait()
		close(done)
	}()
	select {
	case <-done:
		return
	case <-time.After(3 * time.Second):
		_ = proc.Signal(syscall.SIGKILL)
	}
}

// filterClaudeEnv removes CLAUDE_CODE_* and CLAUDECODE env vars from the
// environment slice. Prevents recursive agent detection when the orchestrator
// is itself running inside a Claude Code session.
func filterClaudeEnv(env []string) []string {
	filtered := make([]string, 0, len(env))
	for _, e := range env {
		key := e
		if idx := strings.IndexByte(e, '='); idx >= 0 {
			key = e[:idx]
		}
		if strings.HasPrefix(key, "CLAUDE_CODE_") || key == "CLAUDECODE" {
			continue
		}
		filtered = append(filtered, e)
	}
	return filtered
}

// WriteMCPConfig writes a JSON MCP server configuration file for a Claude
// subprocess. Returns the path to the written file.
func WriteMCPConfig(cliBinary, role, agentID, runID, dbPath string) (string, error) {
	dir := "/tmp/spore-orchestrator"
	if err := os.MkdirAll(dir, 0755); err != nil {
		return "", fmt.Errorf("creating MCP config dir: %w", err)
	}

	// Use first 8 chars of runID for filename (safe — ASCII hex)
	runIDShort := runID
	if len(runIDShort) > 8 {
		runIDShort = runIDShort[:8]
	}
	filename := fmt.Sprintf("mcp-%s-%s.json", role, runIDShort)
	path := filepath.Join(dir, filename)

	config := mcpConfig{
		MCPServers: map[string]mcpServerDef{
			"mycelica": {
				Command: cliBinary,
				Args: []string{
					"mcp-server",
					"--agent-role", role,
					"--agent-id", agentID,
					"--run-id", runID,
					"--db", dbPath,
				},
				Type: "stdio",
			},
		},
	}

	data, err := json.MarshalIndent(config, "", "  ")
	if err != nil {
		return "", fmt.Errorf("marshaling MCP config: %w", err)
	}

	if err := os.WriteFile(path, data, 0644); err != nil {
		return "", fmt.Errorf("writing MCP config to %s: %w", path, err)
	}

	return path, nil
}

type mcpConfig struct {
	MCPServers map[string]mcpServerDef `json:"mcpServers"`
}

type mcpServerDef struct {
	Command string   `json:"command"`
	Args    []string `json:"args"`
	Type    string   `json:"type"`
}

// SelectModelForRole returns the default model for a pipeline role.
// Opus for coder (A/B validated: 39% cheaper per-task than Sonnet due to fewer turns).
// Opus for verifier (accuracy matters). Sonnet for summarizer (sufficient quality).
func SelectModelForRole(role AgentRole) string {
	switch role {
	case RoleCoder:
		return "claude-opus-4-6"
	case RoleVerifier:
		return "claude-opus-4-6"
	case RoleSummarizer:
		return "claude-sonnet-4-6"
	case RoleOperator:
		return "claude-opus-4-6"
	default:
		return "claude-sonnet-4-6"
	}
}

// EstimateComplexity returns a 0-10 heuristic complexity score for a task
// description. Used for logging and budget decisions, not routing.
func EstimateComplexity(task string) int {
	score := 3 // baseline

	// Length penalty: +1 per 100 chars over 200
	if len(task) > 200 {
		extra := (len(task) - 200) / 100
		score += extra
	}

	lower := strings.ToLower(task)

	if strings.Contains(lower, "refactor") || strings.Contains(lower, "architecture") {
		score++
	}
	if strings.Contains(lower, "test") && strings.Contains(lower, "implement") {
		score++
	}
	if strings.Contains(lower, "multiple files") || strings.Contains(lower, "across") {
		score++
	}

	if score > 10 {
		score = 10
	}
	return score
}

// ResolveAgentName checks if a .claude/agents/<role>.md file exists by
// searching upward from workDir. Returns the role name if found, empty string
// if not. This matches git's directory-walking convention.
func ResolveAgentName(role AgentRole, workDir string) string {
	dir := workDir
	for {
		agentFile := filepath.Join(dir, ".claude", "agents", string(role)+".md")
		if _, err := os.Stat(agentFile); err == nil {
			return string(role)
		}

		parent := filepath.Dir(dir)
		if parent == dir {
			break // reached filesystem root
		}
		dir = parent
	}
	return ""
}

// cappedBuffer is a bytes.Buffer that stops writing after a byte limit.
// Used to capture stderr without unbounded memory growth.
type cappedBuffer struct {
	mu    sync.Mutex
	buf   bytes.Buffer
	limit int
}

func (c *cappedBuffer) Write(p []byte) (int, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	remaining := c.limit - c.buf.Len()
	if remaining <= 0 {
		return len(p), nil // pretend we wrote it all
	}
	toWrite := p
	if len(toWrite) > remaining {
		toWrite = toWrite[:remaining]
	}
	_, err := c.buf.Write(toWrite)
	// Always report full input length to satisfy io.Writer contract
	// (cmd.Stderr expects all bytes accepted)
	return len(p), err
}

func (c *cappedBuffer) String() string {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.buf.String()
}
