# Spore CLI Reference

Complete command reference for `mycelica-cli spore` and related commands.

All spore commands are subcommands of `mycelica-cli spore`. Global flags apply to any `mycelica-cli` command.

---

## Global Flags

| Flag | Type | Description |
|------|------|-------------|
| `--db <PATH>` | String | Database path (default: auto-detect by walking up directories for `.mycelica.db`) |
| `--json` | bool | Output as JSON for scripting |
| `-q, --quiet` | bool | Suppress progress output |
| `-v, --verbose` | bool | Detailed logging |
| `--remote <URL>` | String | Route commands to a team server instead of local DB |

---

## Core Commands

### `spore orchestrate <TASK>`

Orchestrate a Coder -> Verifier bounce loop for a task. Compiles context from the knowledge graph, runs a coder agent, verifies the result, and optionally summarizes lessons.

**Syntax:**
```
mycelica-cli spore orchestrate "task description" [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--max-bounces` | usize | 3 | Maximum coder/verifier bounce iterations before escalation |
| `--max-turns` | usize | 50 | Max turns per agent invocation |
| `--coder-prompt <PATH>` | PathBuf | `docs/spore/agents/coder.md` | Path to coder agent prompt |
| `--verifier-prompt <PATH>` | PathBuf | `docs/spore/agents/verifier.md` | Path to verifier agent prompt |
| `--summarizer-prompt <PATH>` | PathBuf | `docs/spore/agents/summarizer.md` | Path to summarizer agent prompt |
| `--no-summarize` | bool | false | Skip summarizer after verification |
| `--dry-run` | bool | false | Show what would happen without running agents |
| `-v, --verbose` | bool | false | Print agent stdout/stderr |
| `-q, --quiet` | bool | false | Only show phase headers and final status |
| `--timeout <SECS>` | u64 | - | Custom process timeout in seconds (default: `max(turns*120, 600)`) |
| `--agent <ROLE>` | String | - | Run as a single named agent role (skips coder/verifier pipeline) |
| `--agent-prompt <PATH>` | PathBuf | - | Path to agent prompt file (used with `--agent`) |
| `--experiment <LABEL>` | String | - | Tag run with experiment label for A/B comparisons |
| `--coder-model <MODEL>` | String | - | Override model for coder role (e.g., "opus", "sonnet", "haiku") |

**Examples:**
```bash
# Simple task
mycelica-cli spore orchestrate "Add a --limit flag to the search command"

# Dry run to preview what would happen
mycelica-cli spore orchestrate "Fix the off-by-one error in pagination" --dry-run

# A/B experiment with model override
mycelica-cli spore orchestrate "Implement retry logic for HTTP client" \
  --experiment "opus-v-sonnet" --coder-model opus

# Run as a single agent (no coder/verifier pipeline)
mycelica-cli spore orchestrate "Audit the error handling in ai_client.rs" --agent operator

# Verbose output with custom timeout
mycelica-cli spore orchestrate "Add WebSocket support" --verbose --timeout 1800
```

---

### `spore loop`

Continuous orchestration loop: reads tasks from a file, dispatches sequentially, tracks cost and state.

**Syntax:**
```
mycelica-cli spore loop --source <FILE> [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--source` | String | **required** | Path to task file (one task per line, `#` comments ignored) |
| `--budget` | f64 | 20.0 | Total cost budget in USD |
| `--max-runs` | usize | 10 | Maximum tasks to dispatch |
| `--max-bounces` | usize | 3 | Per-task bounce limit |
| `--max-turns` | usize | 50 | Max turns per agent invocation |
| `--timeout` | u64 | - | Custom process timeout in seconds per task |
| `--dry-run` | bool | false | Show what would be dispatched without running |
| `--pause-on-escalation` | bool | false | Stop the loop when a task escalates |
| `--summarize` | bool | false | Run summarizer after verified tasks |
| `-v, --verbose` | bool | false | Print agent stdout/stderr |
| `--reset` | bool | false | Delete loop state before starting (clears verified task history) |
| `--experiment` | String | - | Tag runs with experiment label for A/B comparisons |
| `--coder-model` | String | - | Override model for the coder role |

**Examples:**
```bash
# Run tasks from file with defaults
mycelica-cli spore loop --source tasks.txt

# Higher budget, more runs
mycelica-cli spore loop --source tasks.txt --budget 50.0 --max-runs 20

# Reset state and start fresh with experiment label
mycelica-cli spore loop --source tasks.txt --reset --experiment "batch-3"

# Conservative: pause if anything escalates
mycelica-cli spore loop --source tasks.txt --pause-on-escalation --summarize
```

