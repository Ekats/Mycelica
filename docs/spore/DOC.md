# Spore — Architecture Reference

> How Mycelica works as an agent coordination substrate. Read PLAN.md for implementation phases and status. This doc covers the system design.
>
> **For detailed pipeline documentation, see:** [PIPELINE.md](PIPELINE.md), [ORCHESTRATOR-OVERVIEW.md](ORCHESTRATOR-OVERVIEW.md), [CLI-REFERENCE.md](CLI-REFERENCE.md), [agents/README.md](agents/README.md).

## System Overview

Mycelica is a knowledge graph that serves as the persistent memory layer for a multi-agent AI system. Agents read the graph to understand context, write to record decisions and discoveries, and query to check consistency. The human reads the top layer (meta-nodes) to see if the system is coherent.

```
              ┌───────────────────────────────────────────┐
              │         Task + Context Compilation         │
              │  (semantic search, Dijkstra traversal,     │
              │   lessons, graph-compiled task file)        │
              └─────────────────┬─────────────────────────┘
                                ▼
              ┌───────────┐    ┌──────────┐
              │   Coder   │◄──▶│ Verifier │  ◄── bounce loop (opus)
              └─────┬─────┘    └────┬─────┘
                    │               │
                    ▼               ▼
              ┌──────────┐
              │Summarizer│  ◄── creates knowledge nodes (sonnet)
              └──────────┘
              ┌──────────┐
              │ Operator │  ◄── manual full-access agent (opus, 80 turns)
              └──────────┘

Single-flow pipeline. Agents communicate through graph-compiled task files.
Orchestrator handles all inter-agent coordination, graph recording, and cleanup.

         ┌──────────────────────────────────────────────────────┐
         │              mycelica-cli MCP Server(s)               │
         │  Per-agent permission + role-filtered tool sets      │
         │  Agent ID injection on all writes                    │
         │  MCP configs for: coder, verifier, summarizer       │
         └──────────────────┬───────────────────────────────────┘
                            │
                            ▼
         ┌──────────────────────────────────────────────────────┐
         │                  SQLite Database                      │
         │  nodes (39 cols) │ edges (17 cols) │ FTS5 + candle   │
         └──────────────────┬───────────────────────────────────┘
                            │
                            ▼
         ┌──────────────────────────────────────────────────────┐
         │              Tauri Desktop App (GUI)                  │
         │  D3 graph — top layer = meta-nodes                   │
         │  Bounce chains as connected node sequences           │
         │  Agent attribution on every node/edge                │
         └──────────────────────────────────────────────────────┘
```

---

## Data Model

### Node Schema (39 columns, post-Spore migration)

Key fields for agent coordination:

| Column | Type | Purpose |
|--------|------|---------|
| `id` | TEXT PK | UUID-based |
| `title` | TEXT | Node name |
| `content` | TEXT | Full text content |
| `content_type` | TEXT | idea, question, exploration, code_function, concept, decision, etc. |
| `agent_id` | TEXT | Which agent created this: 'spore:coder', 'spore:verifier', 'human', etc. |
| `node_class` | TEXT | 'knowledge' (default), 'meta', 'operational' |
| `meta_type` | TEXT | Only when node_class='meta': 'summary', 'contradiction', 'status' |
| `parent_id` | TEXT | Hierarchy parent |
| `human_created` | INTEGER | If true, survives hierarchy rebuilds (sovereignty system) |
| `is_item` | INTEGER | Leaf node vs category |

### Edge Schema (17 columns, post-Spore migration)

| Column | Type | Purpose |
|--------|------|---------|
| `id` | TEXT PK | UUID-based |
| `source_id` | TEXT FK | Source node |
| `target_id` | TEXT FK | Target node |
| `type` | TEXT | Edge type (see table below) |
| `weight` | REAL | 0.0-1.0 association strength |
| `confidence` | REAL | 0.0-1.0 certainty about this relationship |
| `content` | TEXT | Reasoning/explanation WHY this edge exists |
| `agent_id` | TEXT | Which agent created this edge |
| `edge_source` | TEXT | 'ai', 'user', 'clustering', etc. |
| `evidence_id` | TEXT FK | Node that justifies this edge |
| `superseded_by` | TEXT | Points to replacement edge ID |
| `metadata` | TEXT | JSON: {"run_id": "...", ...} |

