# Spore Go Port — Complete Code Rundown (Phase 1 + Phase 2)

**Date**: 2026-02-22
**Branch**: `feat/context-for-task`
**Status**: 51 tests pass. Cross-validated against Rust on both `spore analyze` and `spore context-for-task`.

## File Inventory

```
spore/                                3,459 lines total (25 Go files, 51 tests)
├── main.go                               7 lines
├── go.mod                               24 lines (+ go.sum)
├── cmd/
│   ├── root.go                         170 lines  — CLI entry, DB discovery, ResolveNode
│   ├── analyze.go                      230 lines  — analyze command + terminal formatting
│   └── context.go                      143 lines  — context-for-task command
└── internal/
    ├── db/
    │   ├── db.go                         46 lines  — SQLite connection wrapper
    │   ├── models.go                     46 lines  — Node, Edge structs
    │   ├── nodes.go                      81 lines  — AllNodes, GetNode, SearchByIDPrefix
    │   ├── edges.go                     162 lines  — AllEdges, GetEdgesForNode, EdgesForContext
    │   ├── embeddings.go                 70 lines  — BLOB → []float32 reads
    │   ├── embeddings_test.go            74 lines  — 4 tests
    │   ├── search.go                     73 lines  — FTS5 search + query preprocessing
    │   ├── search_test.go                49 lines  — 6 tests
    │   ├── context.go                   289 lines  — Dijkstra context expansion
    │   └── context_test.go              592 lines  — 17 tests + helpers
    └── graph/
        ├── unionfind.go                  78 lines  — Union-Find (path compression + rank)
        ├── snapshot.go                  162 lines  — GraphSnapshot, adjacency, regions
        ├── snapshot_db.go                53 lines  — DB → GraphSnapshot bridge
        ├── topology.go                  149 lines  — components, orphans, hubs, degree histogram
        ├── bridges.go                   227 lines  — iterative Tarjan's, fragile connections
        ├── staleness.go                 102 lines  — stale nodes, stale summaries
        ├── health.go                     85 lines  — health score + Analyze assembler
        ├── similarity.go                 67 lines  — cosine similarity, FindSimilar
        ├── similarity_test.go           107 lines  — 9 tests
        └── graph_test.go               397 lines  — 15 tests (topology, Tarjan, staleness, health)
```

**Phase 1**: 1,751 lines, 15 files, 15 tests — graph analyzer
**Phase 2 additions**: +1,708 lines, +10 files, +36 tests — context compilation pipeline
**Total**: 3,459 lines, 25 files, 51 tests

## Dependencies

```
modernc.org/sqlite v1.46.1    — pure Go SQLite (no CGo, cross-compiles)
github.com/spf13/cobra v1.10.2 — CLI framework
```

No ORMs. No frameworks. No ML runtimes. Two dependencies.

---

## Package: `main`

**File**: `spore/main.go` (7 lines)

Entry point. Calls `cmd.Execute()`.

---

## Package: `cmd`

### `cmd/root.go` (170 lines)

Cobra root command, database discovery, and node resolution.

**`DiscoverDB()`** (lines 31-73): Finds `.mycelica.db` with this priority:

1. `MYCELICA_DB` environment variable
2. `--db` CLI flag
3. Walk up from CWD looking for `.mycelica.db` (matches `mycelica-cli` behavior)
4. XDG fallback: `~/.local/share/com.mycelica.app/mycelica.db`

**`OpenDatabase()`** (lines 76-82): Convenience — `DiscoverDB()` then `db.OpenDB()`.

**`ResolveNode(d, reference)`** (lines 86-150): Three-step resolution. Port of `team.rs:110-145`.

1. **Exact ID**: `d.GetNode(reference)` — returns on hit
2. **Prefix match** (if ≥6 hex/dash chars): `d.SearchByIDPrefix(reference, 10)`
   - 1 match → return it
   - 0 → fall through to FTS
   - \>1 → error listing all ambiguous matches
3. **FTS search**: `d.SearchNodes(reference)`, filtered to `is_item == true`
   - 1 match → return it
   - 0 → "node not found"
   - \>1 → error listing up to 10 matches

**`isHexDash(s)`** (lines 152-159): Returns true if string contains only `[0-9a-fA-F-]`. Used to distinguish ID-like references from search terms.

### `cmd/analyze.go` (230 lines)

The `spore analyze` subcommand. (Unchanged from Phase 1.)

