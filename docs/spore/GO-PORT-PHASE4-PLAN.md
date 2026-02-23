# Go Port Phase 4: Loop Mode + Analytics + spore.rs Kill

## Context

Phases 1-3 of the Go port are complete and on master:

| Phase | Commit | What | Lines | Tests |
|-------|--------|------|-------|-------|
| 1 | 3becfc9 | Analyzer (graph stats, hierarchy, clusters) | ~1,200 | 15 |
| 2 | f7d0f0b | Context-for-task (Dijkstra, FTS, embeddings, cosine) | ~2,200 | 36 |
| 3 | 9b46a25 | Orchestration pipeline (bounce loop, Claude spawning, verdict, task files) | ~3,006 | 132 |

The Go binary (`spore`) now handles: `analyze`, `context-for-task`, `orchestrate` (full bounce loop with coder → verifier → summarizer). All graph operations shell out to `mycelica-cli`.

**Goal of Phase 4:** Port everything remaining in `spore.rs` that the Go binary doesn't cover, so `spore.rs` orchestration code can be deleted entirely. After this phase, the `spore` alias points at the Go binary and the Rust `SporeCommands` variants for orchestration/analytics are removed.

`mycelica-cli` stays — it's the graph engine (DB, embeddings, FTS, MCP server, imports, hierarchy). Only the Spore-specific orchestration and analytics code dies.

---

## Step 0: Audit (DO THIS FIRST — NO CODE UNTIL COMPLETE)

Before writing any Go code, enumerate exactly what needs porting.

### 0a. Enumerate all SporeCommands variants

```bash
grep -n "SporeCommands\|^\s*[A-Z][a-zA-Z]*\s*{" src-tauri/src/bin/cli/spore.rs | head -80
```

Or more precisely:

```bash
# Find the SporeCommands enum and list all variants
sed -n '/^pub enum SporeCommands/,/^}/p' src-tauri/src/bin/cli/spore.rs | grep -E '^\s+[A-Z]'
```

### 0b. Cross-reference against Go

For each variant found, check if Go already handles it:

```bash
# List Go subcommands
grep -rn 'Use:.*"' /home/spore/Mycelica/spore/cmd/*.go | grep -v test
```

### 0c. Classify each variant

Produce a table like this:

| SporeCommands variant | Go equivalent | Status |
|-----------------------|---------------|--------|
| Orchestrate | `cmd/orchestrate.go` | ✅ Ported |
| Loop | — | ❌ Needs porting |
| Stats | — | ❌ Needs porting |
| Health | — | ❌ Needs porting |
| CreateEdge | stays in mycelica-cli | 🔧 Keep (Go shells out) |
| ReadContent | stays in mycelica-cli | 🔧 Keep (Go shells out) |
| ... | ... | ... |

### 0d. Check for dead/redundant code

```bash
# Functions only called from handle_orchestrate (now dead since Go replaces it)
grep -n "^pub fn\|^pub async fn\|^fn " src-tauri/src/bin/cli/spore.rs | head -60
```

For each function, check if it's called from anywhere other than the orchestration path:

```bash
grep -rn "function_name" src-tauri/src/bin/cli/spore.rs | grep -v "^.*fn function_name"
```

Functions only called from `handle_orchestrate` or `generate_task_file` are dead code once Go takes over.

### 0e. Measure what's left

```bash
wc -l src-tauri/src/bin/cli/spore.rs
```

After classifying: how many lines are "ported by Go" (deletable), how many are "stays in mycelica-cli" (keep), how many are "needs Phase 4 porting"?

**Report the full audit before proceeding to implementation.**

---

## Step 1: Types and Shared Infrastructure

**File:** `internal/analytics/types.go` (~80 lines)

