# Anthropic Cookbook × Spore: Symbiosis Map

An analysis of how Anthropic's official cookbook patterns, agent SDK, and engineering research map onto Spore's architecture — where they align, where Spore diverges intentionally, and where integration creates leverage.

---

> **⚠️ This document is theoretical and suggestive, not a specification.**
>
> None of this has been thoroughly planned, stress-tested, or validated against the current system's actual bottlenecks. These are pattern observations from reading Anthropic's cookbook — potential ingredients, not a recipe.
>
> **Before integrating anything from this document into Spore/Mycelica:**
>
> 1. **Don't lose the plot.** The graph is the core — the brain. Everything else (agents, loops, task files, model routing) exists to serve the graph's growth and quality. Any integration that doesn't make the graph smarter, denser, or more accurately wired is a distraction. The graph has to be built upon recursively and automatically — neurons firing the right way, edges carrying real semantic weight, not just accumulated noise.
>
> 2. **Don't abandon current work for shiny patterns.** The spore loop runs. Verification rate is climbing. Structured verdicts are landing. Context quality is improving. These concrete gains matter more than theoretical integrations. Finish what's in progress before adopting new patterns.
>
> 3. **Vet integration against the Mycelica vision.** Mycelica isn't a Claude wrapper. It's a graph-based knowledge infrastructure where everything exists as connected nodes with explicit reasoning edges. Any cookbook pattern that pulls the architecture toward "LLM orchestrating LLMs through conversation" rather than "graph-mediated coordination through structure" is moving in the wrong direction, no matter how elegant the pattern looks in isolation.
>
> 4. **Plan thoroughly before implementing.** If the Guide or Hypha decides to adopt something from here, it needs a proper plan: what bottleneck does this fix? What's the minimal viable integration? What breaks if it goes wrong? How does it affect the graph? Don't bolt on features — integrate them into the system's existing bones.
>
> 5. **The graph is the source of truth.** The cookbook patterns treat conversation history, memory tools, and file state as primary. In Spore, these are all secondary to the graph. Lessons live in the graph. Context compiles from the graph. Agent performance improves because the graph improves. Any pattern that creates a parallel state system outside the graph is architectural debt.
>
> Treat this as a menu to glance at when stuck, not a TODO list to work through.

---

## 1. Orchestrator-Workers → Spore's Orchestrator Pipeline

**Cookbook pattern:** Central LLM dynamically breaks down tasks, delegates them to worker LLMs, and synthesizes their results. The orchestrator decides subtasks at runtime based on input.

**Spore's current implementation:** `handle_orchestrate()` in spore.rs runs a pipeline: coder → verifier → summarizer. The orchestrator is Rust code, not an LLM. Agent roles are fixed per pipeline stage, not dynamically determined.

**Key divergence:** Anthropic's pattern has an LLM orchestrator deciding which workers to spawn. Spore's orchestrator is mechanical (Babbage principle) — the pipeline stages are predetermined, and the Rust code dispatches them without LLM reasoning in the dispatch layer. Intelligence lives in the agents themselves and in the graph that compiles their context, not in the coordination logic.

**Symbiosis opportunity:** The simplified pipeline (coder → verifier → summarizer) avoids the coordination overhead of dynamic worker selection. If the system needs specialization in the future, it could be added back as optional stages gated on structured signals from the task description, not LLM reasoning. The key principle: let Rust code route on structured signals, not LLM calls.

---

## 2. Evaluator-Optimizer → Spore's Bounce Loop

**Cookbook pattern:** One LLM generates, another evaluates, feedback loops until PASS. Simple loop with memory of previous attempts.

**Spore's current implementation:** Coder generates → verifier evaluates → if FAIL, coder gets feedback and tries again (bounce). Max bounces configurable. The verifier's verdict drives continuation.

**Key alignment:** This is almost exactly the cookbook pattern. Spore already implements evaluator-optimizer as its core verification loop.

**Key divergence:** The cookbook keeps attempt history in the conversation context ("Previous attempts: ..."). Spore's bounce feedback comes through the task file and verifier stdout, not through accumulated conversation history. Each bounce spawns a fresh agent, so there's no conversation-level memory of previous attempts — only what the orchestrator explicitly threads through.

