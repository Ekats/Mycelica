# Spore — Architectural Concerns

> Originally flagged during plan review, Feb 13 2026. Updated after Phase 6 (orchestrator) shipped.

## Status

Phases 1-6 complete and committed. Phase 6.5 (task file generation) shipped. The full infrastructure exists: typed edges with content/attribution/supersession, meta nodes with transactional creation, query/explain/path/rank commands, status dashboard, MCP server with 16 tools and role-based permissions, agent definitions (Coder + Verifier + Summarizer), and an automated orchestrator (`mycelica-cli spore orchestrate`) with run tracking, streaming output, post-coder cleanup, semantic search context, task file generation, fallback node creation, and optional summarization. 8 end-to-end orchestrator runs validated including first multi-bounce (2 bounces, $2.85) and first summarizer run ($0.38).

**17 concerns tracked. 9 resolved, 8 active/future.**

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

**Severity: RESOLVED.**

DOC.md updated: says "primary coordination channel" instead of "no side channels." Coder writes files, DocWriter writes markdown, Verifier runs bash — these are intentional side channels. The graph records THAT they happened, not the full content.

**Status: Fixed in DOC.md.**

## Concern 8: Premature infrastructure risk

**Severity: GATE PASSED.**

Single-agent test proved the commands produce useful output. MCP server built and tested. Phase 5 bounce loop validated. Phase 6 orchestrator complete with 4 successful end-to-end runs.

```
Phase 1 ✅ (schema)
Phase 2 ✅ (edge CLI)
Single-agent test ✅ (passed)
Phase 3 ✅ (test gap fixes)
Phase 4 ✅ (MCP server)
Phase 5 ✅ (agent definitions, bounce loop validated)
Phase 6 ✅ (orchestrator, streaming, post-run cleanup, --allowedTools)
    ↓
>>> NEXT: Summarizer agent, then Plan Reviewer <<<
```

## Concern 9: Bounce loop convergence

**Severity: RESOLVED — multi-bounce validated.**

6 end-to-end orchestrator runs. First multi-bounce convergence achieved: `spore runs` command (2 bounces, $2.85, 9min). Verifier caught a real UTF-8 panic bug in byte-level string truncation, created `contradicts` edge with specific feedback (line number, exact issue). Coder fixed it on bounce 2 using `chars().count()`. Verifier approved with `supports` edge.

Multi-bounce costs scale linearly: 2 bounces = ~$2.85 (4 agent sessions × ~$0.70 avg). Context window stays fresh per session — no attention decay because each bounce is a new Claude session that reads the task file + graph edges from the previous bounce. The graph IS the memory between sessions.

**Key finding:** `unwrap_or_default()` on raw SQL queries silently hides errors — a wrong column name returned empty results instead of failing. Neither coder nor verifier caught this (they tested output format, not data correctness). Post-run manual fix required. The lesson: error propagation matters more than graceful degradation for correctness-critical queries.

**Mitigations implemented:**
- Verifier prompts require specific feedback (exact error, file, line)
- `--allowedTools` prevents Grep/Glob waste for Coder (saves thousands of tokens)
- Agent prompts slimmed from 375 to 155 lines total (less instruction competition)
- `max-bounces` flag with human escalation after N bounces
- Orchestrator post-run cleanup removes 3 steps from Coder's late-session work
- Task file context via semantic search reduces exploration turns from 15 to 3-5
- Orchestrator fallback node creation when coder doesn't record work

**Status: RESOLVED. Both 1-bounce and 2-bounce convergence validated.**

## Concern 10: Seven agents may be too many for V1

**Severity: Medium — mitigated by phased rollout.**

Seven agents was the original plan. Session 9 simplified to 3 pipeline agents (coder, verifier, summarizer) after finding that complexity-gated routing added coordination overhead without proportional value. The surviving pipeline is single-flow: coder → verifier → summarizer. Researcher, planner, architect, tester, and scout roles were deleted.

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

**Risk:** Runtime context nodes accumulate without curation, becoming noise. The Summarizer's editorial quality (Concern #1) directly determines whether runtime context helps or hurts. Mitigated by: `LIMIT 5` on lesson query (only most recent), lessons are opt-in (summarizer creates them only for genuinely reusable insights).

**Implemented:** Task file generation now queries for `Lesson:` nodes and includes them in a "Lessons from Past Runs" section. The summarizer creates lesson nodes when it identifies reusable patterns from bounce trails. First lesson ("Coders near turn limit may skip self-recording") is now included in every task file — the system teaches agents from its own experience.

