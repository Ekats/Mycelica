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
| 1 | 2026-07-31 | resolved | `.mcp.json:7` points mco MCP at `<repo>/.mco/graph.db`, which existed but held 0 nodes / 0 edges — every `mca_*` tool returned empty from a mycelica-cwd session. Indexed 2026-10-03, now 3,606 edges |
| 2 | 2026-07-31 | invalid | ~~`mca_search` returns `[]` for snake_case symbols — FTS5 tokenization~~ **Not reproducible.** `mca_search("analyze_code_edges")` returns hits once the graph is indexed. The original `[]` was bug 1 (empty DB) compounded by querying mco-only symbols (`resolve_call_edges`, `import_code_files`) against the mycelica graph. No tokenization defect; MCA_GUIDE is accurate |
| 3 | 2026-07-31 | resolved | ~~`src-tauri/src/bin/cli.rs:6010` `collect_paths` takes no depth param — `nav folder --depth` silently ignored and `--summary` unhandled in JSON mode (same class as mca `d145f04`/`0bbeae3`)~~ Ported both: JSON collectors take `current_depth`/`max_depth`, `--summary --json` emits `{"path","files","items"}`, and depth is relative to the path filter in every renderer |
| 4 | 2026-10-03 | resolved | ~~`find_called_functions` keys `known_functions` as `HashMap<name, id>` with no language component, so the string-match call-graph pass can emit cross-language phantom edges~~ Resolved by the `code/` port: `analyze_code_edges`, `find_called_functions` and `extract_function_name` are deleted, along with two further inline copies of the same text-matching pass (`setup` step 3 and a "Refresh Calls edges" block in `run_code_update`). Call edges now come from the AST path, keyed on `(language, name)` |
| 5 | 2026-11-17 | resolved | ~~`spore::tests::test_find_project_root_finds_root_from_subdir` (`src/bin/cli/spore.rs:2416`) fails on any checkout without a `.mycelica.db`: `find_project_root` (`src/bin/cli/spore_runs.rs:205`) walks up looking for that file, and the test asserts it is found. A DB-content dependency in a unit test — make it create a temp marker or skip when absent. 432/433 `mycelica-cli` tests pass otherwise~~ Test is now hermetic: it writes a `.mycelica.db` marker in a tempdir and asserts `find_project_root_from(<tmp>/a/b)` returns the tempdir |
| 6 | 2026-11-17 | resolved | ~~`analyze doc-edges` (`src-tauri/src/bin/cli.rs` `analyze_doc_edges`) is now a second producer of `documents` edges alongside `extract_doc_code_edges`, which the ported `code/mod.rs` runs at the end of every `import_code`/`import_code_files` — running both double-writes doc→code edges by independent heuristics~~ `analyze_doc_edges` now uses the importer's full-id scheme `edge-doc-{doc.id}-{code_id}` (was 8-char prefixes), so for pairs both producers find, INSERT OR IGNORE dedups them and re-runs are no-ops; the command itself is kept |
| 7 | 2026-11-17 | resolved | ~~`nav folder` depth is off by one between renderers (inherited from mco `d145f04`): JSON collectors count the synthetic root `""` as depth 0, `print_folder_tree` starts at root's children — `--depth 1` yields `"",src-tauri` in JSON vs `src-tauri/src` in text (`src-tauri/src/bin/cli.rs` `NavCommands::Folder`)~~ `collect_paths`/`collect_summary` now start at root's children at depth 0 like `print_folder_tree`, so JSON and text agree and no `""` path is emitted; intentional divergence from mco, noted in a code comment |
