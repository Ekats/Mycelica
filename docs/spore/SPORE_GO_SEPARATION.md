# Spore → Go: Separation Plan

## The Principle

SQLite is the interface. Mycelica (Rust/TS) owns the database and the GUI. Spore (Go) reads and writes to the same `.mycelica.db` file. They share nothing except the schema. Two separate binaries, two separate repos, one database.

```
┌──────────────────────────────┐     ┌──────────────────────────────┐
│  MYCELICA (Rust/TypeScript)  │     │       SPORE (Go)             │
│                              │     │                              │
│  Tauri desktop app           │     │  Standalone CLI binary       │
│  mycelica-cli                │     │  You understand every line   │
│  React frontend              │     │                              │
│  Schema owner                │     │  Graph analysis              │
│  Hierarchy builder           │     │  Agent coordination          │
│  Embedding computation       │     │  MCP server                  │
│  Import pipelines            │     │  Orchestration               │
│  GUI rendering               │     │  Web search / dot-connecting │
│                              │     │                              │
│  FROZEN (works, don't break) │     │  ACTIVE DEVELOPMENT          │
└──────────────┬───────────────┘     └──────────────┬───────────────┘
               │                                    │
               │        SQLite (.mycelica.db)       │
               └────────────────┬───────────────────┘
                                │
                    ┌───────────▼───────────┐
                    │   nodes, edges,       │
                    │   embeddings,         │
                    │   hierarchy,          │
                    │   agent_id,           │
                    │   confidence,         │
                    │   superseded_by,      │
                    │   meta_type ...       │
                    └───────────────────────┘
```

## What Stays in Rust (mycelica-cli)

Everything that exists today and works. Don't port, don't rewrite, don't touch unless something breaks.

- `node` commands (get, search, create, edit, delete)
- `link` commands (including the enhanced --content/--agent/--confidence/--supersedes)
- `nav` commands (tree, explore)
- `db` commands (stats, maintenance, migrate)
- `import` commands (conversations, code, etc.)
- `hierarchy` commands (build, rebuild)
- `spore` subcommands that already exist (query-edges, explain-edge, path-between, edges-for-context, create-meta, update-meta, status, health, analyze)

These stay because they're deeply coupled to `mycelica_lib` types, the schema.rs query methods, and the embedding infrastructure. Porting them would mean reimplementing the entire database layer in Go for zero benefit — they already work.

The existing spore subcommands become **read/write primitives**. The Rust CLI is how data gets into and out of the graph. The Go binary orchestrates when and why.

## What Moves to Go (new `spore` binary)

Everything built from now on. The Go binary is the **brain** — it decides what to do, when to analyze, when to dispatch agents, when to search the web. The Rust CLI is the **hands** — it executes graph operations.

### Phase 1: Foundation (~1-2 days)

**Project setup:**
```
spore/
├── go.mod
├── main.go              # CLI entry point
├── cmd/
│   ├── root.go          # cobra root command
│   ├── analyze.go       # spore analyze
│   └── version.go
├── internal/
│   ├── db/
│   │   ├── sqlite.go    # SQLite connection + queries
│   │   ├── models.go    # Node, Edge, etc. structs
│   │   └── queries.go   # Typed query functions
│   ├── graph/
│   │   ├── snapshot.go  # GraphSnapshot from DB
│   │   ├── topology.go  # Connected components, degree, orphans, hubs
│   │   ├── bridges.go   # Tarjan's algorithm
│   │   ├── staleness.go # Stale node/summary detection
│   │   └── report.go    # AnalysisReport assembly + health score
│   └── format/
│       ├── json.go      # JSON output
│       └── terminal.go  # Human-readable colored output
├── internal/db/
│   └── sqlite_test.go
├── internal/graph/
│   ├── topology_test.go
│   ├── bridges_test.go
│   └── staleness_test.go
└── README.md
```

**Dependencies (minimal):**
- `modernc.org/sqlite` — pure Go SQLite (no CGo, cross-compiles cleanly)
- `github.com/spf13/cobra` — CLI framework
- `github.com/fatih/color` — terminal colors (optional, can use ANSI directly)

That's it. No frameworks. No ORMs. No dependency trees. You read the SQL, you read the structs, you read the algorithms.

**DB layer** — this is the critical piece. Define Go structs that map to the SQLite schema:

```go
type Node struct {
    ID          string
    Title       string
    Content     string
    ParentID    *string
    IsItem      bool
    Depth       int
    CreatedAt   int64  // millis
    UpdatedAt   int64  // millis
    AgentID     *string
    NodeClass   *string // "knowledge", "meta", "operational"
    MetaType    *string // "summary", "contradiction", "status"
}

type Edge struct {
    ID           string
    SourceID     string
    TargetID     string
    EdgeType     string  // lowercase: "summarizes", "contradicts", etc.
    Content      *string
    AgentID      *string
    Confidence   *float64
    SupersededBy *string
    CreatedAt    int64
    UpdatedAt    int64
}
```

**Queries** — direct SQL, no ORM:

```go
func (db *DB) AllNodes() ([]Node, error) {
    rows, err := db.conn.Query("SELECT id, title, content, parent_id, is_item, depth, created_at, updated_at, agent_id, node_class, meta_type FROM nodes")
    // ...
}

func (db *DB) AllEdges() ([]Edge, error) {
    rows, err := db.conn.Query("SELECT id, source_id, target_id, type, content, agent_id, confidence, superseded_by, created_at, updated_at FROM edges WHERE superseded_by IS NULL")
    // ...
}
```