```go
package analytics

import "time"

// RunRecord represents a single orchestration run from Tracks edge metadata
type RunRecord struct {
    RunID      string
    TaskNodeID string
    Agent      string
    Status     string    // success, partial, failed, timeout, cancelled
    ExitCode   int
    CostUSD    float64
    NumTurns   int
    DurationMs int64
    Model      string
    Experiment string
    Timestamp  time.Time
}

// HealthCheck represents a single health check result
type HealthCheck struct {
    Name    string
    Status  string // ok, warning, error
    Message string
    Count   int    // affected items, if applicable
}

// LessonRecord represents an extracted lesson from past runs
type LessonRecord struct {
    TaskTitle  string
    RunID      string
    Status     string
    Lesson     string  // extracted insight
    Quality    float64 // 0-1, from embedding similarity to task
    Timestamp  time.Time
}

// DistillRecord represents compressed knowledge from accumulated runs
type DistillRecord struct {
    Pattern    string   // recurring pattern across runs
    Frequency  int      // how many runs exhibited this
    Examples   []string // run IDs
    Confidence float64
}
```

---

## Step 2: Loop Mode

**Files:** `internal/loop/loop.go` + `cmd/loop.go` (~400-500 lines total)

This is the highest-priority command. Without it, you run tasks one at a time.

### loop.go (~350 lines)

```go
package loop

// RunLoop reads tasks and runs orchestration sequentially
func RunLoop(d *db.DB, config LoopConfig) (*LoopResult, error)

// ReadTasksFromFile parses a task list file (one task per line, # comments, blank lines skipped)
func ReadTasksFromFile(path string) ([]TaskEntry, error)

// ReadTasksFromGraph queries the graph for task nodes matching criteria
func ReadTasksFromGraph(d *db.DB, query string, limit int) ([]TaskEntry, error)

// ReadTasksFromStdin reads tasks from stdin (pipe-friendly)
func ReadTasksFromStdin() ([]TaskEntry, error)
```