**Symbiosis opportunity:** Spore's lesson system could serve as cross-run evaluator memory. When a verifier identifies a pattern ("this coder keeps forgetting to run cargo check"), that becomes a lesson node in the graph. Future task files for similar tasks include that lesson as compiled context. This is DSPy's bootstrapped few-shot: the evaluator-optimizer loop generates training data (lessons) that improve future generations without explicit conversation memory.

**Concrete integration:**
- Structured verifier verdicts (in progress) capture `failures[]` with file/line/error
- Failed bounces create lesson nodes: "Task X failed because Y — resolved by Z"
- `generate_task_file()` pulls relevant failure lessons into future coder context
- Verification rate becomes the optimization metric: track per-task-type, identify patterns

---

## 3. Programmatic Tool Calling (PTC) → Graph-Compiled Task Files

**Cookbook pattern:** Instead of sequential LLM → tool → LLM → tool round-trips, Claude writes a Python script that calls multiple tools programmatically. Reduces latency by 37%+, cuts token consumption by keeping intermediate results out of context.

**Spore's analogue:** `generate_task_file()` is Spore's PTC equivalent. Instead of an agent discovering relevant code through sequential tool calls (read file → grep → read another file), the orchestrator pre-computes relevant context by traversing the graph, running semantic search, and assembling everything into a single task file. The agent receives compiled context, not raw tool access.

**Key insight:** PTC lets the LLM write code to orchestrate tools. Spore goes further — the orchestrator writes the "program" (task file) that the LLM executes. The LLM doesn't even need to discover what's relevant; the graph tells it. This is the DSPy principle: separate interface (what to do) from implementation (what context is needed).

**Symbiosis opportunity:** PTC's container model could benefit Spore's native agent porting. Currently, agents are spawned as Claude Code subprocesses. If Spore moves to API-based agents, PTC containers could give agents tool access without subprocess overhead. Agent writes code that calls Mycelica's MCP tools programmatically — graph queries, node creation, edge traversal — all in one execution block instead of sequential tool calls.

**Concrete integration:**
- When porting to Claude Agent SDK, expose MCP tools with `allowed_callers: ["code_execution"]`
- Agents write Python/Rust that queries graph, processes results, creates nodes in one block
- Intermediate graph traversal results stay in the execution sandbox, not in agent context
- Only final relevant context enters the agent's reasoning window

---

## 4. Tool Search with Embeddings → Spore's Anchor Search

**Cookbook pattern:** Instead of loading all tool definitions upfront (consuming context), give Claude a single `tool_search` tool that uses embeddings to find relevant tools on demand. Cuts context usage by 90%+.

**Spore's implementation:** `generate_task_file()` uses anchor search (semantic + FTS, merged and deduplicated) to find relevant graph nodes for a task. The task description is the "query," graph nodes are the "tools," and the assembled task file is the filtered result set.

**Key alignment:** Same fundamental pattern — use embeddings to find what's relevant instead of loading everything. Spore's anchor search IS tool search with embeddings, just applied to knowledge nodes instead of tool definitions.

**Symbiosis opportunity:** As Mycelica's graph grows beyond codebases into the broader Mycelinet vision (web pages, thoughts, emails), the tool search pattern applies directly. When an agent needs to reason about something, it doesn't need the entire graph in context — it searches for relevant subgraphs using the task description as query. The graph itself becomes a searchable tool library where "tools" are knowledge fragments.

**Concrete integration:**
- Anchor search quality directly determines agent performance (already the case)
- Contextual retrieval technique (see §8) could improve anchor search accuracy
- Two-stage search: broad semantic pass → reranking pass → top-K into task file
- As graph scales, search quality becomes the bottleneck — invest here

---

## 5. Automatic Context Compaction → Spore Loop Session Management

**Cookbook pattern:** Long-running agentic workflows exceed context limits. SDK auto-compacts by summarizing conversation history when token usage exceeds threshold. Summary preserves critical state, discards transient tool outputs.

**Spore's current approach:** Each agent in the pipeline gets a fresh context window (spawned as subprocess). No single agent runs long enough to need compaction. The orchestrator handles continuity through task files and run records, not through conversation history.

