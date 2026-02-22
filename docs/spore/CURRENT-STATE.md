# Spore: Ground Truth Reference (from source code)

Generated from reading `src-tauri/src/bin/cli/spore.rs` (9,271 lines) and
`src-tauri/src/bin/cli/spore_runs.rs` (3,623 lines). Every claim references
source lines. This document is accurate enough to reimplement Spore from.

Last verified: 2026-02-21, branch `feat/context-for-task`.

---

## 1. What spore.rs Actually Does (Function Map)

**Total: 9,271 lines. Non-test: 4,948 lines (lines 1-4948). Tests: 4,323 lines (lines 4949-9271).**

### Dispatch / Command Routing (lines 1-1154)

`handle_spore()` (line 8) is the top-level match on `SporeCommands`. It routes to:

| Command | Handler | Lines | What it does |
|---------|---------|-------|-------------|
| `query-edges` | inline | 10-60 | Query edges by type/agent/confidence/since |
| `explain-edge` | inline | 62-120 | Deep-inspect one edge with adjacents and supersession chain |
| `path-between` | inline | 122-153 | DFS all paths between two nodes |
| `edges-for-context` | inline | 155-181 | Scored edges for a node (recency+confidence+type) |
| `create-meta` | inline | 183-281 | Create meta node (summary/contradiction/status) with edges |
| `update-meta` | inline | 283-436 | Supersede a meta node (create new, copy edges, mark old) |
| `status` | inline | 438-614 | Graph health: meta counts, edge activity, coherence |
| `create-edge` | `handle_link()` | 617-619 | Delegate to shared edge creator |
| `read-content` | inline | 622-642 | Print node content |
| `list-region` | inline | 645-678 | List descendants of a category |
| `check-freshness` | inline | 681-764 | Check if summaries are stale relative to targets |
| `runs` | `spore_runs::handle_runs()` | 766-768 | Delegate to analytics module |
| `orchestrate` | `handle_orchestrate()` | 770-812 | Main pipeline entry point |
| `retry` | inline | 814-850 | Re-run an orchestration from its task node |
| `resume` | inline | 852-902 | Resume from checkpoint |
| `batch` | inline | 904-996 | Run multiple tasks from a file sequentially |
| `loop` | `handle_spore_loop()` | 998-1000 | Continuous orchestration engine |
| `context-for-task` | inline | 1002-1050 | Preview Dijkstra context for a node |
| `gc` | inline | 1052-1123 | Garbage-collect orphan operational nodes |
| `lessons` | `spore_runs::handle_spore_lessons()` | 1125-1127 | List lesson nodes |
| `dashboard` | `spore_runs::handle_dashboard()` | 1129-1131 | Recent runs, costs, health |
| `distill` | `spore_runs::handle_distill()` | 1133-1135 | AI-powered run summarization |
| `health` | `spore_runs::handle_health()` | 1137-1139 | System health checks |
| `prompt-stats` | `spore_runs::handle_prompt_stats()` | 1141-1143 | Agent prompt file sizes |

**~1,154 lines (23%) of non-test code is dispatch/routing and graph query commands.**

### Utility Functions (lines 1156-1267)

| Function | Lines | Purpose |
|----------|-------|---------|
| `parse_since_to_millis()` | 1156-1185 | Parse "30m", "1h", "2d", "1w" or ISO date to epoch ms |
| `format_duration_short()` | 1189-1204 | Format ms as "1.2s", "2m 15s", "1h 05m" |
| `count_words()` | 1207-1209 | Whitespace-split word count |
| `is_lesson_quality()` | 1214-1239 | Filter trivial lessons (<20 words, bare commands, imperatives) |
| `truncate_middle()` | 1244-1266 | "foo...bar" middle truncation with char-boundary safety |

### Orchestrator Infrastructure (lines 1420-1990)

| Function | Lines | Purpose |
|----------|-------|---------|
| `make_orchestrator_node()` | 1424-1472 | Factory for operational nodes (boilerplate) |
| `resolve_cli_binary()` | 1475-1490 | Find mycelica-cli path, handle " (deleted)" suffix on Linux |
| `estimate_complexity()` | 1494-1538 | Heuristic 0-10 score: word count, action verbs, cross-cutting keywords, sentence count |
| `select_model_for_role()` | 1542-1552 | coder=opus, verifier=opus, summarizer=sonnet, operator=opus |
| `resolve_agent_name()` | 1579-1586 | Check if `.claude/agents/<role>.md` exists for native agent dispatch |
| `spawn_claude_in_dir()` | 1610-1876 | Core subprocess spawner (detailed below) |
| `write_temp_mcp_config()` | 1879-1919 | Write JSON MCP config to `/tmp/mycelica-orchestrator/` |
| `check_verdict()` | 1938-1969 | 2-pass edge scan: verifier agent_id first, then any supports/contradicts |
| `parse_verdict_from_text()` | 1973-1990 | Keyword scan: "PASS", "FAIL", "supports", "contradicts" |
| `parse_verifier_verdict()` | 1999-2031 | Parse `<verdict>{"verdict":"supports",...}</verdict>` JSON blocks |
| `capture_file_hashes()` | 2035-2055 | `git hash-object` on file set for content-aware diff |