**Flags**:
| Flag | Type | Default | Purpose |
|------|------|---------|---------|
| `--json` | bool | false | JSON output instead of terminal |
| `--region` | string | "" | Scope to descendants of this node ID |
| `--top-n` | int | 10 | Max items per section |
| `--stale-days` | int64 | 60 | Days threshold for staleness |
| `--hub-threshold` | int | 15 | Minimum degree to count as hub |

**RunE flow**:
1. `OpenDatabase()` → get DB connection
2. `graph.SnapshotFromDB(db)` → load full graph
3. Optionally `snap.FilterToRegion(regionID)` → scope down
4. `graph.Analyze(snap, config)` → run all analyses
5. JSON output via `json.NewEncoder` with 2-space indent, or `printHumanReadable()`

**`printHumanReadable()`** (lines 69-211): Renders analysis report with:
- Health bar: `█████████░░░░░░░░░░░` with percentage and sub-score breakdown
- TOPOLOGY: node/edge/component counts, orphan list, degree histogram (log-scale bars), hub list
- STALENESS: stale nodes with age/ref count, stale summaries with drift days
- STRUCTURAL FRAGILITY: articulation points, bridge edges, fragile inter-region connections

### `cmd/context.go` (143 lines) **[NEW in Phase 2]**

The `spore context-for-task <id>` subcommand.

**Flags**:
| Flag | Type | Default | Purpose |
|------|------|---------|---------|
| `--budget` | int | 20 | Max nodes to return |
| `--max-hops` | int | 6 | Max graph traversal depth |
| `--max-cost` | float64 | 3.0 | Dijkstra cost ceiling |
| `--edge-types` | string | "" | Comma-separated edge type allowlist |
| `--not-superseded` | bool | false | Filter out superseded edges |
| `--items-only` | bool | false | Skip categories from results |
| `--json` | bool | false | JSON output |

**RunE flow** (lines 27-85):
1. `OpenDatabase()` → DB connection
2. `ResolveNode(d, args[0])` → find source node by ID/prefix/title
3. Build `db.ContextConfig` from flags
4. Parse `--edge-types` if set (split on `,`, trim spaces)
5. `d.ContextForTask(source.ID, config)` → Dijkstra expansion
6. JSON or human-readable output

**JSON output** (lines 59-81):
```json
{
  "source": {"id": "...", "title": "..."},
  "budget": 20,
  "results": [
    {
      "rank": 1,
      "nodeId": "...",
      "nodeTitle": "...",
      "distance": 0.085,
      "relevance": 0.921,
      "hops": 1,
      "path": [{"edgeId":"...","edgeType":"...","nodeId":"...","nodeTitle":"..."}],
      "nodeClass": null,
      "isItem": true
    }
  ],
  "count": 15
}
```

**`printContextHumanReadable(source, results)`** (lines 99-143):
```
Context for: <title> (<id8>)  budget=20

   1. [I] <title> [class] — dist=0.123 rel=89% hops=2
      →[edge_type]→ <node1> →[edge_type]→ <node2>

15 node(s) within budget
```

- `[I]` = item, `[C]` = category
- Path titles truncated to 40 chars
- Uses `ai_title` when available, falls back to `title`

---

## Package: `internal/db`

The database layer. Direct SQL queries mapped to Go structs. No ORM.

### `db/db.go` (46 lines)

SQLite connection wrapper.

```go
type DB struct {
    conn *sql.DB
    Path string
}
```

**`OpenDB(path)`**: Opens SQLite with:
- `PRAGMA journal_mode=WAL` — concurrent reads
- `PRAGMA foreign_keys=ON` — referential integrity

Uses `modernc.org/sqlite` driver (pure Go, no CGo). Connection pooling via `database/sql`.

**`Conn()`**: Exposes underlying `*sql.DB` for custom queries.

### `db/models.go` (46 lines)

Go structs mapping to SQLite schema. All nullable columns use `*T` pointer types.

**`Node`** (23 fields):
```go
type Node struct {
    ID          string   // e.g. "code-8d8f369fd052a9bc"
    NodeType    string   // "page", "thought", "context", "cluster", "paper", "bookmark"
    Title       string
    URL         *string
    Content     *string
    CreatedAt   int64    // Unix milliseconds
    UpdatedAt   int64
    Depth       int
    IsItem      bool
    IsUniverse  bool
    ParentID    *string
    ChildCount  int
    AITitle     *string
    Summary     *string
    Tags        *string  // JSON string
    Emoji       *string
    IsProcessed bool
    AgentID     *string
    NodeClass   *string  // "knowledge", "meta", "operational"
    MetaType    *string  // "summary", "contradiction", "status"
    ContentType *string
    Source      *string
    Author      *string
}
```

