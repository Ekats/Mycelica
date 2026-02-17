# Spore — Architectural Concerns

> Originally flagged during plan review, Feb 13 2026. Updated after Phase 6 (orchestrator) shipped.

## Status

Phases 1-6 complete and committed. The full infrastructure exists: typed edges with content/attribution/supersession, meta nodes with transactional creation, query/explain/path/rank commands, status dashboard, MCP server with 16 tools and role-based permissions, agent definitions (Coder + Verifier), and an automated orchestrator (`mycelica-cli spore orchestrate`) with run tracking, escalation, and failure handling. Phase 5 bounce loop validated manually. Phase 6 orchestrator testing in progress.

**14 concerns tracked. 5 resolved, 9 active/future.**

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

## Concern 13: Bootstrap vs runtime agent context

**Severity: Low now, Medium when agents run sustained sessions.**

Agent instructions exist on two layers with a chicken-and-egg problem between them:

**Bootstrap (`.md` files):** Identity, MCP tools, hard rules, workflow. Static, checked into repo. The agent reads this before it can connect to the graph. Must stay as files because the agent needs instructions to know how to use the graph — if those instructions were in the graph, it couldn't read them without already being connected.

**Runtime context (graph nodes):** Current priorities, project conventions, learned patterns from past bounces, role-specific knowledge that evolves over time. The agent reads these via MCP after connecting. This layer doesn't exist yet.

The runtime layer is where the system gets interesting. The Summarizer could write "lessons learned" nodes from bounce trails (e.g., "Verifier found that schema.rs lifetime errors always come from the conn.lock() pattern — Coder should use scoped blocks"). The Coder reads those next session and avoids repeating mistakes. The graph teaches agents over time — their effective instructions evolve without editing `.md` files.

**What needs to happen:**
1. Convention for runtime context nodes: `node_class = "operational"`, tagged with agent role, searchable by `mycelica_search("spore:coder context")`
2. Agent prompts updated to query for their own runtime context after connecting
3. A process (manual at first, Summarizer later) for distilling bounce patterns into reusable context nodes

**Risk:** Runtime context nodes accumulate without curation, becoming noise. The Summarizer's editorial quality (Concern #1) directly determines whether runtime context helps or hurts.

**Status: Concept documented. Build after bounce loop validates and agents actually need cross-session learning.**

## Concern 14: Neural pathway architecture — thin sessions over fat sessions

**Severity: Low now, High for scalability and reliability.**

The current architecture uses **fat sessions**: one long Claude session carries the full prompt, all graph reads, code edits, build output, and graph writes. Context windows fill up. Instructions at the top (like "record your work in the graph") get lost to attention decay by session end. This is the root cause of agents skipping graph recording — it's not a prompt problem, it's a cognitive architecture problem.

The alternative is **thin sessions**: each Claude session does one focused thing, writes a signal to the graph, and dies. The next session reads that signal and continues. The context window is temporary and local (like a neuron's membrane potential). The graph is persistent and structural (like the connectome). Intelligence emerges from the pathway, not from any single activation.

**Fat session (current):**
```
[prompt + task + 15 graph reads + code edit + build + test + graph write]
 ← instructions lost here                              still working here →
```

**Thin sessions (target):**
```
Session 1: read task node → plan approach → record plan node → exit
Session 2: read plan node → edit files → record impl node → exit
Session 3: read impl node → cargo check → record verdict → exit
Session 4: read verdict → fix if needed → record fix node → exit
```

**The neural analogy is precise:**

A neuron doesn't hold the whole thought. It fires, passes a signal along a synapse, goes quiet. The next neuron fires. The "thought" is the pathway — the sequence of activations, not the state of any single node.

- **Context window** = membrane potential (temporary, local, gone after firing)
- **Graph node** = neuron body (persistent, stores one unit of meaning)
- **Graph edge** = synapse (carries type, confidence/weight, reasoning)
- **`supports` edge (0.95 confidence)** = strong excitatory synapse
- **`contradicts` edge** = inhibitory synapse
- **Summarizer writing meta-nodes** = memory consolidation (converting short-term distributed activity into long-term structured memory)
- **Thin session** = single neural firing
- **Graph pathway** = the thought itself

**What changes:**
1. Orchestrator evolves from "launch agent, wait, check verdict" to a scheduler that fires micro-sessions along graph paths
2. Agent prompts shrink from 130 lines to ~20 lines per micro-task
3. Graph recording becomes the entire point of each session, not a forgotten cleanup step
4. `max_turns` drops dramatically (5-10 instead of 50)
5. More sessions, each cheap and focused. Latency tradeoff: more startup overhead, but each session is faster and never loses instructions

**The Mycelica thesis comes full circle:** you've been building a brain and calling it a knowledge graph. The graph structure mirrors neural architecture. Nodes are neurons, edges are synapses with weights. What was missing was the activation pattern — thin sessions firing along graph paths IS the neural firing pattern. Spore becomes the action potential propagation mechanism.

**Prerequisite:** Phase 6 orchestrator working and tested. The current fat-session model validates the graph-mediated communication pattern. Thin sessions optimize it.

**Status: Architecture concept documented. The current fat-session model works for V1. Thin sessions are the evolution path once orchestration is stable and the startup-overhead tradeoff is measured.**
