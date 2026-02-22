# Branch Audit: feat/context-for-task vs master

**Date**: 2026-02-22
**Branch**: `feat/context-for-task` (217 commits ahead of master after cleanup)
**Audited**: 216 original commits + 1 cleanup commit

## Summary

| Category | Commits | % |
|----------|---------|---|
| Core feature (still in codebase) | 83 | 38% |
| Bugfix (real bugs) | 22 | 10% |
| Refactor / cleanup | 10 | 5% |
| Docs / config (living) | 21 | 10% |
| Go port (Phase 1 + 2) | 4 | 2% |
| **Total surviving work** | **140** | **65%** |
| PRIORITIES.md journal | 17 | 8% |
| TASK_QUEUE.md journal | 12 | 6% |
| feat(batch) double-commits | 10 | 5% |
| Built-then-deleted features | 18 | 8% |
| Fix-the-fix chains | 8 | 4% |
| Other churn / noise | 11 | 5% |
| **Total waste** | **76** | **35%** |

**Net code impact**: +32,118 / -4,071 lines across 78 files

---

## What Still Exists (the real work)

### Rust: CLI module split
Master had a single `cli.rs` at 12,102 lines. The branch split it into:

| File | Lines | Content |
|------|-------|---------|
| `cli.rs` | 11,015 | Core CLI commands (shrunk but still large) |
| `cli/spore.rs` | ~8,900 | Orchestration pipeline, coder-verifier loop, context compilation |
| `cli/spore_runs.rs` | 3,623 | Analytics: dashboard, runs stats/list/compare, health, lessons, distill |
| `cli/tui.rs` | 2,111 | Terminal UI (extracted from cli.rs) |
| `cli/spore_analyzer.rs` | 171 | Graph structural analysis bridge to graph_analysis.rs |

**Total Rust CLI: ~25,820 lines** (was 12,102 on master -> net +13,718 lines of new functionality)

### Rust: New library files

| File | Lines | What |
|------|-------|------|
| `graph_analysis.rs` | 1,249 | Topology, bridges, staleness, health score |
| `similarity.rs` | 60 | Cosine similarity for embeddings |
| `schema.rs` | +574 | Context compilation (Dijkstra), edge scoring, FTS helpers |
| `mcp.rs` | +320 | MCP tools for agents (code_show, nav_folder, explore) |
| `models.rs` | +22 | New edge types, agent roles |

### Rust: Cargo.toml changes
- `tokio` gains `"signal"` feature (for SIGPIPE/graceful shutdown)
- `libc = "0.2"` (for signal handling)
- `hf-hub` upgraded `0.3 -> 0.4` (fix broken embedding downloads)

### Go: Complete port (Phase 1 + 2)

| Directory | Lines | Tests |
|-----------|-------|-------|
| `spore/internal/graph/` | 1,200 | 24 |
| `spore/internal/db/` | 1,009 | 27 |
| `spore/cmd/` | 543 | -- |
| **Total** | **3,459** | **51** |

### Agent files (`.claude/agents/`, `.claude/skills/`)
- `coder.md` (80), `verifier.md` (81), `summarizer.md` (112), `hypha.md` (89)
- `mycelica-conventions/SKILL.md` (125)

### Docs (tracked, living)
28 doc files, ~7,300 lines total. Key ones:
- `CLI-REFERENCE.md` (~960) -- full command reference
- `CURRENT-STATE.md` (788) -- ground truth from source code audit
- `GO-PORT-PHASE2.md` (731) -- Go port documentation
- `PLAN.md` (613) -- design plan
- `PIPELINE.md` (~458) -- pipeline architecture
- `DOC.md` (454) -- system documentation
- `SPORE_ANALYZER_PLAN.md` (421) -- analyzer design
- `SPORE_CONCERNS.md` (346) -- design concerns registry
- `SPORE_GO_SEPARATION.md` (~275) -- Go separation strategy
- `cookbook-spore-symbiosis.md` (296), `security-as-hygiene.md` (264), `graph-as-memory.md` (200)