The three-step pipeline is complete:
1. Convention: `Lesson:` prefix, `node_class = "operational"` ✅
2. Delivery: task file includes lessons section ✅ (no prompt edit needed — agents read the task file)
3. Curation: Summarizer agent extracts lessons selectively ✅

**Status: RESOLVED for V1. Lesson injection pipeline working. Accumulation risk mitigated by recency limit.**

## Concern 14: Neural pathway architecture — thin sessions over fat sessions

**Severity: Low now, High for scalability and reliability.**

The current architecture uses **fat sessions**: one long Claude session carries the full prompt, all graph reads, code edits, build output, and graph writes. Context windows fill up. Instructions at the top (like "record your work in the graph") get lost to attention decay by session end.

**Phase 6 partially addresses this through structural enforcement rather than thin sessions:**
- Orchestrator post-run cleanup handles re-indexing, CLI reinstall, and code node edges — removing 3 steps from the agent's late-session work
- `--allowedTools` prevents Grep/Glob for Coder — eliminates the biggest context waste
- Agent prompts slimmed from 375 to 155 lines total (75 coder + 80 verifier) — less instruction competition
- Agent's remaining post-coding job: build check, one node, one edge. Three checklist items instead of seven.

These are "medium-thickness" sessions — still single Claude invocations, but with less for the agent to remember and more handled structurally by the orchestrator.