### spawn_claude_in_dir() Detail (lines 1610-1876)

Builds and executes a `claude` CLI subprocess. Key parameters:
- `-p <prompt>` -- the full prompt text
- `--model <model>` -- "opus" or "sonnet"
- `--mcp-config <path>` -- path to temp MCP config JSON
- `--dangerously-skip-permissions` -- always set
- `--output-format stream-json` -- for real-time parsing
- `--verbose` -- always set (for thinking block capture)
- `--max-turns <N>` -- per-agent turn budget
- `--agent <name>` -- optional native agent mode (if `.claude/agents/<name>.md` exists)
- `--resume <session-id>` -- optional session resume (bounce 2+)
- `--allowedTools` / `--disallowedTools` -- per-role tool permissions
- Env: removes `CLAUDECODE` to prevent child-refuses-to-start inside parent session

**Watchdog** (lines 1670-1724): Two-phase timeout on a background thread.
- Phase 1: 90s startup timeout. If no stdout output within 90s, SIGTERM then SIGKILL.
- Phase 2: normal timeout = `max(max_turns * 120s, 600s)`, or custom `--timeout`.
  SIGTERM, 3s grace, then SIGKILL.

**Output parsing** (lines 1738-1849): Reads stream-json line by line.
- `"system"` -- logs MCP server status (name, connected/failed)
- `"assistant"` -- extracts text blocks (verbose), tool_use summaries (not quiet), thinking blocks
- `"result"` -- captures session_id, result text, cost, turns, duration

### Post-Coder Cleanup (lines 2057-2251)

`post_coder_cleanup()` runs after every coder phase:
1. **Diff detection**: Compares pre/post `git diff --name-only` and `git ls-files --others`. Also detects in-place edits to already-dirty files via `git hash-object` content comparison.
2. **Re-index**: Runs `mycelica-cli import code <file> --update` on each changed `.rs` file.
3. **CLI reinstall**: If any `src-tauri/` file changed, runs `cargo +nightly build --release --bin mycelica-cli --features mcp`, then copies binary to `~/.cargo/bin/mycelica-cli` and `binaries/mycelica-cli-x86_64-unknown-linux-gnu`. Uses unlink-before-copy to avoid "Text file busy".
4. **Related edges**: For each changed file, finds code nodes with matching `file_path` in tags JSON, creates `Related` edges from impl node to code nodes (confidence 0.85).

### Context Compilation (lines 2254-2917)

`generate_task_file()` -- detailed in Section 2 below.

### Git Staging (lines 2919-2980)

- `is_spore_excluded()`: Excludes `.loop-state.json`, `.env*`, `target/`, `node_modules/`, `*.db*`
- `selective_git_add()`: `git add -u` for tracked files, then selectively `git add` untracked files that pass exclusion filter

### Single-Agent Dispatch (lines 2999-3154)

`handle_single_agent()` -- entry point for `--agent <role>`. No bounce loop, no
post-coder cleanup, no verification. Creates task node, generates task file,
spawns one agent, records status. Default 80 turns for operator, max_turns for others.

### Main Orchestration Pipeline (lines 3156-3944)

`handle_orchestrate()` -- detailed in Sections 5 and 6 below.

### Loop Engine (lines 3946-4482)

`handle_spore_loop()` -- detailed in Section 7 below.

### Checkpoint System (lines 4484-4549)

Checkpoints stored as JSON in `/tmp/mycelica-orchestrator/<task-node-id-8chars>.checkpoint.json`.
Fields: task, task_node_id, db_path, bounce, max_bounces, max_turns, next_phase,
impl_node_id, last_impl_id, created_at, updated_at.

`find_latest_checkpoint()` scans all `.json` files in the checkpoint directory,
returns the one with highest `updated_at` where `next_phase != "complete"`.

### Run Status Recording (lines 4551-4620)

`record_run_status_with_cost()` creates a self-referential `Tracks` edge
(source_id = target_id = task_node_id) with metadata JSON containing:
run_id, status, exit_code, agent, cost_usd, num_turns, duration_ms, experiment, model.

### Escalation (lines 4622-4694)

`create_escalation()` creates a meta node titled "ESCALATION: <task> (after N bounces)"
with `Flags` edge to the last implementation node and `Tracks` edge to the task node.

### Edge Creation (lines 4696-4770)

`handle_link()` -- shared edge creator used by `create-edge` command and internal callers.

### Non-Test Line Budget Summary

