# Concern 16: Guide Architecture — From Session to Subagent

> Flagged 2026-02-19 during first Guide deployment and live testing.

## Severity: High — architectural direction change. Affects how strategic coordination works.

## What Happened

We designed, built, and deployed a Guide agent as a separate Claude Code session at `/opt/guide/` with scope isolation from Hypha's tactical memory. The implementation was clean: `/opt/guide/CLAUDE.md` for identity, `~/.claude/projects/-opt-guide/memory/GUIDE-MEMORY.md` for strategic memory, `PRIORITIES.md` as shared coordination file. Scope isolation confirmed — Guide didn't inherit Hypha's 200+ lines of tactical memory.

Then we watched it run. Three problems emerged in the first session:

### Problem 1: The Guide became Hypha

The Guide observed that task file context had 0/23 relevant nodes. Good strategic observation. Then it read `generate_task_file()` source code, traced the FTS fallback logic at specific line numbers, wrote a surgical implementation spec ("merge semantic+FTS with HashSet dedup at L6053"), and dispatched the coder directly. Everything after "context is bad" was tactical work that belonged to Hypha.

The Guide's natural behavior when given freedom is to investigate everything itself. Reading code is easier than trusting someone else to diagnose. This is the same failure mode as a CEO who can't stop doing engineering.

### Problem 2: The dispatch loop was Gastown's mistake

The Guide ran: read priorities → write task description → spawn orchestrator → sleep/poll → read output → update priorities → repeat. This is a human workflow automated by an LLM. The mechanical dispatch cycle doesn't need strategic judgment for each iteration — it needs a for-loop in Rust.

The Guide was being a factory foreman (cranking the engine every cycle) when it should have been a board advisor (setting the program, reviewing outcomes). The dispatch cycle belongs in the Rust orchestrator as `spore loop`.

### Problem 3: `--agent operator` creates disposable Hypha

The plan routed complex tasks through `spore orchestrate --agent operator`, which spawns a fresh Claude Code instance with the operator prompt. But Hypha IS the persistent Claude Code session at `~/Mycelica/` with 200+ lines of accumulated memory and full codebase context. Spawning `--agent operator` creates a disposable copy with none of that continuity. It's like hiring a contractor who doesn't know the codebase instead of asking the senior engineer who's been here for months.

## The Fix: Guide as Subagent

Claude Code has a native subagent system (`/agents`). Subagents are markdown files with YAML frontmatter that define specialized assistants with:

- **Own context window** — strategic thinking stays separate from tactical code
- **Custom tool access** — Guide gets Read, Bash, Grep, Glob (no Write, no Edit)
- **Custom system prompt** — the strategic coordinator identity
- **Auto-delegation** — Claude Code delegates based on the description field
- **Resumable sessions** — can continue previous conversation with full context via agentId

### Architecture becomes:

```
Hypha = persistent Claude Code session at ~/Mycelica/ (always open, the working session)
  ├── Guide subagent = consultable perspective within Hypha's session
  │   - Own context window (strategic thinking separate from tactical code)
  │   - Reads: PRIORITIES.md, run history, dashboard, lessons
  │   - Writes: PRIORITIES.md updates, strategic graph observations
  │   - Resumable across invocations
  │
  ├── Spore orchestrator = Rust CLI tool Hypha uses
  │   - mycelica-cli spore orchestrate "task"
  │   - Future: mycelica-cli spore loop (continuous, programmatic)
  │
  └── coder/verifier/summarizer = spawned by Rust orchestrator via spawn_claude
      - Graph-compiled task files (Dijkstra traversal)
      - MCP integration for graph reads/writes
      - Turn budgets, cost tracking, run recording, bounce loops
```

### What this eliminates:

- `/opt/guide/` directory and scope isolation hacks
- Separate terminal for Guide sessions
- Guide spawning orchestrator processes
- `--agent operator` as a disposable Hypha substitute
- Sleep/poll monitoring loops in the Guide