### Edges as Knowledge

The critical design insight: **edges are not just pointers, they are knowledge objects.** An edge carries:

- **type** — nature of the relationship (supports, contradicts, derives-from, etc.)
- **content** — human-readable reasoning explaining why this connection exists
- **confidence** — how certain the creating agent is
- **agent_id** — provenance: who determined this
- **superseded_by** — whether a newer assessment replaced this one

When an agent queries `edges-for-context`, it gets not just which nodes are nearby but WHY they're connected and HOW CONFIDENT the assessment is. This makes the graph a communication channel between agents rather than just a data store.

---

## Node Classes

```
┌──────────────────────────────────────────────────────────┐
│                    TOP HIERARCHY LAYER                     │
│  ┌──────────┐  ┌──────────┐  ┌─────────────┐  ┌───────┐ │
│  │ summary  │  │  status  │  │contradiction│  │ plan  │ │
│  │ (meta)   │  │ (meta)   │  │   (meta)    │  │(meta) │ │
│  └────┬─────┘  └────┬─────┘  └──┬──────┬───┘  └───┬───┘ │
│       │summarizes   │summarizes │      │contradicts│      │
│       ▼             ▼           ▼      ▼          ▼      │
├──────────────────────────────────────────────────────────┤
│                   KNOWLEDGE LAYER                         │
│  conversations, code, papers, ideas, research             │
│  (node_class = 'knowledge')                               │
│  Connected by: semantic, supports, derives-from,          │
│  calls, documents, related, etc.                          │
├──────────────────────────────────────────────────────────┤
│                  OPERATIONAL LAYER                         │
│  implementations, verifications, plan concerns, decisions │
│  (node_class = 'operational')                             │
│  Connected by: implements, supports, contradicts,         │
│  supersedes, questions, prerequisite                      │
└──────────────────────────────────────────────────────────┘
```

Meta-nodes are excluded from semantic clustering. They survive hierarchy rebuilds via `human_created=true`. The human sees them first when opening the graph.

---

## Edge Types Reference

### Existing (26 types in EdgeType enum)

| Type | Direction | Created By | Purpose |
|------|-----------|------------|---------|
| `semantic` | node ↔ similar | AI clustering | Embedding similarity > 0.7 |
| `related` | node ↔ node | AI analysis | General relationship |
| `calls` | function → function | Code analysis | Call graph |
| `defined_in` | code item → file | Code import | File membership |
| `documents` | markdown → code | Code analysis | Doc references code |
| `uses_type` | function → type | Code analysis | Type usage |
| `implements` | code → spec | Code analysis / agents | Implementation of spec |
| `contains` | parent → child | Hierarchy | Tree structure |
| `belongs_to` | node → category | Clustering | Category membership |
| `reference` | node → reference | AI analysis | Citations |
| `sibling` | node ↔ node | Hierarchy | Same-parent nodes |
| `supports` | node → node | Agents | Evidence relationship |
| `contradicts` | node ↔ node | Agents | Conflict marker |
| `evolved_from` | newer → older | AI analysis | Conceptual evolution |
| `questions` | question → node | Agents | Open question |
| `summarizes` | meta → knowledge | Summarizer | Summary cites source |
| `tracks` | meta → node | Agents | Status tracking |
| `flags` | meta → node | Agents | Issue flag |
| `resolves` | node → flag | Agents | Resolution of flag |
| `derives_from` | node → source | Agents | Provenance |

### Edge types in agent bouncing

| Edge | From → To | Meaning in bounce context |
|------|-----------|---------------------------|
| `contradicts` | Verifier → Coder implementation | "This doesn't work because..." |
| `supports` | Verifier → Coder implementation | "Tested and confirmed working" |
| `derives_from` | Coder fix → Verifier concern | "Fixed based on this feedback" |
| `supersedes` | Coder fix → Coder original | "Replaces my previous implementation" |

---

## Agent Architecture

### Three pipeline agents, single flow

All tasks follow the same pipeline: Coder → Verifier → Summarizer. No complexity gating, no pre-phases.