---

### `spore resume [ID]`

Resume an interrupted orchestrator run from its last checkpoint.

**Syntax:**
```
mycelica-cli spore resume [ID] [FLAGS]
```

**Arguments:**

| Arg | Type | Default | Description |
|-----|------|---------|-------------|
| `ID` | String | `last` | Task node ID or `last` for most recent checkpoint |

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `-v, --verbose` | bool | false | Print agent stdout/stderr |

**Example:**
```bash
# Resume most recent interrupted run
mycelica-cli spore resume

# Resume a specific run
mycelica-cli spore resume abc12345
```

---

### `spore retry <RUN_ID>`

Re-run an escalated or failed task with optional higher turn/bounce budget.

**Syntax:**
```
mycelica-cli spore retry <RUN_ID> [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--max-bounces` | usize | 3 | Maximum bounce iterations |
| `--max-turns` | usize | 50 | Max turns per agent invocation |
| `--no-summarize` | bool | false | Skip summarizer after verification |
| `-v, --verbose` | bool | false | Print agent stdout/stderr |

**Example:**
```bash
# Retry with higher limits
mycelica-cli spore retry abc12345 --max-bounces 5 --max-turns 80
```

---

### `spore batch <FILE>`

Run multiple orchestrator tasks from a file. One task per line, `#` lines are comments.

**Syntax:**
```
mycelica-cli spore batch <FILE> [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--max-bounces` | usize | 3 | Maximum bounce iterations per task |
| `--max-turns` | usize | 50 | Max turns per agent invocation |
| `--timeout` | u64 | - | Custom process timeout in seconds |
| `-v, --verbose` | bool | false | Print agent stdout/stderr |
| `--stop-on-failure` | bool | false | Stop on first failure (default: continue all tasks) |
| `--dry-run` | bool | false | Show what would happen without running agents |

**Example:**
```bash
# Run batch with stop-on-failure
mycelica-cli spore batch tasks.txt --stop-on-failure --verbose
```

---

## Run Analytics

### `spore runs list`

List all orchestrator runs with status.

**Syntax:**
```
mycelica-cli spore runs list [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--all` | bool | false | Also show non-Orchestration operational nodes with Tracks edges |
| `--cost` | bool | false | Sort by cost (most expensive first) and show total cost |
| `--escalated` | bool | false | Show only runs with an ESCALATION node |
| `--status <FILTER>` | String | - | Filter by status (comma-separated: `verified,implemented,escalated,cancelled,pending`) |
| `--since <DATE>` | String | - | Only runs after this date (`YYYY-MM-DD` or relative: `1h`, `2d`, `1w`) |
| `--limit` | usize | 0 | Maximum runs to show (0 = no limit) |
| `-v, --verbose` | bool | false | Show full task text instead of truncating |
| `--format` | text/compact/json/csv | text | Output format |
| `--duration <SECS>` | u64 | - | Only runs with total duration >= this many seconds |
| `--agent <NAME>` | String | - | Filter by agent name (e.g., "coder", "spore:coder") |

**Examples:**
```bash
# Recent verified runs
mycelica-cli spore runs list --status verified --since 7d

# Most expensive runs
mycelica-cli spore runs list --cost --limit 10

# CSV export for analysis
mycelica-cli spore runs list --format csv > runs.csv

# Long-running tasks only
mycelica-cli spore runs list --duration 300
```

---

### `spore runs get <RUN_ID>`

Show all edges in a run.

**Syntax:**
```
mycelica-cli spore runs get <RUN_ID> [--json]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--json` | bool | false | Output as JSON |

**Example:**
```bash
mycelica-cli spore runs get abc12345
```

---

### `spore runs timeline <RUN_ID>`

Show a vertical timeline of agent phases for a run.

**Syntax:**
```
mycelica-cli spore runs timeline <RUN_ID>
```

**Example:**
```bash
mycelica-cli spore runs timeline abc12345
```

---

### `spore runs stats`

Show aggregate statistics across all runs.

**Syntax:**
```
mycelica-cli spore runs stats [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--experiment <LABEL>` | String | - | Filter stats to runs tagged with this experiment label |

**Examples:**
```bash
# Overall stats
mycelica-cli spore runs stats

# Stats for a specific experiment
mycelica-cli spore runs stats --experiment "opus-batch"
```

---

### `spore runs history <RUN_ID>`

Show complete timeline of a run: agents, edges, outcomes.

**Syntax:**
```
mycelica-cli spore runs history <RUN_ID>
```

**Example:**
```bash
mycelica-cli spore runs history abc12345
```

---