| Block | Lines | % of 4,948 |
|-------|-------|-----------|
| Dispatch/routing + graph queries | ~1,154 | 23% |
| Utility functions | ~112 | 2% |
| Watch system | ~145 | 3% |
| Orchestrator infrastructure (spawn, verdict, helpers) | ~570 | 12% |
| Post-coder cleanup | ~195 | 4% |
| Context compilation (generate_task_file) | ~663 | 13% |
| Git staging | ~62 | 1% |
| Single-agent dispatch | ~156 | 3% |
| Main bounce loop (handle_orchestrate) | ~788 | 16% |
| Loop engine + state persistence | ~536 | 11% |
| Checkpoint system | ~66 | 1% |
| Run status + escalation + edge creation | ~168 | 3% |
| Memory subsystem | ~176 | 4% |
| **Total (approximate)** | **~4,791** | **~97%** |

The remaining ~157 lines are blank lines, comments, and struct definitions scattered throughout.

---

## 2. Context Compilation Pipeline (Full Detail)

**`generate_task_file()` (lines 2290-2917)** produces a markdown file at
`docs/spore/tasks/task-<run_id-8chars>.md`.

### Step 1: Task Embedding (lines 2305-2316)

- Generates embedding for the full task description using `local_embeddings::generate()`
  which uses the **All-MiniLM-L6-v2** model (384 dimensions, local inference, no API call).
- Also loads ALL node embeddings from the database once (`db.get_nodes_with_embeddings()`).
- Both are reused for anchor search and lesson ranking.

### Step 2: Anchor Node Selection (lines 2317-2412)

Two parallel searches, then merged:

**Semantic search** (lines 2325-2344):
- Calls `similarity::find_similar()` with the task embedding against all stored embeddings.
- Parameters: top 10 results, minimum similarity threshold 0.3.
- Cosine similarity in `similarity.rs` (line 65): dot product / (norm_a * norm_b).
- Filters out operational nodes (`node_class != "operational"`).
- Takes up to 5 non-operational results.

**FTS keyword search** (lines 2351-2380):
- Splits task on whitespace, then splits each token on non-alphanumeric characters.
- Strips leading/trailing non-alphanumeric chars.
- Filters out tokens <= 2 chars and stopwords (the, a, an, in, on, at, to, for, of, is, it, and, or, with, from, by, this, that, as, be).
- Joins remaining tokens with " OR " for FTS5 query.
- Calls `db.search_nodes()`, filters out operational and the task node itself, takes 5.

**Merge** (lines 2386-2411):
- Semantic results have priority (inserted first, deduped by node ID).
- FTS results added after, skipping duplicates.
- Truncated to 5 total anchors.
- Source labels tracked: "Semantic match" or "FTS match".

### Step 3: Dijkstra Expansion (lines 2414-2461)

For each anchor, calls `db.context_for_task()` (defined in `schema.rs` lines 5714-5867):

**Parameters passed from generate_task_file:**
- `budget`: 7 (max context nodes per anchor)
- `max_hops`: 4
- `max_cost`: 2.0
- `exclude_edge_types`: `["clicked", "backtracked", "session_item"]` (browser session noise)
- `not_superseded`: true
- `items_only`: true

**Dijkstra algorithm detail** (schema.rs lines 5754-5859):
- Standard min-heap priority queue. Distance = sum of edge costs along path.
- Relevance = `1.0 / (1.0 + distance)`. So distance 0 = relevance 1.0, distance 1 = 0.5.
- Traverses edges bidirectionally (both incoming and outgoing from each node).

**Edge cost formula** (schema.rs lines 5832-5841):
```
confidence = edge.confidence (default 0.5 if null)
type_priority = edge_type_priority(edge_type)  -- see table below
base_cost = ((1.0 - confidence) * (1.0 - 0.5 * type_priority)).max(0.001)
```

For structural edges (`DefinedIn`, `BelongsTo`, `Sibling`): `edge_cost = max(base_cost, 0.4)`
For all other edges: `edge_cost = base_cost`

**Edge type priorities** (schema.rs lines 5665-5672):
| Priority | Edge types |
|----------|-----------|
| 1.0 | Contradicts, Flags |
| 0.7 | DerivesFrom, Summarizes, Resolves, Supersedes |
| 0.5 | Supports, Questions, Prerequisite, EvolvedFrom |
| 0.3 | Everything else (Calls, Related, Reference, etc.) |

**Example costs** (at confidence 0.9):
- Contradicts (priority 1.0): `(0.1) * (1.0 - 0.5) = 0.05`
- DerivesFrom (priority 0.7): `(0.1) * (1.0 - 0.35) = 0.065`
- Calls (priority 0.3): `(0.1) * (1.0 - 0.15) = 0.085`
- DefinedIn (priority 0.3, confidence 0.9): `max(0.085, 0.4) = 0.4` (structural floor)