### What this preserves:

- Strategic/tactical separation (subagent has own context window)
- PRIORITIES.md as coordination mechanism
- Guide's role: review patterns, set priorities, notice drift
- Democracy principle: coordinate through shared state

## Implementation

### File: `~/Mycelica/.claude/agents/guide.md`

```yaml
---
name: guide
description: Strategic coordinator for Spore. Use when reviewing priorities,
  evaluating run patterns, deciding what to work on next, or assessing system
  health. NOT for tactical work — no code reading, no implementation planning.
tools: Read, Bash, Grep, Glob
model: opus
---

[Guide system prompt — the rewritten CLAUDE.md content about strategic review,
 priority setting, pattern recognition. No dispatch loop, no monitoring.]
```

### Invocation patterns:

```
# Explicit
> Ask the guide to review today's runs and update priorities

# Natural — Claude Code auto-delegates based on description
> What should we focus on next?
> How did that batch of runs go?
> Are we making progress on context quality?

# Resume previous strategic review
> Resume agent [guide-agentId] and check if the priority shift was right
```

### What stays in Rust (not subagent territory):

- `spore loop` — continuous dispatch cycle (read priorities → dispatch → evaluate → loop)
- `spore orchestrate` — single task dispatch with full pipeline
- `spawn_claude` — agent spawning with graph context, MCP, tool permissions
- All cost tracking, run recording, bounce loops, verification

## Relation to Other Concerns

- **Concern 9 (bounce convergence):** Unaffected. Bounce loop is in Rust orchestrator.
- **Concern 14 (thin sessions):** Guide-as-subagent is inherently thin — it has only its own system prompt, no inherited tactical memory. Exactly the thin session thesis.
- **Concern 15 (context-for-task):** Resolved. Guide doesn't need task files — it reads run history and PRIORITIES.md.

## Open Questions

1. **Does the Guide subagent need MCP access?** It writes strategic observations to the graph. But it could also just write to PRIORITIES.md and let Hypha handle graph writes. Simpler = better for V1.

2. **Resumable sessions — how well does this work in practice?** The docs say subagents can be resumed via agentId. If resumption preserves full strategic context, the Guide can maintain a continuous strategic thread across multiple consultations. If not, PRIORITIES.md + GUIDE-MEMORY.md provide the persistence layer.

3. **Auto-delegation accuracy.** Will Claude Code correctly delegate "what should we focus on next?" to the Guide subagent vs. trying to answer it directly? The description field needs to be precise enough to trigger on strategic questions but not on tactical ones like "what function should I fix next?"

4. **Should agent teams be revisited later?** Agent teams (experimental, parallel Claude Code instances with shared task lists and mailbox messaging) are Gastown-shaped — high token burn, coordination overhead, bypasses the knowledge graph. But for specific use cases like parallel code review across security/performance/tests, they could complement Spore. Parked until `spore loop` is working and the subagent model is validated.

## Decision

Migrate Guide from separate session (`/opt/guide/`) to native subagent (`~/Mycelica/.claude/agents/guide.md`). This is simpler, lighter, and correctly positions the Guide as a consultable perspective rather than a dispatch loop operator. Implement after the current priority (spore loop) lands.

The `/opt/guide/` deployment, `--agent operator` routing, and the dispatch loop CLAUDE.md are instructive failures — they clarified what the Guide actually is by showing what it isn't.

## Resolution

Implemented 2026-02-19. Guide migrated to `~/Mycelica/.claude/agents/guide.md` as project-scoped subagent with `tools: Read, Bash, Grep, Glob` (no Write/Edit — returns recommendations to Hypha). Strategic memory seeded at `~/.claude/agent-memory/guide/MEMORY.md`. Global agent at `~/.claude/agents/guide.md` removed. `/opt/guide/` directory and `~/.claude/projects/-opt-guide/` project memory cleaned up.

**Status: RESOLVED.**