### `spore runs show <RUN_ID>`

Alias for `spore runs history`.

---

### `spore runs diff <RUN_ID>`

Show source code files changed by a run.

**Syntax:**
```
mycelica-cli spore runs diff <RUN_ID>
```

**Example:**
```bash
mycelica-cli spore runs diff abc12345
```

---

### `spore runs compare <A> <B>`

Compare two runs side-by-side.

**Syntax:**
```
mycelica-cli spore runs compare <RUN_A> <RUN_B>
```

**Example:**
```bash
mycelica-cli spore runs compare abc12345 def67890
```

---

### `spore runs compare-experiments --experiment <A> <B>`

Compare two experiment batches side-by-side.

**Syntax:**
```
mycelica-cli spore runs compare-experiments --experiment <LABEL_A> <LABEL_B>
```

**Example:**
```bash
mycelica-cli spore runs compare-experiments --experiment opus-batch sonnet-batch
```

---

### `spore runs cost`

Show cost breakdown: total, average, by status, and today's cost.

**Syntax:**
```
mycelica-cli spore runs cost [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--since <DATE>` | String | - | Only include runs after this date |
| `--json` | bool | false | Output as JSON |

**Example:**
```bash
mycelica-cli spore runs cost --since 7d
```

---

### `spore runs top`

Show the top N most expensive runs by total cost.

**Syntax:**
```
mycelica-cli spore runs top [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--limit` | usize | 5 | Number of top results to show |

**Example:**
```bash
mycelica-cli spore runs top --limit 10
```

---

### `spore runs cancel <RUN_ID>`

Cancel a pending run (marks it as cancelled).

**Syntax:**
```
mycelica-cli spore runs cancel <RUN_ID> [--json]
```

**Example:**
```bash
mycelica-cli spore runs cancel abc12345
```

---

### `spore runs rollback <RUN_ID>`

Delete all edges (and optionally nodes) from a run.

**Syntax:**
```
mycelica-cli spore runs rollback <RUN_ID> [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--delete-nodes` | bool | false | Also delete operational nodes created during the run |
| `--force` | bool | false | Skip confirmation prompt |
| `--dry-run` | bool | false | Show what would be deleted without deleting |

**Example:**
```bash
# Preview what would be rolled back
mycelica-cli spore runs rollback abc12345 --dry-run

# Full rollback including nodes
mycelica-cli spore runs rollback abc12345 --delete-nodes --force
```

---

### `spore runs summary`

One-paragraph natural language summary of recent orchestrator activity.

**Syntax:**
```
mycelica-cli spore runs summary
```

---

## Graph Operations

### `spore query-edges`

Query edges with multi-filter support (type, agent, confidence, recency).

**Syntax:**
```
mycelica-cli spore query-edges [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `-t, --type <TYPE>` | String | - | Filter by edge type |
| `--agent <ID>` | String | - | Filter by agent_id (edge creator) |
| `--target-agent <ID>` | String | - | Filter by target node's agent_id |
| `--confidence-min <N>` | f64 | - | Minimum confidence threshold (0.0-1.0) |
| `--since <DATE>` | String | - | Only edges after this date (`YYYY-MM-DD` or relative: `1h`, `2d`, `1w`) |
| `--not-superseded` | bool | false | Exclude superseded edges |
| `--limit` | usize | 20 | Maximum results |
| `--compact` | bool | false | One-line output: `edge_id type source -> target [confidence]` |

**Examples:**
```bash
# Recent high-confidence edges
mycelica-cli spore query-edges --type supports --confidence-min 0.8 --since 7d

# All edges created by the verifier
mycelica-cli spore query-edges --agent spore:verifier --compact

# Active (non-superseded) edges only
mycelica-cli spore query-edges --not-superseded --limit 50
```

---

### `spore explain-edge <ID>`

Explain an edge with full context: source/target nodes, adjacent edges, supersession chain.

**Syntax:**
```
mycelica-cli spore explain-edge <EDGE_ID> [--depth <N>]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--depth` | usize | 1 | Depth of adjacent edge exploration (1 or 2) |

**Example:**
```bash
mycelica-cli spore explain-edge abc12345 --depth 2
```

---

### `spore path-between <FROM> <TO>`

Find all paths between two nodes.

**Syntax:**
```
mycelica-cli spore path-between <FROM> <TO> [FLAGS]
```

**Arguments:** Source and target can be ID prefix or title substring.

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--max-hops` | usize | 5 | Maximum hops |
| `--edge-types <TYPES>` | String | - | Comma-separated edge types to follow |