**`Edge`** (14 fields):
```go
type Edge struct {
    ID           string
    SourceID     string
    TargetID     string
    EdgeType     string   // lowercase: "calls", "summarizes", "defined_in", etc.
    Label        *string
    Weight       *float64
    Confidence   *float64
    AgentID      *string
    Reason       *string
    Content      *string
    CreatedAt    int64    // Unix milliseconds
    UpdatedAt    *int64
    SupersededBy *string
    Metadata     *string  // JSON string
}
```

### `db/nodes.go` (81 lines)

**`scanNode(scanner)`** (lines 4-14): **[NEW in Phase 2]** Shared scan helper for all 23 Node columns. Accepts the `Scan(...any)` interface so it works with both `*sql.Row` and `*sql.Rows`. Eliminates the duplicated 23-positional-arg Scan across `AllNodes`, `GetNode`, `SearchNodes`, and `SearchByIDPrefix`.

**`AllNodes()`** (lines 17-39): `SELECT` all 23 columns from `nodes`, ordered by `created_at DESC`.

**`GetNode(id)`** (lines 42-56): Same SELECT with `WHERE id = ?`. Returns `*Node` or error.

**`SearchByIDPrefix(prefix, limit)`** (lines 59-81): **[NEW in Phase 2]** `WHERE id LIKE ?` with `prefix + "%"`. Returns up to `limit` matches. Used by `ResolveNode()` for prefix-based node lookup.

### `db/edges.go` (162 lines)

**`scanEdge(scanner)`** (lines 6-14): **[NEW in Phase 2]** Shared scan helper for all 14 Edge columns. Same `Scan(...any)` interface pattern as `scanNode`.

**`AllEdges()`** (lines 17-38): All edges, no filtering. Used by the graph analyzer.

**`GetEdgesForNode(nodeID)`** (lines 41-62): **[NEW in Phase 2]** `WHERE source_id = ? OR target_id = ?`. Returns all edges touching a node (both directions). Used by Dijkstra during traversal — called once per visited node.

**`EdgeTypePriority(edgeType)`** (lines 67-78): **[NEW in Phase 2]** Returns the traversal priority for an edge type. Higher priority = lower cost in Dijkstra. Shared by `EdgesForContext` and `ContextForTask`.

| Priority | Edge Types |
|----------|-----------|
| 1.0 | contradicts, flags |
| 0.7 | derives_from, summarizes, resolves, supersedes |
| 0.5 | supports, questions, prerequisite, evolved_from |
| 0.3 | everything else (default) |

Matches `schema.rs:5665-5672`.

**`IsStructuralEdge(edgeType)`** (lines 82-89): **[NEW in Phase 2]** Returns true for `defined_in`, `belongs_to`, `sibling`. These represent file-tree/hierarchy relationships rather than semantic ones. The Dijkstra algorithm applies a cost floor (0.4) to these edges to prevent file-tree crawling.

**`EdgesForContext(nodeID, topN, notSuperseded)`** (lines 94-162): **[NEW in Phase 2]** Returns the top-N most relevant edges for a node, scored by a weighted formula. Port of `schema.rs:5674-5709`.

Score formula per edge:
```
recency = (edge.createdAt - oldest) / timeRange    // 0.0–1.0
confidence = edge.confidence ?? 0.5
typePriority = EdgeTypePriority(edge.type)

score = 0.3*recency + 0.3*confidence + 0.4*typePriority
```

Implementation details:
- Computes time range across all edges for recency normalization
- Filters superseded edges if `notSuperseded` is true (in-place slice filter, no allocation)
- Sorts descending by score, truncates to `topN`

### `db/embeddings.go` (70 lines) **[NEW in Phase 2]**

Reads embedding BLOB data from the `nodes.embedding` column. Embeddings are 384-dimensional float32 vectors (all-MiniLM-L6-v2 model), stored as 1,536-byte little-endian BLOBs.

**`NodeEmbedding`** struct:
```go
type NodeEmbedding struct {
    ID        string
    Embedding []float32
}
```

**`bytesToEmbedding(data)`** (lines 16-27): Converts a little-endian byte slice to `[]float32`. Each 4 bytes → one `math.Float32frombits(binary.LittleEndian.Uint32(chunk))`. Short trailing chunk (< 4 bytes) → `0.0`. Matches Rust `bytes_to_embedding` at `schema.rs:6331-6341`.

