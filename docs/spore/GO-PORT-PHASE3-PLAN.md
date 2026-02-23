# Spore Go Port — Phase 3: Orchestration

**Status**: Plan (draft)
**Depends on**: Phase 1 (analyzer) ✅, Phase 2 (context compilation) ✅
**Replaces**: `spore.rs:handle_orchestrate` (1,092 lines) + `spore.rs:generate_task_file` (623 lines)
**Estimated lines**: ~1,200-1,500 Go (vs 1,715 Rust — same logic, no copy-paste)
**CLI**: `spore orchestrate <task-node-id> [flags]`

## What This Phase Does

Phase 3 ports the core agent pipeline: the thing that takes a task node, compiles context for it, spawns Claude Code agents (coder → verifier → summarizer), captures results, and writes outcomes back to the graph. This is the reason Spore exists — everything else (analyzer, context compilation) is infrastructure supporting this loop.

The Rust implementation is a 1,092-line function (`handle_orchestrate`) that bundles all three agent phases, verdict parsing, git operations, stderr capture, thinking log writes, and status tracking into a single function. The Go port decomposes this into discrete functions that each do one thing.

## What Already Exists (Phase 1-2)

Phase 3 doesn't start from scratch. The hard algorithmic work is done:

| Capability | Where | Phase |
|-----------|-------|-------|
| Dijkstra context expansion | `internal/db/context.go` | 2 |
| Embedding reads (BLOB → float32) | `internal/db/embeddings.go` | 2 |
| Cosine similarity + FindSimilar | `internal/graph/similarity.go` | 2 |
| FTS5 search + query preprocessing | `internal/db/search.go` | 2 |
| Node resolution (ID/prefix/FTS) | `cmd/root.go:ResolveNode()` | 2 |
| Graph snapshot + topology | `internal/graph/` | 1 |
| DB connection + WAL mode | `internal/db/db.go` | 1 |
| Full node/edge models (23+14 fields) | `internal/db/models.go` | 1 |

What's new in Phase 3: task file generation (markdown templating), Claude Code spawning (process management), result parsing (structured output extraction), graph writes (shelling out to `mycelica-cli`), and the orchestration loop that ties them together.

## Architecture

### Rust Monolith vs Go Decomposition

```
RUST (spore.rs)                          GO (spore/)
─────────────────                        ──────────────────────────
handle_orchestrate() — 1,092 lines  →    orchestrate.go:
                                           RunOrchestration()    ~80 lines   (coordinator)
                                           runCoder()            ~150 lines  (coder phase)
                                           runVerifier()         ~150 lines  (verifier phase)
                                           runSummarizer()       ~120 lines  (summarizer phase)

generate_task_file() — 623 lines    →    taskfile.go:
                                           GenerateTaskFile()    ~200 lines  (context + template)
                                           buildContextSection() ~80 lines
                                           buildSimilarSection() ~60 lines
                                           renderMarkdown()      ~40 lines

(scattered across spore.rs)         →    claude.go:
                                           SpawnClaude()         ~100 lines  (process management)
                                           parseResult()         ~60 lines   (output extraction)
                                           captureThinking()     ~40 lines   (thinking log)

(5 copies in spore_runs.rs)         →    status.go:
                                           DetermineRunStatus()  ~80 lines   (ONE function)
                                           ExtractCosts()        ~40 lines   (ONE function)

(2 copies in spore.rs)              →    git.go:
                                           CaptureGitState()     ~30 lines   (ONE function)
                                           CheckDirty()          ~20 lines
```

Total: ~1,250 lines across 6 files, vs 1,715 lines in 2 Rust functions. Same logic, zero copy-paste.

### Package Structure

```
spore/
├── cmd/
│   ├── orchestrate.go          — CLI command and flag parsing
│   ├── ...existing...
├── internal/
│   ├── db/
│   │   ├── ...existing...
│   │   ├── writes.go           — Graph write operations via mycelica-cli
│   │   └── writes_test.go
│   ├── graph/
│   │   └── ...existing...
│   └── orchestrate/
│       ├── orchestrate.go      — RunOrchestration() coordinator
│       ├── taskfile.go         — GenerateTaskFile() context assembly + markdown
│       ├── claude.go           — SpawnClaude() process management
│       ├── status.go           — DetermineRunStatus(), ExtractCosts()
│       ├── git.go              — CaptureGitState()
│       ├── types.go            — OrchestrationConfig, RunResult, AgentPhase, etc
│       ├── orchestrate_test.go
│       ├── taskfile_test.go
│       ├── claude_test.go
│       └── status_test.go
```