**Build Loop** (core of every orchestration):
```
Coder writes code (opus, graph-compiled task file)
         ↓
Verifier checks (opus, structured verdict)
         ↓
  ┌── PASS: Summarizer creates knowledge nodes → done
  ├── FAIL: bounce to Coder with verifier feedback (session resume on bounce 2+)
  └── UNKNOWN: bounce to Coder with feedback
```

### Agent specifications

| Agent | Model | Max Turns | Writes | Key Constraint |
|-------|-------|-----------|--------|----------------|
| Coder | opus | 50 | code files | No Grep/Glob (uses MCP search) |
| Verifier | opus | 50 | structured verdict | Read-only (no Edit/Write) |
| Summarizer | sonnet | 15 | knowledge nodes | No code access |
| Operator | opus | 80 | anything (manual mode) | Full access (not part of automated pipeline) |

**Agent resolution:** `resolve_agent_name(role)` checks for `.claude/agents/<role>.md` (native Claude Code agent with memory + skills injection). Falls back to inline templates from `docs/spore/agents/<role>.md`. This enables portability -- foreign repos without agent files get inline prompts automatically.

**MCP configs:** Coder, verifier, and summarizer have MCP role configurations that control which graph tools are available. The MCP server filters tools based on `--agent-role`, so agents cannot access tools outside their permission set.

See [agents/README.md](agents/README.md) for detailed per-agent specifications including permissions, retry behavior, and MCP access.

---

## Bounce Protocol

### How agents argue through the graph

1. **Agent A creates a concern node** (operational class) describing a specific issue
2. **Agent A creates a typed edge** from concern → B's work node (`contradicts`, `questions`, or `suggests`; confidence reflects severity)
3. **Agent B queries for bounces**: `spore query-edges --type contradicts,questions --target-agent <B> --since <last_run> --not-superseded`
4. **Agent B responds** with a new node linked by `derives-from` to the concern and either `supersedes` (agrees + fixes) or `contradicts` (pushes back) to its original work
5. **Cycle repeats** until all concerns have `supports` edges (resolution) or hit escalation threshold

### Escalation

After max bounces without convergence:
1. Orchestrator creates an escalation node linking to the bounce chain
2. Summarizer surfaces "UNRESOLVED" at top layer with edges to both sides
3. Human reads both arguments, creates a decision node
4. Coder reads decision and implements

### Bounce tracking

Each concern node tracks iteration via `metadata.bounce_count`. Incremented when the responding agent creates a follow-up on the same implementation. The orchestrator monitors bounce counts to detect stuck loops.

---

## Observability: The Graph IS the Dashboard

There is no separate monitoring system. The top layer of Mycelica contains meta-nodes created by the Summarizer:

- **Summary nodes** — "Phase 5 sovereignty: 4/5 complete, edge signing blocked on schema migration"
- **Status nodes** — "Ingestor: last run 14:30, 12 nodes. Synthesizer: 14:35, 8 edges."
- **Contradiction nodes** — "CONFLICT: Schema migration assumes edge parents exist, sovereignty spec removes them"
- **Deliberation summaries** — "Schema migration: Coder+Verifier bounced 3x, resolved."

Each meta-node has `summarizes` edges pointing down. Follow any edge to verify the summary against sources.

### Bounce chain visibility

When Coder and Verifier bounce:

```
[Coder: "Implemented edge signing in schema.rs"]
    ↑ contradicts (confidence: 0.9)
[Verifier: "cargo check fails: lifetime error at schema.rs:142"]
    ↑ derives-from
[Coder: "Fixed: restructured to drop MutexGuard before mutation"]
    ↑ supports (confidence: 0.95)
[Verifier: "cargo check passes, cargo test passes, manual review clear"]
```

Summarizer compresses: "Edge signing: failed on lifetime error, fixed by restructuring MutexGuard scope, verified passing."

---

## Orchestrator

### `spore orchestrate`

```bash
mycelica-cli spore orchestrate "task description" [--max-bounces 3] [--verbose]
```

Automates the full agent pipeline: Coder → Verifier bounce loop → Summarizer.

### Pipeline flow