**`GetNodeEmbedding(id)`** (lines 30-40): Returns the embedding for a single node. Returns `nil, nil` if embedding is NULL. Single-row query: `SELECT embedding FROM nodes WHERE id = ?`.

**`GetNodesWithEmbeddings()`** (lines 43-63): Returns all `(id, embedding)` pairs for nodes with non-NULL embeddings. Used by `FindSimilar` for brute-force nearest-neighbor search.

**`CountNodesWithEmbeddings()`** (lines 66-70): `SELECT COUNT(*)` for diagnostics.

### `db/search.go` (73 lines) **[NEW in Phase 2]**

FTS5 full-text search with query preprocessing. Port of `schema.rs:2312-2330` (search) and `spore.rs:2368-2379` (preprocessing).

**`stopwords`** (lines 8-13): Set of 20 English stopwords: `the, a, an, in, on, at, to, for, of, is, it, and, or, with, from, by, this, that, as, be`.

**`BuildFTSQuery(query)`** (lines 18-35): Preprocesses natural language into FTS5 MATCH syntax:
1. Split on whitespace → `strings.Fields()`
2. Trim non-letter/digit/underscore from both ends (preserves `_` for identifiers like `generate_task_file`)
3. Filter: skip words < 3 chars AND skip stopwords (case-insensitive check)
4. Join remaining words with `" OR "`

Examples:
- `"Add the flag to a function for parsing"` → `"Add OR flag OR function OR parsing"`
- `"go do run fast"` → `"run OR fast"` (go, do too short)
- `"the a an in on at"` → `""` (all stopwords)

**`SearchNodes(query)`** (lines 39-73): FTS5 search returning full `Node` structs.
- Returns empty slice if preprocessed query is empty
- Joins `nodes` with `nodes_fts` on `rowid`
- Uses `ORDER BY rank` (FTS5 built-in BM25 ranking)
- Gracefully handles missing FTS table: if error contains `"no such table"`, returns empty slice instead of error. This prevents crashes on databases that haven't been indexed.

### `db/context.go` (289 lines) **[NEW in Phase 2]**

The core Dijkstra context expansion algorithm. Port of `schema.rs:5714-5867`.

**Types**:

```go
type ContextNode struct {
    Rank      int       `json:"rank"`
    NodeID    string    `json:"nodeId"`
    NodeTitle string    `json:"nodeTitle"`
    Distance  float64   `json:"distance"`
    Relevance float64   `json:"relevance"`    // 1.0 / (1.0 + distance)
    Hops      int       `json:"hops"`
    Path      []PathHop `json:"path"`
    NodeClass *string   `json:"nodeClass"`
    IsItem    bool      `json:"isItem"`
}

type PathHop struct {
    EdgeID    string `json:"edgeId"`
    EdgeType  string `json:"edgeType"`
    NodeID    string `json:"nodeId"`
    NodeTitle string `json:"nodeTitle"`
}

type ContextConfig struct {
    Budget           int
    MaxHops          int       // default 6
    MaxCost          float64   // default 3.0
    EdgeTypes        []string  // allowlist (nil = all)
    ExcludeEdgeTypes []string  // blocklist
    NotSuperseded    bool
    ItemsOnly        bool
}
```

**`DefaultContextConfig()`** (lines 41-47): `Budget=20, MaxHops=6, MaxCost=3.0`.

**Priority queue** (lines 57-82): `container/heap` min-heap with deterministic tie-breaking:

```go
type dijkstraHeap []dijkstraEntry

func (h dijkstraHeap) Less(i, j int) bool {
    if h[i].distance != h[j].distance {
        return h[i].distance < h[j].distance
    }
    return h[i].nodeID < h[j].nodeID  // tie-break: lexicographic
}
```

The Rust implementation does NOT break ties (`schema.rs:5747-5751` only compares distance). Go needs explicit tie-breaking because `container/heap` doesn't guarantee stable ordering and Go map iteration is non-deterministic. Cross-validation compares sets at equal distances, not exact ordering.

**`ContextForTask(sourceID, config)`** (lines 87-252): The main algorithm.

