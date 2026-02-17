# Spore — Architecture Reference

> How Mycelica works as an agent coordination substrate. Read PLAN.md for implementation phases and status. This doc covers the system design.

## System Overview

Mycelica is a knowledge graph that serves as the persistent memory layer for a multi-agent AI system. Agents read the graph to understand context, write to record decisions and discoveries, and query to check consistency. The human reads the top layer (meta-nodes) to see if the system is coherent.

```
                    ┌──────────┐
                    │  Planner │◄──── reads ALL, checks plan alignment
                    └────┬─────┘
                         │ writes: plan nodes, drift flags, revision proposals
                         ▼
    ┌─────────┐    ┌───────────┐    ┌──────────┐
    │Ingestor │───▶│   Coder   │◄──▶│ Verifier │  ◄── bounce loop
    └─────────┘    └─────┬─────┘    └────┬─────┘
                         │               │
                         ▼               ▼
                   ┌─────────────┐  ┌──────────┐
                   │ Synthesizer │  │Summarizer│  ◄── reads all (except meta), writes top layer
                   └─────────────┘  └──────────┘
                                    ┌──────────┐
                                    │ DocWriter│  ◄── reads graph + code, writes .md
                                    └──────────┘

All agents communicate THROUGH the graph. No side channels.
Read graph → write nodes/edges → other agents read those → respond.

         ┌──────────────────────────────────────────────────────┐
         │              mycelica-cli MCP Server(s)               │
         │  Per-agent permission scoping                        │
         │  Agent ID injection on all writes                    │
         │  Recursion guard for Summarizer                      │
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
| `questions` | Planner → Coder implementation | "This doesn't match the plan because..." |
| `supports` | Verifier → Coder implementation | "Tested and confirmed working" |
| `derives_from` | Coder fix → Verifier concern | "Fixed based on this feedback" |
| `supersedes` | Coder fix → Coder original | "Replaces my previous implementation" |
| `supports` | Planner → plan revision | "Drift acknowledged, plan updated" |

---

## Agent Architecture

### Seven agents, three execution patterns

**Build Loop** (continuous — tightest feedback cycle):
```
Coder writes code → creates implementation node
         ↓
Verifier checks → creates verification/failure node
         ↓
  ┌── Pass: supports edge (loop done for this item)
  └── Fail: contradicts edge → Coder reads, fixes, new node with supersedes → Verifier re-checks
