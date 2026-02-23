package orchestrate

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestSelectModelForRole(t *testing.T) {
	tests := []struct {
		role AgentRole
		want string
	}{
		{RoleCoder, "claude-opus-4-6"},
		{RoleVerifier, "claude-opus-4-6"},
		{RoleSummarizer, "claude-sonnet-4-6"},
		{RoleOperator, "claude-opus-4-6"},
		{AgentRole("unknown"), "claude-sonnet-4-6"},
	}

	for _, tt := range tests {
		t.Run(string(tt.role), func(t *testing.T) {
			got := SelectModelForRole(tt.role)
			if got != tt.want {
				t.Errorf("SelectModelForRole(%q) = %q, want %q", tt.role, got, tt.want)
			}
		})
	}
}

func TestEstimateComplexity(t *testing.T) {
	tests := []struct {
		name    string
		task    string
		wantMin int
		wantMax int
	}{
		{
			name:    "short simple task",
			task:    "Fix the typo in README",
			wantMin: 3,
			wantMax: 3,
		},
		{
			name:    "medium task with refactor",
			task:    "Refactor the database module to use connection pooling",
			wantMin: 4,
			wantMax: 5,
		},
		{
			name:    "complex task with multiple signals",
			task:    "Implement tests across multiple files for the new architecture refactor of the orchestration system. The implementation needs to handle edge cases in the verification pipeline and ensure backward compatibility.",
			wantMin: 6,
			wantMax: 10,
		},
		{
			name:    "very long task",
			task:    strings.Repeat("a", 800),
			wantMin: 6, // 3 base + (800-200)/100 = 3+6 = 9
			wantMax: 10,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := EstimateComplexity(tt.task)
			if got < tt.wantMin || got > tt.wantMax {
				t.Errorf("EstimateComplexity(%q) = %d, want [%d, %d]", tt.task[:min(len(tt.task), 50)], got, tt.wantMin, tt.wantMax)
			}
		})
	}
}

func TestEstimateComplexity_Cap(t *testing.T) {
	// Even a maximally complex task should cap at 10
	task := strings.Repeat("refactor architecture implement test across multiple files ", 50)
	got := EstimateComplexity(task)
	if got > 10 {
		t.Errorf("EstimateComplexity should cap at 10, got %d", got)
	}
}

func TestResolveAgentName(t *testing.T) {
	// The Mycelica project has .claude/agents/coder.md etc.
	projectRoot := "/home/spore/Mycelica"

	// Check that coder agent file actually exists (test precondition)
	agentFile := filepath.Join(projectRoot, ".claude", "agents", "coder.md")
	if _, err := os.Stat(agentFile); err != nil {
		t.Skipf("Skipping: agent file not found at %s", agentFile)
	}

	tests := []struct {
		role AgentRole
		want string
	}{
		{RoleCoder, "coder"},
		{RoleVerifier, "verifier"},
		{RoleSummarizer, "summarizer"},
	}

	for _, tt := range tests {
		t.Run(string(tt.role), func(t *testing.T) {
			got := ResolveAgentName(tt.role, projectRoot)
			if got != tt.want {
				t.Errorf("ResolveAgentName(%q, %q) = %q, want %q", tt.role, projectRoot, got, tt.want)
			}
		})
	}
}

func TestResolveAgentName_Subdirectory(t *testing.T) {
	// Should find agents from a subdirectory by walking up
	projectRoot := "/home/spore/Mycelica"
	subDir := filepath.Join(projectRoot, "src-tauri", "src")

	agentFile := filepath.Join(projectRoot, ".claude", "agents", "coder.md")
	if _, err := os.Stat(agentFile); err != nil {
		t.Skipf("Skipping: agent file not found at %s", agentFile)
	}

	got := ResolveAgentName(RoleCoder, subDir)
	if got != "coder" {
		t.Errorf("ResolveAgentName from subdirectory = %q, want %q", got, "coder")
	}
}

func TestResolveAgentName_NotFound(t *testing.T) {
	got := ResolveAgentName(AgentRole("nonexistent"), "/tmp")
	if got != "" {
		t.Errorf("ResolveAgentName for nonexistent role = %q, want empty", got)
	}
}