1. **Setup** (lines 88-123): Validate config defaults, build allow/exclude sets for O(1) edge type lookup
2. **Initialize** (lines 120-126): `dist[source] = 0.0`, push source onto heap
3. **Main loop** (lines 129-243):
   - Pop minimum-distance entry from heap
   - Skip if already visited (stale heap entry)
   - **Collect** (lines 141-166): If not source, look up node via `GetNode()`, apply `ItemsOnly` filter. Append to results. Break if budget reached.
   - **Expand** (lines 169-243): Stop if at max hops. Get edges via `GetEdgesForNode()`. For each edge:
     - Filter: superseded, edge type allow/exclude lists
     - Bidirectional: determine neighbor as the "other end" of the edge
     - Skip if visited
     - **Compute cost** (lines 207-218):
       ```
       confidence = edge.Confidence ?? 0.5
       typePriority = EdgeTypePriority(edge.EdgeType)
       baseCost = max((1 - confidence) * (1 - 0.5 * typePriority), 0.001)

       if IsStructuralEdge(edge.EdgeType):
           baseCost = max(baseCost, 0.4)    // THE STRUCTURAL PENALTY
       ```
     - Skip if `currentDist + baseCost > maxCost`
     - Relax: if new distance is shorter, update `dist`, `prev`, push to heap
4. **Assign ranks** (lines 247-249): 1-indexed, in distance order (already sorted by heap extraction)

**The structural penalty** (lines 217-219) is the most critical detail in the algorithm. Without the `max(baseCost, 0.4)` floor on `defined_in`, `belongs_to`, and `sibling` edges, Dijkstra would crawl the entire file tree through `defined_in` edges (which typically have high confidence → low cost). The floor ensures that semantic edges (documents, supports, etc.) rank far above structural ones even when the structural edges are fewer hops away.

Cross-validation confirmed: for node `code-8d8f369fd052a9bc` (get_node):
- `documents` edges (semantic): distance 0.085/hop → rank 1
- `defined_in` edge (structural): distance 0.400 → rank 254
- `calls` edges (structural): distance 0.425 → rank 257+

**`reconstructPath(prev, source, target)`** (lines 256-289): Walks backward through the `prev` map from target to source, resolving node titles via `GetNode()` at each hop. Uses `ai_title` if available, falls back to `title`, then first 8 chars of ID. Reverses the path to source-to-target order.

---

## Package: `internal/graph`

Pure computation. No database dependency (except `snapshot_db.go` bridge).

### `graph/unionfind.go` (78 lines)

Union-Find (disjoint set) with path compression and union by rank.

```go
type UnionFind struct {
    parent map[string]string
    rank   map[string]int
    size   map[string]int
}
```

- `NewUnionFind(ids)`: Each element starts as its own component
- `Find(id)`: Returns root with path compression (O(alpha(n)) amortized)
- `Union(a, b)`: Merges by rank, returns true if they were separate
- `Components()`: Groups all elements by root

Uses string-keyed maps (not integer indices) so it works directly with node UUIDs.

### `graph/snapshot.go` (162 lines)

Core data structures for graph analysis.

**`NodeInfo`** — lightweight node (8 fields, subset of `db.Node`):
```go
type NodeInfo struct {
    ID, Title, NodeType string
    CreatedAt, UpdatedAt int64
    ParentID *string
    Depth int
    IsItem bool
}
```

**`GraphSnapshot`** — the main analysis structure:
```go
type GraphSnapshot struct {
    Nodes   map[string]*NodeInfo      // ID → node
    Edges   []EdgeInfo                // all edges
    Adj     map[string][]string       // undirected adjacency
    OutAdj  map[string][]string       // directed: source → targets
    InAdj   map[string][]string       // directed: target → sources
    Regions map[string]string         // node_id → depth-1 ancestor ID
}
```

- `NewSnapshot(nodes, edges)`: Builds all three adjacency maps, computes region map
- `FilterToRegion(regionNodeID)`: Returns new snapshot containing only descendants
- `NodeIDs()`: Sorted slice of all node IDs for deterministic iteration

### `graph/snapshot_db.go` (53 lines)

Bridge between `internal/db` and `internal/graph`. Converts `db.Node` → `*NodeInfo` and `db.Edge` → `EdgeInfo`. Careful with pointer copying (allocates new values to avoid aliasing).

### `graph/topology.go` (149 lines)

Connected components, orphan detection, degree distribution, hub identification.

**`ComputeTopology(snap, hubThreshold, topN)`**:
1. Components via UnionFind over all edges
2. Orphans: `degree == 0`, sorted alphabetically, truncated to `topN`
3. Degree histogram: log-scale buckets (0, 1, 2-3, 4-7, 8-15, 16-31, 32+)
4. Hubs: `degree > hubThreshold`, sorted descending

### `graph/bridges.go` (227 lines)

Iterative Tarjan's algorithm.