The full **thin session** alternative remains the long-term path: each Claude session does one focused thing, writes a signal to the graph, and dies. The next session reads that signal and continues. The context window is temporary and local (like a neuron's membrane potential). The graph is persistent and structural (like the connectome). Intelligence emerges from the pathway, not from any single activation.

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

**Nested bounce loops emerge from this model.** Planning isn't a mode inside a fat session — it's a separate agent. The plan is a graph node, not a mental state. The orchestrator becomes:

```
Build Loop:  Coder → Verifier → (bounce until verified) → Summarizer
```

Context comes from graph-compiled task files via Dijkstra traversal (Concern #15). The human can intervene at any point in the bounce loop.

**Prerequisite:** Phase 6 orchestrator working and tested. Summarizer agent for context quality (Concern #16). Dijkstra context retrieval (Concern #15).

**Status: Architecture concept documented. The current fat-session model works for V1. Thin sessions are the evolution path once orchestration is stable and the startup-overhead tradeoff is measured.**

## Concern 15: Dijkstra context retrieval — the graph as attention mechanism

**Severity: RESOLVED.**

Agents don't need the whole graph. They need the weighted shortest path from their task to relevant context. The graph has 1300+ nodes. A Plan Reviewer evaluating "Add WebSocket support to team server" needs maybe 20 of them. Which 20?

Edge confidence IS the weight. A `derives_from` edge at 0.95 is a highway. A `related` edge at 0.3 is a dirt road. Dijkstra naturally follows strong connections and ignores noise.

```
Task node
  ↓ dijkstra (maximize cumulative confidence)
Implementation Plan doc          (1 hop,  confidence 0.95)  ← rank 1 (92% relevance)
spawn_claude() call              (1 hop,  confidence 0.66)  ← rank 9 (85% relevance)
Architecture Reference           (2 hops, related edge)     ← rank 17 (83% relevance)
Mycelica Vision                  (3 hops, doc chain)        ← rank 24 (80% relevance)
```

**Implemented:** `spore context-for-task <node-id> --budget <N>` — Dijkstra outward from a node, returns the N most relevant nodes by weighted proximity. The context window budget becomes a graph radius. This is the agent's "attention mechanism."

**Edge cost formula:** `(1.0 - confidence) * (1.0 - 0.5 * type_priority)`, floored at 0.001 for semantic edges. Structural edges (`defined_in`, `belongs_to`, `sibling`) get a cost floor of 0.4 to prevent flooding — discovered via dogfooding when the first version filled all 15 budget slots with "other functions in the same 12,000-line file."

**Features:**
- CLI: `mycelica-cli spore context-for-task <id> --budget N --max-hops N --max-cost N --edge-types x,y --items-only --not-superseded`
- MCP tool: `mycelica_context_for_task` (slim output with `SlimEdge` for token efficiency)
- Extracted `edge_type_priority()` helper shared with `edges_for_context()`
- Path reconstruction showing exact traversal route to each result

**Status: Resolved. CLI + MCP tool shipped. Tested on real graph data.**

## Concern 16: Summarizer is the bottleneck for context quality

**Severity: Medium now, High when Plan Reviewer and thin sessions exist.**

Every agent that needs project context depends on the quality of summarized knowledge. The dependency chain:

```
Raw conversations       ← import pipeline (exists)
       ↓
Knowledge nodes (noisy) ← graph (exists)
       ↓
Decision/context nodes  ← Summarizer (doesn't exist yet)  ← BOTTLENECK
       ↓
Plan Reviewer           ← reads distilled context
       ↓
Coder                   ← reads approved plan + context
```

Raw conversation imports are noisy. A discussion about "should we use SQLite or Postgres" produces dozens of nodes. The Plan Reviewer needs the conclusion: "chose SQLite for V1, revisit for concurrent writes." That's the Summarizer's job — distill conversations into decision nodes with explicit reasoning edges.

Without the Summarizer:
- Plan Reviewer either flies blind (no context) or burns its entire context window on raw conversation nodes
- Dijkstra traversal returns noisy conversation fragments instead of clean decision nodes
- Runtime context (Concern #13) accumulates without curation
- The top layer (what the human reads) doesn't exist

The Summarizer is memory consolidation — converting short-term distributed activity into long-term structured knowledge. In the neural analogy, it's the hippocampus: it doesn't generate new thoughts, it organizes recent activity into durable memories that other brain regions can efficiently access.

**Dependency:** The Summarizer is the most important agent after Coder+Verifier. It's the prerequisite for: Plan Reviewer (needs distilled context), thin sessions (need focused context), runtime context (Concern #13, needs curation), and human readability (Success Criterion #1 and #9).

**Risk:** Summarizer editorial quality was partially validated in the single-agent test (Concern #1). Structural/temporal analysis works. Deliberation summarization (reading bounce trails, extracting lessons) is now validated.

**First successful summarizer run:** After verifier approved the `--all` flag for `spore runs`, the summarizer (15 turns/$0.38/74s):
1. Read the full trail (task → impl → verification)
2. Created a concise summary node: outcome, files changed, verification result
3. Identified a reusable lesson: "Coders near turn limit may skip self-recording" — with pattern, impact, possible fix, and evidence

The summary quality is high: specific (file paths, turn counts, dollar amounts), concise (10 lines), and actionable. The lesson node is genuinely reusable — it captures a pattern that future agents and humans should know.

**Implemented:** Agent definition (`docs/spore/agents/summarizer.md`), MCP config, role permissions (`mycelica_create_node` + `mycelica_create_edge` + meta tools), `--summarize` flag in orchestrator. The summarizer is disallowed from running Bash/Edit/Write (read-only + graph writes).

**Status: RESOLVED for V1. Summarizer agent built, tested, and integrated into orchestrator. Quality validated on real bounce trail data.**

## Concern 17: Babbage/DSPy as structural constraints

**Severity: Design constraint — shapes all future decisions.**

Two principles that should filter every architectural choice in Spore:

### Babbage principle (mechanical dispatch)

The dispatch loop is pure mechanism. No LLM in the orchestrator. Graph nodes are registers, task files are compiled programs. Intelligence lives in graph structure, not in dispatch logic.

**Current state:** The orchestrator is Babbage-compliant — pure Rust, no LLM calls in the loop itself. `select_model_for_role()` maps roles to models: coder → opus, verifier → opus, summarizer → sonnet. Model routing is implemented and A/B validated (opus coder 39% cheaper than sonnet).

**The filter:** If you're tempted to add intelligence to the orchestrator (LLM calls for routing, scheduling, priority selection), redirect that intelligence into graph structure instead. A priority node with weighted edges is better than an LLM deciding what to do next. The orchestrator reads the graph and follows the weights. Mechanism, not cognition.

### DSPy principle (interface/implementation split)

The Guide declares interfaces (what an agent should accomplish). The system compiles implementation context from the graph (which files, which patterns, which lessons). Graph-compiled prompts replace prose behavioral rules. The graph IS the few-shot examples.

**Current state:**
- Task file generation already compiles context from the graph (semantic+FTS anchors, Dijkstra expansion, lesson injection)
- But agent prompts are still heavy prose — coder.md and verifier.md contain behavioral rules that should emerge from graph patterns
- Lessons exist but aren't structured as input-output examples (DSPy's core insight)

**The trajectory:**
1. Lessons become structured input-output pairs (situation -> action -> outcome)
2. Agent prompts shrink as graph density increases — the graph teaches, not the prompt
3. Prompt shrinkage is the measure of progress. If agent prompts are getting longer, the system is going the wrong direction.

**Current gaps:**
- No model routing (all agents get Opus regardless of task complexity)
- Lessons not structured as input-output examples
- Prose prompts still heavy (~75-80 lines per agent)
- No machine-readable priority format (PRIORITIES.md is human prose, not graph nodes)

**Status: Design constraint documented. Partially implemented (task file compilation, lesson injection). Model routing and prompt shrinkage are next concrete steps.**