**Example:**
```bash
mycelica-cli spore path-between "clustering" "hierarchy" --max-hops 3 --edge-types "calls,documents"
```

---

### `spore edges-for-context <ID>`

Get the most relevant edges for a node, ranked by composite score.

**Syntax:**
```
mycelica-cli spore edges-for-context <ID> [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--top` | usize | 10 | Number of top edges to return |
| `--not-superseded` | bool | false | Exclude superseded edges |

**Example:**
```bash
mycelica-cli spore edges-for-context "similarity.rs" --top 20 --not-superseded
```

---

### `spore context-for-task <ID>`

Dijkstra context retrieval: find the N most relevant nodes by weighted graph proximity.

**Syntax:**
```
mycelica-cli spore context-for-task <ID> [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--budget` | usize | 20 | Maximum number of context nodes to return |
| `--max-hops` | usize | 6 | Maximum hops from source |
| `--max-cost` | f64 | 3.0 | Maximum cumulative path cost (lower = stricter) |
| `--edge-types <TYPES>` | String | - | Comma-separated edge types to follow |
| `--not-superseded` | bool | false | Exclude superseded edges |
| `--items-only` | bool | false | Only include item nodes (categories still traversed) |

**Example:**
```bash
mycelica-cli spore context-for-task "import pipeline" --budget 30 --max-cost 2.0 --items-only
```

---

### `spore create-edge`

Create an edge between two existing nodes.

**Syntax:**
```
mycelica-cli spore create-edge --from <NODE> --to <NODE> --type <TYPE> [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--from` | String | **required** | Source node (ID prefix or title substring) |
| `--to` | String | **required** | Target node (ID prefix or title substring) |
| `-t, --type <TYPE>` | String | **required** | Edge type (e.g., supports, contradicts, derives_from) |
| `--content` | String | - | Full reasoning/explanation for this edge |
| `--reason` | String | - | Short provenance |
| `--confidence` | f64 | - | Confidence score (0.0-1.0) |
| `--agent` | String | `spore` | Agent attribution |
| `--supersedes` | String | - | Edge ID this supersedes |

**Example:**
```bash
mycelica-cli spore create-edge \
  --from "clustering algorithm" \
  --to "hierarchy build" \
  --type supports \
  --confidence 0.9 \
  --reason "clustering feeds hierarchy"
```

---

### `spore read-content <ID>`

Read full content of a node (no metadata noise).

**Syntax:**
```
mycelica-cli spore read-content <ID>
```

**Example:**
```bash
mycelica-cli spore read-content abc12345
```

---

### `spore list-region <ID>`

List all descendants of a category node.

**Syntax:**
```
mycelica-cli spore list-region <ID> [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--class` | String | - | Filter by node_class (knowledge, meta, operational) |
| `--items-only` | bool | false | Only show items (is_item=true) |
| `--limit` | usize | 50 | Maximum results |

**Example:**
```bash
mycelica-cli spore list-region "Database Layer" --items-only --limit 100
```

---

### `spore check-freshness <ID>`

Check if summary meta-nodes are stale relative to summarized nodes.

**Syntax:**
```
mycelica-cli spore check-freshness <ID>
```

**Example:**
```bash
mycelica-cli spore check-freshness "Architecture Summary"
```

---

### `spore create-meta`

Create a meta node (summary/contradiction/status) with edges to existing nodes.

**Syntax:**
```
mycelica-cli spore create-meta --type <TYPE> --title <TITLE> [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `-t, --type <TYPE>` | String | **required** | Meta type: `summary`, `contradiction`, `status` |
| `--title` | String | **required** | Title for the meta node |
| `--content` | String | - | Content/body for the meta node |
| `--agent` | String | `human` | Agent attribution |
| `--connects-to <IDS>` | String[] | **required** | Node IDs to connect to (one or more) |
| `--edge-type` | String | `summarizes` | Edge type for connections |

**Example:**
```bash
mycelica-cli spore create-meta \
  --type summary \
  --title "Import Pipeline Summary" \
  --content "The import pipeline handles..." \
  --connects-to abc12345 def67890