func TestWriteMCPConfig(t *testing.T) {
	path, err := WriteMCPConfig(
		"/usr/local/bin/mycelica-cli",
		"coder",
		"spore:coder",
		"abcdef12-3456-7890-abcd-ef1234567890",
		"/home/user/.mycelica.db",
	)
	if err != nil {
		t.Fatalf("WriteMCPConfig failed: %v", err)
	}
	defer os.Remove(path)

	// Verify file was created
	if _, err := os.Stat(path); err != nil {
		t.Fatalf("MCP config file not found at %s: %v", path, err)
	}

	// Read and parse
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("Reading MCP config: %v", err)
	}

	var config mcpConfig
	if err := json.Unmarshal(data, &config); err != nil {
		t.Fatalf("Parsing MCP config JSON: %v", err)
	}

	// Verify structure
	mycelica, ok := config.MCPServers["mycelica"]
	if !ok {
		t.Fatal("MCP config missing 'mycelica' server")
	}

	if mycelica.Command != "/usr/local/bin/mycelica-cli" {
		t.Errorf("command = %q, want /usr/local/bin/mycelica-cli", mycelica.Command)
	}
	if mycelica.Type != "stdio" {
		t.Errorf("type = %q, want stdio", mycelica.Type)
	}

	// Verify args contain expected values
	argsStr := strings.Join(mycelica.Args, " ")
	for _, want := range []string{"mcp-server", "--agent-role", "coder", "--agent-id", "spore:coder", "--run-id", "--db"} {
		if !strings.Contains(argsStr, want) {
			t.Errorf("args missing %q: %v", want, mycelica.Args)
		}
	}
}

func TestWriteMCPConfig_FileLocation(t *testing.T) {
	path, err := WriteMCPConfig(
		"/bin/cli",
		"verifier",
		"spore:verifier",
		"12345678-abcd-ef01-2345-67890abcdef0",
		"/tmp/test.db",
	)
	if err != nil {
		t.Fatalf("WriteMCPConfig failed: %v", err)
	}
	defer os.Remove(path)

	// Should be in /tmp/spore-orchestrator/
	if !strings.HasPrefix(path, "/tmp/spore-orchestrator/") {
		t.Errorf("path = %q, want prefix /tmp/spore-orchestrator/", path)
	}

	// Should contain role and run ID prefix
	base := filepath.Base(path)
	if !strings.Contains(base, "verifier") {
		t.Errorf("filename %q should contain 'verifier'", base)
	}
	if !strings.Contains(base, "12345678") {
		t.Errorf("filename %q should contain run ID prefix '12345678'", base)
	}
}

func TestStreamJSONParsing_ResultEvent(t *testing.T) {
	input := `{"type":"result","session_id":"sess-abc-123","total_cost_usd":1.5,"num_turns":3,"duration_ms":5000}` + "\n"
	result := ParseStreamJSON(strings.NewReader(input))

	if result.SessionID != "sess-abc-123" {
		t.Errorf("SessionID = %q, want %q", result.SessionID, "sess-abc-123")
	}
	if result.CostUSD != 1.5 {
		t.Errorf("CostUSD = %f, want %f", result.CostUSD, 1.5)
	}
	if result.NumTurns != 3 {
		t.Errorf("NumTurns = %d, want %d", result.NumTurns, 3)
	}
	if result.Duration != 5*time.Second {
		t.Errorf("Duration = %v, want %v", result.Duration, 5*time.Second)
	}
}

func TestStreamJSONParsing_SystemAndResult(t *testing.T) {
	input := strings.Join([]string{
		`{"type":"system","mcp_servers":[{"name":"mycelica","status":"connected"}]}`,
		`{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"I need to analyze the code."}]}}`,
		`{"type":"result","session_id":"sess-xyz","total_cost_usd":2.75,"num_turns":8,"duration_ms":12000}`,
	}, "\n") + "\n"

	result := ParseStreamJSON(strings.NewReader(input))

	if result.MCPStatus != "connected" {
		t.Errorf("MCPStatus = %q, want %q", result.MCPStatus, "connected")
	}
	if result.Thinking != "I need to analyze the code." {
		t.Errorf("Thinking = %q, want %q", result.Thinking, "I need to analyze the code.")
	}
	if result.SessionID != "sess-xyz" {
		t.Errorf("SessionID = %q, want %q", result.SessionID, "sess-xyz")
	}
	if result.CostUSD != 2.75 {
		t.Errorf("CostUSD = %f, want %f", result.CostUSD, 2.75)
	}
	if result.NumTurns != 8 {
		t.Errorf("NumTurns = %d, want %d", result.NumTurns, 8)
	}
}