New package: `internal/orchestrate`. Keeps orchestration logic separate from DB and graph packages. The orchestrate package calls into `db` for reads and `db.writes` for graph mutations.

## Core Data Flow

```
┌─────────────────────────────────────────────────────────┐
│  spore orchestrate <task-node-id>                       │
│                                                         │
│  1. ResolveNode(taskID) → task node                     │
│  2. GenerateTaskFile(task) → /tmp/spore-task-XXXXX.md   │
│     ├── ContextForTask(task.ID, budget=500)             │  ← Phase 2
│     ├── FindSimilar(task.embedding, top=10)             │  ← Phase 2
│     ├── SearchNodes(task.title)                         │  ← Phase 2
│     └── renderMarkdown(context + similar + search)      │
│  3. CaptureGitState() → dirty files, branch, commit    │
│  4. runCoder(taskFile, gitState)                        │
│     ├── SpawnClaude(role="coder", prompt=taskFile)      │
│     ├── parseResult(stdout) → code changes              │
│     └── WriteRunNode(result) → mycelica-cli             │  ← NEW
│  5. runVerifier(coderResult)                            │
│     ├── SpawnClaude(role="verifier", prompt=diff)       │
│     ├── parseVerdict(stdout) → pass/fail/partial        │
│     └── WriteRunNode(result) → mycelica-cli             │
│  6. runSummarizer(coderResult, verifierResult)          │
│     ├── SpawnClaude(role="summarizer", prompt=outcomes)  │
│     ├── parseSummary(stdout) → knowledge nodes          │
│     └── WriteMetaNodes(nodes) → mycelica-cli            │
│  7. UpdateTaskStatus(task, finalVerdict)                 │
└─────────────────────────────────────────────────────────┘
```

## Detailed Component Specs

### 1. Task File Generation (`taskfile.go`)

Ports `generate_task_file()` (623 lines). Phase 2 already ported the expensive parts (Dijkstra, embeddings, FTS). What remains is assembly and markdown rendering.

```go
type TaskFileConfig struct {
    ContextBudget   int     // default 500
    SimilarTopN     int     // default 10
    SimilarMinScore float32 // default 0.3
    SearchLimit     int     // default 20
    MaxHops         int     // default 6
    MaxCost         float64 // default 3.0
}

func GenerateTaskFile(d *db.DB, taskNode *db.Node, config *TaskFileConfig) (string, error)
```

**Sections in the generated markdown:**

1. **Task header**: Title, ID, node type, creation date, parent chain
2. **Task content**: Full content of the task node
3. **Graph context** (from Dijkstra): Top-N relevant nodes by graph distance, with path traces showing how they connect to the task. Format: rank, title, distance, relevance score, connection path
4. **Semantic neighbors** (from embeddings): Nodes with similar content that may not be graph-connected. Catches related work the graph structure misses
5. **Search results** (from FTS): Keyword matches on the task title. Catches nodes with matching terminology but different embeddings
6. **Git state**: Current branch, dirty files, last commit. Gives the coder agent project context

The Rust version interleaves context gathering with rendering. The Go version separates them: gather all data first, then render once. This makes testing straightforward — you can test the gathering and rendering independently.

### 2. Claude Code Spawning (`claude.go`)

Ports the `spawn_claude` pattern (3-4 copies in spore.rs, now one function).

```go
type ClaudeConfig struct {
    Role            string   // "coder", "verifier", "summarizer"
    Prompt          string   // task file path or inline prompt
    WorkDir         string   // project root
    MaxTurns        int      // default 50 for coder, 20 for verifier/summarizer
    AllowedTools    []string // per-role tool permissions
    MCPConfig       string   // path to MCP config TOML (optional)
    SkipPermissions bool     // --dangerously-skip-permissions (always true for now)
    SessionID       string   // for resume (empty = new session)
    OutputFormat    string   // "json" for structured parsing
}

type ClaudeResult struct {
    SessionID   string
    Stdout      string
    Stderr      string
    ExitCode    int
    DurationMs  int64
    TokensIn    int    // parsed from JSON output
    TokensOut   int
    CostUSD     float64
    Thinking    string // extracted thinking blocks
}

func SpawnClaude(config ClaudeConfig) (*ClaudeResult, error)
```