---

## What Was Waste

### 1. PRIORITIES.md journal (17 commits)

Every session updated PRIORITIES.md to log what was done. Each commit overwrites the previous one. The file was 72 lines on the branch. These should have been a single commit or not committed at all.

```
b5c5e19 chore: update priorities -- 11/12 self-audit fixes verified
985fc7f chore: update priorities -- lesson structuring done, agent retry next
acf3d2b chore: update priorities -- prompt shrinkage done, lesson structuring next
a160c96 chore: update priorities -- contextual embeddings done, new direction
414f5af chore: update priorities -- native porting done, pending run triage next
40d0560 chore: update priorities -- triage done, real verification rate is 73%
d75a1bc chore: update priorities -- CWD fix done, graph-as-memory is next
5e416e0 chore: update priorities -- verification analysis done, 92% post-update rate
a52c261 chore: update priorities -- plan-only status done, contextual retrieval v2 is next
a4a448e chore: pivot to portability...
d7c3a18 chore: update priorities -- pause-after-plan done, portability test next
bbd377d chore: update priorities -- experiment tagging done, portability complete
3abf0a5 chore: update priorities -- Spore-on-Spore verified, output bug identified
8e13b5e chore: update priorities -- compare-experiments done, context quality next
e901b2b chore: update priorities -- FTS fix done, semantic search improvement next
4b4450a chore: update priorities -- spore loop + model routing done, new directions
cf2bdd9 chore: hard A/B complete -- opus advantage grows with complexity
```

**Status**: PRIORITIES.md gitignored and untracked in cleanup commit `23f400e`.

### 2. TASK_QUEUE.md journal (12 commits)

Same pattern -- session status updates to a queue file:

```
dd7cb2a docs: update task queue with completed items and new priorities
ea0a62f docs: update task queue after architect agent + verifier improvements
c519915 docs: update task queue with MCP fixes and batch-built helpers
05c6a98 docs: update task queue -- error recovery, --quiet, --cost, stale node cleanup
fe77d0a docs: update task queue -- stale detection, tester improvements, plan summarizer
2000996 docs: update task queue -- health command, compact format
b5cf7ca docs: update task queue -- researcher agent complete
268c3f9 docs: update task queue -- researcher agent + architect-planner bouncing
27a5ceb docs: update task queue -- status filter, runs top
a40387a docs: update task queue with completed items
ecf0b6f feat: --stale flag for spore dashboard + tester improvements task queue update
5dcf2b8 docs: update CLAUDE.md quick reference + task queue
```

**Status**: TASK_QUEUE.md gitignored and untracked in cleanup commit `23f400e`.

### 3. feat(batch) double-commit pattern (10 commits)

Spore generates rough code via `feat(batch)`, then the next commit rewrites it. The batch commit is noise:

```
76a5fa3 feat(batch): Add a --limit flag...   <- noise
6cabb22 feat: orchestrator checkpoint/resume + --json lessons + --limit runs   <- real

c037394 feat(batch): Add a function called count_words...   <- noise
4db5676 feat(batch): count_words + truncate_middle helpers   <- real

95695f2 feat(batch): Add a --quiet flag...   <- noise
(rolled into next commit)

8d191c2 feat(batch): Add a `--agent` filter flag...   <- noise
02329a5 feat: --agent filter + runs timeline subcommand   <- real

a7ad708 feat(batch): Add a `--cost` flag...   <- noise
9e33acd feat: --cost sort flag + runs summary subcommand   <- real

e3a2a25 feat(batch): Add a `--json` flag...   <- noise
747434c feat: --json on runs timeline + runs compare subcommand   <- real

b3c5510 feat(batch): Add a `--limit` flag to `spore runs top`...   <- noise
(rolled into stats commit)

7fd0ae3 feat(batch): Add a `--status` filter flag...   <- noise
60633df [spore-plan:] Add --status flag...   <- real

ed69864 feat(batch): Add a `--topic` filter flag...   <- noise
b78cb86 [spore-plan:] Add --topic flag...   <- real
```