func TestStreamJSONParsing_EmptyAndMalformed(t *testing.T) {
	input := strings.Join([]string{
		"",
		"   ",
		"not json at all",
		`{"type":"unknown_event"}`,
		`{"broken json`,
		`{"type":"result","session_id":"ok","total_cost_usd":0.5,"num_turns":1,"duration_ms":1000}`,
	}, "\n") + "\n"

	result := ParseStreamJSON(strings.NewReader(input))

	// Should gracefully skip bad lines and parse the result
	if result.SessionID != "ok" {
		t.Errorf("SessionID = %q, want %q", result.SessionID, "ok")
	}
	if result.CostUSD != 0.5 {
		t.Errorf("CostUSD = %f, want %f", result.CostUSD, 0.5)
	}
}

func TestStreamJSONParsing_MCPFailed(t *testing.T) {
	input := `{"type":"system","mcp_servers":[{"name":"mycelica","status":"failed"}]}` + "\n"
	result := ParseStreamJSON(strings.NewReader(input))

	if result.MCPStatus != "failed" {
		t.Errorf("MCPStatus = %q, want %q", result.MCPStatus, "failed")
	}
}

func TestStreamJSONParsing_MultipleThinkingBlocks(t *testing.T) {
	// Should keep only the last thinking block
	input := strings.Join([]string{
		`{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"First thought."}]}}`,
		`{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Second thought."}]}}`,
		`{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Final thought."}]}}`,
		`{"type":"result","session_id":"s","total_cost_usd":0.1,"num_turns":1,"duration_ms":100}`,
	}, "\n") + "\n"

	result := ParseStreamJSON(strings.NewReader(input))

	if result.Thinking != "Final thought." {
		t.Errorf("Thinking = %q, want %q", result.Thinking, "Final thought.")
	}
}

func TestStreamJSONParsing_NoResult(t *testing.T) {
	// No result event — should return empty result without panic
	input := `{"type":"system"}` + "\n"
	result := ParseStreamJSON(strings.NewReader(input))

	if result.SessionID != "" {
		t.Errorf("SessionID should be empty, got %q", result.SessionID)
	}
	if result.CostUSD != 0 {
		t.Errorf("CostUSD should be 0, got %f", result.CostUSD)
	}
}

func TestFilterClaudeEnv(t *testing.T) {
	env := []string{
		"HOME=/home/user",
		"PATH=/usr/bin",
		"CLAUDECODE=1",
		"CLAUDE_CODE_SESSION=abc",
		"CLAUDE_CODE_SOMETHING_ELSE=xyz",
		"OTHER_VAR=keep",
	}

	filtered := filterClaudeEnv(env)

	// Should keep HOME, PATH, OTHER_VAR (3 items)
	if len(filtered) != 3 {
		t.Errorf("len(filtered) = %d, want 3; got: %v", len(filtered), filtered)
	}

	for _, e := range filtered {
		key := e
		if idx := strings.IndexByte(e, '='); idx >= 0 {
			key = e[:idx]
		}
		if key == "CLAUDECODE" || strings.HasPrefix(key, "CLAUDE_CODE_") {
			t.Errorf("should have filtered %q", e)
		}
	}
}

func TestCappedBuffer(t *testing.T) {
	var buf cappedBuffer
	buf.limit = 10

	n, err := buf.Write([]byte("hello"))
	if err != nil || n != 5 {
		t.Errorf("first write: n=%d, err=%v", n, err)
	}

	// Write more than remaining capacity
	n, err = buf.Write([]byte("world!!!"))
	if err != nil {
		t.Errorf("second write error: %v", err)
	}
	// Should report full length written (pretend success)
	if n != 8 {
		t.Errorf("second write n=%d, want 8", n)
	}

	// But buffer should only contain first 10 bytes
	got := buf.String()
	if len(got) != 10 {
		t.Errorf("buffer len = %d, want 10; content = %q", len(got), got)
	}
	if got != "helloworld" {
		t.Errorf("buffer = %q, want %q", got, "helloworld")
	}

	// Further writes should be silently dropped
	n, err = buf.Write([]byte("overflow"))
	if err != nil || n != 8 {
		t.Errorf("overflow write: n=%d, err=%v", n, err)
	}
	if buf.String() != "helloworld" {
		t.Errorf("buffer after overflow = %q", buf.String())
	}
}