**Key divergence:** Spore's architecture avoids the compaction problem entirely by design. Fresh contexts per agent stage means no accumulation. The graph stores persistent state, not the conversation. This is actually a *strength* — Anthropic's compaction is a workaround for a problem Spore doesn't have.

**Where it becomes relevant:** The spore loop. When running 10+ sequential tasks, the loop itself doesn't accumulate context (it's Rust code). But if/when Spore moves to persistent agent sessions (native agent porting), those sessions WILL need compaction. A persistent Hypha session processing 20 tasks will hit context limits.

**Symbiosis opportunity:** When porting to Claude Agent SDK:
- Use `compaction_control` with aggressive thresholds between tasks (each task is independent)
- Custom summary prompts that preserve: current task state, accumulated lessons, key file locations
- Discard: intermediate tool outputs, previous task details, exploration dead-ends
- Natural compaction points: after each verified task, between pipeline stages

**Concrete integration:**
- Spore loop state persistence (.loop-state.json) already tracks completed tasks
- Compaction summary should reference loop state file for recovery
- Per-task compaction with custom prompts: "Preserve lessons and architectural decisions, discard code diffs"
- Graph nodes serve as external memory that survives compaction

---

## 6. Claude Agent SDK → Spore's Native Agent Future

**SDK capabilities:** `query()` for one-shot tasks, `ClaudeSDKClient` for interactive sessions. Subagents with tool restrictions and model selection. Hooks for pre/post tool validation. MCP integration. Session management with resume.

**Spore's current architecture:** Agents spawned as Claude Code CLI subprocesses via `spawn_claude()`. Fresh context each time. Communication through stdout parsing. No hooks, no session persistence, no tool-level control.

**This is the biggest integration opportunity.** The Claude Agent SDK is essentially what Spore's native agent porting (Priority #3) needs to become. Instead of `spawn_claude()` creating a subprocess, it becomes:

```python
async for msg in query(
    prompt=task_file_content,
    options=ClaudeAgentOptions(
        allowed_tools=["Read", "Write", "Edit", "Bash", "Grep", "Glob"],
        model="sonnet",  # Babbage: cost-aware model routing
        max_turns=50,
        agents={
            "explorer": AgentDefinition(
                description="Code exploration and analysis",
                tools=["Read", "Grep", "Glob"],
                model="haiku"  # Cheapest for exploration
            )
        },
        mcp_servers={"mycelica": mycelica_mcp_config}
    )
):
    process_agent_output(msg)
```

**What this unlocks:**
- Model routing per agent role (already designed in Spore, SDK supports `model: 'sonnet'`)
- Tool restrictions per role (verifier gets Read-only, coder gets Write)
- Hooks for safety (PreToolUse blocks dangerous operations, PostToolUse logs to graph)
- Session persistence (resume coder session after bounce instead of fresh context)
- Subagent delegation (coder spawns explorer subagent for code discovery)
- Skills injection (`.claude/skills/` loaded automatically)

**Critical question:** Can Spore's Rust orchestrator call the Python/TypeScript Agent SDK? Options:
1. **Python subprocess from Rust** — similar to current `spawn_claude` but calling SDK script
2. **TypeScript subprocess from Rust** — SDK available in TS
3. **HTTP API wrapper** — run SDK as a service, Rust calls via HTTP
4. **Wait for Rust SDK** — Anthropic may release one (unlikely near-term)
5. **Direct API calls from Rust** — bypass SDK, call Messages API directly with tool definitions

Option 5 is most Babbage-compliant: Rust makes direct API calls, no SDK dependency. Tool execution happens in Rust. The SDK's value-add (hooks, compaction, subagents) would need reimplementation, but Spore's orchestrator already handles most of this at the Rust level.

---

## 7. Memory & Context Editing → Spore's Graph as Persistent Memory

**Cookbook pattern:** Memory tool gives agents cross-conversation learning. Session 1 discovers a pattern, writes it down. Session 2 applies the learned pattern. Context editing clears old tool results to manage token budget.

