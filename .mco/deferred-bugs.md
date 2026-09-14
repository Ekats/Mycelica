# Deferred Bugs

Bugs found *on the way* to a task — parked here so the task stays on the happy path.
This file is the single home for "noticed but not now." It is a triage queue, not a fix log.

## Protocol (agents follow this — no exceptions)

1. **Find a bug mid-task → append ONE line to the log below** (date, one-sentence symptom + `file:line`). That's it. Do **not** fix it, investigate it, or replan the task around it.
2. **Stay on the happy path.** Finish and report the task you were actually given.
3. **At the end of the task, list the deferred bugs you logged** (in your final report). Also surface one *earlier* if it becomes directly relevant to the current task — a one-line "note: this touches deferred bug X" is enough.
4. **Never fix a deferred bug unless explicitly asked.** The human triages this list during review — right after the task's data/deliverable lands. Fixing out of order is how a task gets lost.

A bug on the way is a **note**, not a detour. One line here beats a five-file tangent every time.

## Log

| id | date | status | bug |
|----|------|--------|-----|
| 1 | 2026-07-31 | open | `.mcp.json:7` points mco MCP at `/home/spore/Repos/mycelica/.mco/graph.db` which does not exist — every `mca_*` tool returns empty from a mycelica-cwd session |
| 2 | 2026-07-31 | open | `mca_search` returns `[]` for snake_case symbols (e.g. `resolve_call_edges`) though the node is indexed (`code-58bbe0033e19b87e`) — FTS5 tokenization, contradicts MCA_GUIDE "exact symbol names" |
| 3 | 2026-07-31 | open | (mco repo) `src-tauri/src/db/schema.rs:708,726` adds `vital_level` column twice — redundant guarded migration |
| 4 | 2026-07-31 | open | (mco repo) `src-tauri/src/db/schema.rs:805,821` creates `crawl_queue` table + wiki/domain indices on every DB open; referenced nowhere — dead crawler scaffolding |
| 5 | 2026-07-31 | open | (mco repo) `src-tauri/src/bin/cli.rs:6590` doc comment references `crawl import-dump --gpu-embed`, a subcommand that no longer exists |
| 6 | 2026-07-31 | open | mycelica `src-tauri/src/bin/cli.rs:6010` `collect_paths` takes no depth param — `nav folder --depth` silently ignored and `--summary` unhandled in JSON mode (same class as mca `d145f04`/`0bbeae3`) |