**Anti-flooding**: Structural edges always cost at least 0.4, preventing "same file"
traversals from consuming the entire 2.0 budget. Browser session edges are excluded entirely.

### Step 4: Dedup and Rank (lines 2463-2467)

- All context nodes from all anchors deduplicated by node ID, keeping highest relevance.
- Filtered to exclude the task node itself.
- Sorted by descending relevance.

### Step 5: Format Markdown (lines 2469-2917)

The task file contains these sections in order:

1. **Header** (lines 2473-2478): Task title (truncated 60 chars), run ID (8 chars),
   agent role, bounce N/max, UTC timestamp.

2. **Task** (lines 2480-2482): Full untruncated task description.

3. **Previous Bounce / Implementation to Check / Implementation to Summarize** (lines 2484-2516):
   Conditional section. For coder on bounce 2+: points to previous impl node and says to check
   contradicts edges. For verifier: "Implementation to Check" with node ID. For summarizer:
   "Implementation to Summarize" with node ID.

   If the last verdict was `Unknown`, a different message is shown: "verifier could not parse
   a verdict... review your changes carefully".

4. **Graph Context table** (lines 2518-2534): Markdown table with columns:
   `#`, `Node`, `ID` (12 chars), `Relevance` (percentage), `Via` (anchor + edge path).

5. **Code Locations** (lines 2537-2559): For `code-*` nodes, extracts `file_path`,
   `line_start`, `line_end` from the node's tags JSON. Formatted as bullet list:
   `` - `path` L{start}-{end} -- {title} ``

6. **Key Code Snippets** (lines 2562-2639): Top 5 code nodes (functions prioritized over
   structs). Reads actual file content, shows first 30 lines of each. Language detection
   from file extension for syntax highlighting. Shows "... (N more lines)" if truncated.

7. **Files Likely Touched** (lines 2641-2669): Groups code locations by file, ranked by
   node count per file. Shows up to 8 files with up to 3 node names each.

8. **Call Graph** (lines 2671-2753): For top 3 function nodes, queries incoming/outgoing
   `Calls` edges and shows callers/callees (up to 3 each).

9. **Lessons from Past Runs** (lines 2756-2874): Queries up to 20 lesson nodes
   (`title LIKE 'Lesson:%'`, `node_class = 'operational'`), ranks by embedding similarity
   to task (threshold 0.15), takes top 5 (pads with recency if <5 similar). Extracts
   "Pattern" and "Fix" sections from lesson content. Filters by `is_lesson_quality()`.

10. **Checklist** (lines 2900-2902): Static two-item reminder.

**Typical task file size**: 50-150 lines depending on graph density. Of which:
- ~8 lines header/metadata (boilerplate)
- ~5-10 lines task description
- ~20-40 lines graph context table + code locations
- ~30-60 lines code snippets (when present)
- ~5-15 lines call graph (when present)
- ~5-10 lines lessons (when present)
- ~3 lines checklist

---

## 3. Agent Communication Through the Graph

### Graph Writes by Role

**Orchestrator** (spore.rs, not an LLM agent -- pure Rust code):
- Creates `Orchestration: <task>` task node (operational, lines 3259-3268)
- Creates implementation nodes (`Implemented: <task>` or `Partial: <task>`, operational, lines 3588-3609)
- Creates `DerivesFrom` edge: impl -> task (lines 3612-3629)
- Creates `Related` edges: impl -> code nodes for changed files (lines 2207-2244)
- Creates verdict nodes when verifier uses structured `<verdict>` blocks (lines 3747-3787)
- Creates `Supports` or `Contradicts` edges from verdict node -> impl node (lines 3766-3787)
- Creates `Tracks` self-edges on task node for run status recording (lines 4599-4618)
- Creates `ESCALATION:` meta nodes with `Flags` and `Tracks` edges (lines 4630-4694)

**Coder** (via MCP tools):
- Does NOT create graph nodes or edges. The coder prompt explicitly says "the orchestrator
  handles all graph bookkeeping" (coder.md line 11). The orchestrator creates implementation
  nodes from git diff after the coder finishes.

**Verifier** (via MCP tools):
- Creates verification nodes and `Supports`/`Contradicts` edges via MCP calls.
- Also emits `<verdict>` JSON blocks in stdout, which the orchestrator parses as fallback
  (lines 1999-2031, 3738-3793).
- Three verdict detection layers: (1) graph edge from verifier agent_id, (2) structured
  `<verdict>` JSON parsed from stdout, (3) keyword scan of stdout text.

**Summarizer** (via MCP tools):
- Creates `Summary: <description>` nodes (operational, via `mycelica_create_node`)
- Creates `Summarizes` edges from summary -> task node
- Creates `Lesson: <insight>` nodes (operational) with `DerivesFrom` edges to summary

### Graph Reads by Role