### 4. Features built then deleted (18 commits)

**Researcher agent** (built sessions 5-6, deleted session 9):
- `c1744f7` feat: researcher agent
- `47d7d04` feat: researcher pre-phase in orchestrator pipeline
- `fe22636` [spore-plan] Remove Bash(cargo:*) from researcher allowedTools
- `871c25c` feat(loop): stdout output requirement for researcher

**Planner agent** (built session 4, deleted session 9):
- `2a83682` feat: planner agent for task decomposition
- `cf7d754` fix: planner turn budget, JSON parser, and stdout fallback
- `524406f` feat: architect-planner bouncing

**Architect agent** (built session 5, deleted session 9):
- `8c63864` feat: add Architect agent
- `7fcc28f` [spore-plan] Add Architect variant to AgentRole

**Tester agent** (built session 5, deleted session 9):
- `fe719f5` feat: tester agent + auto-commit between batch tasks
- `870ba68` feat: tester agent improvements
- `d86f7d1` fix: add Tester variant to AgentRole
- `2972734` fix: portable tester -- remove .rs extension gate

**Scout** (built session 4, deleted session 9):
- `f66e081` feat: --scout flag
- `64f97e3` fix: scout prompt and turn budget

**Parallel worktrees** (built session 5, deleted session 9):
- `648e621` feat: add parallel subtask execution with git worktrees

All 18 commits represent real engineering work that was correctly identified as redundant in session 9 and deleted. Not "churn" -- it was valid experimentation that informed the simplification decision. But the code is gone.

### 5. Fix-the-fix chains (8 commits)

**post_coder_cleanup** (3 fixes for the same subsystem):
```
790141a fix: cargo install fallback in post_coder_cleanup
0677de7 fix: improve post_coder_cleanup reinstall diagnostics
31a1bc0 fix: prevent 'Text file busy' during post-coder CLI reinstall
```

**gc** (fix immediately after feature):
```
f3cfe97 feat: spore gc -- actual deletion
f2c0ff0 fix: gc must check outgoing edges too, not just incoming
```

**dashboard** (fix immediately after feature):
```
e4f5078 feat: spore dashboard
6081242 feat: --limit flag + remove dead variable   (dead variable = bug from previous commit)
```

**planner** (fix immediately after feature -- then entire feature deleted):
```
2a83682 feat: planner agent
cf7d754 fix: planner turn budget, JSON parser, stdout fallback
```

### 6. feat(loop) commits (26 total)

Self-modifying commits from the Spore loop engine. Truncated commit messages (`feat(loop): In src-tauri/src/bin/cli...`). Of 26:
- ~16 survive (changes still visible in spore.rs)
- ~10 were later overwritten or deleted with the role deletion

Notable survivors: verdict parsing, cost anomaly detection, multi-line task files, contextual embeddings, startup retry, lesson quality threshold, selective git staging

Notable garbage: paired commits doing the same thing twice (`8f7743d` + `4cc4d41` both say "for consistency", `ab873e5` + `8562a56` both say "schema.rs logic")

---

## All 216 Commits (Chronological, Categorized)

**Legend**: F=core feature, B=bugfix, R=refactor, D=docs/config, G=Go port, J=journal, X=batch noise, DEL=deleted feature, C=churn/fix-the-fix, L=loop commit