**LoopConfig:**
- Source: file path, graph query, or stdin
- MaxConcurrent: 1 (sequential for now, but the field exists for future parallelism)
- StopOnFailure: bool (abort remaining tasks on first failure)
- DryRun: bool (generate task files only, don't spawn agents)
- OrchestrationConfig: embedded (max_bounces, max_turns, etc.)

**RunLoop flow:**
1. Read task list from source
2. For each task:
   a. Log: `[loop] Task {i}/{n}: {title}`
   b. Call `orchestrate.RunOrchestration(d, task, config.OrchestrationConfig)`
   c. Collect result
   d. If failed and StopOnFailure → abort with partial results
3. Print summary: total tasks, passed, failed, total cost, total duration

**Task file format** (simple, one task per line):

```
# Spore task list — comments start with #
# Blank lines ignored

Implement rate limiting for the API endpoint
Fix the off-by-one error in pagination
Add integration tests for the import pipeline
```

Or from graph: tasks are nodes with `node_class = 'operational'` and `meta_type = 'task'` that don't have a `Tracks` edge with `status = 'success'`.

### cmd/loop.go (~120 lines)

```
spore loop [flags]

Flags:
  --file <path>         Read tasks from file (one per line)
  --query <string>      Read tasks from graph (FTS query for task nodes)
  --stdin               Read tasks from stdin
  --stop-on-failure     Abort remaining tasks on first failure
  --dry-run             Generate task files only
  --max-bounces <int>   Override per-task max bounces (default: 3)
  --max-turns <int>     Override per-task max turns (default: 50)
  --verbose             Verbose output
  --json                JSON output for each task result
```

**Tests** (`loop_test.go` ~150 lines):
- TestReadTasksFromFile (comments, blank lines, whitespace trimming)
- TestReadTasksFromFile_Empty
- TestReadTasksFromFile_NonExistent
- TestRunLoop_DryRun (no agents spawned, task files generated)
- TestRunLoop_StopOnFailure (mock: 2nd task fails, 3rd not attempted)

---

## Step 3: Stats

**Files:** `internal/analytics/stats.go` + `cmd/stats.go` (~250-300 lines total)

### stats.go (~200 lines)

```go
// CollectRunStats gathers statistics from Tracks edges in the graph
func CollectRunStats(d *db.DB, since time.Time) (*RunStats, error)

// RunStats aggregates across all runs
type RunStats struct {
    TotalRuns     int
    ByStatus      map[string]int     // success: 12, failed: 3, ...
    TotalCostUSD  float64
    AvgCostUSD    float64
    TotalDuration time.Duration
    AvgDuration   time.Duration
    ByModel       map[string]int     // claude-opus-4-20250514: 8, claude-sonnet-4-20250514: 7
    ByExperiment  map[string]int
    RecentRuns    []RunRecord        // last 10
    CostPerDay    map[string]float64 // date → total cost
}
```

**Implementation:** Shell out to `mycelica-cli spore query-edges --type tracks --since <date> --json`, parse the metadata JSON from each edge, aggregate.

### cmd/stats.go (~80 lines)

```
spore stats [flags]

Flags:
  --since <duration>   Time window (default: 7d). Accepts: 1d, 7d, 30d, all
  --experiment <name>  Filter by experiment name
  --json               JSON output
```

**Output (human-readable):**

```
Spore Run Statistics (last 7 days)
──────────────────────────────────
Runs:     15 total (12 success, 2 failed, 1 timeout)
Cost:     $4.23 total, $0.28 avg/run
Duration: 2h 14m total, 8m 56s avg/run
Models:   claude-opus-4-20250514: 10, claude-sonnet-4-20250514: 5

Recent:
  [✓] Fix pagination (0m 42s, $0.12)
  [✓] Add rate limiting (3m 18s, $0.45)
  [✗] Refactor auth module (8m 02s, $0.89) — contradicts after 3 bounces
```

**Tests** (~80 lines):
- TestCollectRunStats_Empty
- TestCollectRunStats_WithData (fixture Tracks edges)
- TestCollectRunStats_FilterExperiment

---

## Step 4: Health

**Files:** `internal/analytics/health.go` + `cmd/health.go` (~200-250 lines total)

### health.go (~160 lines)

```go
// RunHealthChecks performs all health checks against the graph
func RunHealthChecks(d *db.DB) ([]HealthCheck, error)
```

**Checks:**
1. **Stale tasks** — task nodes with no Tracks edges (created but never run). Query: `mycelica-cli spore query-edges --type tracks` cross-referenced with task nodes.
2. **Orphaned impl nodes** — implementation nodes whose task node was deleted. Check: impl nodes with DerivesFrom edges pointing to non-existent targets.
3. **Missing embeddings** — nodes without embeddings that should have them. Query: `mycelica-cli embeddings status --json` or equivalent.
4. **DB integrity** — basic: can we connect, read, write? Query: `mycelica-cli db stats --json`.
5. **Claude availability** — is `claude` in PATH? Quick check: `which claude`.
6. **Disk space** — is there enough space for task files in `--output-dir`?

### cmd/health.go (~60 lines)

```
spore health [flags]

Flags:
  --json    JSON output
  --fix     Auto-fix what's fixable (delete orphaned nodes, etc.)
```

**Output:**

```
Spore Health Check
──────────────────
[✓] Database connection          OK
[✓] Claude CLI available         /home/spore/.local/bin/claude
[⚠] Stale task nodes             3 tasks never executed
[✓] Orphaned implementation      0 orphans
[✓] Embeddings coverage          98.2% (1594/1624)
[✓] Disk space                   45GB free
```

**Tests** (~60 lines):
- TestRunHealthChecks_AllOK (mock all passing)
- TestRunHealthChecks_StaleTask
- TestRunHealthChecks_NoClaude

---

## Step 5: Lessons

**Files:** `internal/analytics/lessons.go` + `cmd/lessons.go` (~200-250 lines total)

### lessons.go (~160 lines)

```go
// ExtractLessons finds patterns in past run outcomes
func ExtractLessons(d *db.DB, config LessonsConfig) ([]LessonRecord, error)

// LessonsConfig controls extraction
type LessonsConfig struct {
    Since      time.Time
    MinQuality float64 // minimum embedding similarity to include
    Limit      int     // max lessons to return
    Status     string  // filter: "failed", "success", "all"
}
```

**Implementation:**
1. Query all Tracks edges for runs in the time window
2. For failed runs: extract the verifier's reason from the verdict metadata
3. For successful runs: extract what changed (files modified, bounce count)
4. Group by pattern — similar failure reasons cluster into lessons
5. Rank by frequency and recency

This is read-only graph analysis. The "lessons" are stored as nodes in the graph (via `mycelica-cli node create --node-class meta --meta-type lesson`) so future task file generation can include them (Phase 3's taskfile.go already has `findLessons()`).

### cmd/lessons.go (~60 lines)

```
spore lessons [flags]

Flags:
  --since <duration>   Time window (default: 30d)
  --status <string>    Filter: failed, success, all (default: all)
  --limit <int>        Max lessons (default: 10)
  --save               Save extracted lessons as graph nodes
  --json               JSON output
```

**Tests** (~60 lines):
- TestExtractLessons_NoRuns
- TestExtractLessons_FailedOnly
- TestExtractLessons_Save (creates nodes)

---

## Step 6: Distill

**Files:** `internal/analytics/distill.go` + `cmd/distill.go` (~200-250 lines total)

### distill.go (~160 lines)

```go
// DistillKnowledge compresses accumulated run data into patterns
func DistillKnowledge(d *db.DB, config DistillConfig) ([]DistillRecord, error)

// DistillConfig controls the distillation
type DistillConfig struct {
    Since        time.Time
    MinFrequency int     // minimum occurrences to report
    Limit        int     // max patterns
}
```

**Implementation:**
1. Gather all lessons (call `ExtractLessons` with broad parameters)
2. Compute pairwise similarity between lesson texts (embeddings + cosine)
3. Cluster similar lessons into patterns
4. For each cluster: extract the common theme, count frequency, list example run IDs
5. Rank by frequency × recency

This is a higher-level aggregation over lessons. It answers "what keeps going wrong?" or "what consistently works?" across many runs.

### cmd/distill.go (~60 lines)

```
spore distill [flags]

Flags:
  --since <duration>     Time window (default: 90d)
  --min-frequency <int>  Minimum occurrences (default: 3)
  --limit <int>          Max patterns (default: 5)
  --save                 Save patterns as graph nodes
  --json                 JSON output
```

**Tests** (~50 lines):
- TestDistillKnowledge_NoLessons
- TestDistillKnowledge_SingleCluster
- TestDistillKnowledge_MultiplePatterns

---

## Step 7: Prompt-Stats (Optional — Low Priority)

**Files:** `internal/analytics/promptstats.go` + `cmd/promptstats.go` (~150 lines total)

Parse Claude's stream-JSON cost data from past runs to show token usage patterns. This is a nice-to-have dashboard, not critical. Defer if time is short.

```
spore prompt-stats [flags]

Flags:
  --since <duration>   Time window
  --by-role            Break down by agent role
  --json               JSON output
```

---

## Step 8: Wire Up Root Command + Delete Rust

### 8a. Add all new subcommands to root.go

```go
rootCmd.AddCommand(
    loopCmd,
    statsCmd,
    healthCmd,
    lessonsCmd,
    distillCmd,
    // promptStatsCmd,  // if implemented
)
```

### 8b. Verify complete coverage

Run the audit from Step 0 again. Every SporeCommands variant should now be either:
- ✅ Ported to Go
- 🔧 Staying in mycelica-cli (create-edge, read-content, query-edges, etc.)
- 🗑️ Dead code (only called from handle_orchestrate, which is replaced)

### 8c. Update the spore alias

```bash
# In .bashrc or .zshrc
alias spore='/home/spore/Mycelica/spore/spore'
```

### 8d. Delete Rust Spore orchestration code

In `src-tauri/src/bin/cli/spore.rs`:
- Remove `handle_orchestrate` function (~1,092 lines)
- Remove `generate_task_file` function (~623 lines)
- Remove `SporeCommands::Orchestrate` variant
- Remove `SporeCommands::Loop` variant (if it exists)
- Remove analytics command variants (Stats, Health, PromptStats, Lessons, Distill)
- Remove any helper functions that were only called from the above
- Keep: CreateEdge, ReadContent, CreateMeta, UpdateMeta, QueryEdges, ExplainEdge, PathBetween, EdgesForContext, Status, and any other graph-operation commands

### 8e. Rebuild and verify

```bash
cd src-tauri && cargo +nightly check --bin mycelica-cli --features mcp 2>&1
cargo +nightly test --bin mycelica-cli --features mcp 2>&1
cargo +nightly install --path . --bin mycelica-cli --features mcp --force
cp ~/.cargo/bin/mycelica-cli /home/spore/Mycelica/binaries/mycelica-cli-x86_64-unknown-linux-gnu
```

### 8f. Integration test

```bash
# Go binary handles orchestration
./spore/spore orchestrate "test task" --dry-run --verbose

# Go binary handles loop
echo "test task 1" | ./spore/spore loop --stdin --dry-run

# Go binary handles analytics
./spore/spore stats --since 7d
./spore/spore health

# mycelica-cli still handles graph operations
mycelica-cli spore status --all
mycelica-cli spore query-edges --type tracks --limit 5
```

### 8g. Commit

Two commits:
1. `feat(spore): Go port Phase 4 — loop mode + analytics` (Go code)
2. `refactor(cli): remove Spore orchestration from Rust` (Rust deletions)

---

## File Summary

| File | Purpose | Est. Lines |
|------|---------|------------|
| `internal/analytics/types.go` | Shared types | ~80 |
| `internal/loop/loop.go` + test | Loop mode orchestration | ~350 + ~150 |
| `internal/analytics/stats.go` + test | Run statistics | ~200 + ~80 |
| `internal/analytics/health.go` + test | Health checks | ~160 + ~60 |
| `internal/analytics/lessons.go` + test | Lesson extraction | ~160 + ~60 |
| `internal/analytics/distill.go` + test | Knowledge distillation | ~160 + ~50 |
| `internal/analytics/promptstats.go` + test | Token usage stats (optional) | ~100 + ~40 |
| `cmd/loop.go` | CLI: loop command | ~120 |
| `cmd/stats.go` | CLI: stats command | ~80 |
| `cmd/health.go` | CLI: health command | ~60 |
| `cmd/lessons.go` | CLI: lessons command | ~60 |
| `cmd/distill.go` | CLI: distill command | ~60 |
| `cmd/promptstats.go` | CLI: prompt-stats (optional) | ~50 |
| **Total Go (without optional)** | | **~1,930** |
| **Total Go (with optional)** | | **~2,120** |
| **Total including tests** | | **~2,530** |
| `src-tauri/src/bin/cli/spore.rs` | Rust deletions | **~-2,300** |

## Dependency Graph

```
Step 0 (audit)           ─── BLOCKS everything
Step 1 (types.go)        ─── BLOCKS Steps 2-7
    ├── Step 2 (loop.go)         ─ depends on orchestrate package (Phase 3)
    ├── Step 3 (stats.go)        ─ independent (reads Tracks edges)
    ├── Step 4 (health.go)       ─ independent (reads graph state)
    ├── Step 5 (lessons.go)      ─ independent (reads Tracks + verdicts)
    └── Step 6 (distill.go)      ─ depends on Step 5 (calls ExtractLessons)
Step 7 (prompt-stats)    ─── independent, optional
Step 8 (wire up + delete)─── depends on ALL Steps 2-6
```

Steps 2-5 can be built in parallel after Step 1. Step 6 needs Step 5. Step 8 integrates everything.

## Verification

```bash
# Go builds
cd spore && go build ./...

# Go tests pass
cd spore && go test ./...

# Rust still builds after deletions
cd src-tauri && cargo +nightly check --bin mycelica-cli --features mcp

# Integration: Go orchestrate still works
./spore/spore orchestrate "test" --dry-run

# Integration: loop works
echo "test task" | ./spore/spore loop --stdin --dry-run

# Integration: analytics work
./spore/spore stats --since 7d
./spore/spore health

# mycelica-cli graph operations unaffected
mycelica-cli db stats
mycelica-cli spore status --all
```

## What's NOT Phase 4

- Redundancy detection (Phase B — separate plan exists)
- Session resume on bounce 2+ (optimization)
- Concurrent loop execution (future, field exists in LoopConfig)
- Auto-commit after coder (deferred)
- GUI integration for analytics (future)