**Context compiler** (runs before each agent, in `generate_task_file()`):
- Reads all node embeddings
- Runs semantic search + FTS search
- Runs Dijkstra expansion from anchors
- Reads code node tags for file_path/line_start/line_end
- Reads actual file content for code snippets
- Reads call graph edges (Calls type)
- Reads lesson nodes and their embeddings
- Reads memory store entries

**Coder** (via MCP tools during execution):
- `mycelica_explore` -- search + source + call graph in one call
- `mycelica_search` -- semantic search
- `mycelica_read_content` -- read node content by ID
- `mycelica_query_edges` -- query edges

**Verifier** (via MCP tools during execution):
- `mycelica_read_content` -- read implementation node
- `mycelica_nav_edges` -- check for supersession
- `mycelica_search` -- code exploration

**Summarizer** (via MCP tools during execution):
- `mycelica_read_content` -- read trail nodes (task, impl, verify)
- `mycelica_nav_edges` -- walk derives_from, contradicts, supports, supersedes, tracks edges
- `mycelica_create_node` -- write summary and lesson nodes
- `mycelica_create_edge` -- write summarizes and derives_from edges

### Data Flow (Complete Cycle)

```
1. generate_task_file() reads graph --> task file (markdown)
2. Coder reads task file + MCP reads --> edits code files
3. Orchestrator captures git diff --> creates impl node + DerivesFrom edge
4. post_coder_cleanup() re-indexes code --> creates Related edges to code nodes
5. generate_task_file() reads graph --> verifier task file
6. Verifier reads impl node + runs tests --> creates verdict edge (or <verdict> block)
7. Orchestrator detects verdict --> if PASS: summarizer; if FAIL: bounce to step 1
8. generate_task_file() reads graph --> summarizer task file
9. Summarizer reads trail --> creates Summary node + Summarizes edge + optional Lesson nodes
10. Future runs: generate_task_file() finds Lesson nodes via embedding similarity
```

---

## 4. What Each Agent Actually Receives

### Coder

**Agent file**: `.claude/agents/coder.md` (81 lines)
- Frontmatter: `name: coder`, `memory: project`, `skills: [mycelica-conventions]`
- Turn budget instructions (turns 1-3 read, 4-6 code locations, 7+ write code)
- Rules: focus on code, no graph writes, budget turns, output summary

**Task file**: `docs/spore/tasks/task-<id>.md` (~50-150 lines, varies)
- Contains: header, full task, graph context table, code locations, code snippets,
  files likely touched, call graph, lessons, memory entries, checklist

**Prompt composition** (lines 3385-3411):
- If native agent (`.claude/agents/coder.md` exists): task file path + task description only
- If no native agent: full template prepended to task file path + task

On bounce 2+ (lines 3386-3411):
- Additional feedback: "The Verifier found issues with node <id>. <reason>. Fix these specific issues."
- If session resume available: simplified prompt "The verifier rejected your changes. <feedback>"

**Tools allowed** (lines 3083-3086):
- Allowed: `Read, Write, Edit, Bash(*), mcp__mycelica__*`
- Disallowed: `Grep, Glob`

**Model**: opus (default), overridable via `--coder-model`

**Max turns**: 50 (default from CLI), configurable via `--max-turns`

**Total pre-work context**: ~81 lines agent file + ~50-150 lines task file + CLAUDE.md (loaded
via `memory: project`) + mycelica-conventions skill (~100 lines). Roughly 250-400 lines before
the agent starts doing anything.

### Verifier

**Agent file**: `.claude/agents/verifier.md` (82 lines)
- Frontmatter: `name: verifier`, `model: opus`, `memory: project`, `skills: [mycelica-conventions]`
- Turn budget: 1-2 read task, 3-4 read impl, 5-6 build+test, 7+ review
- Rules: never fix code, exact error messages, always output `<verdict>` JSON

**Task file**: Same structure as coder but with "Implementation to Check" section containing
the impl node ID instead of "Previous Bounce".

**Prompt composition** (lines 3670-3681):
- Native agent: "Read the task file... Verify implementation node <id>"
- Non-native: template + task file path + impl node ID

**Tools allowed** (lines 3087-3089):
- Allowed: `Read, Grep, Glob, Bash(cargo:*), Bash(cd:*), Bash(mycelica-cli:*), mcp__mycelica__*`
- Not explicitly disallowed (no Write/Edit though -- not in allowed list)

**Model**: opus (hardcoded via `select_model_for_role`)

**Max turns**: same as coder (shared `--max-turns`)

### Summarizer

**Agent file**: `.claude/agents/summarizer.md` (113 lines)
- Frontmatter: `name: summarizer`, `model: sonnet`, `memory: project`, `skills: [mycelica-conventions]`
- Instructions: read trail, create ONE summary node, optionally create lesson nodes
- Quality standards: 10-20 lines per summary, specific file paths/function names

**Task file**: Same structure but with "Implementation to Summarize" section.