**Implementation**: `exec.Command("claude", args...)` with:
- `--output-format json` for structured output
- `--print` for headless mode (or `-p`)
- `--dangerously-skip-permissions` (by design — agents need full tool access)
- `--max-turns N` per role
- Timeout via `context.WithTimeout` (configurable, default 30 min for coder, 10 min for verifier/summarizer)
- Stderr capture with **safe truncation** using `utf8.Valid` check + boundary walk (fixing the Rust UTF-8 bug at 2 of 3 sites)

**Tool permissions per role** (from Rust `mcp.rs` permission model):

| Role | Tools |
|------|-------|
| coder | All (read/write files, bash, mcp) |
| verifier | Read files, bash (no writes), mcp read-only |
| summarizer | MCP read + write meta nodes, no filesystem |

### 3. Orchestration Coordinator (`orchestrate.go`)

Ports `handle_orchestrate()` (1,092 lines → ~80 lines coordinator + phase functions).

```go
type OrchestrationConfig struct {
    TaskFile     TaskFileConfig
    Coder        ClaudeConfig
    Verifier     ClaudeConfig
    Summarizer   ClaudeConfig
    SkipVerifier bool   // --no-verify
    SkipSummary  bool   // --no-summarize
    DryRun       bool   // generate task file only, don't spawn agents
    OutputDir    string // where to write task files and logs
}

type OrchestrationResult struct {
    TaskNode       *db.Node
    TaskFilePath   string
    CoderResult    *PhaseResult
    VerifierResult *PhaseResult
    SummaryResult  *PhaseResult
    FinalStatus    RunStatus   // success, partial, failed, cancelled
    TotalCostUSD   float64
    TotalDurationMs int64
    RunNodeID      string      // the node created to track this run
}

type PhaseResult struct {
    Phase      AgentPhase // coder, verifier, summarizer
    Claude     *ClaudeResult
    Status     PhaseStatus // success, failed, skipped, timeout
    NodesCreated []string  // IDs of nodes written to graph
    EdgesCreated []string  // IDs of edges written to graph
}

func RunOrchestration(d *db.DB, taskID string, config *OrchestrationConfig) (*OrchestrationResult, error)
```

**The coordinator is simple because each phase is self-contained:**

```
RunOrchestration:
  1. Resolve task node
  2. Generate task file (or exit if --dry-run)
  3. Capture git state
  4. Create run tracking node in graph
  5. Run coder phase
     - If failed → mark run failed, return
  6. Run verifier phase (unless --no-verify)
     - If failed → mark run as "needs review"
  7. Run summarizer phase (unless --no-summarize)
  8. Compute final status
  9. Update run tracking node with results
  10. Return result
```

Each phase function (runCoder, runVerifier, runSummarizer) handles its own prompt construction, spawning, result parsing, and graph writes. The coordinator just sequences them and handles early termination.

### 4. Graph Writes (`internal/db/writes.go`)

Phase 1-2 were read-only. Phase 3 introduces writes. Strategy: **shell out to `mycelica-cli`**.

Why not direct SQLite writes? Because `mycelica-cli` handles:
- Embedding generation (calls the Rust embedding model)
- FTS index updates
- Hierarchy recalculation triggers
- Timestamp management
- ID generation (UUID v4)

Writing directly to SQLite would skip all of these and leave the database in an inconsistent state. The CLI is the write API.

```go
// Creates a node via mycelica-cli and returns the new node ID
func (d *DB) CreateNode(title, content, nodeType string, opts CreateNodeOpts) (string, error)

// Creates an edge via mycelica-cli
func (d *DB) CreateEdge(sourceID, targetID, edgeType string, opts CreateEdgeOpts) (string, error)

// Updates a node's content or metadata
func (d *DB) UpdateNode(nodeID string, opts UpdateNodeOpts) error

type CreateNodeOpts struct {
    ParentID  *string
    AgentID   *string
    NodeClass *string  // "knowledge", "meta", "operational"
    MetaType  *string  // "summary", "contradiction", "status"
    Tags      []string
}

type CreateEdgeOpts struct {
    Content    *string
    Confidence *float64
    AgentID    *string
    Reason     *string
    Supersedes *string
}
```

**Implementation**: Each function builds a `mycelica-cli` command, executes it, parses the output for the created ID. Example:

```go
func (d *DB) CreateNode(title, content, nodeType string, opts CreateNodeOpts) (string, error) {
    args := []string{"node", "create", "--type", nodeType, "--title", title}
    if content != "" {
        args = append(args, "--content", content)
    }
    if opts.ParentID != nil {
        args = append(args, "--parent", *opts.ParentID)
    }
    if opts.AgentID != nil {
        args = append(args, "--agent", *opts.AgentID)
    }
    // ... etc

    cmd := exec.Command("mycelica-cli", args...)
    output, err := cmd.CombinedOutput()
    if err != nil {
        return "", fmt.Errorf("mycelica-cli node create: %w\n%s", err, output)
    }
    return parseCreatedID(output)
}
```