**`ComputeBridges(snap)`**:
1. Index mapping: string IDs → integer indices
2. Deduplicated undirected adjacency via `edgePair` with canonical ordering
3. Iterative DFS with explicit `[]frame` stack (avoids recursion stack overflow):
   - Bridge: `low[child] > disc[parent]`
   - AP (non-root): `low[child] >= disc[parent]`
   - AP (root): 2+ tree children
4. Fragile connections: cross-region edge counts, report pairs with ≤ 2

### `graph/staleness.go` (102 lines)

**`ComputeStaleness(snap, staleDays)`**:
1. Stale nodes: `updatedAt > staleDays ago` AND has recent (≤ 7 days) incoming edges
2. Stale summaries: `"summarizes"` edges where `target.UpdatedAt > source.UpdatedAt`

### `graph/health.go` (85 lines)

**`Analyze(snap, config)`**: Runs topology + staleness + bridges, computes composite health score.

```
health = 0.30 * connectivity + 0.25 * components + 0.25 * staleness + 0.20 * fragility
```

Sub-scores (each 0.0–1.0):
- Connectivity = `clamp(1 - min(orphans/total, 0.2) * 5, 0, 1)`
- Components = `clamp(1/numComponents, 0, 1)`
- Staleness = `clamp(1 - min(staleCount/total, 0.1) * 10, 0, 1)`
- Fragility = `clamp(1 - min(apCount/total, 0.05) * 20, 0, 1)`

**`DefaultConfig()`**: `HubThreshold=15, TopN=10, StaleDays=60` — aligned with CLI flag defaults.

### `graph/similarity.go` (67 lines) **[NEW in Phase 2]**

Pure math, no DB dependency.

**`CosineSimilarity(a, b []float32) float32`** (lines 19-39): Standard cosine similarity. Returns 0.0 for zero-norm vectors or mismatched lengths. Computes `dot / (normA * normB)` in a single pass. Port of `similarity.rs:7-21`.

Implementation note: accumulates in `float32` but uses `math.Sqrt(float64(...))` for norms, matching standard practice for numerical stability.

**`FindSimilar(target, candidates, excludeID, topN, minSimilarity)`** (lines 44-67): Linear scan over all candidates, compute cosine similarity, filter by minimum threshold, sort descending, truncate to topN. Port of `similarity.rs:65-88`.

- Excludes `excludeID` (typically the source node itself)
- O(n) scan — acceptable because embedding search is not in the hot path of Dijkstra

---

## Tests

### `internal/db/embeddings_test.go` (74 lines, 4 tests)

| Test | What it validates |
|------|-------------------|
| `TestBytesToEmbedding_KnownValues` | LE bytes for 1.0 and -0.5 round-trip correctly |
| `TestBytesToEmbedding_Empty` | nil and empty slice → empty result |
| `TestBytesToEmbedding_ShortChunk` | 5 bytes → 2 floats, trailing chunk → 0.0 |
| `TestBytesToEmbedding_384Dim` | Full 1536-byte embedding (384 dims), spot-checked at indices 0 and 100 |

### `internal/db/search_test.go` (49 lines, 6 tests)

| Test | What it validates |
|------|-------------------|
| `TestBuildFTSQuery_StopwordRemoval` | "Add the flag to a function for parsing" → "Add OR flag OR function OR parsing" |
| `TestBuildFTSQuery_ShortWords` | "go do run fast" → "run OR fast" (go, do filtered) |
| `TestBuildFTSQuery_PunctuationTrimming` | Preserves underscores: `generate_task_file()` → `generate_task_file` |
| `TestBuildFTSQuery_AllStopwords` | All stopwords → empty string |
| `TestBuildFTSQuery_MixedCase` | Case-insensitive: "The AND From THIS function" → "function" |
| `TestBuildFTSQuery_Empty` | Empty input → empty output |

### `internal/db/context_test.go` (592 lines, 17 tests)

Test infrastructure: `setupTestDB()` creates in-memory SQLite with full schema (nodes + edges tables). `insertNode()` and `insertEdge()` helpers for concise test setup. `f64()` helper returns `*float64`.