**Prompt composition** (lines 3879-3890):
- Native agent: "Read the task file... Summarize the orchestrator run for task node <id>"
- Non-native: template + task file path + task node + impl node + bounce count

**Tools allowed** (lines 3898-3901, via spawn_claude):
- Allowed: not explicitly set (None), meaning defaults
- Disallowed: `Bash, Edit, Write`
- Effectively: MCP tools + Read + Grep only

**Model**: sonnet

**Max turns**: 15 (hardcoded at line 3897, not configurable)

### Operator (single-agent only, not in pipeline)

No `.claude/agents/operator.md` file exists (confirmed -- file not found).

**Tools allowed** (lines 3079-3082):
- Allowed: `Read, Write, Edit, Bash(*), Glob, Grep, mcp__mycelica__*`
- Full access to everything

**Model**: opus

**Max turns**: 80 (when default 50 is passed, line 3020)

---

## 5. Where Tokens Go

### Token Flow in a Typical Verified Run

**Context compilation** (CPU, no LLM tokens):
- Embedding generation: ~10ms (local MiniLM model, CPU)
- Similarity search: ~5ms (linear scan of ~2000 embeddings)
- Dijkstra expansion: ~20ms (SQLite queries)
- File reads for snippets: ~5ms
- Total: ~40ms, zero LLM tokens

**Coder phase**:
- Input context: agent file (~81 lines ~2K tokens) + task file (~100 lines ~3K tokens)
  + CLAUDE.md (~200 lines ~5K tokens) + mycelica-conventions skill (~100 lines ~3K tokens)
  = ~13K tokens before first action
- Per-turn overhead: Read/Edit tool calls add ~500-2K tokens each
- Typical run: 5-15 turns, ~30K-80K total input tokens, ~5K-15K output tokens
- Cost: $1-4 at opus pricing

**Verifier phase**:
- Input context: similar to coder (~13K tokens initial)
- Typical: 5-10 turns, ~20K-50K total input tokens, ~3K-8K output tokens
- Cost: $0.50-2.00 at opus pricing

**Summarizer phase**:
- Input context: ~13K tokens initial
- Typical: 3-8 turns, ~15K-30K total input tokens, ~2K-5K output tokens
- Cost: $0.10-0.30 at sonnet pricing (much cheaper per token)

**Overhead ratio** (approximate):
- Boilerplate context (CLAUDE.md + skills + agent file): ~10K tokens, ~30% of initial load
- Task-specific context (graph context + snippets + lessons): ~3K-8K tokens, ~20-25%
- Actual work (reading/writing code): ~50% of total tokens
- Summary: roughly 50% of tokens go to actual work, 30% to fixed boilerplate, 20% to
  graph-gathered context

### Cost Breakdown by Phase (from run data, typical verified single-bounce run)

| Phase | Cost | % of total |
|-------|------|-----------|
| Context compilation | $0.00 | 0% (CPU only) |
| Coder | $1.50-3.00 | 60-70% |
| Verifier | $0.50-1.50 | 20-30% |
| Summarizer | $0.10-0.30 | 5-10% |
| Orchestrator overhead | $0.00 | 0% (Rust code) |
| **Typical total** | **$2.10-4.80** | 100% |

---

## 6. The Bounce Loop (Mechanical Detail)

**`handle_orchestrate()` (lines 3156-3944)** implements the coder-verifier bounce loop.

### Setup (lines 3176-3301)

1. Resolve CLI binary path
2. Check `claude` is in PATH (fail-fast)
3. Resolve prompt file paths (CWD first, then repo root)
4. Read agent prompt templates from disk
5. Clean up old MCP configs (`rm -rf /tmp/mycelica-orchestrator/`)
6. Create or reuse task node
7. Initialize checkpoint
8. Define phase ordering: coder=0, verifier=1, summarizer=2

### Per-Bounce Sequence (lines 3303-3934)

The loop runs `for bounce in 0..max_bounces`:

#### Coder Phase (lines 3318-3644)

1. **Pre-snapshot**: Capture `git diff --name-only` and `git ls-files --others` + content hashes
2. **MCP config**: Write temp config for coder agent
3. **Task file**: Call `generate_task_file()` (detailed in Section 2)
4. **Compose prompt**:
   - Bounce 1: Full task file path + task description
   - Bounce 2+ with session resume: Simplified "verifier rejected... fix the code"
   - Bounce 2+ without session: Full prompt with verifier feedback injected
5. **Spawn coder**: `spawn_claude()` with allowed/disallowed tools
6. **Session resume fallback** (lines 3451-3464): If `--resume` failed, retry with fresh session
7. **Zero-turn retry** (lines 3466-3479): If 0 turns returned, wait 10s, retry once
8. **MCP failure check** (lines 3482-3484): If ALL MCP servers failed, abort
9. **Git diff capture** (lines 3529-3551): Same as pre-snapshot but "after"
10. **Abort check** (lines 3554-3560): If coder failed AND no file changes, abort run
11. **Create impl node** (lines 3588-3609): Node content includes task, files changed,
    diff stat, coder summary, agent stats (turns/cost/duration/exit code)