```

---

### `spore update-meta <ID>`

Update an existing meta node.

**Syntax:**
```
mycelica-cli spore update-meta <ID> [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--content` | String | - | New content |
| `--title` | String | - | New title |
| `--agent` | String | `human` | Agent attribution |
| `--add-connects <IDS>` | String[] | - | Additional node IDs to connect to |
| `--edge-type` | String | `summarizes` | Edge type for new connections |

**Example:**
```bash
mycelica-cli spore update-meta abc12345 --content "Updated summary..." --add-connects xyz98765
```

---

## System

### `spore dashboard`

Combined dashboard: recent runs, lessons, costs, and graph health.

**Syntax:**
```
mycelica-cli spore dashboard [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--limit` | usize | 5 | Number of recent runs to show |
| `--format` | text/json/csv/compact | text | Output format |
| `--count` | bool | false | Show only summary counts (runs, verified, cost) |
| `--cost` | bool | false | Show today's cost |
| `--stale` | bool | false | Show count of stale code nodes |

**Examples:**
```bash
# Full dashboard
mycelica-cli spore dashboard

# Quick cost check
mycelica-cli spore dashboard --cost --count

# JSON for scripting
mycelica-cli spore dashboard --format json
```

---

### `spore health`

Check system health: database, CLI binary, agent prompts, MCP sidecar.

**Syntax:**
```
mycelica-cli spore health
```

---

### `spore status`

Spore status dashboard with graph overview.

**Syntax:**
```
mycelica-cli spore status [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--all` | bool | false | Show all details (meta nodes, edge breakdown, contradictions) |
| `--format` | String | `compact` | Output format: `compact` or `full` |

**Example:**
```bash
mycelica-cli spore status --all --format full
```

---

### `spore prompt-stats`

Show line counts for all agent prompt files.

**Syntax:**
```
mycelica-cli spore prompt-stats
```

---

### `spore lessons`

List all Lesson: nodes from the graph.

**Syntax:**
```
mycelica-cli spore lessons [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--compact` | bool | false | One lesson per line (title only, no content preview) |

**Example:**
```bash
mycelica-cli spore lessons --compact
```

---

### `spore gc`

Find stale operational nodes with no incoming edges (GC candidates).

**Syntax:**
```
mycelica-cli spore gc [FLAGS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--days` | u32 | 7 | Age threshold in days |
| `--dry-run` | bool | false | Only print candidates without deleting |
| `--force` | bool | false | Include `Lesson:` and `Summary:` nodes in GC candidates |

**Example:**
```bash
# Preview GC candidates
mycelica-cli spore gc --dry-run

# Clean up nodes older than 30 days
mycelica-cli spore gc --days 30
```

---

### `spore distill [RUN]`

Distill an orchestrator run into a summary node with lessons learned.

**Syntax:**
```
mycelica-cli spore distill [RUN] [FLAGS]
```

**Arguments:**

| Arg | Type | Default | Description |
|-----|------|---------|-------------|
| `RUN` | String | `latest` | Run ID prefix or `latest` for most recent run |

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--compact` | bool | false | One-line summary (outcome, duration, bounces) without full trail |

**Example:**
```bash
mycelica-cli spore distill latest --compact
```

---

## Configuration Files

| File | Purpose |
|------|---------|
| `.mycelica.db` | SQLite database (auto-discovered by walking up directories) |
| `*.loop-state.json` | Loop state tracking (completed tasks, costs) |
| `/tmp/mycelica-orchestrator/*.json` | Temporary task files and MCP configs for agent runs |
| `docs/spore/agents/*.md` | Agent prompt templates (coder, verifier, summarizer, operator) |
| `.claude/agents/*.md` | Native Claude Code agent definitions (coder, verifier, summarizer) |
| `CLAUDE.md` | Project conventions (injected into agent context) |
| `~/.local/share/com.mycelica.app/logs/` | Daily log files (`mycelica-YYYY-MM-DD.log`, auto-cleaned after 7 days) |

---

## Model Routing

Default model assignment per agent role. Overridable for coder via `--coder-model`.

| Role | Default Model | Notes |
|------|---------------|-------|
| coder | opus | A/B validated: 39% cheaper than sonnet (fewer turns) |
| verifier | opus | Needs deep code understanding |
| summarizer | sonnet | Extraction task, cost-optimized |
| operator | opus | Full-access manual mode |
| (unknown) | opus | Fallback for unrecognized roles |

---

## Environment

| Variable | Required | Description |
|----------|----------|-------------|
| `ANTHROPIC_API_KEY` | Yes (for agent commands) | API key for spawning Claude agents. Not needed for read-only graph commands. |

Agent-spawning commands: `orchestrate`, `loop`, `batch`, `retry`, `resume`, `distill`, `runs summary`.

Read-only commands (no API key needed): `query-edges`, `explain-edge`, `path-between`, `edges-for-context`, `context-for-task`, `read-content`, `list-region`, `check-freshness`, `status`, `dashboard`, `health`, `prompt-stats`, `lessons`, `gc`, `runs list/stats/timeline/history/show/diff/compare/cost/top/cancel`, `memory *`, `watch`.