```
1. Generate graph-compiled task file (compile_context_for_task + generate_task_file)
2. Core bounce loop (max 3):
   a. Spawn Coder with task file (opus, --allowedTools, streaming)
   b. Orchestrator creates impl node from git diff
   c. post_coder_cleanup(): re-index files, reinstall CLI if needed
   d. Spawn Verifier (opus, structured verdict)
   e. Extract verdict + feedback:
      - PASS -> Summarizer creates knowledge nodes -> done
      - FAIL -> bounce to Coder with extracted feedback
      - UNKNOWN -> bounce to Coder with feedback
   f. On bounce 2+: session resume (reuses previous coder session via --resume)
3. Max bounces reached: create escalation node
4. Selective git staging (selective_git_add, excludes spore internals)
5. Auto-commit with run metadata
```

### Key orchestrator features

- **Graph-compiled task files:** `generate_task_file()` writes task files with semantic search anchors, Dijkstra-traversed code context, accumulated lessons, and verifier feedback. This is the primary context delivery mechanism.
- **Session resume:** Coder bounces 2+ resume the previous Claude Code session via `--resume` flag. Falls back to fresh session on failure. Tracked via `last_coder_session_id`.
- **Verifier feedback extraction:** `extract_feedback_from_verifier()` parses structured verdicts. Text-fallback verdicts (no JSON block) still create graph edges.
- **Native agent dispatch:** `resolve_agent_name()` checks for `.claude/agents/<role>.md`. Uses native Claude Code agents (with memory + skills) when available, inline templates otherwise.
- **Selective git staging:** `selective_git_add()` + `is_spore_excluded()` replace `git add -A`. Excludes task files, loop state, and other spore internals from commits.
- **Streaming output:** `--output-format stream-json` with line-by-line parsing. Prints `[role] $ command`, `[role] mcp: tool`, `[role] Read: file`. Completion includes turn count, cost, duration.

### Tool restrictions

Enforced via `--allowedTools` flag on Claude Code:

| Agent | Allowed Tools |
|-------|---------------|
| Coder | `Read,Write,Edit,Bash(*),mcp__mycelica__*` |
| Verifier | `Read,Grep,Glob,Bash(cargo:*),Bash(cd:*),Bash(mycelica-cli:*),mcp__mycelica__*` |

Coder cannot use Grep or Glob -- forced to use `mycelica-cli search` (semantic, indexed) instead. This prevents thousands of wasted context tokens per session.

See [PIPELINE.md](PIPELINE.md) for the complete 592-line pipeline reference with line numbers and function signatures.

---

## MCP Server Design

### Transport

Stdio via `rmcp` crate (v0.15). Each agent gets its own MCP server instance:

```bash
mycelica-cli mcp-server --stdio --agent-role ingestor --agent-id ingestor-1
mycelica-cli mcp-server --stdio --agent-role verifier --agent-id verifier-1
```

### Permission enforcement

Server reads `--agent-role` on startup, only registers permitted tools. Unregistered tools return MCP error. Server doesn't rely on agent good behavior -- tools that aren't allowed simply don't exist in the agent's tool list.

**MCP role configurations exist for 3 pipeline agent roles:** coder, verifier, summarizer. Each gets a filtered subset of the 16 MCP tools. The operator role does not have an MCP config (gets full manual access). (Architect, planner, researcher, and tester MCP configs were removed in session 9 along with those roles.)

### Agent ID injection

On every write operation, the MCP server overrides any `agent_id` in the request with the server's `--agent-id` value. Agents cannot set their own attribution.

### Concurrent access

SQLite WAL mode handles concurrent readers. For the pipeline architecture (sequential within each execution pattern), only one writer is typically active per pattern. If multiple patterns overlap, writes go through the existing `conn.transaction()` mechanism.

---

## Edge Querying

### `edges-for-context` ranking formula

```
score = (confidence × 0.4) + (recency × 0.3) + (type_priority × 0.3)
```

**Type priority:**