| Test | What it validates |
|------|-------------------|
| `TestEdgeTypePriority` | All priority tiers: contradicts=1.0, summarizes=0.7, supports=0.5, related=0.3 |
| `TestDijkstra_SimpleChain` | A→B→C: B closer than C, correct ranks, correct path length |
| `TestDijkstra_BudgetCutoff` | Star graph with 10 spokes, budget=3 → exactly 3 results |
| `TestDijkstra_MaxHopsCutoff` | 5-node chain, maxHops=2 → only 2 reachable |
| `TestDijkstra_MaxCostCutoff` | Low confidence edges (0.1), maxCost=1.0 → B at 0.765 (in), C at 1.53 (out) |
| `TestDijkstra_SupersededFilter` | Edge with superseded_by set, NotSuperseded=true → 0 results |
| `TestDijkstra_ItemsOnlyFilter` | Category (is_item=false) traversed but not in results |
| `TestDijkstra_DeterministicTieBreaking` | Star with X,Y,Z at same distance → always X,Y,Z order (3 runs) |
| `TestDijkstra_StructuralPenalty` | **defined_in (0.4) vs supports (0.075)**: verifies exact computed distances |
| `TestDijkstra_BidirectionalTraversal` | Edge A→B, start at B → reaches A |
| `TestDijkstra_PathReconstruction` | A→B→C path: correct edge types and node IDs at each hop |
| `TestDijkstra_EdgeTypeAllowlist` | Only "supports" allowed → "related" edges invisible |
| `TestDijkstra_EdgeTypeExclude` | "supports" excluded → only "related" edges visible |
| `TestDijkstra_EmptyGraph` | Isolated node → 0 results |
| `TestDijkstra_RelevanceCalculation` | `relevance = 1 / (1 + distance)` matches within 0.0001 |
| `TestDijkstra_NilConfig` | nil config → uses DefaultContextConfig |
| `TestDijkstra_ShortestPathWins` | Direct low-confidence edge vs indirect high-confidence: takes shorter path |

The structural penalty test (`TestDijkstra_StructuralPenalty`) is the most important:
```
supports (confidence=0.9): cost = (1-0.9) * (1-0.5*0.5) = 0.1 * 0.75 = 0.075
defined_in (confidence=0.9): cost = max((1-0.9)*(1-0.5*0.3), 0.4) = max(0.085, 0.4) = 0.4
```
The floor makes the structural edge 5.3x more expensive despite identical confidence.

### `internal/graph/similarity_test.go` (107 lines, 9 tests)

| Test | What it validates |
|------|-------------------|
| `TestCosineSimilarity_Identical` | [1,2,3] vs [1,2,3] → 1.0 |
| `TestCosineSimilarity_Orthogonal` | [1,0,0] vs [0,1,0] → 0.0 |
| `TestCosineSimilarity_Opposite` | [1,0] vs [-1,0] → -1.0 |
| `TestCosineSimilarity_ZeroNorm` | [0,0,0] → 0.0 |
| `TestCosineSimilarity_MismatchedLength` | [1,0] vs [1,0,0] → 0.0 |
| `TestCosineSimilarity_Empty` | nil vs nil → 0.0 |
| `TestFindSimilar_Basic` | Correct ordering: perfect match > near match > orthogonal |
| `TestFindSimilar_ExcludesSelf` | Self-exclusion works |
| `TestFindSimilar_MinThreshold` | Below-threshold candidates filtered out |

### `internal/graph/graph_test.go` (397 lines, 15 tests)