```
35b2df4 B   fix: update hf-hub 0.3 -> 0.4 to fix broken embedding downloads
b1b536a F   feat: spore context-for-task -- Dijkstra context retrieval
2d32c04 B   fix: structural edges no longer flood context-for-task
ce179c0 F   feat: Phase 6.5 -- task file generation with graph context
d6d4ff0 F   feat: first successful orchestrator run with task file generation
1971000 F   feat: add --format full to spore status
4edd281 F   feat: semantic search for task file anchors, dry-run preview
f8a7530 F   feat: spore distill -- walk orchestrator run trails and summarize
272773e F   feat: spore gc -- find stale operational nodes
62eab9e F   feat: orchestrator fallback node + coder-verified --dry-run for gc
c3555f2 D   chore: untrack docs/spore/tasks/ files
8b4730f F   feat: spore runs -- list orchestrator runs with status detection
26e9350 F   feat: summarizer agent + --compact distill + error propagation
71c832e F   feat: --all flag for spore runs + first summarizer run
7c3d388 F   feat: inject lesson nodes into task files (Concern 13)
2951826 F   feat: --cost flag for spore runs + cost tracking
790141a C   fix: cargo install fallback in post_coder_cleanup
e4f5078 F   feat: spore dashboard -- combined view
6081242 C   feat: --limit flag for dashboard + remove dead variable
ac5febf F   feat: show contradiction details in dashboard
f66e081 DEL feat: --scout flag for thin-session experiment
64f97e3 DEL fix: scout prompt and turn budget
03ef14f F   feat: spore runs compare -- side-by-side
ab1a79e F   feat: add files_changed to runs compare
e7cae75 B   fix: strip backticks and validate paths in compare
c1d2743 F   feat: dashboard success rate + fix derives_from
c793fc9 F   feat: spore retry command
bb47761 F   feat: add code locations to task file
16ada51 F   feat: --escalated flag for spore runs list
64f9cfe F   feat: spore runs history -- timeline
f649fa1 B   fix: history command -- walk task node edges
737be87 B   fix: show actual verifier contradiction reason
ddc1a95 F   feat: spore lessons subcommand
f3cfe97 F   feat: spore gc -- actual deletion, Lesson exclusion
f2c0ff0 C   fix: gc must check outgoing edges too
f4bde09 F   feat: spore runs cancel
5e0ef46 B   fix: dashboard counts cancelled runs separately
368ff61 R   refactor: split cli.rs into modules (14,616 -> 8,322 + 4,191 + 2,111)
1ad6b8f D   docs: update CLAUDE.md key files table
763a7ad F   feat: always show cost column in runs list
1699660 B   fix: summarizer runs by default
65faf3b F   feat: agents use opus model + ultrathink
2a83682 DEL feat: planner agent for task decomposition
cf7d754 DEL fix: planner turn budget, JSON parser, stdout fallback
469521c B   fix: handle SIGPIPE gracefully + process timeout
dd7cb2a J   docs: update task queue
3052a98 F   feat: add spore health command
3a55426 F   [spore-plan] Implement MemoryStore struct and CLI
09496e3 F   feat: wire MemoryStore into orchestration pipeline
16b7bd2 B   fix: improve verifier verdict detection with fallbacks
f0aa6c2 B   fix: detect in-place edits on dirty files
eb71d0a D   docs: improve coder + verifier turn efficiency
c8c2b32 F   feat: add --timeout flag to spore orchestrate
0677de7 C   fix: improve post_coder_cleanup reinstall diagnostics
8c63864 DEL feat: add Architect agent
ea0a62f J   docs: update task queue after architect + verifier
41fb3be F   [spore-plan] Add notify + extract import-code-update helper
108e53d F   [spore-plan] Implement handle_watch
d11a3e0 F   feat: add spore watch command
648e621 DEL feat: add parallel subtask execution with git worktrees
9f863e2 F   feat: add inline code snippets to task files
f4cd650 DEL fix: pass timeout to planner/architect + SIGKILL fallback
a8c100f F   feat: add Files Likely Touched and Call Graph to task files
c32422d F   feat: add --format flag to dashboard (text/json/csv)
fb4921b F   feat: add spore batch command
8fad347 F   feat: --compact flag for lessons + runs cancel metadata
31a1bc0 C   fix: prevent 'Text file busy' during reinstall
70a1108 B   fix: prevent orchestrator hangs from MCP init failures
b3564b7 F   feat: add mycelica_explore MCP tool
e1e0911 F   feat: add 'spore runs show' alias
22626f6 F   feat: semantic lesson matching for cross-run learning
b931645 F   feat: content-aware dirty file detection + --since/--count
fe719f5 DEL feat: tester agent + auto-commit between batch tasks
76a5fa3 X   feat(batch): --limit flag (rough)
6cabb22 F   feat: orchestrator checkpoint/resume + --json lessons
a40387a J   docs: update task queue
dcb4288 F   feat: phase-level resume + batch MCP fix + format_duration_short
d86f7d1 DEL fix: add Tester variant to AgentRole
c037394 X   feat(batch): count_words helper (rough)
4db5676 F   feat(batch): count_words + truncate_middle helpers (clean)
c519915 J   docs: update task queue
58bed49 R   refactor: replace manual duration formatting
1b508a4 F   feat: error recovery for failed coders + MCP validation + --verbose
0f68d86 R   refactor: use truncate_middle() in output + --verbose tests
95695f2 X   feat(batch): --quiet flag (rough)
df35763 F   [spore-plan] --cost flag to dashboard
05c6a98 J   docs: update task queue
f27d4ea F   feat: code stale command + auto-cleanup
870ba68 DEL feat: tester agent improvements
ecf0b6f J   feat: --stale flag for dashboard + task queue update
ce44781 F   feat: architecture context injection + plan-level summarizer
fe77d0a J   docs: update task queue
c37e089 F   [spore-plan] count_dead_edges() + enhanced health
4bab00c F   [spore-plan] Compact variant for runs list
2000996 J   docs: update task queue
c1744f7 DEL feat: researcher agent
b5cf7ca J   docs: update task queue -- researcher complete
524406f DEL feat: architect-planner bouncing
268c3f9 J   docs: update task queue -- architect-planner bouncing
b78cb86 F   [spore-plan] --topic flag for research
ed69864 X   feat(batch): --topic filter (rough)
6f8537e F   feat: spore runs cost subcommand
5dcf2b8 D   docs: update CLAUDE.md quick reference
60633df F   [spore-plan] --status flag for runs list
7fd0ae3 X   feat(batch): --status filter (rough)
a3ab4da F   feat: spore runs top subcommand
27a5ceb J   docs: update task queue
b3c5510 X   feat(batch): --limit for runs top (rough)
5dcd988 F   feat: spore runs stats subcommand
742424c B   fix: record num_turns/duration_ms + fix bounce counting
47d7d04 DEL feat: researcher pre-phase in orchestrator
4fdd5d0 F   feat: --duration flag on runs list
8d191c2 X   feat(batch): --agent filter (rough)
02329a5 F   feat: --agent filter + runs timeline subcommand
a7ad708 X   feat(batch): --cost flag (rough)
9e33acd F   feat: --cost sort flag + runs summary subcommand
e3a2a25 X   feat(batch): --json for runs timeline (rough)
747434c F   feat: --json on runs timeline + runs compare
0bf1db3 F   feat: orchestrator-driven impl node creation
5ec8e0b F   feat: Guide agent architecture -- thinking, operator, --agent dispatch
ed5bb2c F   [spore-plan] is_lesson_quality() filter
3ae48ac F   feat: Guide-Hypha infrastructure -- design constraints, skills
b719605 F   feat: merged semantic+FTS anchor search, expanded context budget
36e972a F   feat: spore loop -- continuous orchestration engine
ca174fc L   feat(loop): add to _spore_loop
d2cfc8b F   [spore-plan] select_model_for_role() + 12 call sites
4b4450a J   chore: update priorities -- loop + model routing done
4b0063f L   feat(loop): verifier.md agent prompt
6199daf L   feat(loop): verdict markers
77762ec L   feat(loop): integrate into handle_spore_loop
2846ac5 L   feat(loop): structured data returns
7aa8c0b F   [spore-plan] confidence in VerifierVerdict + graph recording
27bc24a F   [spore-plan] simplify verifier.md
44eb28a L   feat(loop): from CLI match arm
5b58284 L   feat(loop): cost anomaly detection
244743c L   feat(loop): multi-line task file support
57680c8 L   feat(loop): contextual embedding text before encoding
0b7220e L   feat(loop): embedding format without prefix
2c1e3a5 L   feat(loop): apply same contextual prefix
3bc0281 F   feat: unit test for contextual embedding prefix
a160c96 J   chore: update priorities
a97866b B   fix: resolve 5 pre-existing test failures
a3c1578 F   [spore-plan] count_agent_prompt_lines() + prompt-stats command
048a56e F   [spore-plan] line count telemetry in generate_task_file()
db8c572 F   [spore-plan] prompt_size health check
acf3d2b J   chore: update priorities
74ceccc F   [spore-plan] Fix section extraction + summarizer template
a67883a B   fix: make test CWD-independent
985fc7f J   chore: update priorities
6c9424b F   feat: single-retry with 10s cooldown for agent startup hangs
da377c2 F   [spore-plan] coder.md + agent_name parameter
3056344 F   [spore-plan] --native-agent flag + conditional prompt logic
5248522 D   chore: track .claude/agents/ and .claude/skills/
414f5af J   chore: update priorities
40d0560 J   chore: update priorities
29713f6 B   fix: make health check and prompt_stats CWD-independent
d75a1bc J   chore: update priorities
a6c3f03 F   [spore-plan] Link subtask nodes via derives_from edges
5e416e0 J   chore: update priorities
d2e17d7 F   [spore-plan] planned status in compact + stats + dashboard
6029d03 B   fix: add planned status to stats aggregation
a52c261 J   chore: update priorities
a4a448e J   chore: pivot to portability
9b10603 F   feat: portable agent prompts + --pause-after-plan flag
d7c3a18 J   chore: update priorities
2972734 DEL fix: portable tester -- remove .rs extension gate
4a712f0 F   feat: --experiment flag for A/B run comparison
bbd377d J   chore: update priorities
afccaa5 D   docs: add doc comment to handle_runs
3abf0a5 J   chore: update priorities
b87b26a F   feat: --coder-model flag for A/B comparison
7f6c1c5 D   chore: A/B experiment complete -- opus 28% cheaper
9258d6b F   feat: make opus the default coder model
cf2bdd9 D   chore: hard A/B complete -- opus advantage grows
818fcbf F   [spore-plan] CompareExperiments CLI + handler
8e13b5e J   chore: update priorities
933cb75 B   fix: sanitize FTS queries -- prevent FTS5 crashes
e901b2b J   chore: update priorities
a6d3227 F   [spore-plan] graph nodes/edges in text-fallback verdict
c0413b3 L   feat(loop): text-fallback parsing
f96f5e1 L   feat(loop): contradicts edges
7fcc28f DEL [spore-plan] Add Architect variant to AgentRole
71080b9 L   feat(loop): design documentation
3db561f L   feat(loop): graph nodes
9e850d8 L   feat(loop): checklist items
f09f037 L   feat(loop): file path
c25a090 L   feat(loop): correct label
fbc9415 L   feat(loop): threshold
ad634d3 B   fix: use floor_char_boundary() for string truncations
ab873e5 L   feat(loop): schema.rs logic (1/2)
8562a56 L   feat(loop): schema.rs logic (2/2)
8f7743d L   feat(loop): consistency rename (1/2)
4cc4d41 L   feat(loop): consistency rename (2/2)
d21e1a2 L   feat(loop): startup retry pattern
b58bb56 F   [spore-plan] selective_git_add() + is_spore_excluded()
b5c5e19 J   chore: update priorities -- 11/12 audit fixes
eca77d7 F   feat(agents): create native agent files, add skills+memory
fe22636 DEL [spore-plan] Remove Bash(cargo:*) from researcher
871c25c L   feat(loop): stdout output requirement
c00325e L   feat(loop): no reason available
dbc805d F   feat: session resume for bounce-2+ coders, verifier feedback
fc28045 F   feat: auto-detect native agent mode for all 8 roles
2b4414c D   docs: comprehensive Spore pipeline documentation
99fb0a8 R   simplify: default pipeline to core loop only
c122abc R   delete: remove researcher, planner, architect, tester, scout
79934f9 R   extract: move analytics/reporting to spore_runs.rs
9d5a3a2 D   chore: delete dead-weight docs, open docs/spore in gitignore
66d1246 D   chore: untrack guide/ and GASTOWN-COMPARISON
c7bcb03 D   docs: rewrite 4 canonical docs for 3-role pipeline
3466a9c R   chore: fix all stale role references, delete PARALLEL.md
9a986e1 D   docs: add CURRENT-STATE.md -- ground truth reference
b443164 F   feat: gate task file context behind --experiment no-context
c503983 F   feat: add graph structural analyzer
3becfc9 G   feat: Go port Phase 1 -- analyzer with cross-validated output
ff098ef G   feat: Go port Phase 2 -- context compilation with cross-validated Dijkstra
23f400e R   chore: branch audit cleanup -- delete MemoryStore, spore watch, untrack journals
```

