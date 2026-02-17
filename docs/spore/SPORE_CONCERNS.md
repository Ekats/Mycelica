# Spore — Architectural Concerns

> Originally flagged during plan review, Feb 13 2026. Updated after Phase 4 (MCP server) shipped.

## Status

Phases 1-4 complete and committed. The full infrastructure exists: typed edges with content/attribution/supersession, meta nodes with transactional creation, query/explain/path/rank commands, status dashboard, and an MCP server with 16 tools, role-based permissions, and recursion guards. Phase 5 (agent definitions) is next — Coder + Verifier pair only.

**Next step: Coder + Verifier bounce loop validation.**

## Concern 1: The Summarizer is editorial work, not engineering

**Severity: High — partially validated.**

The single-agent test proved doc summarization works (70% value, 95% reliability for structural/temporal analysis). Deliberation summarization — reading a bounce trail between Coder and Verifier and explaining what happened — is untested. That's the harder task and the one humans actually need.

**Status: Structural summarization validated. Deliberation summarization untested until bounce loop runs.**

## Concern 2: Contradiction detection is hard but scoped

**Severity: Medium — structural works, semantic unproven.**

The dashboard shows unresolved contradictions. `explain-edge` gives full context. Verifier contradiction detection is trivially reliable (compiler output is binary). Semantic contradiction detection (two nodes that say opposite things but aren't linked) remains unproven.

**Status: Rendering done. Verifier detection reliable. Semantic detection deferred.**

## Concern 3: Pipeline feedback loops → bouncing architecture

**Severity: Low — replaced by bouncing model.**

The original concern about linear pipeline feedback loops is moot. The bouncing architecture (agents argue through graph edges, 3-bounce escalation) replaces strict sequencing. Convergence is a separate concern (#9).

**Status: Superseded by bouncing model. See Concern #9.**

## Concern 4: Coherence score — useful as delta, useless as absolute

**Severity: Low — addressed.**

`spore status --all` shows counts, specifics, and per-agent breakdowns via SQL aggregates. The coherence number is secondary to actual contradiction edges with readable context.

**Status: Resolved in practice.**

## Concern 5: Node content updates need an audit trail

**Severity: RESOLVED in Phase 3.**

`update-meta` now creates a NEW meta node linked to the old via `Supersedes` edge. Old node's outgoing edges copied (excluding superseded and Supersedes-typed). `get_meta_nodes()` filters superseded nodes from dashboard. Full history preserved, consistent with edge supersession model.

**Status: Fixed. Option 2 (node supersession) implemented.**

## Concern 6: Non-atomic compound writes

**Severity: RESOLVED.**

`create_meta_node_with_edges()` uses `conn.transaction()`. MCP server's write tools reuse this method. All compound writes are atomic.

**Status: Resolved.**

## Concern 7: "No side channels" is an overpromise

**Severity: Low — documentation wording.**

DOC.md should say "primary coordination channel" instead of "exclusive." Coder writes files, DocWriter writes markdown, Verifier runs bash — these are intentional side channels. The graph records THAT they happened, not the full content.

**Status: Still needs DOC.md wording fix.**

## Concern 8: Premature infrastructure risk

**Severity: GATE PASSED.**

Single-agent test proved the commands produce useful output. MCP server built and tested. Risk now shifts to Phase 5: can the bouncing model actually work?

```
Phase 1 ✅ (schema)
Phase 2 ✅ (edge CLI)
Single-agent test ✅ (passed)
Phase 3 ✅ (test gap fixes)
Phase 4 ✅ (MCP server)
    ↓
>>> PHASE 5: BOUNCE LOOP TEST <<<  ← you are here
    ↓
  ├─ Bounce works?    → add remaining agents
  └─ Bounce fails?    → simplify or iterate prompts
```

## Concern 9: Bounce loop convergence

**Severity: High — untested.**

Context window exhaustion during bounces. Each bounce is: read graph (tool calls + responses) → do work → write graph. Three bounces between Coder and Verifier means 6 operational nodes, 6+ edges, and ~6 read-content calls per agent per bounce. At bounce 3, the agent's context window has 18 tool call/response pairs from graph reads alone, plus all the file editing. A 200k context window fills fast.

The 3-bounce escalation rule is a safeguard, not a solution. If agents can't converge in 3 bounces, the task decomposition is wrong or the feedback isn't specific enough.

**Mitigations:**
- Verifier MUST be specific (exact error, file, line) — vague feedback forces extra bounces
- SlimNode/SlimEdge reduces token usage per tool call
- Human decides after 3 bounces — doesn't rely on agents to detect deadlock

**Status: Untested. Phase 5 validation test will surface this.**

## Concern 10: Seven agents may be too many for V1

**Severity: Medium — mitigated by phased rollout.**

Seven agents is a coordination surface area of 21 pairwise interactions. Starting with Coder + Verifier (1 interaction) is correct. Add Planner next (3 interactions), then Summarizer (6). Only add agents that solve problems the current set can't handle.

**Status: Mitigated. Phase 5 starts with 2 agents only.**

## Concern 11: Scalability limits

**Severity: Low now, Medium at scale.**

Three axes:

**SQLite concurrent writes:** One writer at a time (WAL mode). V1 runs agents sequentially — no problem. Concurrent agents (Phase 6+) will hit `SQLITE_BUSY` if two MCP server processes write simultaneously. Current architecture: each Claude Code instance spawns its own `mycelica-cli mcp-server` process, each with its own DB connection. Fix when it matters: single MCP server process with TCP/SSE transport that all agents connect to, serializing writes through one mutex.

**Graph size:** Fine. FTS search, indexed queries, SQL aggregates — all O(index) not O(table). Recursive CTE for `get_descendants` is O(subtree). The `get_all_nodes`/`get_all_edges` calls were removed from hot paths (Phase 4 optimization). Batch operations (hierarchy building, embeddings) run offline. Thousands of nodes, tens of thousands of edges — no issues.

**Agent count / context windows:** The real bottleneck. Not infrastructure — cognitive. Each MCP tool call + response consumes context window tokens. An agent reading 20 nodes of context before starting work uses 30-40% of its window. Good summaries (Summarizer) are the scaling mechanism: agents read one meta-node instead of twenty source nodes. Without effective summarization, agent count is capped by how much context each agent needs.

**Status: Not a problem at current scale. Monitor at Phase 6+.**

## Concern 12: Graph needs garbage collection

**Severity: Low now, High at sustained agent use.**

The supersession model preserves everything — that's the audit trail. But the graph only grows. After months of active agent use: thousands of superseded operational nodes, dead bounce trails, orphaned nodes from crashed sessions. No active query touches them, but they bloat the database.

Three tiers of garbage:

**Tier 1 — Superseded chains:** A → B → C, only C active. After retention period (30d), collapse: keep C, delete A and B, optionally preserve a "collapsed N versions" annotation. Reasoning already lives in the surviving node.

**Tier 2 — Resolved bounce trails:** Coder implements → Verifier rejects → Coder fixes → Verifier approves. The rejection node and `contradicts` edge are operationally dead after the fix is verified. Could be archived (cold table or `gc_eligible` flag) after retention.

**Tier 3 — Orphaned nodes:** Agent creates node, session crashes before edges. Detectable: `SELECT id FROM nodes WHERE node_class = 'operational' AND id NOT IN (SELECT source_id FROM edges) AND id NOT IN (SELECT target_id FROM edges) AND created_at < threshold`.

**When to build:** Not now. The existing `maintenance` command pattern (`mycelica-cli maintenance tidy/prune/rebuild`) is the right place. Future: `mycelica-cli maintenance gc --dry-run --older-than 30d` that walks supersession chains and collapses them.

**Prerequisite:** Indexes on `nodes.created_at` and `edges.created_at` if they don't already exist. Every GC query filters by age.

**Status: Not needed yet. Design documented for when scale demands it.**
