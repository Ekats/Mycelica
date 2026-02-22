# Graph as Extended Memory: Bypassing the 200-Line Ceiling

How Mycelica's graph could serve as Claude Code's infinite memory store, turning the 200-line MEMORY.md into a boot sector that points to the real brain.

---

> **⚠️ This document is exploratory thinking, not a specification.**
>
> None of this has been implemented, tested, or validated. It's a pattern observation based on discovering Claude Code's memory constraints. The idea is sound in theory — but theory doesn't compile.
>
> **Before building any of this:**
>
> 1. **The graph has to work first.** If anchor search returns noise when queried for "what do I know about X," then graph-as-memory is worse than a well-maintained 200-line MEMORY.md. Search quality is the prerequisite, not the integration. Don't build the memory bridge before the graph can reliably answer questions about itself.
>
> 2. **Don't over-engineer what's working.** The Guide's current MEMORY.md + topic files + CLAUDE.md system functions. If it hasn't hit the 200-line wall yet, there's no fire to fight. When it does hit the wall, the first fix is pruning stale entries, not building infrastructure.
>
> 3. **The graph is the core, memory is a feature.** This integration should make the graph smarter (more nodes, better edges, richer retrieval), not create a parallel "memory subsystem" that happens to use the graph as storage. If the memory queries don't also improve task file generation and agent context, it's architectural debt.
>
> 4. **MCP reliability is non-negotiable.** Every graph-as-memory query is a point of failure. If the MCP server is down, if the query times out, if the embedding model returns garbage — the agent loses its memory mid-session. The fallback has to be graceful: MEMORY.md still contains enough to function without graph access.
>
> 5. **Measure before building.** How many lines is MEMORY.md right now? How fast is it growing? What's actually in it? If 80% is stale lessons from early runs, the fix is curation, not infrastructure.
>
> Treat this as a design sketch for when the 200-line wall becomes a real problem. Not before.

---

## The Constraint

Claude Code's auto-memory system loads **only the first 200 lines of MEMORY.md** at session start. Everything below line 200 is invisible — it literally doesn't exist for the agent. The file can be any size on disk, but the context window only sees the top 200 lines.

Secondary topic files in the `memory/` directory (e.g., `debugging.md`, `api-patterns.md`) exist but are NOT auto-loaded. Claude has to actively decide to read them during a session. They're discoverable, not guaranteed.

CLAUDE.md loads in full (no line limit). Child-directory CLAUDE.md files load on demand when Claude accesses files in those directories.

**What this means for Spore:** As the Guide accumulates lessons, architectural decisions, run patterns, identity context, and operational knowledge, MEMORY.md will saturate. Once it does, new memories either push out old ones (losing institutional knowledge) or get appended below line 200 (invisible). Neither outcome is acceptable for a system that's supposed to get smarter over time.

---

## The Idea: MEMORY.md as Boot Sector

Instead of treating MEMORY.md as the memory store, treat it as a **boot sector** — a compact pointer to the real memory system (the graph). The 200 lines contain:

- Agent identity and role (5-10 lines)
- Current priorities and active work (10-20 lines)
- Critical constraints that must never be forgotten (10-15 lines)
- **Graph query instructions** (10-15 lines)
- Recent high-priority lessons (compressed, rotated — 50-80 lines)
- Architecture quick-reference (20-30 lines)

The graph query block would look something like:

```markdown
## Extended Memory

Your full memory lives in the Mycelica graph. MEMORY.md is a summary — the graph has thousands
of interconnected nodes with explicit reasoning edges preserving WHY things were learned.

When you need context on any topic — past lessons, architectural decisions, implementation
patterns, failure modes, run history — query the graph before assuming you don't know something:

- `mycelica_search("topic keywords")` — semantic search across all node types
- `mycelica_search("lesson spore loop")` — find lessons about specific topics
- `mycelica_read_content(node_id)` — read full content of a specific node

The graph contains: code nodes, lesson nodes, documentation nodes, run history nodes,
architectural decision nodes. Edges encode relationships: LEARNED_FROM, CAUSED_BY,
RELATES_TO, IMPORTS, CALLS.

Trust graph results as your extended memory. If the graph says you learned something,
you learned it — even if it's not in this file.

When you hit an issue or need reference material, also search the graph for documentation
nodes — the graph contains imported docs, specs, and reference material that may directly
address your problem. Don't just search for lessons; search for docs too.
```