12. **DerivesFrom edge** (lines 3612-3629): impl -> task node
13. **Post-coder cleanup** (line 3642): Re-index, rebuild CLI, create Related edges

#### Verifier Phase (lines 3655-3856)

1. **MCP config**: Write temp config for verifier
2. **Task file**: `generate_task_file()` with `last_impl_id` set
3. **Compose prompt**: Points verifier at impl node ID
4. **Spawn verifier**: With restricted tools (no Write/Edit)
5. **Single retry** (lines 3697-3710): If subprocess fails, wait 10s, retry once
6. **Thinking log** (lines 3733-3736): Written to `docs/spore/tasks/think-verifier-<id>.log`
7. **Verdict detection** (3-layer fallback, lines 3739-3856):
   - Layer 1: `check_verdict()` -- scan graph edges for supports/contradicts from verifier
   - Layer 2: `parse_verifier_verdict()` -- parse `<verdict>{"verdict":"supports",...}</verdict>` from stdout
   - Layer 3: `parse_verdict_from_text()` -- keyword scan for "PASS"/"FAIL" in stdout

   Layers 2 and 3 create graph nodes/edges themselves when they fire (the orchestrator
   materializes the verdict into the graph).

#### Verdict Branching (lines 3857-3934)

**Supports** (lines 3858-3921):
1. Print "TASK COMPLETE"
2. Clear checkpoint
3. Run summarizer if enabled (lines 3866-3918):
   - Generate summarizer task file
   - Spawn summarizer (15 turns, sonnet, no Bash/Edit/Write)
   - Record status
4. Return task_node_id

**Contradicts** (lines 3923-3927):
1. Set `last_impl_id` for next bounce
2. Set `last_verdict = Some(Contradicts)`
3. Loop continues to next bounce

**Unknown** (lines 3928-3932):
1. Warning printed
2. Set `last_verdict = Some(Unknown)`
3. Loop continues (treated same as Contradicts -- another bounce attempt)

### Session Resume Detail (lines 3419-3464)

On bounce 2+:
- `last_coder_session_id` is set from the previous coder's `session_id` field
- If available, passes `--resume <session_id>` to claude CLI
- The prompt is simplified: "The verifier rejected your changes. <feedback>"
- Does NOT pass `--agent` when resuming (line 3433)
- If resume fails (non-zero exit), falls back to fresh session with full prompt

### Verifier Feedback Injection (lines 3378-3384, 3390-3391)

The orchestrator extracts the verifier's rejection reason and injects it into the
coder's bounce-2 prompt:

- If `last_verdict == Unknown`: "verifier could not parse a verdict... review carefully"
- If `last_verdict_reason` is set (from structured `<verdict>` JSON): "verifier rejected: <reason>"
- Otherwise: "Check its incoming contradicts edges and fix the code"

For text-fallback contradicts (lines 3800-3812), the orchestrator extracts useful
failure lines from verifier stdout by scanning for keywords: "FAIL", "error", "Error",
"failed", "Failed", "panicked", "assertion", "expected", "not found", "compile error".
Truncated to 500 chars.

---

## 7. The Loop Engine

**`handle_spore_loop()` (lines 4249-4482)** runs multiple tasks sequentially from a file.

### Initialization (lines 4266-4316)

1. Read tasks from source file via `read_task_source()` (lines 4065-4079)
2. Load persisted loop state from `<source>.loop-state.json` (lines 4269-4276)
3. If `--reset`, delete state file first
4. Load agent prompt paths and memory store
5. Print config summary

### Task File Format (lines 4081-4129)

`parse_task_content()` supports two formats:
- **One task per line**: Simple, blank lines and `#` comments ignored
- **Multi-line tasks**: Sections separated by `---` on its own line. Lines within each
  section are joined with spaces.

### Per-Task Loop (lines 4318-4477)

For each task:

1. **Budget check** (line 4320): Stop if `total_cost >= budget`
2. **Max runs check** (line 4326): Stop if `results.len() >= max_runs`
3. **Escalation check** (line 4332): Stop if 3 consecutive escalations
4. **Skip check** (line 4338): Skip if task already verified in loop state
5. **Dispatch** (lines 4358-4366): Call `handle_orchestrate()` with full pipeline
6. **Cost extraction** (lines 4371-4374): Find orchestration node created after start
   timestamp, sum cost from tracks edges
7. **Status determination** (lines 4376-4394): Ok = Verified, Err with escalation = Escalated,
   Err otherwise = Failed
8. **State persistence** (lines 4407-4410): Immediately persist to loop state file
9. **Auto-commit** (lines 4419-4439): On verified task, run `selective_git_add()` + `git commit`
   with message `feat(loop): <task>`