**Error handling**: CLI failures return the stderr message. The orchestrator decides whether to retry (transient failure) or abort (permanent failure like "node not found").

### 5. Run Status Determination (`status.go`)

Ports the 5 copy-pasted status blocks (400 wasted lines → one 80-line function).

```go
type RunStatus string
const (
    StatusSuccess  RunStatus = "success"
    StatusPartial  RunStatus = "partial"
    StatusFailed   RunStatus = "failed"
    StatusTimeout  RunStatus = "timeout"
    StatusCancelled RunStatus = "cancelled"
)

// DetermineRunStatus checks the run tracking node's edges to determine overall status.
// The Rust version has 5 identical copies of this logic across different view functions.
func DetermineRunStatus(d *db.DB, runNodeID string) (RunStatus, error)
```

**Logic** (extracted from the repeated Rust pattern):
1. Get all `derives_from` edges where run node is target → these are phase result nodes
2. For each phase result, check for `supports` edge with verdict content
3. If any phase has a `cancels` edge → `cancelled`
4. If all phases have passing verdicts → `success`
5. If coder passed but verifier failed → `partial`
6. If coder failed → `failed`
7. If any phase timed out → `timeout`

### 6. Git State Capture (`git.go`)

Ports the 2 identical blocks (24 wasted lines → one 30-line function).

```go
type GitState struct {
    Branch      string
    CommitHash  string
    CommitMsg   string
    DirtyFiles  []string
    UntrackedFiles []string
    IsDirty     bool
}

func CaptureGitState(repoDir string) (*GitState, error)
```

Uses `exec.Command("git", ...)` for `branch --show-current`, `rev-parse HEAD`, `log -1 --format=%s`, `diff --name-only`, `ls-files --others --exclude-standard`.

## CLI Interface

```
spore orchestrate <task-node-id> [flags]

Flags:
  --budget int          Context compilation budget (default 500)
  --no-verify           Skip verifier phase
  --no-summarize        Skip summarizer phase
  --dry-run             Generate task file only, don't spawn agents
  --max-turns int       Max turns for coder agent (default 50)
  --timeout duration    Per-phase timeout (default 30m)
  --output-dir string   Directory for task files and logs (default /tmp/spore/)
  --json                JSON output for pipeline integration
  --verbose             Show agent stdout in real-time
```

The `--dry-run` flag is critical for development: it generates the task file and prints it without spending API tokens. This is how you iterate on context compilation quality without running the full pipeline.

## Error Handling Philosophy

Phase 1-2 were read-only diagnostics. Errors meant "abort and show message." Phase 3 is a multi-step write pipeline. The error handling is different:

| Error type | Strategy | Example |
|-----------|----------|---------|
| Task node not found | Abort immediately | Bad ID, deleted node |
| Context compilation fails | Continue with degraded context | FTS table missing, embedding gaps |
| Task file write fails | Abort (no point spawning without context) | Disk full, permissions |
| Claude spawn fails | Abort with actionable error | `claude` not in PATH, API key missing |
| Coder phase fails | Write failure status to graph, return | Agent error, timeout |
| Verifier phase fails | Mark as "needs manual review" | Flaky test, ambiguous verdict |
| Summarizer fails | Log warning, don't block success | Summary is nice-to-have |
| Graph write fails | Retry once, then log and continue | CLI crash, DB locked |

The key principle: **never lose work silently**. If a phase completes but the graph write fails, the agent output is still in the log files. The orchestrator reports exactly what succeeded and what failed.

## Cross-Validation Strategy

Phase 3 is harder to cross-validate than Phase 1-2 because it has side effects (spawning agents, writing to graph). The strategy:

1. **Task file generation**: Cross-validate against Rust. Same task node → same markdown output (modulo timestamp formatting). This is deterministic and testable.

2. **Claude spawning**: Not cross-validated (process management is implementation detail). Test via integration tests that mock the `claude` binary.

3. **Graph writes**: Cross-validate by running both Rust and Go orchestrators on the same task, comparing the resulting graph state (nodes created, edges created, types, content).

4. **Status determination**: Cross-validate against Rust. Same run node with same edges → same status. This is deterministic.

## Test Plan

### Unit Tests

