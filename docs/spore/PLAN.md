# Spore — Implementation Plan

> The agent coordination layer for Mycelica. Spore turns the knowledge graph into a self-describing substrate for Claude Enterprise multi-agent orchestration.

## Context

Mycelica is a Tauri desktop app (Rust backend, React frontend, SQLite) with CLI, GUI, and team server. It stores knowledge as a navigable graph — nodes with content, edges with relationships, organized into a drillable hierarchy. The CLI (`mycelica-cli`) reads/writes the same database with commands for node CRUD, edge traversal, semantic search, hierarchy building, and maintenance.

The goal: make Mycelica the **memory layer** for a multi-agent Claude Enterprise system. Agents read and write the graph through MCP tools. They communicate by creating nodes and edges that other agents read and respond to. The human operator reads the top layer of the graph (meta-nodes) to see whether the system is coherent.

Gas Town and Beads become unnecessary — Enterprise handles orchestration, Mycelica handles persistent knowledge.

## Current State

### What Exists

- SQLite database with `nodes` (39 columns) and `edges` (17 columns) tables
- Node types: `idea`, `question`, `exploration`, `paper`, `code_function`, `code_struct`, `code_enum`, `code_impl`, `code_doc`, `concept`, `decision`
- Edge types (26 total): `calls`, `documents`, `related`, `reference`, `contains`, `belongs_to`, `defined_in`, `uses_type`, `implements`, `sibling`, `supports`, `evolved_from`, `questions`, `summarizes`, `tracks`, `flags`, `resolves`, `derives_from`, plus legacy cased variants
- Edge schema: `id, source_id, target_id, type, label, weight, edge_source, evidence_id, confidence, created_at, updated_at, author, reason, content, agent_id, superseded_by, metadata`
- Node Spore fields: `agent_id`, `node_class` (default 'knowledge'), `meta_type`
- Hierarchy: Universe (root) → intermediate levels → Items (leaves), with tiered child limits
- CLI commands: `nav tree`, `nav edges`, `node get`, `node search`, `import`, `setup`, `maintenance`, `tui`, `link`, `migrate spore-schema`
- Spore commands: `spore query-edges`, `spore explain-edge`, `spore path-between`, `spore edges-for-context`, `spore create-meta`, `spore update-meta`, `spore status`
- Holerabbit Firefox extension sending browsing activity to the backend
- Conversation import pipeline (Claude conversations → nodes + edges)
- Code import pipeline (Rust source → function/struct/edge nodes)
- Local embeddings via candle framework
- Semantic similarity edge computation (0.7 threshold)
- 1624 nodes (1137 items, 487 categories), 7530 edges, all embedded
- All existing nodes backfilled with `agent_id='human'`, `node_class='knowledge'`
- All existing edges backfilled with `agent_id='human'`
- Meta nodes survive hierarchy rebuilds via `human_created=true` (existing sovereignty system)

---

## Phase 1: Schema Evolution ✅ COMPLETE

Committed Feb 13, 2026. +394 lines across 14 files. ~1 hour.

1. ✅ Edge content/reasoning fields — edges now carry full reasoning text
2. ✅ Agent attribution on nodes and edges — `agent_id` field on both
3. ✅ Meta-node type system — `node_class` ('knowledge'|'meta'|'operational') + `meta_type` ('summary'|'contradiction'|'status')
4. ✅ Five new edge types — Summarizes, Tracks, Flags, Resolves, DerivesFrom
5. ✅ Supersession tracking — `superseded_by` on edges, queries default to `WHERE superseded_by IS NULL`
6. ✅ Six new indexes for agent/class/meta/supersession/confidence queries
7. ✅ Migration CLI command with pre-migration backup

Known issues (non-blocking):
- `serde rename_all="lowercase"` vs `as_str()` mismatch for multi-word edge types (pre-existing)
- CLI link error message lists only 8 of 26 edge types (cosmetic)
- Backfill set `agent_id='human'` on system-generated edges too — acceptable inconsistency

---

## Phase 2: Edge-Centric CLI Commands ✅ COMPLETE

Committed Feb 14, 2026. ~1 hour.

1. ✅ `spore query-edges` — multi-filter with dynamic WHERE, JOINs source+target
2. ✅ `spore explain-edge` — edge + nodes + adjacent edges + supersession chain
3. ✅ `spore path-between` — BFS with edge-type filtering and cycle prevention
4. ✅ `spore edges-for-context` — ranked top-N by composite score (recency × confidence × type priority)
5. ✅ `spore create-meta` — transactional meta-node creation with typed edges, hierarchy-protected
6. ✅ `spore update-meta` — in-place meta-node updates
7. ✅ `spore status` — dashboard with meta-node counts, contradiction list, coverage metric
8. ✅ Enhanced `link` command with `--content`, `--agent`, `--confidence`, `--supersedes`