**Spore's implementation:** The graph IS the memory system. Lessons are graph nodes. Run history is graph nodes. Code structure is graph nodes. Every agent's "memory" comes from `generate_task_file()` pulling relevant graph context.

**Key advantage over cookbook pattern:** The cookbook's memory tool is key-value style — agent writes "debugging pattern: X" and retrieves it later. Spore's graph preserves *relationships*: "this lesson was learned BECAUSE this task failed WHEN working on this file WHICH imports this module." The edges matter as much as the nodes. An agent doesn't just remember "check error handling" — it remembers the specific error handling pattern that failed in the specific context it's about to work in.

**Symbiosis opportunity:** The cookbook's memory poisoning mitigations are relevant. If Spore allows agents to create lesson nodes, a misbehaving agent could poison the graph with bad lessons that degrade future performance. Validation: lessons should only be created by the orchestrator from verified runs, not by individual agents directly.

**Context editing maps to:** Spore already does this mechanically — task files have a token budget, and `generate_task_file()` truncates/prioritizes within that budget. This is context editing at the compilation layer rather than the conversation layer.

**Concrete integration:**
- Lessons from verified runs → high-confidence graph nodes
- Lessons from failed runs → lower confidence, included only when directly relevant  
- Graph node confidence scores influence task file inclusion priority
- Agent memory (Claude Code's `.claude/agent-memory/`) stores tactical context
- Graph stores strategic context (lessons, patterns, architectural decisions)

---

## 8. Contextual Retrieval → Improving Anchor Search Quality

**Cookbook pattern:** RAG chunks lose context when isolated. Fix: prepend each chunk with LLM-generated context from the full document before embedding. Reduces retrieval failures by 49% (67% with reranking).

**Spore's relevance:** When code files are imported into the graph, they're chunked into nodes (functions, structs, modules). These nodes can lose context — a function named `process()` means nothing without knowing which module it's in. Current anchor search finds nodes by semantic similarity, but decontextualized nodes produce lower-quality matches.

**Symbiosis opportunity:** Apply contextual retrieval to graph node embeddings. When importing code:
1. Parse into nodes (current behavior)
2. For each node, generate a contextual prefix: "This function is in `src/bin/cli/spore.rs`, in the orchestration module. It handles the coder-verifier bounce loop."
3. Embed the contextualized content
4. Store original content in node, contextualized version in embedding

This directly improves `generate_task_file()` quality — the primary bottleneck identified in multiple Spore runs ("context quality bad").

**Concrete integration:**
- `import code` already parses files into nodes
- Add contextual prefix generation during import (can use Haiku — cheap, fast)
- Re-embed existing nodes with contextual prefixes (batch operation)
- Measure: compare anchor search recall before/after contextualization
- Cost estimate: ~$1/million tokens for contextualization (Anthropic's figure)

---

## 9. Routing → Spore's Model Routing (Already Implemented)

**Cookbook pattern:** Route different input types to different models. Simple questions → Haiku (cheap, fast). Complex questions → Opus (expensive, thorough).

**Spore's implementation:** `select_model_for_role()` already maps agent roles to models. Coder with complexity ≥ 5 → Opus, < 5 → Sonnet. Verifier → Opus. Explorer → Haiku. This is the Babbage principle applied to cost optimization.

**Already aligned.** The cookbook validates Spore's approach. One enhancement from the cookbook: routing based on *task characteristics* not just *role*. A documentation-only task shouldn't use Opus for the coder even at complexity 5. Task type (code change, config change, docs, refactor) could be a routing signal alongside complexity.

---

## 10. Building Evals → Spore's Verification as Eval System

**Cookbook pattern:** Systematic evaluation: input prompt → model output → compare to golden answer. Measure accuracy, iterate on prompts.

**Spore's implementation:** Every spore run IS an eval. Task description (input) → coder output → verifier evaluation. The verifier is the eval system. Verification rate (currently 53%) is the primary metric.

**What's missing from Spore vs. cookbook evals:**
- No golden answers (there's no "expected output" for a coding task)
- No systematic regression testing (does a prompt change improve verification rate?)
- No A/B comparison (does Sonnet vs. Opus produce different verification rates for the same task?)

**Symbiosis opportunity:** Build a Spore eval framework:
- Run the same task file with different agent configurations
- Track verification rate, cost, and latency per configuration
- Use this to tune: model routing thresholds, agent prompts, task file generation
- The spore loop with `--dry-run` already supports this — extend to `--eval-mode` that runs each task N times with different configs

---

## 11. Parallelization → Future Spore Loop Enhancement

**Cookbook pattern:** Run independent subtasks in parallel for speed. Or run the same task multiple times (voting) for confidence.

**Spore's current state:** Sequential only. Tasks dispatch one at a time. The spore loop processes tasks in order.

**Where this matters:**
- Independent tasks in a task file could run in parallel (if they don't modify the same files)
- Voting pattern: run verifier 3x independently, require 2/3 agreement for PASS

**Not a priority now** — sequential execution is simpler and Spore's bottleneck is context quality, not throughput. But worth noting as the system scales.

---

## 12. Batch Processing API → Cost Reduction for Spore Runs

**Cookbook pattern:** Process large volumes of Messages requests asynchronously with 50% cost reduction.

**Spore opportunity:** Non-urgent tasks (re-embedding nodes, generating contextual prefixes, batch lesson extraction) could use the Batch API at 50% cost. The spore loop could have a `--batch` mode that queues tasks for async processing instead of real-time dispatch. Trade latency for cost.

---

## Summary: Integration Priority

Ranked by leverage (impact × feasibility):

| Priority | Pattern | Spore Integration | Impact | Effort |
|----------|---------|-------------------|--------|--------|
| 1 | Contextual Retrieval | Improve anchor search with contextual embeddings | High — directly fixes "context quality bad" | Medium |
| 2 | Claude Agent SDK | Replace `spawn_claude()` with SDK calls | High — unlocks hooks, model routing, sessions | High |
| 3 | Memory & Context | Graph-based lesson feedback into task files | High — DSPy principle realized | Medium |
| 4 | PTC | Graph queries as programmatic tool calls | Medium — reduces agent token usage | Medium |
| 5 | Context Compaction | Manage persistent agent sessions | Medium — needed after SDK migration | Low |
| 6 | Evals | Systematic verification rate optimization | Medium — data-driven prompt tuning | Low |
| 7 | Batch API | Cost reduction for non-urgent operations | Low-Medium — 50% savings on batch work | Low |
| 8 | Parallelization | Concurrent task dispatch | Low (now) — sequential is fine for current scale | High |

---

## Architectural Observation

Anthropic's patterns assume LLMs in the coordination layer. Orchestrator-workers has an LLM orchestrator. Evaluator-optimizer has LLMs on both sides. PTC has an LLM writing orchestration code.

Spore's design philosophy puts Rust in the coordination layer and LLMs only in the execution layer. The orchestrator doesn't reason — it traverses graphs, assembles context, routes mechanically. This is an intentional divergence, not a gap. The cookbook validates that the patterns work; Spore's contribution is showing they work *better* when the coordination is compiled rather than reasoned.

The cookbook patterns are ingredients. Spore's graph-mediated coordination is the recipe. The integration path — *if* integration makes sense for a specific bottleneck — is: adopt the ingredients (contextual retrieval, SDK subagents, memory tools) while maintaining Spore's architectural thesis (intelligence in the graph, mechanism in the orchestrator, reasoning only at the agent level).

Where Anthropic says "LLM decides which worker to invoke," Spore says "graph structure determines which worker runs." Where the cookbook says "keep conversation history for context," Spore says "compile context from the graph fresh each time." These aren't contradictions — they're the difference between a framework and an engine.

---

## Final Reminder

This document exists so the Guide has a reference when it encounters a problem that one of these patterns might address. It does NOT exist as a backlog of things to implement. The system is working. The loop is running. The graph is growing. Protect that momentum.

The graph is the brain. Every improvement to the system should be measured by: does this make the graph's neurons fire more accurately? Does it grow the graph's density and reach? Does it make the automatic, recursive building of the graph more reliable?

If a cookbook pattern helps with that — great, plan it properly and integrate it. If it's just interesting — note it and move on. The vision is Mycelica: graph-based infrastructure where reasoning is traceable through explicit edges. Not "Claude with extra steps."