| Type | Priority | Rationale |
|------|----------|-----------|
| contradicts | 1.0 | Conflicts are the most important signal |
| supports | 0.8 | Evidence matters |
| derives_from | 0.7 | Provenance |
| summarizes | 0.6 | Meta-context |
| questions | 0.5 | Open questions |
| implements | 0.4 | Implementation links |
| calls | 0.3 | Code structure |
| documents | 0.3 | Documentation links |
| semantic | 0.2 | Similarity (abundant, low specificity) |
| related | 0.1 | Weakest signal |

Superseded edges excluded by default. `--include-superseded` for audit trail.

### `path-between` output

Returns all unique paths up to max-hops with full edge metadata per hop:

```json
{
  "hops": [
    {
      "from_node": {"id": "...", "title": "..."},
      "edge": {"type": "supports", "content": "...", "confidence": 0.85},
      "to_node": {"id": "...", "title": "..."}
    }
  ],
  "total_confidence": 0.595
}
```

`total_confidence` = product of edge confidences. Lower means weaker chain of reasoning.

---

## Run Tracking

### Commands

```bash
mycelica-cli spore runs list                       # recent runs with stats
mycelica-cli spore runs get <run-id>               # nodes/edges created in this run
mycelica-cli spore runs rollback <run-id>          # remove a run's output
mycelica-cli spore runs rollback <run-id> --dry-run # preview what would be deleted
mycelica-cli spore query-edges --compact           # one line per edge
```

### Run metadata

Every node/edge created by an agent carries:
```json
{"run_id": "2026-02-15T14:30:00Z", "agent_role": "coder", "trigger": "manual"}
```

Stored in `metadata` JSON field.

### Rollback

`spore runs rollback <run-id>` removes all nodes/edges where `metadata.run_id` matches. Foreign key cascades handle edges pointing to deleted nodes. Rollback of a Synthesizer run removes its edges but preserves Ingestor's nodes. Rollback of a Coder run removes implementation nodes AND any edges pointing to them (including Verifier verification edges).

---

## Future Integration Points

### Mechanist (Fernando)

Fifth agent with read-only graph access + ability to create `verified` edges backed by formal verification (topological/homological analysis). Synthesizer hypothesizes relationships; Mechanist proves or disproves them.

### Cryptographic Signing (Martin)

Nodes and edges signed by creating agent via `metadata` JSON field. Turns the graph into a verifiable knowledge structure where every relationship has provable provenance. This is the Kyberpunk product thesis: trust infrastructure for autonomous agents, dogfooded on the team's own knowledge graph.

### Mycelinet Protocol (mcn://)

MCP server is localhost-only now. Eventually: MCP tools become a network protocol. Remote agents read/write over authenticated connections. Multiple Mycelica instances federate — sharing subgraphs with cryptographic provenance. The mcn:// vision, built incrementally from local-first infrastructure.

---

## Key File Locations

| Component | Path |
|-----------|------|
| CLI binary | `src-tauri/src/bin/cli.rs` |
| Spore orchestrator | `src-tauri/src/bin/cli/spore.rs` (~15,152 lines) |
| Database schema | `src-tauri/src/db/schema.rs` |
| Data models (EdgeType enum) | `src-tauri/src/db/models.rs` |
| Hierarchy algorithm | `src-tauri/src/hierarchy.rs` |
| Tauri commands | `src-tauri/src/commands/graph.rs` |
| AI client | `src-tauri/src/ai_client.rs` |
| Local embeddings | `src-tauri/src/local_embeddings.rs` |
| Code parser | `src-tauri/src/code/rust_parser.rs` |
| Code import | `src-tauri/src/code/mod.rs` |
| Team server | `src-tauri/src/bin/server.rs` |
| MCP server | `src-tauri/src/mcp.rs` |
| Agent defs (native) | `.claude/agents/*.md` (3 pipeline + operator + guide + hypha) |
| Agent templates (inline) | `docs/spore/agents/*.md` |
| Pipeline reference | `docs/spore/PIPELINE.md` |
| CLI reference | `docs/spore/CLI-REFERENCE.md` |
| Orchestrator overview | `docs/spore/ORCHESTRATOR-OVERVIEW.md` |
| Agent reference | `docs/spore/agents/README.md` |
| Spore plan | `docs/spore/PLAN.md` |
| Spore concerns | `docs/spore/SPORE_CONCERNS.md` |