DB query methods added to schema.rs:
- `query_edges()`, `explain_edge()`, `edges_for_context()`, `path_between()`
- `supersede_edge()`, `get_supersession_chain()`, `get_meta_nodes()`
- `create_meta_node_with_edges()` — transactional, uses `conn.transaction()`

---

## Single-Agent Validation Test ✅ PASSED

Executed Feb 14, 2026. Gate test from SPORE_CONCERNS.md Concern #8.

CLI Claude used spore commands to read a 9-node design documentation region and write a summary:

**Artifacts created:**
- 1 summary meta-node (302f3ec6) — "Design Documentation Region: State Assessment" with 9 `summarizes` edges
- 2 contradiction meta-nodes:
  - b75b0dd9 — Vision doc lists edge type classification as future, but Spore Phase 1 already shipped it
  - 29f92d43 — Clustering Spec documents entirely deleted code module (~2,570 lines removed in v0.9.1)

**Assessment:** Summary was genuinely useful editorial work, not filler. Caught real issues a developer would want to know. 70% of value achievable with 95% reliability for structural/temporal analysis. Editorial judgment (severity, "this spec is actively harmful to follow") harder to automate but still produced useful results.

**Dashboard after test:** 3 meta nodes, 12 new edges, 0.8% coverage (9/1137 nodes), coherence 0.9996.

**Six gaps identified (must fix before MCP server):**