```
taskfile_test.go:
  TestGenerateTaskFile_BasicOutput       — task node → valid markdown with all sections
  TestGenerateTaskFile_EmptyContext       — no Dijkstra results → still generates (with warning)
  TestGenerateTaskFile_NoEmbedding        — task without embedding → skips similarity section
  TestGenerateTaskFile_GitState           — git state included in output
  TestGenerateTaskFile_LongContent        — large task content handled without truncation

claude_test.go:
  TestSpawnClaude_MockBinary             — mock claude binary, verify args passed correctly
  TestSpawnClaude_Timeout                — context cancellation after timeout
  TestSpawnClaude_StderrTruncation       — UTF-8 safe truncation (the bug we're fixing)
  TestSpawnClaude_JSONParsing            — structured output → ClaudeResult fields
  TestSpawnClaude_ExitCodeHandling       — non-zero exit → error with stderr

status_test.go:
  TestDetermineStatus_AllPass            — all phases pass → success
  TestDetermineStatus_CoderFail          — coder fails → failed
  TestDetermineStatus_VerifierFail       — verifier fails → partial
  TestDetermineStatus_Timeout            — phase timeout → timeout
  TestDetermineStatus_Cancelled          — cancellation edge → cancelled
  TestDetermineStatus_NoPhaseResults     — empty run → failed

orchestrate_test.go:
  TestOrchestration_DryRun               — dry run generates task file, no agents spawned
  TestOrchestration_SkipVerifier         — --no-verify skips verifier phase
  TestOrchestration_CoderFailAborts      — coder failure stops pipeline
  TestOrchestration_SummarizerFailOk     — summarizer failure doesn't block success
```

### Integration Tests (require `claude` and `mycelica-cli` in PATH)

```
TestOrchestration_EndToEnd              — real task node → agents spawn → graph updated
TestTaskFile_CrossValidation            — compare Go vs Rust task file output
TestStatus_CrossValidation              — compare Go vs Rust status determination
```

## Migration Notes

### What changes for Claude Code CLI

After Phase 3 ships, `spore orchestrate` replaces the current Rust orchestration. The transition:

1. Deploy Go binary alongside Rust binary (they share the same DB)
2. Run both on the same tasks, compare outputs
3. Once confident, switch `.claude/agents/` to invoke Go binary
4. Eventually remove orchestration code from `spore.rs` (the 1,092 + 623 lines)

The Rust MCP server, schema operations, and CLI commands remain — only the orchestration loop moves to Go.

### What NOT to port

Some things in `spore.rs` that should NOT be carried over:

- 246 `println!` calls scattered through logic — Go uses structured logging or just returns data
- Inline formatting mixed with computation — Go separates data flow from presentation
- `map_err(|e| e.to_string())` boilerplate (55+ copies) — Go's `fmt.Errorf("context: %w", err)` is cleaner
- The 37 copies of `&id[..8.min(id.len())]` — Go has `truncID()` already (one function, called everywhere)
- The unsafe byte-slice stderr truncation (2 of 3 sites) — Go version does UTF-8 safe truncation from day one

## Decisions to Make

1. **Async or sequential phases?** The Rust version runs phases sequentially. Could run coder + verifier in parallel if verifier operates on the diff rather than live state. Start sequential, optimize later if needed.

2. **Session resume?** Claude Code supports `--resume <session-id>`. Should the orchestrator resume sessions on retry, or start fresh? Start fresh — session resume adds state management complexity for marginal token savings.

3. **MCP vs CLI for graph writes?** Current plan: shell out to `mycelica-cli`. Alternative: use MCP server directly. MCP is cleaner but adds a dependency on the MCP server being running. CLI is self-contained. Start with CLI, migrate to MCP in Phase 5.

4. **Thinking log persistence?** The Rust version writes thinking blocks to `/tmp/spore-thinking/`. Worth keeping? Yes — thinking logs are the audit trail for why agents made decisions. Write to `--output-dir/thinking/`.

5. **How to handle the verifier verdict?** The Rust version parses a structured verdict from verifier output. The Go version should define a clear verdict schema (JSON) and include it in the verifier's prompt template. Ambiguous verdicts default to "needs review" rather than guessing.

## Dependencies

No new external dependencies. Still just:
- `modernc.org/sqlite` — pure Go SQLite
- `github.com/spf13/cobra` — CLI framework

External tool dependencies (must be in PATH):
- `claude` — Claude Code CLI
- `mycelica-cli` — Mycelica CLI for graph writes
- `git` — for git state capture