---

## Actions Taken

### Completed in cleanup commit `23f400e`

1. **Deleted MemoryStore** (~330 lines from spore.rs, ~36 from cli.rs, ~62 from docs)
   - Never used -- no `.spore-memory.json` file existed anywhere
   - Removed struct, impl, handle_memory(), find_memory_file(), CLI subcommand
   - Removed memory parameter from generate_task_file/handle_orchestrate/handle_single_agent
   - Removed 6 MemoryStore::load() blocks from dispatch paths
   - Removed Agent Memory task file injection section

2. **Deleted spore watch** (~150 lines from spore.rs, notify dep from Cargo.toml, ~125 from docs)
   - Never validated -- no tests, no evidence of use
   - Removed handle_watch(), collect_watch_paths(), watch_matches_filter()
   - Removed Watch CLI variant and all flags
   - Removed `notify = "7"` dependency (compile time savings)
   - Deleted `docs/spore/WATCH.md`

3. **Gitignored and untracked journal files**
   - PRIORITIES.md, docs/spore/TASK_QUEUE.md -> gitignored
   - `src-tauri/docs/spore/tasks/` (generated task specs) -> gitignored
   - Files kept locally, removed from git tracking

### Still outstanding

4. **feat(batch) double-commits**: Process change -- either don't commit raw batch output separately, or squash batch+cleanup into one commit.

5. **DefaultConfig drift in Go port**: `health.go` DefaultConfig may have been reset to HubThreshold=10, TopN=50, StaleDays=30 -- should be 15, 10, 60 to match CLI defaults.

6. **handle_prompt_stats scans wrong directory**: Looks in `docs/spore/agents/` but agent files are at `.claude/agents/`. Reports "no agent files found".

7. **Operator has no native agent file**: `.claude/agents/operator.md` doesn't exist. Falls back to non-native mode with template from `docs/spore/agents/operator.md`.

8. **Task files in `src-tauri/docs/spore/tasks/`**: Now gitignored but 5 files still exist locally. Ephemeral, not reference material.

### Merge strategy

**Squash-merge recommended**: `git merge --squash feat/context-for-task` from master. Single commit with comprehensive message. Clean history on master, full history preserved on branch. 35% of commits are noise anyway.