(Unchanged from Phase 1 — topology, Tarjan's, staleness, health.)

| Test | What it validates |
|------|-------------------|
| `TestTopology_EmptyGraph` | Empty graph → all zeros |
| `TestTopology_SingleComponent` | 5-node chain → 1 component, 0 orphans |
| `TestTopology_TwoComponents` | 3+2 split → 2 components |
| `TestOrphan_Detection` | Disconnected node detected |
| `TestHub_Detection` | Star center detected as hub |
| `TestTarjan_Bridge` | Chain → bridges and AP detected |
| `TestTarjan_CycleNoBridges` | Triangle → no bridges, no APs |
| `TestTarjan_TwoCyclesJoined` | Two triangles joined → bridge and APs |
| `TestStaleness_Detected` | Old node with recent refs → stale |
| `TestStaleness_NoFalsePositive` | Old node with old refs → not stale |
| `TestStale_Summary` | Target updated after summary → stale summary |
| `TestRegion_Computation` | Hierarchy depth → correct regions |
| `TestFragile_Connections` | Single cross-region edge → fragile |
| `TestHealthScore_Range` | Health always in [0, 1] |
| `TestHealthScore_Perfect` | Triangle → health ≥ 0.95 |

---

## Cross-Validation Results

### Phase 1: `spore analyze`

Run against live database (2,253 nodes, 4,215 edges):

```
Go:   health_score=0.5186746987951807  components=297  orphans=43  APs=313  bridges=976
Rust: health_score=0.5186746987951807  components=297  orphans=43  APs=313  bridges=976
```

All counts identical. Health score matches to 16 decimal places.

### Phase 2: `spore context-for-task`

**Test 1**: `code-bcd35704815bd055` (run_cli), budget=10

| Metric | Value |
|--------|-------|
| Node ID overlap | 8/10 (80%) |
| Distance mismatches | 0 across shared nodes |
| Tier 1 (dist 0.085) | Identical: 2 nodes match |
| Tier 2 (dist 0.170) | 6/8 match — 2 differ at budget boundary (same distance) |

The 2 differing nodes are both at distance 0.170 (equal-cost tier). The budget cutoff picks different nodes from the same-distance pool due to tie-breaking differences. Expected.

**Test 2**: `code-8d8f369fd052a9bc` (get_node), budget=500 — **structural penalty verification**

| Metric | Value |
|--------|-------|
| Total nodes per impl | 500 |
| Nodes in both | **490 (98%)** |
| Distance mismatches (>1e-6) | **0** across 490 shared nodes |
| Differing nodes | 10 — all at boundary distance 0.7650 |

**Structural penalty validated**:

| Edge type path | Example | Rank | Distance |
|---------------|---------|------|----------|
| `documents` (1 hop, semantic) | Tauri Commands Reference | **1** | **0.085** |
| `documents` x2 (2 hops) | get_nodes, create_node, ... | 2-253 | **0.170** |
| `defined_in` (1 hop, structural) | graph.rs | **254** | **0.400** |
| `calls` (1 hop, structural) | merge_sessions, ... | 257-295 | **0.425** |

Semantic edges rank far above structural edges despite being more hops away. The cost floor works.

---

## Architecture Notes

### What Go reads vs. writes

| Operation | Method |
|-----------|--------|
| Read nodes/edges | Direct SQLite queries (`internal/db`) |
| Read embeddings | Direct BLOB read (`embeddings.go`) |
| FTS search | Direct FTS5 MATCH query (`search.go`) |
| Context expansion | Dijkstra with per-node edge queries (`context.go`) |
| Write nodes/edges | Not yet — future phases shell out to `mycelica-cli` |

### Why context.go is in `internal/db`, not `internal/graph`

The analyzer loads the entire graph into memory as a `GraphSnapshot`, then computes on it purely. The Dijkstra expansion is different: it queries the database *during* traversal (`GetEdgesForNode()` per visited node, `GetNode()` per collected result, `GetNode()` per path hop for title resolution). This is an online algorithm, not a batch one. Putting it in the `graph` package would either require passing the DB through or pre-loading all edges (defeating the purpose of bounded expansion).

### Deterministic output

Go maps iterate in random order. Every function that produces ordered output uses either:
- `snap.NodeIDs()` (sorted) for iteration
- `sort.Slice()` for result ordering
- Heap tie-breaking by node ID for Dijkstra

The Rust Dijkstra does NOT break ties — when multiple nodes have the same distance, the order depends on `BinaryHeap` internal state (which is deterministic in Rust because Rust's heap is deterministic for the same insertion order). Go's `container/heap` is also deterministic for the same insertion order, but Go map iteration is not, which can affect edge discovery order. The lexicographic tie-breaking in the Go port ensures reproducibility regardless of map iteration order.

### Error propagation

All DB queries return `(result, error)`. If `rows.Scan()` fails mid-iteration, functions return `(nil, err)` immediately. The Dijkstra loop is more lenient: if `GetEdgesForNode()` fails for one node, it `continue`s (skips that node's expansion) rather than aborting. This is appropriate because the algorithm is best-effort — missing a few edges degrades quality but doesn't invalidate the result.

`SearchNodes()` has explicit graceful degradation: if the FTS5 table doesn't exist, it returns an empty slice instead of an error. This prevents crashes on unindexed databases.

### Performance characteristics

- **Dijkstra traversal**: O(budget * E_avg * log(budget)) where E_avg is average edges per node
- **Per-node DB queries**: `GetEdgesForNode()` is an indexed lookup (`WHERE source_id = ? OR target_id = ?`). On SQLite this hits the primary key index, ~0.1ms per call.
- **Path reconstruction**: O(hops) per result, with one `GetNode()` per hop. Worst case for budget=20, max_hops=6: ~120 queries. Each is a PK lookup — negligible on SQLite.
- **Embedding reads**: `bytesToEmbedding` is a tight loop with `binary.LittleEndian.Uint32`. No allocations beyond the result slice.
- **FTS5 search**: Single query, BM25 ranked. SQLite's FTS5 is fast for moderate-size corpora (~2K nodes).