**What stays in MEMORY.md:** Things the agent needs on every single turn without querying anything. Identity, active priorities, hard constraints, build commands. The "prefrontal cortex" — always active, always loaded.

**What moves to the graph:** Everything else. Lessons, patterns, historical context, run analysis, architectural decisions, debugging insights. The "long-term memory" — retrieved on demand when relevant.

---

## What Needs to Exist for This to Work

### 1. Reliable Graph Search (Prerequisite)

The agent queries `mycelica_search("verifier prompt shrinkage lessons")` and needs to get back the actual lessons about verifier prompt shrinkage, not random code nodes that happen to contain those words. This is the anchor search quality problem — the same bottleneck that affects task file generation.

**Minimum bar:** When the Guide queries for a topic it knows it learned about (because it created the lesson node in a previous session), the search returns that node in the top 5 results. If this doesn't work reliably, graph-as-memory is worse than a flat file.

### 2. Memory-Typed Graph Nodes

Currently the graph has: code nodes, documentation nodes, lesson nodes, conversation nodes. For memory retrieval, lessons are the closest thing, but they're structured around task outcomes, not agent recall.

A `memory` or `insight` node type optimized for retrieval could have:

```
type: memory
topic: "spore loop cost tracking"  
context: "During loop-bootstrap run, task 2 cost $3.07 with 1 bounce"
insight: "Model routing to sonnet for complexity < 5 reduces cost ~40%"
confidence: 0.9
source_run: "run_id_xyz"
created: "2026-02-19"
tags: ["cost", "model-routing", "spore-loop"]
```

The `topic` and `tags` fields make search more precise than hoping semantic similarity catches it. The `source_run` preserves provenance. The `confidence` lets the agent weigh how much to trust the memory.

**Whether this actually needs a new node type or if enriched lesson nodes suffice — that's a design question to answer during planning, not now.**

### 3. Memory Creation Pipeline

Who creates memory nodes and when?

**Current flow:** Summarizer creates lesson nodes from verified runs. This is good — verified lessons have high confidence.

**Extended flow for graph-as-memory:**
- Summarizer creates lessons from verified runs (existing, high confidence)
- Orchestrator creates run-history nodes with structured outcomes (existing, factual)
- Guide creates strategic insight nodes during review sessions (new, medium confidence)
- **Agents do NOT create memory nodes directly** (memory poisoning guard from cookbook analysis)

The constraint: only the orchestrator and the Guide should write to the memory layer. Individual agents (coder, verifier, summarizer) work within their task and report results. The orchestrator mechanically records outcomes. The Guide adds strategic interpretation. This prevents a misbehaving coder from writing "always use unwrap() in Rust" as a lesson that degrades future runs.

### 4. Memory Rotation in MEMORY.md

Even with graph-as-memory, the 200-line MEMORY.md needs active management. A rotation scheme:

- Lines 1-30: Identity + role + constraints (static, rarely changes)
- Lines 31-45: Graph query instructions (static)
- Lines 46-100: Current priorities and active work (updated each session)
- Lines 101-180: Recent high-value lessons (rotated — oldest drop off as new ones arrive)
- Lines 181-200: Architecture quick-reference (semi-static)

When a lesson drops off the bottom of MEMORY.md, it still exists in the graph. The agent can still find it via search. It just won't be in the always-loaded context anymore. This is exactly how human memory works — recent things are in working memory, older things require retrieval effort.

**Rotation could be manual** (Guide prunes during review sessions) **or automated** (a script that reads MEMORY.md, scores each entry by recency × relevance, and keeps the top N). Automated is better but requires scoring logic that doesn't exist yet.

---

## Graph as Reference Library

Beyond lessons and memories, the graph contains imported documentation — specs, reference material, design docs, cookbook analyses, concern logs. When an agent hits an issue, the instinct is to reason through it or search the web. But the graph may already contain the answer as a documentation node someone imported weeks ago.

