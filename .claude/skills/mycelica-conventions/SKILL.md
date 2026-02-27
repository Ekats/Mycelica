---
name: mycelica-conventions
description: Mycelica codebase conventions, CLI patterns, key files, and mandatory workflows for implementation work
user-invocable: false
---

# Mycelica Conventions

## Database Safety

**NEVER run destructive/modifying commands on ANY database without explicit permission.** Always ask before running maintenance commands, hierarchy operations, imports, or anything that writes. Read-only commands (search, nav, stats) are fine.

## Codebase Exploration

Prefer `mycelica-cli` over grep/glob. The graph knows relationships grep doesn't.

Pre-computed embeddings enable instant semantic search (no LLM calls), call graph is pre-indexed, single SQLite query vs. iterative file traversal.

```bash
# Find relevant code
mycelica-cli search "clustering"

# View actual source code (reads file, shows line range)
mycelica-cli code show <id>

# Get node metadata (file path, line numbers in tags JSON)
mycelica-cli node get <id>

# Who calls this function?
mycelica-cli nav edges <id> --type calls --direction incoming

# What does this function call?
mycelica-cli nav edges <id> --type calls --direction outgoing

# What docs reference this code?
mycelica-cli nav edges <id> --type documents --direction incoming

# Browse file structure
mycelica-cli nav folder src-tauri/src/db/
```

**Auto-discovery:** Commands find `.mycelica.db` by walking up directories (like `.git`).

## After Editing: Mandatory

After any code change, update the index:
```bash
mycelica-cli import code <file-or-directory> --update
```

Do not proceed without updating the index. Deletes old nodes, reimports, regenerates embeddings, refreshes edges. Seconds, not minutes.

After editing CLI or library code, always reinstall and update sidecar:
```bash
cd src-tauri
cargo install --path . --bin mycelica-cli --features mcp --force
cp ~/.cargo/bin/mycelica-cli binaries/mycelica-cli-x86_64-unknown-linux-gnu
```

Never run CLI from `./target/release/` — always install globally with mcp feature.
The sidecar copy is needed for the GUI to spawn the CLI (Tauri bundles it from `binaries/`).

## Key Files

| What | Where |
|------|-------|
| Database schema | `src-tauri/src/db/schema.rs` |
| Data models | `src-tauri/src/db/models.rs` |
| CLI commands (core) | `src-tauri/src/bin/cli.rs` |
| CLI spore/orchestrator | `src-tauri/src/bin/cli/spore.rs` |
| CLI terminal UI (TUI) | `src-tauri/src/bin/cli/tui.rs` |
| Tauri commands | `src-tauri/src/commands/graph.rs` |
| Hierarchy build | `src-tauri/src/hierarchy.rs` |
| Adaptive tree algorithm | `src-tauri/src/dendrogram.rs` |
| Similarity computation | `src-tauri/src/similarity.rs` |
| Code parsing | `src-tauri/src/code/rust_parser.rs` |
| Code import | `src-tauri/src/code/mod.rs` |
| AI client | `src-tauri/src/ai_client.rs` |
| Local embeddings | `src-tauri/src/local_embeddings.rs` |
| HTTP server | `src-tauri/src/http_server.rs` |
| Browser sessions | `src-tauri/src/holerabbit.rs` |
| OpenAIRE API | `src-tauri/src/openaire.rs` |

## Edge Types

**Auto-generated:** `Calls` (function calls), `DefinedIn` (code in file/module), `Documents` (markdown references code), `Sibling` (shared parent category), `BelongsTo` (item in category), `Clicked`/`Backtracked`/`SessionItem` (browser sessions).

**Manual:** `Reference`, `Because`, `Related`, `Contains`.

**Future:** `UsesType`, `Implements`, `Tests`, `Imports`.

## Constraints

- **No emoji in prompts** — AI prompts exclude emoji to avoid bias; emoji are still generated for display
- **No OpenAI embeddings** — Local only (all-MiniLM-L6-v2)
- **Code items skip AI** — Keep signatures as titles
- **Edge columns** — `source_id`/`target_id` (not `source`/`target`)

## Common Patterns

### Add CLI Command
1. Add enum variant in `cli.rs` (find `enum Commands`)
2. Add `async fn handle_X()`
3. Wire in `run_cli()` match

### Add Edge Type
1. Add to `EdgeType` in `db/models.rs`
2. Add string conversion
3. Use `db.insert_edge()`

## First-Time Setup

Only needed if `.mycelica.db` doesn't exist:
```bash
cd src-tauri
cargo install --path . --bin mycelica-cli --features mcp --force
cd ..
mycelica-cli db new .mycelica.db
mycelica-cli import code .
mycelica-cli setup  # clusters + embeddings + call graph (~2-3 min)
```

## Agent System Work

When modifying the agent system itself (agent definitions, skills, memory files, CLAUDE.md), consult the `claude-code-guide` agent for current best practices on frontmatter fields, skill injection, and agent configuration.
