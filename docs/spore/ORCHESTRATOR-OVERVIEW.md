# Spore Orchestrator Overview

*Last updated: 2026-02-21*

## What Is Spore?

Multi-agent orchestration for code tasks. A pure Rust orchestrator compiles graph context into task files, dispatches Claude Code agents through a 3-role pipeline (coder, verifier, summarizer), and records outcomes back to the knowledge graph. Works on any codebase with a CLAUDE.md.

## Architecture

```
Task Description
  |
  v
Context Compilation (semantic search + FTS + Dijkstra + lessons)
  |
  v
Coder (opus, writes code)
  |
  v
Verifier (opus, checks correctness)
  |  |
  |  +-- contradicts --> Coder (bounce, max 3)
  |  +-- max bounces --> Escalation node
  |
  v
Summarizer (sonnet, optional -- creates knowledge nodes)
```

## Agent Roles

| Agent | Model | Turns | Role |
|-------|-------|-------|------|
| Coder | opus | 50 | Writes code. No Grep/Glob -- uses MCP search. Session resume on bounce 2+. |
| Verifier | opus | 50 | Checks correctness. Read-only. Outputs structured verdicts. |
| Summarizer | sonnet | 15 | Creates Summary/Lesson nodes in graph. Read-only. Optional. |

The **operator** role (opus, 80 turns, full access) exists for manual single-agent use via `spore agent operator`. It is not part of the pipeline.

## Key Technologies

**Task file generation.** Graph-compiled context with semantic search (embeddings), FTS, Dijkstra expansion over the knowledge graph, call graph traversal, and lessons from past runs. Highest-leverage part of the system -- right anchors cut coder turns from 23 to 5.

**Native agents.** `.claude/agents/<role>.md` with `memory: project` and `skills: [mycelica-conventions]`. Auto-detected by `resolve_agent_name()`. Falls back to inline templates for foreign repos.

**Session resume.** Bounce-2+ coders use `--resume` to keep full context from bounce 1, with verifier feedback injected. Falls back to fresh session on failure.

**Verdict system.** Three-tier detection: graph edges (strongest), structured JSON `<verdict>` blocks, keyword scan (fallback). All tiers create graph edges.

**Loop.** Sequential dispatch with state persistence, auto-commit between tasks, budget tracking. `--reset` clears state for fresh runs.

**Selective staging.** `selective_git_add()` excludes `.env`, `.db`, `target/`, `node_modules/` from auto-commits.

## Graph Integration

Nodes created by the orchestrator:
- Task, Implementation (from git diff), Verdict, Escalation

Edges created by the orchestrator:
- DerivesFrom, Supports/Contradicts, Related, Tracks (self-loop analytics), Flags

Nodes created by agents via MCP:
- Summary, Lesson

Lessons feed back into future task files via embedding similarity (threshold 0.15).

## Cost Model

| Role | Model | Typical Cost |
|------|-------|-------------|
| Coder | opus | $0.50-2.00 |
| Verifier | opus | $0.60-1.40 |
| Summarizer | sonnet | $0.10-0.30 |

Median total: ~$1.48/run. Opus coder is 39% cheaper than sonnet overall due to fewer turns (A/B validated).

## Key Files

| What | Where |
|------|-------|
| Orchestrator + pipeline logic | `src-tauri/src/bin/cli/spore.rs` (~9.3K lines) |
| Run analytics | `src-tauri/src/bin/cli/spore_runs.rs` (~3.6K lines) |
| CLI command definitions | `src-tauri/src/bin/cli.rs` |
| MCP server + role filtering | `src-tauri/src/mcp.rs` |
| Native agent definitions | `.claude/agents/{coder,verifier,summarizer}.md` |
| Agent prompt templates | `docs/spore/agents/{coder,verifier,summarizer,operator}.md` |
| Database schema | `src-tauri/src/db/schema.rs` |
| AI client (model routing) | `src-tauri/src/ai_client.rs` |

## CLI Quick Reference

```bash
# Single task
mycelica-cli spore orchestrate "Fix the pagination bug"

# With options
mycelica-cli spore orchestrate "task" --max-bounces 2 --no-summarize --verbose

# Batch via loop
mycelica-cli spore loop --source tasks.txt --budget 50 --max-runs 10

# Dashboard and analytics
mycelica-cli spore dashboard
mycelica-cli spore runs stats
mycelica-cli spore runs timeline <run-id>

# A/B experiments
mycelica-cli spore orchestrate "task" --experiment label-a
mycelica-cli spore runs stats --experiment label-a
```

## Cross-Codebase Portability

Spore works on any codebase with a CLAUDE.md. Proven on:
- **Mycelica** (Rust, Tauri) -- 100+ verified runs
- **fd** (Rust CLI) -- 18/18 verified
- **commander.js** (TypeScript) -- 3/3 verified

When orchestrating on foreign repos, `resolve_agent_name()` falls back to inline prompt templates if no `.claude/agents/` files exist.

## Known Limitations

1. **Complex tasks ~50% success at 7+ complexity.** Keep tasks focused: one deliverable, minimal prose.
2. **Sequential pipeline.** No parallel subtask execution.
3. **No verifier retry.** Verifier crash kills the run (coder has retry with 10s cooldown).
4. **spore.rs is ~9.3K lines.** Large single file, but split from analytics (spore_runs.rs).