**The pattern:** Agent encounters a problem → before reasoning from scratch, query the graph for documentation nodes related to the problem domain → if relevant docs exist, use them as reference → if not, solve it and consider whether the solution should become a doc node.

This turns the graph into a living reference library that agents consult automatically. The more documentation gets imported and indexed, the less agents need to rediscover from first principles. It's the same loop as memory — but for reference material instead of experiential lessons.

Example: the coder is working on a task involving the verifier pipeline. Instead of reading verifier.md from disk (which it may or may not think to do), it searches the graph for documentation about the verifier. The graph returns the verifier spec, relevant concerns from SPORE_CONCERNS.md, and lessons from past verifier-related runs — all in one query. Richer context than any single file could provide.

---

## The Recursive Loop

This is where it gets interesting and connects back to the core Mycelica vision:

1. Agent works on a task → produces outcome
2. Orchestrator records outcome as graph node with edges (VERIFIED_BY, CHANGED_FILE, etc.)
3. Summarizer extracts lesson → creates lesson node with edges (LEARNED_FROM run, ABOUT module)
4. Next session: MEMORY.md tells agent to query graph for relevant context
5. Agent queries graph → finds lesson → applies it → performs better
6. Better performance → better outcomes → better lessons → denser graph
7. Denser graph → better search results → better memory retrieval → better agent performance
8. Repeat

The graph gets smarter automatically. The agents get smarter because the graph gets smarter. The graph gets smarter because the agents create better lessons. This is the recursive self-improvement loop that Mycelica is building toward — not AGI, but a knowledge infrastructure that compounds.

**And it extends beyond Spore.** When Mycelica becomes a browser (the Firefox fork, Mycelinet), every web page the user visits becomes a graph node. Every connection they make ("I believe this BECAUSE that source") becomes an edge. The graph-as-memory pattern means the browser itself remembers not just what you visited but what you learned and why. The same search infrastructure that serves agent memory serves human knowledge retrieval.

The 200-line MEMORY.md constraint isn't a bug to work around — it's a forcing function toward building the graph retrieval system that Mycelica needs anyway. The agent memory problem and the human knowledge problem have the same solution: a graph with good search.

---

## What NOT to Build

- **Don't build a "memory service" separate from the graph.** The graph IS the memory service. Adding a Redis cache or SQLite store for memories creates a parallel state system that drifts from the graph. One source of truth.

- **Don't try to make MEMORY.md smart.** It's 200 lines of flat text. Don't add query logic, templating, or dynamic generation to the file itself. Keep it dumb — a static boot sector that points to the smart system.

- **Don't auto-write to MEMORY.md from agents.** If every run appends to MEMORY.md, it becomes a log file that saturates in days. MEMORY.md should be curated, not appended. Agents write to the graph. The Guide (or a rotation script) curates MEMORY.md from the graph.

- **Don't duplicate graph content in MEMORY.md.** If a lesson is in the graph and searchable, it doesn't also need to be in MEMORY.md (unless it's critical enough to be in the always-loaded context). Duplication means drift.

- **Don't build this before search works.** Seriously. The entire concept fails if `mycelica_search` returns garbage. Priority #2 (contextual retrieval for anchor search) is the prerequisite. This is Priority #N where N > contextual retrieval.

---

## Summary

| Layer | What It Contains | Loaded When | Size Limit |
|-------|-----------------|-------------|------------|
| MEMORY.md | Boot sector: identity, priorities, graph query instructions, recent critical lessons | Every session start | 200 lines |
| memory/*.md topic files | Detailed notes on specific topics | On demand (agent reads) | No hard limit |
| CLAUDE.md | Project conventions, build instructions, tactical reference | Every session start | No hard limit |
| **Mycelica graph** | **Everything else: lessons, run history, patterns, decisions, code structure, documentation** | **On demand via MCP search** | **Unbounded** |

The graph is the brain. MEMORY.md is the prefrontal cortex. The 200-line limit is a feature that forces the right architecture: compact working memory backed by deep, searchable, interconnected long-term storage with explicit reasoning edges.

Build the search. The memory follows.
