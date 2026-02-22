# Spore Pipeline Reference

Spore is a multi-agent orchestration system built in Rust. It dispatches Claude Code agents through graph-compiled task files, with automatic verification, bounce recovery, and lesson accumulation. The pipeline has three roles: coder, verifier, and summarizer.

All orchestration logic lives in `src-tauri/src/bin/cli/spore.rs` (~9,300 lines, ~4,950 non-test). Run analytics live in `src-tauri/src/bin/cli/spore_runs.rs` (~3,600 lines).

---

## Overview

Spore takes a task description, compiles context from the knowledge graph, runs a coder agent to implement it, verifies the result, and optionally summarizes lessons back into the graph. The entire pipeline is a single linear flow with a bounce loop for retry.

Three agents, three models:
- **Coder** (opus, 50 turns) -- implements the task
- **Verifier** (opus, 50 turns) -- checks the implementation
- **Summarizer** (sonnet, 15 turns) -- extracts lessons (optional)

An **operator** role (opus, 80 turns) exists for manual single-agent use outside the pipeline.

---

## Pipeline Flow

```
spore orchestrate "task description"
  |
  v
[Context Compilation]
  generate_task_file()
  -> semantic search (embeddings)
  -> FTS keyword search
  -> merge + deduplicate anchors
  -> Dijkstra expansion per anchor
  -> lesson matching
  -> build markdown task file
  |
  v
[Bounce Loop] (max 3 iterations, configurable)
  |
  +---> [Coder Phase]
  |       bounce 0: fresh spawn with full task file
  |       bounce 1+: --resume with verifier feedback
  |       10s startup retry on 0-turn failure
  |       |
  |       v
  |     [Post-Coder Cleanup]
  |       detect git diff -> create Implementation node
  |       selective_git_add()
  |       re-index changed files
  |       create Related edges to changed code
  |       |
  |       v
  |     [Verifier Phase]
  |       fresh spawn each time (no resume)
  |       single retry on subprocess failure
  |       |
  |       v
  |     [Verdict Handling]
  |       Supports  -> break loop, run summarizer
  |       Contradicts -> extract reason, next bounce
  |       Unknown -> next bounce (generic feedback)
  |
  v
[Max Bounces Reached] -> create Escalation node
  |
  v
[Summarizer] (if verdict=Supports and --no-summarize not set)
  creates Summary + Lesson nodes via MCP
```

---

## Context Compilation

`generate_task_file()` builds a graph-aware markdown task file for each agent invocation.

### Steps

1. **Compute task embedding** -- all-MiniLM-L6-v2, 384-dim vector from task description.

2. **Semantic search** -- embedding similarity against all node embeddings. Threshold 0.3, returns top 10 candidates, filtered to 5 (skips operational nodes).

3. **FTS keyword search** -- splits task into keywords, removes stopwords, joins with OR. Runs against SQLite FTS5 index. Catches token matches that semantic search misses. Returns top 5.

4. **Merge and deduplicate** -- semantic results have priority. Deduplicated by node ID. Final count: max 5 anchors.

5. **Dijkstra expansion** -- for each anchor, traverse the graph: 7 hops max, exclude browser edges (clicked, backtracked, session_item) and operational nodes. Results merged across anchors, keeping highest relevance score per node.

6. **Lesson matching** -- query last 20 Lesson nodes, rank by embedding similarity to task (threshold 0.15). Quality filter rejects lessons under 20 words, bare commands, short imperatives. Extract Pattern/Situation + Fix sections. Pad to 5 with recency if fewer than 5 match.

7. **Build markdown** -- assembles sections:
   - Code Locations: file paths + line ranges for direct file access
   - Key Code Snippets: top 5 code nodes, 30-line inline snippets (functions prioritized over structs)
   - Files Likely Touched: grouped by file, ranked by node count (top 8)
   - Call Graph: callers + callees for top 3 function nodes (3 each)
   - Related Nodes: non-code context from Dijkstra expansion
   - Recent Lessons: ranked by relevance
   - Role-specific sections (verifier gets impl node ID, bounce 2+ coder gets previous verdict)

Output path: `/tmp/mycelica-orchestrator/task-<run_id>.md`

---

## Bounce Loop

The core coder-verifier cycle inside `handle_orchestrate()`. Default max bounces: 3 (configurable via `--max-bounces`).

### Coder Phase

**Bounce 0 (fresh start):**
- Full task file from generate_task_file()
- Behavioral template included in prompt (unless native agent file detected)
- Spawned with `claude -p <prompt> --model opus --max-turns 50`