| # | Gap | Severity | Description |
|---|-----|----------|-------------|
| 1 | No `spore create-edge` | **Critical** | Synthesizer agent blocked. Only `create-meta` exists — forces meta-node for every annotation. Need direct edge creation between existing nodes. |
| 2 | Content truncation | **Critical** | `node get` truncates. Agent had to read source files via filesystem, which won't work through MCP. Need `--full` flag or `spore read-content`. |
| 3 | No region discovery | **Important** | Test required hardcoded node IDs. Need `spore list-region <category-id>` for agents to discover what to analyze. |
| 4 | `code show` fails on markdown | Low | Markdown docs lack `line_start` in tags. Workaround: use `read-content` instead. |
| 5 | No staleness primitives | Low | `spore check-freshness` to compare doc timestamps vs referenced code. Approximated manually for now. |
| 6 | `update-meta` destroys history | **Important** | Old content lost on update. Need node supersession (Concern #5). |

---

## Phase 3: Test Gap Fixes ✅ COMPLETE

Completed Feb 16, 2026. All 6 gaps closed. ~280 lines added across 3 files.

### 3a. `spore create-edge` ✅
Delegates to `handle_link()` with `edge_source="spore"`. All edge types supported.
```bash
mycelica-cli spore create-edge --from <id> --to <id> --type supports --agent spore:synthesizer --confidence 0.85
```

### 3b. `spore read-content` + `node get --full` ✅
- `spore read-content <id>` — prints raw content only (no metadata noise)
- `node get <id> --full` — full content + tags, node_class, meta_type, agent_id, source fields

### 3c. `spore list-region` ✅
Recursive CTE `get_descendants()` in schema.rs. Filters by `--class` and `--items-only`.
```bash
mycelica-cli spore list-region <category-id> --items-only --limit 50
```

### 3d. Node supersession for `update-meta` ✅
`update-meta` now creates a NEW meta node linked to old via `Supersedes` edge. Old node's outgoing edges copied (excluding superseded edges AND Supersedes-typed edges). `get_meta_nodes()` filters superseded nodes from dashboard.

### 3e. `code show` markdown fix ✅
`line_start`/`line_end` now `Option<usize>` with `#[serde(default)]`. Missing values = show whole file. Also handles `language` field.

### 3f. `spore check-freshness` ✅
Compares `updated_at` timestamps between summary and source nodes. Reports STALE/fresh status.

---

## Phase 4: MCP Server ✅ COMPLETE

Committed Feb 17, 2026. ~1370 lines in `src-tauri/src/mcp.rs`. Feature-gated behind `--features mcp`.

### 4a. Implementation

- Use `rmcp` crate (v0.15) with stdio transport
- New module: `src-tauri/src/mcp.rs`, feature-gated behind `mcp`
- Launch: `mycelica-cli mcp-server --stdio --agent-role <role> --agent-id <id>`
- Per-agent permission scoping — server only registers tools permitted for role
- Agent ID injection — server stamps `--agent-id` on all writes automatically
- Compound operations wrapped in SQLite transactions
- All responses as structured JSON
- `Parameters<T>` wrapper required for rmcp v0.15 tool parameter extraction (Axum-style extractor pattern)
- SQL aggregate queries for status/stats (no full-table scans)
- SlimNode/SlimEdge output structs to minimize token usage in LLM context windows

### 4b. MCP tools (16 total: 12 read + 4 write)

**Read tools (all agents):**

| MCP Tool | Maps To |
|----------|---------|
| `mycelica_search` | `node search <query>` |
| `mycelica_node_get` | `node get <id> --full` |
| `mycelica_read_content` | `spore read-content <id>` |
| `mycelica_nav_edges` | `nav edges <id>` |
| `mycelica_query_edges` | `spore query-edges [filters]` |
| `mycelica_explain_edge` | `spore explain-edge <id>` |
| `mycelica_path_between` | `spore path-between <a> <b>` |
| `mycelica_edges_for_context` | `spore edges-for-context <id>` |
| `mycelica_list_region` | `spore list-region <category-id>` |
| `mycelica_check_freshness` | `spore check-freshness <id>` |
| `mycelica_status` | `spore status --json` |
| `mycelica_db_stats` | `db stats --json` |

**Write tools (scoped per role):**

| MCP Tool | Maps To |
|----------|---------|
| `mycelica_create_node` | Node creation with class and optional connections |
| `mycelica_create_edge` | `spore create-edge [params]` |
| `mycelica_create_meta` | `spore create-meta [params]` |
| `mycelica_update_meta` | `spore update-meta [params]` (supersession-based) |

### 4c. Permission matrix

| Tool Category  | Ingestor | Coder | Verifier | Planner | Synthesizer | Summarizer | DocWriter | Human |
|----------------|----------|-------|----------|---------|-------------|------------|-----------|-------|
| Read (all)     | ✓        | ✓     | ✓        | ✓       | ✓           | ✓ (no meta)| ✓         | ✓     |
| Node create    | ✓        | ✓     | ✓        | ✓       |             |            |           | ✓     |
| Edge create    | ✓        | ✓     | ✓        | ✓       | ✓           | ✓          | ✓         | ✓     |
| Meta create    |          |       |          | ✓       |             | ✓          |           | ✓     |
| Meta update    |          |       |          |         |             | ✓          |           | ✓     |

Summarizer's read tools apply recursion guard: filter meta nodes from results (except contradictions).
Synthesizer's read tools apply recursion guard: only knowledge/operational items.

### Smoke tests passed:
- initialize → valid JSON-RPC response
- tools/list → all 16 tools with JSON Schema input schemas
- tools/call → structured JSON responses
- Permission filtering → role-specific tool visibility
- Permission denial → clear error on unauthorized tool calls

### Effort: ~3 hours (including SQL aggregate optimization)

---

## Phase 5: Agent Definitions -- COMPLETE

**Goal:** Define agent roles with system prompts, MCP configs, and interaction patterns. Started with Coder + Verifier. Session 9 simplified to 3 pipeline agents (coder, verifier, summarizer) plus operator for manual use.

### Two-layer agent context model

Agent knowledge exists on two layers:

**Bootstrap layer (`.md` files, static):** Identity, MCP tool list, hard rules, workflow instructions. The minimum an agent needs to connect and start working. Checked into the repo, rarely changes. This is what the agent reads before it can use the graph.

**Runtime context layer (graph nodes, evolves):** Current priorities, project-specific conventions, learned patterns from past bounces, role-specific knowledge accumulated over time. The agent reads these via MCP tools after connecting.

The flow: agent reads `coder.md` (bootstrap) → connects to MCP → calls `mycelica_search("spore:coder context")` → reads runtime context nodes → starts working.

The runtime layer is where agents learn over time. The Summarizer writes "lessons learned" nodes from bounce trails. The Coder reads those next session and avoids repeating mistakes. The graph teaches agents — their instructions evolve without editing `.md` files.

**Bootstrap stays as files because of the chicken-and-egg problem:** the agent needs instructions to know how to use the graph, but those instructions would be in the graph. Static `.md` files break the cycle.

### Interaction model: bouncing, not pipeline

Agents don't run in strict sequence. They communicate through the graph by writing nodes and edges that other agents read and respond to. The graph accumulates the deliberation. The Summarizer reads all of it and explains conclusions to the human.

Three concurrent execution patterns:

**The Build Loop** (continuous):
```
Coder writes → Verifier checks → bounces back → Coder fixes → Verifier re-checks → ...
```

**The Summary Pass** (on-demand):
```
Summarizer reads everything (except meta) → creates/updates top-layer meta-nodes
```

### The bounce pattern

Agent A writes a concern node + typed edge (contradicts/questions) targeting Agent B's work. Agent B reads it, responds with its own node linked by `supersedes` (agrees, fixes) or `contradicts` (pushes back). The graph records every exchange. After max bounces without resolution → orchestrator escalates → Summarizer surfaces "UNRESOLVED" at top layer → human decides.

> **Historical note (session 9):** The agent definitions below (Agents 1-7) document the original Phase 5 design. Session 9 simplified to 3 pipeline agents: coder, verifier, summarizer (+ operator for manual use). The ingestor, planner, synthesizer, and doc writer roles were never fully deployed; researcher, architect, and tester were deployed then deleted.

### Agent 1: Ingestor

**ID:** `spore:ingestor` | **Writes:** knowledge/operational nodes, initial edges | **Cannot:** create meta-nodes, supersede edges

Gets new content in. Searches before creating to prevent duplicates. Conservative confidence (0.3-0.6) on initial edges — the Synthesizer refines them.

### Agent 2: Coder

**ID:** `spore:coder` | **Writes:** code files, operational nodes, edges | **Cannot:** create meta-nodes, verify own work

Before coding: queries graph for context, decisions, constraints, and plan nodes. After coding: creates operational node describing what was implemented and why, linked to plan/decision nodes with `implements`/`derives-from`. When Verifier bounces back (a `contradicts` edge): reads concern, fixes code, creates new explanation node with `supersedes` edge. **Never marks own work as verified.**

### Agent 3: Verifier

**ID:** `spore:verifier` | **Writes:** operational nodes (verification/failure), edges | **Can:** bash (cargo check/test) | **Cannot:** fix code, create meta-nodes

The bounce loop:
- Read Coder's recent implementation nodes → read the actual code → run `cargo check`, `cargo test`, manual review
- **Pass:** verification node with `supports` edge (high confidence) to implementation
- **Fail:** failure node with specific error details (error message, line number, logic flaw) and `contradicts` edge
- **Partial:** separate nodes for what passes and fails
- Loop terminates when all implementation nodes have high-confidence `supports` edges, or human intervenes

Must be specific: "cargo check fails: lifetime error in schema.rs:142, self.conn borrowed while mutation attempted" — not "code has issues."

### Agent 4: Planner

**ID:** `spore:planner` | **Writes:** operational + plan meta nodes, edges | **Reads:** ALL agent output (only agent besides Summarizer with full scope)

Checks plan alignment. Reads plan nodes + all recent activity from every agent. When aligned: status node with `supports` edge to plan. When drifting: concern node with edges to BOTH the plan node AND the diverging implementation, explaining the specific gap. When the plan itself is wrong (reality revealed bad assumptions): plan revision node with `supersedes` edge — **human must confirm revisions.**

Other agents read Planner concerns. Coder adjusts or pushes back with `questions` edge. The graph records the negotiation.

### Agent 5: Synthesizer

**ID:** `spore:synthesizer` | **Writes:** edges only (create + supersede) | **Cannot:** create or modify nodes

Creates typed edges between existing nodes. Edge `content` must be specific ("A's schema migration adds the column B's sovereignty spec requires") not vague. Contradiction detection is highest priority. Supersedes outdated edges with explanation of what changed.

Recursion guard: `WHERE (node_class = 'knowledge' OR node_class = 'operational') AND is_item = 1`

### Agent 6: Summarizer (The Auditor)

**ID:** `spore:summarizer` | **Writes:** meta-nodes, summarizes/contradicts edges | **Cannot:** create knowledge/operational nodes, supersede edges

Creates and maintains top-layer meta-nodes — what the human reads. Summarizes *deliberations between agents*, not just facts:

- Coder+Verifier bounced 3x on schema: "Schema migration: failed twice (lifetime errors), resolved third iteration. Verifier confirms."

Every meta-node MUST have `summarizes` edges to source nodes — these are the citations the human follows to verify. Updates via supersession (Phase 3d), not in-place destruction.

Recursion guard: `WHERE node_class != 'meta' OR (node_class = 'meta' AND meta_type = 'contradiction')`

### Agent 7: Doc Writer

**ID:** `spore:docwriter` | **Writes:** .md files, optionally `documents` edges | **Cannot:** create nodes

Reads meta-nodes first (high-level state), drills into knowledge layer for details. Validates own output against graph after writing. Flags uncertainty.

### Files created (Coder + Verifier only):

| File | Purpose |
|------|---------|
| `docs/spore/agents/coder.md` | Coder bootstrap prompt |
| `docs/spore/agents/verifier.md` | Verifier bootstrap prompt |
| `docs/spore/agents/mcp-coder.json` | MCP config with `--agent-role coder` |
| `docs/spore/agents/mcp-verifier.json` | MCP config with `--agent-role verifier` |
| `docs/spore/agents/README.md` | Launch guide + validation test |

### Lessons from Phase 5 testing:

1. **`--features mcp` must be in install command** — agent ran `cargo install` without it, overwrote MCP-enabled binary. Fixed in CLAUDE.md.
2. **Graph recording is optional unless enforced** — agents skip "after coding" steps due to attention decay. Phase 5 fix: emphatic rules. Phase 6 fix: orchestrator post-run cleanup handles re-indexing and code node edges structurally, reducing agent's post-coding job to one node + one edge.
3. **Agents default to Grep over mycelica-cli search** — Grep burns thousands of tokens per call, accelerating attention decay. Phase 5 fix: prompt instructions. Phase 6 fix: `--allowedTools` blocks Grep for Coder entirely.
4. **Full path required in MCP config** — Claude Code doesn't inherit shell PATH. Must use full absolute path (e.g. `~/.cargo/bin/mycelica-cli`) not bare `mycelica-cli`.

### Effort: 1-2 days for Coder + Verifier. Remaining agents after bounce loop validates.

---

## Phase 6: Pipeline Orchestration ✅ COMPLETE

Committed Feb 17, 2026. ~950 lines across cli.rs. 4 successful end-to-end runs validated.

### What was built:

**Orchestrate command:** `mycelica-cli spore orchestrate "task description" --max-bounces N --verbose`

Automates the full Coder → Verifier bounce loop:
1. Creates task node in graph
2. Generates Coder prompt with task + agent instructions
3. Spawns Claude Code with MCP config, streams output in real-time
4. Finds Coder's operational node, runs post-coder cleanup
5. Swaps MCP config to verifier role, spawns Verifier
6. Reads verdict (supports/contradicts edge), decides continue or bounce
7. On contradiction: bounces back to Coder with feedback context
8. On support or max-bounces: exits with summary

**Streaming output:** `spawn_claude()` uses `--output-format stream-json` with line-by-line parsing. Prints real-time progress: `[coder] $ cargo check`, `[coder] mcp: mycelica_search`, `[verifier] tool: Read`. Replaces 2-4 minute black box with visible agent activity.

**Post-coder cleanup (structural enforcement):** Between Coder exit and Verifier launch, the orchestrator:
- Captures dirty + untracked files before/after via `git diff --name-only` and `git ls-files --others`
- Re-indexes all changed .rs files: `mycelica-cli import code <file> --update`
- Reinstalls CLI + sidecar if src-tauri/ files changed
- Creates `related` edges from impl node to code nodes matching changed files
- All failures warn but don't abort orchestration

This removes code index updates, CLI reinstall, and code node edge creation from the agent's responsibilities entirely. The agent's post-coding job is now: build check, create one node, create one edge.

**Tool restrictions via --allowedTools:** Coder gets `Read,Write,Edit,Bash(*),mcp__mycelica__*` — no Grep or Glob. Verifier gets `Read,Grep,Glob,Bash(cargo:*),Bash(cd:*),Bash(mycelica-cli:*),mcp__mycelica__*`. Structural enforcement, zero prompt tokens, impossible to ignore.

**Run tracking:**
- `spore runs list` — all runs with edge counts, timestamps, agents
- `spore runs get <id>` — nodes/edges from a specific run
- `spore runs rollback <id>` — remove a run's output
- `spore runs rollback <id> --dry-run` — preview what would be deleted
- `spore query-edges --compact` — one-line-per-edge output

**Prompt slimming:** Agent prompts reduced from 375 to 155 lines total (59% reduction). MCP tool listings removed (agents discover via handshake). Steps handled by orchestrator removed from agent prompts. Verifier graph hygiene checks removed (orchestrator re-indexes before Verifier runs).

**Escalation:** max-bounces flag (default 3). After N bounces without convergence, orchestrator exits with status for human review. Planner-driven escalation deferred to Phase 7.

**Validated runs (all converged in 1 bounce):**
1. `--json` flag for `db stats` ($0.67)
2. `--compact` flag for `spore query-edges` ($0.96)
3. `--dry-run` flag for `spore runs rollback` ($1.44)
4. Post-run cleanup + --allowedTools (self-hosted)

### Lessons from Phase 6:

1. **Agents forget late-session instructions** — the root cause of skipped graph recording is attention decay, not bad prompts. Solution: move post-coding work from agent to orchestrator (structural enforcement).
2. **Grep wastes context** — a single Grep on cli.rs burns thousands of tokens, accelerating attention decay. Solution: --allowedTools blocks Grep for Coder, mycelica-cli search is the enforced alternative.
3. **Streaming output is essential** — 2-4 minute black box sessions are unusable. Real-time progress makes the system trustworthy.
4. **Cost baseline:** ~$0.50-0.80 per agent invocation (Opus, 15-30 turns). Full orchestration ~$1-1.50.

---

## Phase 6.5: Task File Generation ✅ COMPLETE

Implemented in Session 3. `generate_task_file()` writes graph-compiled task files to disk before spawning each agent.

### What was built:

`generate_task_file()` creates `docs/spore/tasks/task-{run_id_prefix}.md` containing:
- Task description with agent role and expectations
- Relevant code anchors (from semantic search + Dijkstra traversal)
- Inline code snippets from anchor nodes
- Accumulated lessons (from summarizer, filtered by `is_lesson_quality()`)
- Verifier feedback (on bounces)
- Checklist of required outputs

### Key design:

Task files are the primary context delivery mechanism. The orchestrator compiles graph context into a single file that the agent reads at session start. This replaced fat system prompts with focused, per-task context. Files are committed with the agent's code changes, creating a complete audit trail: task file → code changes → graph nodes → edges.

Context compilation uses `compile_context_for_task()` which runs semantic search against the task description, then gathers code anchors via graph edges and Dijkstra-weighted traversal (Phase 8 partial implementation).

---

## Post-Phase 6 Progress (Sessions 3-8)

### Session 3: Foundation Hardening

- **Native agent porting:** `.claude/agents/coder.md` agent file, `agent_name` param in `spawn_claude()`, `--native-agent` CLI flag. A/B test showed cost parity ($0.33 vs $0.32 per coder invocation).
- **Agent startup retry:** 10-second cooldown + single retry for 0-turn failures.
- **Lesson structuring:** Situation/Mistake/Fix/Evidence triples in `generate_task_file()` and summarizer output.
- **Prompt shrinkage metric:** `spore prompt-stats` command + health check integration. Baseline 732 lines.
- **Pending run triage:** Diagnosed all 26 pending runs. 17 ghosts cancelled, 8 parent planners identified, 1 abandoned. Real verification rate: 73% (vs 53% headline).
- **Code index refresh:** 1507 stale nodes reimported. Edge count cleaned from 12,146 to 3,922.
- **Pre-existing test fixes:** 1 fix + 4 `#[ignore]`. 511 tests pass, 5 ignored.

### Session 5: Cross-Codebase Portability

- **Portability proven:** 10/10 verified on fd (Rust codebase) + 3/3 on commander.js (TypeScript). Multi-file, multi-language. $0.96 avg cost per task.
- **Tester language gate fix (historical):** Removed `.rs` extension check. (Tester role later deleted in session 9.)
- **Tester post-verification fix (historical):** Removed hardcoded `cargo +nightly test`. (Tester role later deleted in session 9.)
- **`--experiment` flag:** A/B run comparison. Stores label in Tracks metadata. `spore runs stats --experiment <label>`.
- **Lesson quality filter:** `is_lesson_quality()` rejects trivial lessons in task files and dashboard.
- **Spore-on-Spore validated:** Self-modification (editing spore.rs) verified working.

### Session 6: A/B Model Validation

- **A/B experiments:** Two batches (moderate + hard tasks). Opus coder 39% cheaper than sonnet overall (fewer turns compensate for higher per-token price; advantage compounds with complexity: 28% to 47%).
- **Model routing update:** `select_model_for_role("coder")` always returns opus. Data-driven decision from A/B results.
- **`--coder-model` flag:** Override coder model per-run. Model metadata stored in Tracks edges.

### Session 7: System Audit and Hardening

- **Full system audit:** 4 parallel Hypha agents audited orchestration, verification, context compilation, and loop/MCP layers. Found 5 bugs, 9 design concerns, 6 silent failures.
- **11/12 audit fixes verified** ($24.17, ~2h total): Text-fallback verdict graph edge, unknown verdict feedback, architect MCP role, dry-run ghost nodes, stale checklist removal, code snippet language detection, anchor label fix, lesson similarity threshold (0.15), browser edge Dijkstra filtering, silent failure logging, selective git staging.
- **Crash fix:** Researcher output with em-dash crashed loop at string truncation site. Fixed with `floor_char_boundary()` across all 20+ truncation sites.
- **Critical bug found:** Text-fallback verdicts created no graph edge -- verification rate was under-counted. Fixed.

### Session 8: Pipeline Overhaul and Documentation

- **Session resume:** Coder bounces 2+ resume previous Claude Code session via `--resume`. Fallback to fresh session on failure.
- **Verifier feedback quality:** Structured feedback extraction from verifier output for coder bounces.
- **Native agent files:** Agent files in `.claude/agents/` with `resolve_agent_name()` for portability (falls back to inline templates for foreign repos). (Session 9 simplified to 3 pipeline agents: coder, verifier, summarizer. Researcher, planner, architect, tester, scout roles deleted.)
- **Selective git staging:** `selective_git_add()` + `is_spore_excluded()` replace `git add -A` at all 3 call sites.
- **Comprehensive documentation:** PIPELINE.md, CLI-REFERENCE.md, ORCHESTRATOR-OVERVIEW.md, agents/README.md.

---

## Phase 7: GUI Implications (Deferred)

- Meta-nodes render at top hierarchy level with distinct visual per `meta_type` (summary=teal, contradiction=red, status=green/yellow, plan=purple)
- Bounce chains visible as connected node sequences (Coder→Verifier→Coder) with typed edges
- Agent attribution on every node/edge in detail view
- Supersession chains visible (node and edge)
- Agent activity sidebar (last run per agent, counts, active contradictions, unresolved escalations)
- Meta-node hierarchy floating: currently they survive rebuilds via `human_created=true` but don't auto-place at top. Need placement logic in `hierarchy.rs` that pins `node_class='meta'` nodes at depth 1.

---

## Phase 8: Neural Pathway Architecture (Partially Complete)

**Goal:** Replace fat sessions with thin sessions. The graph becomes the continuity mechanism, not the context window.

**Implemented so far:** Dijkstra context retrieval (`context-for-task`) is implemented and active in `compile_context_for_task()`. The orchestrator uses weighted graph traversal to select relevant anchors for task files. Browser session edges are filtered from traversal results. Thin sessions are not yet implemented -- agents still run in single fat sessions, but context quality is dramatically improved by graph-compiled task files.

### The problem with fat sessions

The current model runs one long Claude session per agent invocation. The session carries the full 130-line system prompt, all graph reads, code edits, build output, test results, and graph writes. By the time the agent reaches "record your work in the graph" at the end, it's consumed 60-80% of its context window. Instructions at the top suffer attention decay. This is the structural root cause of agents skipping graph recording — not a prompt engineering problem, but a cognitive architecture problem.

### Thin sessions as neural firing

Each Claude session does one focused thing, writes a signal to the graph, and dies. The next session reads that signal and continues. The "thought" is the pathway through the graph, not the state of any single session.

```
Fat session (current):
  [prompt + task + 15 graph reads + code edit + build + test + graph write]
   ← instructions lost here                              still working here →

Thin sessions (target):
  Session 1: read task node → plan approach → record plan node → exit
  Session 2: read plan node → edit files → record impl node → exit
  Session 3: read impl node → cargo check → record verdict → exit
  Session 4: read verdict → fix if needed → record fix node → exit
```

### The neural analogy

This maps precisely to how brains process information:

- **Context window** = neuron membrane potential (temporary, local, gone after firing)
- **Graph node** = neuron body (persistent, stores one unit of meaning)
- **Graph edge** = synapse (carries type, confidence/weight, reasoning)
- **`supports` edge at 0.95** = strong excitatory synapse
- **`contradicts` edge** = inhibitory synapse
- **Summarizer writing meta-nodes** = memory consolidation (short-term distributed activity → long-term structured memory)
- **Thin session** = single neural firing (action potential)
- **Graph pathway** = the thought itself (sequence of activations)

A neuron doesn't hold the whole thought. It fires, passes a signal along a synapse, goes quiet. The next neuron fires. Intelligence emerges from the pathway, not from any single activation. Fat sessions try to make one neuron hold an entire thought. It can't.

### What changes

1. **Orchestrator becomes a scheduler:** Instead of "launch agent, wait 5 minutes, check verdict," it fires micro-sessions along graph paths. Each session gets a 20-line prompt with one job.
2. **`max_turns` drops to 5-10** instead of 50. Sessions are fast and focused.
3. **Graph recording becomes the entire point** of each session, not a forgotten cleanup step. The session exists to write one node or edge.
4. **Agent prompts shrink dramatically.** The graph carries the context. The prompt just says "read node X, do Y, write result."
5. **More sessions, each cheap.** Tradeoff: more startup overhead, but each session never loses instructions to attention decay.

### Build Loop

The core bounce loop that drives all orchestration:

```
Build Loop:
  Coder:    read task file → implement → exit
  Verifier: read impl → check against requirements + code → exit
  (bounce until verified or max bounces reached)
  Summarizer: read trail → create knowledge nodes → exit
```

The human can intervene at any point — override a Verifier rejection, adjust the task description, stop the loop.

### Dijkstra context retrieval: the graph prunes itself

An agent doesn't need the whole graph. It needs the **weighted shortest path** from its task to relevant context. Edge confidence IS the weight.

```
Task: "Add WebSocket support to team server"
         ↓ dijkstra (maximize cumulative confidence)
Finds: team_server.rs code node         (2 hops, confidence 0.95)
       → "chose axum for HTTP" decision (3 hops, confidence 0.85)
       → "SQLite concurrent writes"     (4 hops, confidence 0.80)
       → Phase 6 orchestration plan     (3 hops, confidence 0.90)
Skips: 1100+ unrelated nodes
```

A `derives_from` edge at 0.95 is a highway. A `related` edge at 0.3 is a dirt road. Dijkstra naturally follows strong connections and ignores noise. The context window budget becomes a graph radius: start from the task node, expand outward by weighted proximity, stop when you've collected N nodes or path weight drops below threshold.

Existing primitives that support this:
- `spore path-between` — BFS with edge-type filtering (exists, Phase 2)
- `spore edges-for-context` — ranks by composite score: recency × confidence × type priority (exists, Phase 2)
- **Missing:** `spore context-for-task <node-id> --budget <N>` — Dijkstra outward from a node, returns the N most relevant nodes by weighted proximity. This is the agent's "attention mechanism."

The Plan Reviewer gets the 20 most relevant nodes by graph proximity, not 20 random nodes or 20 most recent. **The graph does the pruning.** 90% of nodes are irrelevant to any given task. Dijkstra skips them structurally.

### The Summarizer is the knowledge consolidation layer

Every agent that needs context depends on the quality of summarized knowledge. Raw conversation imports are noisy — a discussion about "should we use SQLite or Postgres" produces dozens of nodes. Future agents need the conclusion: "chose SQLite for V1, revisit for concurrent writes."

The dependency chain:

```
Conversations (raw)          ← import pipeline (exists)
       ↓
Knowledge nodes (noisy)      ← graph (exists)
       ↓
Decision/context nodes       ← Summarizer
       ↓
Coder reads distilled context via task file compilation
```

The Summarizer is memory consolidation — it converts short-term distributed activity into long-term structured knowledge.

### The Mycelica thesis completes

The graph structure already mirrors neural architecture — nodes are neurons, edges are weighted synapses. What was missing was the activation pattern. Thin sessions firing along graph paths IS neural firing. The orchestrator becomes the action potential propagation mechanism. Spore becomes the brain's executive function, deciding which pathways to activate next.

Dijkstra context retrieval is the attention mechanism — selecting which neurons to activate based on connection strength. The Summarizer is memory consolidation — converting distributed activity into structured long-term knowledge.

### Prerequisites

- Phase 6 orchestrator stable and tested
- Summarizer agent implemented (the bottleneck for context quality) -- DONE
- Measured startup-overhead per Claude Code invocation (if >10s, thin sessions have a latency cost)
- `spore context-for-task` -- Dijkstra weighted traversal -- DONE
- Conversation import pipeline tested with Summarizer distillation

---

## Implementation Order

| Order | Phase | Status | Effort |
|-------|-------|--------|--------|
| 1 | Phase 1: Schema Evolution | ✅ Complete | ~1h |
| 2 | Phase 2: Edge CLI Commands | ✅ Complete | ~1h |
| -- | Single-agent validation test | ✅ Passed | Manual |
| 3 | Phase 3: Test Gap Fixes | ✅ Complete | Feb 16 2026 |
| 4 | Phase 4: MCP Server | ✅ Complete | ~3h |
| 5 | Phase 5: Agent Definitions | ✅ Complete | Coder+Verifier validated |
| 6 | Phase 6: Pipeline Orchestration | ✅ Complete | ~800 lines |
| 6.5 | Phase 6.5: Task File Generation | ✅ Complete | Session 3 |
| -- | Sessions 3-8: Post-Phase 6 | ✅ Complete | See above |
| 7 | Phase 7: GUI | Deferred | TBD |
| 8 | Phase 8: Neural Pathways | Partial | Dijkstra done, thin sessions pending |

**Current state (Session 9):** Full pipeline operational. 3 pipeline agents (coder, verifier, summarizer) with single-flow pipeline (no complexity gating). Researcher, planner, architect, tester, and scout roles were deleted in session 9 as part of pipeline simplification. Cross-codebase portability proven on Rust (fd) and TypeScript (commander.js). Opus coder validated 39% cheaper than sonnet via A/B testing. All orchestration in `src-tauri/src/bin/cli/spore.rs`.

**For detailed pipeline documentation, see:** [PIPELINE.md](PIPELINE.md), [ORCHESTRATOR-OVERVIEW.md](ORCHESTRATOR-OVERVIEW.md), [CLI-REFERENCE.md](CLI-REFERENCE.md), [agents/README.md](agents/README.md).

---

## Success Criteria

1. Open Mycelica → top-level meta-nodes accurately describe project state
2. Contradictions explicitly visible as meta-nodes with edges to both sides
3. Coder+Verifier bounce loop produces working code — Verifier catches real bugs, Coder fixes them, graph shows deliberation chain
4. Summarizer explains bounce chains — human reads conclusions without needing every operational node
6. Edge reasoning specific enough to understand connections without full node content
7. Failed runs identifiable and rollback-able
8. No duplicate nodes for the same concept
9. Trust the top layer enough to make decisions from it
10. (Phase 8) A third party can follow a graph pathway and reconstruct the complete reasoning chain without access to any agent's context window — the graph IS the thought, not a log of it
11. (Phase 8) `context-for-task` returns the 20 most relevant nodes for any task via Dijkstra — agents get focused context without manual curation
12. (Phase 8) Summarizer distills raw conversation imports into decision nodes — the dependency chain from conversations to actionable context works end-to-end
