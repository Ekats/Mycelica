package orchestrate

import "time"

// AgentRole identifies the pipeline stage
type AgentRole string

const (
	RoleCoder      AgentRole = "coder"
	RoleVerifier   AgentRole = "verifier"
	RoleSummarizer AgentRole = "summarizer"
	RoleOperator   AgentRole = "operator"
)

func (r AgentRole) String() string { return string(r) }

// Verdict from verifier analysis
type Verdict int

const (
	VerdictUnknown     Verdict = iota
	VerdictSupports            // implementation passes verification
	VerdictContradicts         // implementation fails verification
)

func (v Verdict) String() string {
	switch v {
	case VerdictSupports:
		return "supports"
	case VerdictContradicts:
		return "contradicts"
	default:
		return "unknown"
	}
}

// VerifierVerdict is the structured output from verdict detection
type VerifierVerdict struct {
	Verdict    Verdict `json:"verdict"`
	Reason     string  `json:"reason"`
	Confidence float64 `json:"confidence"`
}

// RunStatus is the outcome of an orchestration run
type RunStatus string

const (
	StatusSuccess   RunStatus = "success"
	StatusPartial   RunStatus = "partial"
	StatusFailed    RunStatus = "failed"
	StatusTimeout   RunStatus = "timeout"
	StatusCancelled RunStatus = "cancelled"
)

// ClaudeConfig configures a Claude Code subprocess
type ClaudeConfig struct {
	Role          AgentRole
	Prompt        string
	Model         string
	MaxTurns      int
	AllowedTools  string // comma-separated
	DisallowTools string // comma-separated
	MCPConfig     string // path to MCP JSON config
	AgentName     string // .claude/agents/<name>.md
	ResumeID      string // session ID to resume
	Timeout       time.Duration
	WorkDir       string
	Verbose       bool
}

// ClaudeResult captures output from a Claude Code subprocess
type ClaudeResult struct {
	ExitCode  int           `json:"exit_code"`
	SessionID string        `json:"session_id"`
	CostUSD   float64       `json:"cost_usd"`
	NumTurns  int           `json:"num_turns"`
	Duration  time.Duration `json:"duration"`
	Thinking  string        `json:"thinking"`  // last thinking block
	MCPStatus string        `json:"mcp_status"` // connected/failed/none
	Stderr    string        `json:"stderr"`
}

// TaskFileConfig controls task file generation parameters
type TaskFileConfig struct {
	Budget     int     // Dijkstra relevance budget (default 7)
	MaxAnchors int     // max anchor nodes from search (default 5)
	SimilarTop int     // top-N for embedding similarity (default 10)
	Threshold  float64 // minimum similarity threshold (default 0.3)
	MaxHops    int     // Dijkstra max hops (default 4)
	MaxCost    float64 // Dijkstra max edge cost (default 2.0)
	MaxLessons int     // max lessons from past runs (default 5)
}

// DefaultTaskFileConfig returns production defaults matching the Rust implementation
func DefaultTaskFileConfig() TaskFileConfig {
	return TaskFileConfig{
		Budget:     7,
		MaxAnchors: 5,
		SimilarTop: 10,
		Threshold:  0.3,
		MaxHops:    4,
		MaxCost:    2.0,
		MaxLessons: 5,
	}
}

// OrchestrationConfig controls the full orchestration run
type OrchestrationConfig struct {
	TaskFile    TaskFileConfig
	MaxBounces  int    // max coder→verifier bounces (default 3)
	MaxTurns    int    // max Claude turns per agent (default 50)
	CoderModel  string // override coder model
	OutputDir   string // task file output dir (default /tmp/spore/)
	Experiment  string // A/B experiment label
	DryRun      bool   // generate task file only, don't spawn agents
	NoSummarize bool   // skip summarizer after verification
	Verbose     bool
	Quiet       bool
	JSON        bool
}

// DefaultOrchestrationConfig returns production defaults
func DefaultOrchestrationConfig() OrchestrationConfig {
	return OrchestrationConfig{
		TaskFile:   DefaultTaskFileConfig(),
		MaxBounces: 3,
		MaxTurns:   50,
		OutputDir:  "/tmp/spore/",
	}
}

// PhaseResult captures the outcome of one pipeline phase (coder/verifier/summarizer)
type PhaseResult struct {
	Role         AgentRole       `json:"role"`
	Claude       *ClaudeResult   `json:"claude"`
	ImplNodeID   string          `json:"impl_node_id,omitempty"`
	Verdict      *VerifierVerdict `json:"verdict,omitempty"`
	ChangedFiles []string        `json:"changed_files,omitempty"`
}

// OrchestrationResult is the full outcome of an orchestration run
type OrchestrationResult struct {
	TaskNodeID string         `json:"task_node_id"`
	RunID      string         `json:"run_id"`
	Bounces    int            `json:"bounces"`
	Verdict    Verdict        `json:"verdict"`
	Status     RunStatus      `json:"status"`
	TotalCost  float64        `json:"total_cost_usd"`
	Phases     []PhaseResult  `json:"phases"`
}

// GitState captures repository state before/after an agent run
type GitState struct {
	Branch    string            `json:"branch"`
	Commit    string            `json:"commit"`
	Dirty     map[string]bool   `json:"dirty"`     // modified tracked files
	Untracked map[string]bool   `json:"untracked"` // untracked files
	Hashes    map[string]string `json:"hashes"`    // file content hashes
}