**Bounce 1+ (after contradiction):**
- If previous coder session ID available: `--resume <session_id>` with simplified prompt containing verifier feedback
- Verifier's rejection reason injected: "The verifier rejected your implementation: {reason}"
- If resume fails: fallback to fresh session with full prompt

**Startup retry:** If coder returns 0 turns (no output at all), wait 10 seconds and retry once. This catches transient MCP initialization hangs.

### Post-Coder Cleanup

Runs after every coder phase. Failures warn but do NOT abort orchestration.

1. **Detect changed files** -- compare git dirty/untracked state before vs. after coder. Uses content hash comparison to detect in-place edits to already-dirty files.

2. **Create Implementation node** -- orchestrator (not the coder) creates an Implementation node from the git diff. This node is the verifier's input.

3. **Selective git staging** -- `selective_git_add()` stages changes, excluding .env, .db, target/, node_modules/, .claude/, and loop state files.

4. **Re-index changed files** -- runs `mycelica-cli import code <file> --update` for each changed source file. Refreshes embeddings and edges.

5. **Create Related edges** -- links the task node to graph nodes corresponding to changed code files.

### Verifier Phase

- Fresh spawn each time (no session resume)
- Gets task file with "Implementation to Check" section containing the Implementation node ID
- Read-only: no Edit, no Write
- Single retry on subprocess failure (10s cooldown)

### Verdict Handling

After verifier completes, the orchestrator checks for a verdict through 3 fallback methods (see Verdict Detection below).

- **Supports** -- task complete. Clear checkpoint. Run summarizer if enabled.
- **Contradicts** -- extract failure reason from verdict. Set `last_impl_id` and `last_verdict`. Continue to next bounce with feedback.
- **Unknown** -- treat as contradiction. Coder gets generic "verifier could not parse a verdict" feedback.

### Max Bounces Reached

`create_escalation()` creates an Escalation node linked to the task via a Flags edge. Human review required. The run is marked as escalated in analytics.

---

## Verdict Detection

Three-tier cascade. The orchestrator tries each method in order and stops at the first success.

### 1. Graph Edges (highest priority)

Query the Implementation node for incoming Supports or Contradicts edges created by the verifier agent (via MCP). Confidence comes directly from the verifier.

### 2. Structured JSON

Parse verifier stdout for `<verdict>` blocks:
```
<verdict>{"verdict":"supports","reason":"All checks pass","confidence":0.95}</verdict>
```

Fields: `verdict` or `result` (synonyms). Values: `supports`/`pass` or `contradicts`/`fail`. Optional `reason` and `confidence` (default 0.9).

The orchestrator creates both a Verdict node and a graph edge (Supports/Contradicts) from the parsed JSON.

### 3. Text Keyword Scan (lowest priority)

Scan verifier stdout for keywords: PASS, FAIL, supports, contradicts. Creates a text-fallback Verdict node AND a graph edge with confidence 0.5. This ensures even unstructured verdicts are recorded in the graph.

---

## Agent Spawn System

### resolve_agent_name()

Checks if `.claude/agents/<role>.md` exists in the working directory. If yes, passes `--agent <role>` to the claude CLI (native agent mode). When using native agents, the behavioral template is omitted from the prompt -- the agent file provides it.

Falls back to inline prompt templates for foreign repos (repos without `.claude/agents/` files). This is how Spore maintains cross-codebase portability.

### spawn_claude()

Builds and executes a `claude` CLI subprocess:

```
claude -p <prompt>
  --model <model>
  --mcp-config <config_path>
  --dangerously-skip-permissions
  --output-format stream-json
  --verbose
  --max-turns <N>
  [--allowedTools <tools>]
  [--disallowedTools <tools>]
  [--agent <name>]        # native agent mode
  [--resume <session_id>] # session resume for bounce 2+
```

The `CLAUDECODE` environment variable is removed so child Claude processes don't refuse to start when the orchestrator runs inside a Claude Code session.

### Tool Permissions

| Role | Allowed | Disallowed |
|------|---------|------------|
| coder | Read, Write, Edit, Bash(*), mcp__mycelica__* | Grep, Glob |
| verifier | Read, Grep, Glob, Bash(cargo:*, cd:*, mycelica-cli:*), mcp__mycelica__* | -- |
| summarizer | (default) | Bash, Edit, Write |
| operator | (all tools) | -- |

### Model Selection

`select_model_for_role()` maps roles to models:

| Role | Model |
|------|-------|
| coder | opus |
| verifier | opus |
| operator | opus |
| summarizer | sonnet |
| (unknown) | opus |

The `--coder-model` flag overrides coder model selection for A/B experiments.

### MCP Configuration

Each agent run gets a temporary MCP config at `/tmp/mycelica-orchestrator/mcp-<role>-<run_id>.json`. The config points to `mycelica-cli mcp-server` with role, agent_id, run_id, and db_path arguments. This gives agents graph read/write access scoped to their role.

### Two-Phase Watchdog

Thread-based watchdog monitors agent processes:

| Phase | Timeout | Trigger |
|-------|---------|---------|
| Startup | 90 seconds | No stdout output at all (MCP init hang) |
| Execution | max(turns * 120s, 600s) | Custom timeout or calculated default |

Kill sequence: SIGTERM -> 3 second grace period -> SIGKILL.

---

## Graph Integration

### Node Types Created by Orchestrator

| Node | Created When | node_class |
|------|-------------|------------|
| Task | Start of orchestration | operational |
| Implementation | After coder phase (from git diff) | operational |
| Verdict | After verifier phase (structured or text-fallback) | operational |
| Escalation | Max bounces reached | operational |

Agents create nodes via MCP:
| Node | Created By |
|------|-----------|
| Summary | Summarizer |
| Lesson | Summarizer |

All orchestrator nodes use `node_class = "operational"` so they are filtered out of context compilation (prevents self-reference loops).

### Edge Types

| Edge Type | Source -> Target | Created By | Confidence |
|-----------|-----------------|------------|------------|
| DerivesFrom | Implementation -> Task | Orchestrator | 0.9 (success) / 0.5 (partial) |
| Supports | Verdict -> Implementation | Verifier (MCP) or Orchestrator (fallback) | From verifier |
| Contradicts | Verdict -> Implementation | Verifier (MCP) or Orchestrator (fallback) | From verifier |
| Related | Task -> Changed code nodes | post_coder_cleanup | (default) |
| Tracks | Task -> Task (self-loop) | record_run_status_with_cost | Analytics metadata |
| Flags | Escalation -> Task | create_escalation | 0.9 |

### Tracks Edge Metadata

Self-loop edges on task nodes carry run analytics in their metadata JSON:
```json
{
  "run_id": "uuid",
  "status": "completed|failed|failed-startup|failed-partial",
  "exit_code": 0,
  "agent": "spore:coder",
  "cost_usd": 1.23,
  "num_turns": 12,
  "duration_ms": 45000,
  "experiment": "opus-v-sonnet",
  "model": "opus"
}
```

### Data Flow Between Agents

Agents never communicate directly. All inter-agent data flows through either:
1. The orchestrator (passing strings/file paths between phases)
2. The graph (nodes and edges read via MCP in future runs)

```
Coder -> (orchestrator creates impl node from git diff) -> Verifier
Summarizer -> (graph lessons via MCP) -> Future task files (via lesson embedding search)
```

---

## Checkpoint System

`OrchestratorCheckpoint` enables resume and retry after interruptions.

### Fields

| Field | Type | Purpose |
|-------|------|---------|
| task | String | Original task description |
| task_node_id | String | Graph node ID for this run |
| db_path | String | Database path for resume |
| bounce | usize | Current bounce number |
| max_bounces | usize | Total bounces allowed |
| max_turns | usize | Per-agent turn limit |
| next_phase | String | "coder", "verifier", "summarizer", "complete" |
| impl_node_id | Option | Current implementation node |
| last_impl_id | Option | Previous implementation (for bouncing) |

### Storage

Path: `/tmp/mycelica-orchestrator/<task_node_id_prefix>.checkpoint.json`

Updated at every phase transition. Cleared on successful completion or max bounces reached.

### Resume

`spore resume` finds the most recent incomplete checkpoint and resumes from the saved phase. Phase ordering: coder (0) -> verifier (1) -> summarizer (2).

### Retry

`spore retry <run_id>` re-runs an escalated or failed task with fresh context compilation. Optionally accepts higher `--max-bounces` and `--max-turns`.

---

## Loop Mode

`spore loop --source tasks.txt` dispatches tasks sequentially from a file.

### Flow

```
handle_spore_loop()
  -> read tasks from source file
  -> load LoopState (or create new)
  -> for each task:
       -> budget check (total_cost >= budget?)
       -> max_runs check
       -> consecutive escalation check (3 in a row = stop)
       -> skip if already verified in state
       -> handle_orchestrate(task)
       -> query run cost from graph
       -> persist LoopState immediately
       -> if verified: selective_git_add() + auto-commit
       -> if escalated: increment streak, maybe pause
       -> cost anomaly detection (> 3x running average)
       -> 5s cooldown between tasks
  -> print loop summary
```