10. **Cost anomaly detection** (lines 4458-4469): If current task cost > 3x running average,
    print warning
11. **Cooldown** (lines 4473-4476): 5-second pause between tasks

### Loop State Persistence (lines 3970-4062)

`LoopState` stored as JSON:
```json
{
  "source": "tasks.txt",
  "verified_tasks": ["task description 1", ...],
  "total_cost": 12.50,
  "runs": [
    {"task": "...", "status": "verified", "cost": 2.50, "duration_ms": 45000, ...}
  ],
  "created_at": "2026-02-21T...",
  "updated_at": "2026-02-21T..."
}
```

State file path: same directory as task file, with `.loop-state.json` suffix
(e.g., `tasks.txt` -> `tasks.loop-state.json`).

### Stop Conditions

1. Budget exhausted (`total_cost >= budget`)
2. Max runs reached
3. 3 consecutive escalations (systemic failure detection)
4. `--pause-on-escalation` flag (stop on first escalation)
5. All tasks processed

### Summary Output (lines 4185-4247)

`print_loop_summary()` prints: total dispatched, verified count/rate, escalated count,
failed count, total cost vs budget, average cost per task, total duration, per-task breakdown.

---

## Also: spore_runs.rs (3,623 lines)

Analytics and reporting module, extracted from spore.rs. Key functions:

| Function | Lines | Purpose |
|----------|-------|---------|
| `handle_health()` | 17-195 | 5 checks: database, stale code, orphan edges, embedding coverage, prompt size |
| `count_agent_prompt_lines()` | 215-243 | Count lines in `docs/spore/agents/*.md` |
| `handle_prompt_stats()` | 245-260 | Print prompt size per agent |
| `handle_spore_lessons()` | 266-346 | List lesson nodes (compact/full/json modes) |
| `handle_dashboard()` | 348-771 | Recent runs, costs, stale detection, experiment comparison |
| `handle_runs()` | 773-3093 | Run list, timeline, detail, compare subcommands |
| `handle_runs_compare_experiments()` | 3095-3372 | A/B experiment comparison |
| `handle_distill()` | 3374-3623 | AI-powered run distillation (spawns claude for analysis) |

**Note**: `handle_prompt_stats()` scans `docs/spore/agents/` which is the OLD location.
The actual agent files are now in `.claude/agents/`. This is a **stale code path** -- the
prompt stats command will report "no agent files found" because it looks in the wrong directory.

---

## Observations and Surprises

1. **Coder creates zero graph nodes.** The orchestrator creates ALL implementation nodes
   from git diff. This was a deliberate design choice (line 3523 comment: "eliminates the
   28% fallback rate and saves 2-3 coder turns per run").

2. **Verifier has retry.** Despite the MEMORY.md note "Verifier retry not yet implemented",
   there IS a single retry at lines 3697-3710 (subprocess failure retry with 10s cooldown).
   This was apparently added and the memory not updated.

3. **Prompt paths are stale.** The retry command (line 838) and batch command (line 917) both
   hardcode `docs/spore/agents/{coder,verifier,summarizer}.md` as prompt paths. But the actual
   agent definitions live at `.claude/agents/{coder,verifier,summarizer}.md`. When native agent
   mode is active (which it always is, since those files exist), the template file content
   is never injected into the prompt (line 3385-3387: native agent mode skips the template).
   So the stale paths don't break anything in practice, but the template files at
   `docs/spore/agents/` are dead weight if they still exist.

4. **The structural edge floor (0.4) is critical.** Without it, the Dijkstra expansion would
   follow high-confidence DefinedIn edges (cost ~0.001) and fill the entire budget with
   "things in the same file" rather than semantically relevant nodes.

5. **handle_prompt_stats scans wrong directory.** See note in spore_runs.rs section above.

6. **Operator has no native agent file.** `.claude/agents/operator.md` doesn't exist, so
   `resolve_agent_name("operator")` returns `None`. The operator falls back to reading
   `docs/spore/agents/operator.md` as its template (which does exist -- 885 bytes). This
   means the operator runs in non-native mode: the full template is prepended to the prompt
   instead of using Claude Code's `--agent` flag. This is functional but inconsistent with
   the other 3 roles which all have native agent files.

7. **Summarizer is hardcoded to 15 turns** (line 3897) regardless of `--max-turns` flag.
   This is not documented anywhere.

8. **Run costs are stored as self-referential edges.** `record_run_status_with_cost()` creates
   edges where `source_id = target_id = task_node_id` with type `Tracks`. This is semantically
   odd (a node tracking itself) but works as a metadata attachment mechanism.

9. **The `--experiment` label flows through to tracks edge metadata** but has no effect on
    agent behavior. It's purely for analytics comparison in `handle_runs_compare_experiments()`.