You will understand this code because it's SQL queries mapped to structs. No macros, no trait implementations, no lifetime annotations.

### Phase 2: Port the Analyzer (~1 day)

Port the 542-line Rust `graph_analysis.rs` to Go. The algorithms are identical — they're graph theory, not Rust-specific:

| Rust | Go |
|---|---|
| `HashMap<String, Vec<...>>` | `map[string][]...` |
| `UnionFind` | Rewrite in Go (~40 lines, simple) |
| Tarjan's iterative DFS | Same algorithm, Go slices instead of Rust Vecs |
| `impl GraphSnapshot` | Methods on `GraphSnapshot` struct |
| `#[cfg(test)] mod tests` | `_test.go` files |
| `serde_json::to_string` | `json.Marshal` |

The port is mechanical. Same logic, different syntax. Go's simplicity means fewer abstractions between you and the algorithm.

**Validation:** Run both the Rust analyzer and Go analyzer against the same database. Output should match: same health score, same component count, same orphans, same bridges. If they don't match, one of them has a bug — diff the output and find it.

### Phase 3: Go Becomes Primary (~1 day)

Once the Go analyzer produces identical output to the Rust one:

1. Add `spore analyze` as the first Go subcommand
2. Alias or document: `spore analyze` (Go) replaces `mycelica-cli spore analyze` (Rust)
3. The Rust version stays but stops getting new features

From this point forward, all new Spore features go into the Go binary.

## How They Interact

Two interaction patterns, both simple:

### Pattern 1: Go reads SQLite directly (primary)

For analysis, queries, reading graph state — Go opens the database read-only and runs SQL. This is what the analyzer does. No Rust involvement.

```go
db, err := OpenDB("/home/you/.local/share/com.mycelica.app/mycelica.db")
// or wherever .mycelica.db lives
snapshot := graph.NewSnapshot(db)
report := graph.Analyze(snapshot, config)
```

### Pattern 2: Go calls mycelica-cli for writes (when needed)

For creating nodes, edges, meta-nodes — operations that need to maintain schema invariants, trigger FTS indexes, update hierarchy sovereignty rules — Go shells out to `mycelica-cli`:

```go
func (s *Spore) CreateMetaNode(title, content, agent string, connectsTo []string) error {
    args := []string{"spore", "create-meta",
        "--type", "summary",
        "--title", title,
        "--content", content,
        "--agent", agent,
    }
    for _, id := range connectsTo {
        args = append(args, "--connects-to", id)
    }
    return exec.Command("mycelica-cli", args...).Run()
}
```

This is the right tradeoff: Rust handles the complex write logic (FTS triggers, sovereignty checks, transaction wrapping) while Go handles the decision logic (what to create, when, why). Go is the brain, Rust is the hands.

**When Go writes directly to SQLite:** Only for tables/columns that Spore fully owns — like a future `spore_runs` table for tracking analysis history. Never for nodes/edges directly, because Mycelica's FTS indexes and hierarchy integrity depend on writes going through the proper code paths.

### Pattern 3: MCP Server in Go (future)

The Phase 3 MCP server (originally planned in Rust with `rmcp`) gets built in Go instead. Go has good MCP libraries, the stdio transport is trivial, and the permission model maps to Go interfaces cleanly. The MCP tools call mycelica-cli under the hood for writes, or read SQLite directly for queries.

This means agent orchestration (Enterprise/OpenCode connecting to the MCP server) talks to a Go process, not a Rust one. The Go process you understand and can debug.

## Database Path Discovery

The Go binary needs to find `.mycelica.db`. Three options in priority order:

1. `MYCELICA_DB` environment variable (explicit override)
2. `--db /path/to/file.db` CLI flag
3. Default: walk up from CWD looking for `.mycelica.db` (same behavior as mycelica-cli)
4. Fallback: `~/.local/share/com.mycelica.app/` (XDG data dir on Linux)

## What About the Rust Spore Commands?

They stay. They're useful as primitives:

```bash
# Rust CLI — low-level graph operations (stays)
mycelica-cli spore query-edges --type contradicts --not-superseded
mycelica-cli spore create-meta --type summary --title "..." --content "..."
mycelica-cli spore explain-edge <id>
mycelica-cli spore status
mycelica-cli spore health
mycelica-cli spore analyze  # Rust version, eventually deprecated

# Go binary — high-level intelligence (new)
spore analyze              # Port of Rust analyzer, then extended
spore orchestrate          # Future: agent coordination
spore connect              # Future: cross-region dot-connecting
```

Over time, the Go binary accumulates all the "thinking" functionality. The Rust CLI remains the low-level toolkit for graph manipulation. They coexist permanently.

## Migration Path

```
NOW:     mycelica-cli has everything, you understand none of the Rust
         │
         ▼
WEEK 1:  Go binary exists, reads SQLite, analyzer works
         Go output matches Rust output (validated)
         │
         ▼
WEEK 2:  New features go into Go only
         Agent coordination prototyped in Go
         MCP server started in Go
         │
         ▼
MONTH 1: Go binary is the primary Spore interface
         Rust spore subcommands are stable primitives
         You can debug any Spore issue by reading Go code
         │
         ▼
FUTURE:  Spore grows in Go indefinitely
         Mycelica Rust stays frozen (bug fixes only)
         You own the part that matters
```

## Decision

Start with Phase 1 foundation. Get `spore analyze` working in Go with identical output to the Rust version. That proves the SQLite-as-interface architecture works and gives you a Go codebase you can build on.

Don't port anything else from Rust yet. The existing Rust commands are stable and useful as write primitives. Only build NEW things in Go.
