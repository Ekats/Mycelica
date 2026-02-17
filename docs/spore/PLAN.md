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

## Phase 4: MCP Server

**Goal:** Wrap mycelica-cli as an MCP server so Enterprise agents call graph operations as structured tools.

**Gate: PASSED.** Single-agent test proved the commands produce useful output.

### 4a. Implementation

- Use `rmcp` crate (v0.15, pin exact version) with stdio transport
- New module: `src-tauri/src/mcp.rs` or subcommand in cli.rs
- Launch: `mycelica-cli mcp-server --stdio --agent-role <role> --agent-id <id>`
- Per-agent permission scoping — server only registers tools permitted for role
- Agent ID injection — server stamps `--agent-id` on all writes automatically
- Compound operations wrapped in SQLite transactions (`BEGIN IMMEDIATE`/`COMMIT`)
- All responses as structured JSON

### 4b. MCP tools

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
| `mycelica_meta_list` | `spore status --json` |
| `mycelica_db_stats` | `db stats --json` |

**Write tools (scoped per role):**

| MCP Tool | Maps To |
|----------|---------|
| `mycelica_node_create` | `node create [params]` |
| `mycelica_create_edge` | `spore create-edge [params]` |
| `mycelica_supersede_edge` | `link --supersedes [params]` |
| `mycelica_meta_create` | `spore create-meta [params]` |
| `mycelica_meta_update` | `spore update-meta [params]` |
| `mycelica_import_code` | `import code <path> --update` |
| `mycelica_link` | `link [params]` |

### 4c. Permission matrix

| Tool Category  | Ingestor | Coder | Verifier | Planner | Synthesizer | Summarizer | DocWriter | Human |
|----------------|----------|-------|----------|---------|-------------|------------|-----------|-------|
| Read (all)     | ✓        | ✓     | ✓        | ✓       | ✓           | ✓ (no meta)| ✓         | ✓     |
| Node create    | ✓        | ✓ (op)| ✓ (op)   | ✓ (op)  |             | ✓ (meta)   |           | ✓     |
| Edge create    | ✓        | ✓     | ✓        | ✓       | ✓           | ✓          |           | ✓     |
| Edge supersede |          |       |          |         | ✓           |            |           | ✓     |
| Meta create    |          |       |          | ✓ (plan)|             | ✓          |           | ✓     |
| Meta update    |          |       |          |         |             | ✓          |           | ✓     |
| Import         | ✓        |       |          |         |             |            |           | ✓     |
| File write     |          | ✓     |          |         |             |            | ✓         | ✓     |
| Bash/test      |          | ✓     | ✓        |         |             |            |           | ✓     |
| Maintenance    |          |       |          |         |             |            |           | ✓     |

Summarizer's read tools automatically append `WHERE node_class != 'meta'` (recursion guard).

### Effort: 6-8 hours

---

## Phase 5: Agent Definitions

**Goal:** Define 7 agent roles with system prompts, MCP tool access, and interaction patterns.

### Interaction model: bouncing, not pipeline

Agents don't run in strict sequence. They communicate through the graph by writing nodes and edges that other agents read and respond to. The graph accumulates the deliberation. The Summarizer reads all of it and explains conclusions to the human.

Three concurrent execution patterns:

**The Build Loop** (continuous):
```
Coder writes → Verifier checks → bounces back → Coder fixes → Verifier re-checks → ...
```

**The Oversight Sweep** (periodic):
```
Planner reads all recent activity → flags drift or confirms alignment
Synthesizer reads recent nodes → creates relationship edges
```

**The Summary Pass** (on-demand):
```
Summarizer reads everything (except meta) → creates/updates top-layer meta-nodes
DocWriter reads meta + graph → updates .md files
```

### The bounce pattern

Agent A writes a concern node + typed edge (contradicts/questions) targeting Agent B's work. Agent B reads it, responds with its own node linked by `supersedes` (agrees, fixes) or `contradicts` (pushes back). The graph records every exchange. After 3 bounces without resolution → Planner escalates → Summarizer surfaces "UNRESOLVED" at top layer → human decides.

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
- Planner flagged drift: "Sovereignty per-field locking: Planner flagged skip, Coder deferred to Phase 6, Planner accepted."

Every meta-node MUST have `summarizes` edges to source nodes — these are the citations the human follows to verify. Updates via supersession (Phase 3d), not in-place destruction.

Recursion guard: `WHERE node_class != 'meta' OR (node_class = 'meta' AND meta_type = 'contradiction')`