### Task File Formats

1. One task per line (simple)
2. Multi-line tasks separated by `---` delimiters
3. Lines starting with `#` are comments (ignored)

### Loop State

Persisted to `<source>.loop-state.json` (next to the task file):
- Tracks verified tasks (skipped on resume)
- Cumulative cost across restarts
- Per-task run records with status, cost, duration

### Safety Checks

- **Budget**: stops when `total_cost >= budget`
- **Max runs**: stops at configured limit
- **Consecutive escalation limit**: 3 in a row = systemic problem, stops loop
- **Cost anomaly warning**: current task cost > 3x running average
- **Pause on escalation**: optional (--pause-on-escalation flag)

### Auto-Commit

Between loop tasks, verified changes are committed with message format: `feat(loop): {truncated_task_description}`

---

## Key Constants

| Constant | Value | Description |
|----------|-------|-------------|
| MAX_BOUNCES | 3 | Default bounce limit (configurable) |
| MAX_TURNS (coder/verifier) | 50 | Per-agent turn limit |
| MAX_TURNS (summarizer) | 15 | Summarizer turn limit |
| MAX_TURNS (operator) | 80 | Operator turn limit |
| SEMANTIC_THRESHOLD | 0.3 | Context search similarity threshold |
| LESSON_THRESHOLD | 0.15 | Lesson similarity threshold |
| MAX_ANCHORS | 5 | Context compilation anchor limit |
| DIJKSTRA_MAX_HOPS | 7 | Graph traversal depth per anchor |
| CODE_SNIPPET_LIMIT | 5 nodes, 30 lines | Inline code in task files |
| CALL_GRAPH_TOP | 3 functions, 3 callers/callees each | Call graph context |
| STARTUP_WATCHDOG | 90s | No-output timeout |
| EXECUTION_WATCHDOG | max(turns * 120s, 600s) | Process timeout |
| STARTUP_RETRY_COOLDOWN | 10s | Retry delay for 0-turn agents |
| LOOP_INTER_TASK_COOLDOWN | 5s | Delay between loop tasks |
| CONSECUTIVE_ESCALATION_LIMIT | 3 | Loop stop threshold |
| COST_ANOMALY_THRESHOLD | 3x average | Cost warning trigger |

---

## File Layout

| Artifact | Path |
|----------|------|
| Orchestrator code | `src-tauri/src/bin/cli/spore.rs` |
| Analytics code | `src-tauri/src/bin/cli/spore_runs.rs` |
| CLI command definitions | `src-tauri/src/bin/cli.rs` |
| MCP server | `src-tauri/src/mcp.rs` |
| Native agent files | `.claude/agents/{coder,verifier,summarizer}.md` |
| Agent prompt templates | `docs/spore/agents/{coder,verifier,summarizer,operator}.md` |
| Task files (generated) | `/tmp/mycelica-orchestrator/task-<run_id>.md` |
| Thinking logs | `docs/spore/tasks/think-<role>-<run_id>.log` |
| MCP configs (generated) | `/tmp/mycelica-orchestrator/mcp-<role>-<run_id>.json` |
| Checkpoints | `/tmp/mycelica-orchestrator/<task_id>.checkpoint.json` |
| Loop state | `<source>.loop-state.json` |
| Database | `.mycelica.db` |

---

## Error Handling

### Retry Behavior

| Agent | On 0 turns | On subprocess failure | On resume failure |
|-------|------------|----------------------|-------------------|
| Coder | 10s cooldown + retry once | Continue if files changed | Fallback to fresh session |
| Verifier | -- | 10s cooldown + retry once | N/A |
| Summarizer | -- | Non-fatal (skip) | N/A |

### Failure Modes

- **Coder fails + no changes**: abort immediately. Nothing to verify.
- **Coder fails + has changes**: partial recovery -- continue to verifier with what exists.
- **Verifier fails**: save checkpoint at verifier phase. `spore resume` retries from verifier.
- **MCP all-failed**: abort. Agent had no graph tools, results would be meaningless.
- **3 consecutive escalations in loop**: stop loop. Likely systemic problem.

### Escalation

When max bounces are exhausted without a Supports verdict, the orchestrator creates an Escalation node in the graph linked to the task via a Flags edge. The run is recorded as escalated in analytics. Human review is required.