```

**Oversight Sweep** (periodic — doesn't block build loop):
```
Planner reads plan + all recent activity → confirms alignment or flags drift
Synthesizer reads recent nodes → creates relationship edges between existing nodes
```

**Summary Pass** (on-demand or periodic — always runs last):
```
Summarizer reads everything except meta → creates/updates top-layer meta-nodes
DocWriter reads meta + graph → updates .md files
```

### Agent specifications

| Agent | ID | Writes | Reads | Cannot |
|-------|----|--------|-------|--------|
| Ingestor | `spore:ingestor` | knowledge/operational nodes, edges | Full graph | Create meta, supersede edges |
| Coder | `spore:coder` | code files, operational nodes, edges | Full graph + filesystem | Create meta, verify own work |
| Verifier | `spore:verifier` | operational nodes, edges; bash | Full graph + filesystem | Fix code, create meta |
| Planner | `spore:planner` | operational + plan meta nodes, edges | ALL agent output | Modify code, import |
| Synthesizer | `spore:synthesizer` | Edges only (create + supersede) | Full graph (knowledge+operational) | Create or modify nodes |
| Summarizer | `spore:summarizer` | Meta-nodes, summarizes/contradicts edges | Full graph EXCEPT meta | Create knowledge/operational nodes |
| DocWriter | `spore:docwriter` | .md files, optionally documents edges | Full graph | Create nodes |

### Recursion guards

**Summarizer:** All read queries append `WHERE node_class != 'meta' OR (node_class = 'meta' AND meta_type = 'contradiction')`. Enforced at MCP server level. Reads knowledge and operational nodes, writes meta-nodes about them. Never reads its own summaries.

**Synthesizer:** Queries scoped to `WHERE (node_class = 'knowledge' OR node_class = 'operational') AND is_item = 1`. Doesn't process meta-nodes or category nodes.

---

## Bounce Protocol

### How agents argue through the graph

1. **Agent A creates a concern node** (operational class) describing a specific issue
2. **Agent A creates a typed edge** from concern → B's work node (`contradicts`, `questions`, or `suggests`; confidence reflects severity)
3. **Agent B queries for bounces**: `spore query-edges --type contradicts,questions --target-agent <B> --since <last_run> --not-superseded`
4. **Agent B responds** with a new node linked by `derives-from` to the concern and either `supersedes` (agrees + fixes) or `contradicts` (pushes back) to its original work
5. **Cycle repeats** until all concerns have `supports` edges (resolution) or hit escalation threshold

### Escalation

After 3 bounces on the same issue without convergence:
1. Planner creates an escalation node (`content_type = 'escalation'`) linking to the full bounce chain
2. Summarizer surfaces "UNRESOLVED" at top layer with edges to both sides
3. Human reads both arguments, creates a decision node
4. Relevant agent reads decision and implements

### Bounce tracking

Each concern node tracks iteration via `metadata.bounce_count`. Incremented when the responding agent creates a follow-up on the same implementation. Planner monitors bounce counts to detect stuck loops.

---

## Observability: The Graph IS the Dashboard

There is no separate monitoring system. The top layer of Mycelica contains meta-nodes created by the Summarizer:

- **Summary nodes** — "Phase 5 sovereignty: 4/5 complete, edge signing blocked on schema migration"
- **Status nodes** — "Ingestor: last run 14:30, 12 nodes. Synthesizer: 14:35, 8 edges."
- **Contradiction nodes** — "CONFLICT: Schema migration assumes edge parents exist, sovereignty spec removes them"
- **Deliberation summaries** — "Schema migration: Coder+Verifier bounced 3x, resolved. Per-field locking: Planner flagged drift, Coder deferred Phase 6, accepted."

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

### Staleness detection

Summarizer compares its meta-nodes' `updated_at` against the `updated_at` of nodes they summarize. If underlying nodes changed after the summary, the summary is stale. Summarizer updates or marks "STALE: underlying data changed since last analysis."

---

## MCP Server Design

### Transport

Stdio via `rmcp` crate (v0.15). Each agent gets its own MCP server instance:

```bash
mycelica-cli mcp-server --stdio --agent-role ingestor --agent-id ingestor-1
mycelica-cli mcp-server --stdio --agent-role verifier --agent-id verifier-1
```

### Permission enforcement

Server reads `--agent-role` on startup, only registers permitted tools. Unregistered tools return MCP error. Server doesn't rely on agent good behavior — tools that aren't allowed simply don't exist in the agent's tool list.

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
| Database schema | `src-tauri/src/db/schema.rs` |
| Data models (EdgeType enum) | `src-tauri/src/db/models.rs` |
| Hierarchy algorithm | `src-tauri/src/hierarchy.rs` |
| Tauri commands | `src-tauri/src/commands/graph.rs` |
| AI client | `src-tauri/src/ai_client.rs` |
| Local embeddings | `src-tauri/src/local_embeddings.rs` |
| Code parser | `src-tauri/src/code/rust_parser.rs` |
| Code import | `src-tauri/src/code/mod.rs` |
| Team server | `src-tauri/src/bin/server.rs` |
| MCP server (new) | `src-tauri/src/mcp.rs` or subcommand in cli.rs |
| Agent prompts (new) | `docs/spore/agents/*.md` |
| Spore plan | `docs/spore/PLAN.md` |
| Spore concerns | `docs/spore/SPORE_CONCERNS.md` |
| Single-agent test | `docs/spore/SINGLE_AGENT_TEST.md` |