### Agent 7: Doc Writer

**ID:** `spore:docwriter` | **Writes:** .md files, optionally `documents` edges | **Cannot:** create nodes

Reads meta-nodes first (high-level state), drills into knowledge layer for details. Validates own output against graph after writing. Flags uncertainty.

### Effort: 1-2 days per agent for system prompts + testing. Start with Coder+Verifier (tightest feedback loop, most testable).

---

## Phase 6: Pipeline Orchestration

**Goal:** Run tracking, bounce protocol implementation, failure handling, escalation.

### 6a. Run tracking

Each agent stamps `metadata.run_id` (ISO timestamp) on every node/edge per run.

```bash
mycelica-cli spore runs list                  # recent runs with stats
mycelica-cli spore runs get <run-id>          # nodes/edges created in this run
mycelica-cli spore runs rollback <run-id>     # remove incomplete run's output
```

### 6b. Bounce protocol queries

Agents need to discover unresolved concerns targeting their work:

```bash
# "What bounces are waiting for me?"
mycelica-cli spore query-edges \
  --type contradicts,questions \
  --target-agent spore:coder \
  --not-superseded \
  --since <last_run>
```

This requires extending `query-edges` with a `--target-agent` filter (find edges where the TARGET node was created by a specific agent). Different from `--agent` which filters by the edge's own creator.

### 6c. Escalation

New `content_type = 'escalation'` for unresolved bounces. After 3 bounces on the same issue:
1. Planner creates escalation node linking to the bounce chain
2. Summarizer surfaces "UNRESOLVED" at top layer
3. Human decides, creates decision node
4. Relevant agent implements

### 6d. Failure handling

- Failed runs leave partial output marked with `run_id`
- Summarizer creates status meta-node: "Coder run at 14:30 may be incomplete — 3/8 expected files modified"
- Verifier explicitly skips verification of incomplete runs
- `spore runs rollback` removes all nodes/edges from a failed run

### Effort: 2-3 days

---

## Phase 7: GUI Implications (Deferred)

- Meta-nodes render at top hierarchy level with distinct visual per `meta_type` (summary=teal, contradiction=red, status=green/yellow, plan=purple)
- Bounce chains visible as connected node sequences (Coder→Verifier→Coder) with typed edges
- Agent attribution on every node/edge in detail view
- Supersession chains visible (node and edge)
- Agent activity sidebar (last run per agent, counts, active contradictions, unresolved escalations)
- Meta-node hierarchy floating: currently they survive rebuilds via `human_created=true` but don't auto-place at top. Need placement logic in `hierarchy.rs` that pins `node_class='meta'` nodes at depth 1.

---

## Implementation Order

| Order | Phase | Status | Effort |
|-------|-------|--------|--------|
| 1 | Phase 1: Schema Evolution | ✅ Complete | ~1h |
| 2 | Phase 2: Edge CLI Commands | ✅ Complete | ~1h |
| — | Single-agent validation test | ✅ Passed | Manual |
| 3 | Phase 3: Test Gap Fixes | **Complete** | Feb 16 2026 |
| 4 | **Phase 4: MCP Server** | **Next** | Est. 6-8h |
| 5 | Phase 5: Agent Definitions | Pending Phase 4 | Est. 1-2 days/agent |
| 6 | Phase 6: Pipeline Orchestration | Pending Phase 5 | Est. 2-3 days |
| 7 | Phase 7: GUI | Deferred | TBD |

**Critical path:** Gap fixes (`create-edge` + `read-content` are blockers) → MCP server → Coder+Verifier bounce loop (first real multi-agent test) → remaining agents → orchestration.

**Recommended first multi-agent test:** Coder + Verifier only. Two MCP server instances, same database, small task. Does the bounce loop produce working code with a visible deliberation trail?

---

## Success Criteria

1. Open Mycelica → top-level meta-nodes accurately describe project state
2. Contradictions explicitly visible as meta-nodes with edges to both sides
3. Coder+Verifier bounce loop produces working code — Verifier catches real bugs, Coder fixes them, graph shows deliberation chain
4. Planner flags real drift — concerns appear in graph before the human notices
5. Summarizer explains bounce chains — human reads conclusions without needing every operational node
6. Edge reasoning specific enough to understand connections without full node content
7. Failed runs identifiable and rollback-able
8. No duplicate nodes for the same concept
9. Trust the top layer enough to make decisions from it
