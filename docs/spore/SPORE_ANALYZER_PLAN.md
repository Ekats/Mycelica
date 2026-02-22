# Spore Analyzer — Replacing the Guide with Computation

## The Problem

The "guide" agent was supposed to maintain big-picture awareness of the system — watching what other agents do, catching architectural drift, connecting dots across the codebase. It doesn't work because it's an LLM told to "think about the big picture," which degenerates into the same looping/skimming behavior as every other agent. After 200+ runs and $324 in costs, Spore's core hypothesis (graph-compiled context improves agent effectiveness) remains unvalidated partly because the thing responsible for *seeing* the big picture can't actually see it.

The guide failed because it's the wrong tool for the job. Big-picture structural awareness is a **computation problem**, not a **language problem**. You don't need an LLM to detect that a subgraph is disconnected, that bridge nodes are fragile, or that 47 nodes were created yesterday with zero verification edges. You need graph algorithms.

## Current Approach vs Proposed

### Current: Guide Agent (prompt-driven)

```
┌─────────────────────────────────────────┐
│         GUIDE (CLAUDE.md prompt)        │
│                                         │
│  "Read the graph. Think about the big   │
│   picture. Find issues. Suggest fixes." │
│                                         │
│  Tools: spore query-edges, explain-edge │
│         path-between, node get          │
│                                         │
│  Failure modes:                         │
│  - Skims instead of reading deeply      │
│  - Loops on the same observations       │
│  - Creates confident-sounding summaries │
│    that miss structural problems        │
│  - Can't hold 2,200 nodes in context    │
│  - Burns tokens exploring randomly      │
│  - No termination condition             │
└─────────────────────────────────────────┘
```

The guide reads a few nodes, writes a summary, maybe creates an edge, and declares victory. It has no way to systematically scan the entire graph because LLMs process information sequentially and lose earlier context as they go. Asking it to "find all disconnected subgraphs" is asking it to hold the entire adjacency matrix in working memory. It can't.

### Proposed: Spore Analyzer (programmatic)

```
┌─────────────────────────────────────────┐
│    LAYER 1: GRAPH ANALYZER (Rust)       │
│    mycelica-cli spore analyze           │
│                                         │
│    Pure computation. No LLM.            │
│    Runs in <1s on 2,200 nodes.          │
│    Deterministic. Reproducible.         │
│                                         │
│    Outputs: StructuralReport (JSON)     │
│    - topology metrics                   │
│    - anomaly list                       │
│    - staleness report                   │
│    - bridge fragility scores            │
│    - coverage gaps                      │
│    - health score                       │
└──────────────────┬──────────────────────┘
                   │
                   │ structured data (not prose)
                   │
┌──────────────────▼──────────────────────┐
│    LAYER 2: INTERPRETATION (optional)   │
│                                         │
│    Human reads the report directly      │
│    OR                                   │
│    LLM reads pre-computed facts and     │
│    proposes architectural actions        │
│                                         │
│    The LLM never touches the graph.     │
│    It reasons about verified metrics.   │
│    No skimming. No looping. No random   │
│    exploration. Just "here are the      │
│    structural facts, what should we do?"│
└─────────────────────────────────────────┘
```

The key difference: Layer 1 does the hard part (scanning the entire graph systematically) and Layer 2 only does the easy part (interpreting a short structured report). The guide tried to do both simultaneously and failed at both.

## What Layer 1 Computes

### 1. Topology Metrics

These are standard graph theory computations. Your graph is in SQLite, so these operate on the full adjacency structure loaded once.

**Degree distribution:** How many edges does each node have? Nodes with 0-1 edges in active regions are orphans. Nodes with 50+ edges might be over-connected hubs that need decomposition.

**Connected components:** Are there disconnected subgraphs? If "spore orchestration" and "mycelica GUI" share zero edges, that's a structural gap — changes to one can break the other without the graph knowing.

**Clustering coefficient:** Within each region, how interconnected are the nodes? High coefficient = dense, well-understood area. Low coefficient = loosely related nodes dumped together by clustering without real semantic connections.

**Modularity:** How well does the current hierarchy grouping match the actual edge structure? If nodes in "Architecture Decisions" are more connected to nodes in "Schema Design" than to each other, the hierarchy is wrong.

```rust
pub struct TopologyMetrics {
    pub total_nodes: usize,
    pub total_edges: usize,
    pub connected_components: usize,
    pub largest_component_size: usize,
    pub orphan_nodes: Vec<NodeId>,       // degree 0-1
    pub hub_nodes: Vec<(NodeId, usize)>, // degree > threshold
    pub avg_degree: f64,
    pub clustering_coefficient: f64,
    pub modularity_score: f64,           // vs current hierarchy
}
```

### 2. Staleness Detection

Compare node `updated_at` timestamps against edge references. A node last modified 90 days ago that's referenced by a `summarizes` edge created yesterday is suspicious — the summary might be based on stale content. A doc node describing functions that no longer exist in the codebase (you already proved this works in the single-agent test) is definitely stale.

```rust
pub struct StalenessReport {
    pub stale_nodes: Vec<StaleNode>,
    pub stale_summaries: Vec<StaleSummary>,   // meta nodes referencing old data
    pub doc_code_mismatches: Vec<DocMismatch>, // docs describing deleted code
}

pub struct StaleNode {
    pub node_id: NodeId,
    pub title: String,
    pub last_modified: DateTime,
    pub days_stale: u64,
    pub referenced_by_recent_edges: usize, // edges created in last 7d
    pub severity: Severity,                 // based on how many things depend on it
}
```

### 3. Bridge & Fragility Analysis

**Betweenness centrality:** Which nodes sit on the most shortest paths between other nodes? These are structural bridges. If removing one node (or its edges) would split the graph into disconnected components, that's a single point of failure in your knowledge structure.

**Articulation points:** Nodes whose removal increases the number of connected components. In a codebase graph, an articulation point might be "schema.rs" — everything connects through it. That's fine for code (it IS central), but if it's an architecture decision node, it means your understanding of the system depends on a single document.

```rust
pub struct BridgeAnalysis {
    pub articulation_points: Vec<(NodeId, String)>,
    pub bridge_edges: Vec<(EdgeId, String)>,  // edges whose removal disconnects
    pub top_betweenness: Vec<(NodeId, String, f64)>, // top 10 by centrality
    pub fragile_connections: Vec<FragileConnection>,
}

pub struct FragileConnection {
    pub region_a: String,
    pub region_b: String,
    pub connecting_nodes: Vec<NodeId>,  // if only 1-2, that's fragile
    pub edge_count: usize,
}
```

### 4. Agent Activity Analysis

Since every edge has `agent_id` and `created_at`, you can compute agent behavior patterns without any LLM interpretation:

- How many nodes/edges did each agent create in the last N hours?
- What's the verification ratio? (edges created by `spore:coder` vs `spore:verifier` — if coder created 47 and verifier created 3, that's a red flag)
- Are agents writing to the same regions (potential conflicts) or isolated regions (no coordination)?
- Supersession rate: how often are edges being replaced? High rate = agents disagreeing. Zero rate = no quality control.

```rust
pub struct AgentActivity {
    pub agents: Vec<AgentStats>,
    pub conflict_regions: Vec<ConflictRegion>,  // regions with edges from 2+ agents
    pub unverified_count: usize,                 // coder edges with no verifier edge
    pub supersession_rate: f64,
}

pub struct AgentStats {
    pub agent_id: String,
    pub nodes_created_24h: usize,
    pub edges_created_24h: usize,
    pub avg_confidence: f64,
    pub regions_active: Vec<String>,
}
```

### 5. Coverage Gaps

Which knowledge regions have no summary meta-nodes? Which have summaries but no contradiction checks? Which have high node count but low edge density (lots of information dumped in but not connected)?

```rust
pub struct CoverageReport {
    pub unsummarized_regions: Vec<(NodeId, String, usize)>, // region, name, child count
    pub unchecked_regions: Vec<(NodeId, String)>,            // no contradiction edges
    pub sparse_regions: Vec<SparseRegion>,                   // high node, low edge
}

pub struct SparseRegion {
    pub region_id: NodeId,
    pub region_name: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub density: f64,  // edge_count / max_possible_edges
}
```

### 6. Cross-Region Connection Analysis (The "Dot Connecting")

This is the part you specifically asked about — finding meaningful connections that aren't just shortest-path. The programmatic version:

**Semantic similarity between regions:** You already have embeddings. Compute cosine similarity between region centroids (average embedding of all nodes in a region). If two regions have high semantic similarity but zero edges between them, they're probably related and nobody's connected them yet.

**Shared tag analysis:** Nodes with overlapping tags in different regions suggest cross-cutting concerns. Pure string matching, no LLM needed.

**Temporal co-occurrence:** Nodes created/modified in the same time window across different regions often represent work on the same problem from different angles. The graph knows this — it just hasn't been asked.

```rust
pub struct CrossRegionAnalysis {
    pub missing_connections: Vec<MissingConnection>,
    pub shared_tags: Vec<SharedTagCluster>,
    pub temporal_clusters: Vec<TemporalCluster>,
}

pub struct MissingConnection {
    pub region_a: (NodeId, String),
    pub region_b: (NodeId, String),
    pub similarity: f64,           // embedding cosine similarity
    pub shared_tags: Vec<String>,
    pub suggested_edge_type: String,
    pub confidence: f64,
}
```

This is the "connecting dots" part — but done through computation, not through an LLM guessing. The graph already contains the information; you just need algorithms that surface it.

## The Output

```bash
$ mycelica-cli spore analyze --json --since 7d

{
  "timestamp": "2026-02-21T14:30:00Z",
  "health_score": 0.72,
  "topology": {
    "total_nodes": 2243,
    "total_edges": 7612,
    "connected_components": 3,
    "orphan_nodes": 14,
    "hub_nodes": [
      {"id": "xxx", "title": "schema.rs architecture", "degree": 47}
    ]
  },
  "staleness": {
    "stale_nodes": 12,
    "critical": [
      {"title": "AI Clustering Spec", "days_stale": 94, "reason": "references deleted code"}
    ]
  },
  "bridges": {
    "articulation_points": 3,
    "fragile_connections": [
      {"a": "Spore Pipeline", "b": "MCP Server", "connecting_nodes": 1}
    ]
  },
  "agents": {
    "unverified_count": 47,
    "verification_ratio": 0.06,
    "conflict_regions": []
  },
  "coverage": {
    "unsummarized_regions": 8,
    "sparse_regions": [
      {"name": "Team Mode", "nodes": 34, "edges": 6, "density": 0.01}
    ]
  },
  "cross_region": {
    "missing_connections": [
      {
        "a": "Spore Orchestration",
        "b": "Trust Infrastructure",
        "similarity": 0.87,
        "shared_tags": ["agent-coordination", "verification", "attribution"],
        "suggestion": "These regions discuss the same problem from different angles"
      }
    ]
  }
}
```

Human-readable version (default, no `--json`):

```
═══ SPORE GRAPH ANALYSIS ═══
Health: 72/100  |  2,243 nodes  |  7,612 edges  |  3 components

⚠ STRUCTURAL ISSUES
  3 articulation points (single points of failure)
  14 orphan nodes (0-1 edges)
  1 fragile cross-region connection: Spore Pipeline ↔ MCP Server (1 bridge node)

⚠ STALENESS
  12 stale nodes (>60 days, still referenced)
  CRITICAL: "AI Clustering Spec" describes deleted code (94 days stale)

⚠ AGENT HEALTH
  Verification ratio: 6% (47 coder nodes, 3 verifier nodes)
  0 conflict regions (agents working in isolation — is that intentional?)

⚠ COVERAGE GAPS
  8 regions with no summary meta-nodes
  "Team Mode" region: 34 nodes, 6 edges (density 0.01 — disconnected dump)

💡 POTENTIAL CONNECTIONS
  "Spore Orchestration" ↔ "Trust Infrastructure"
    similarity: 0.87 | shared tags: agent-coordination, verification, attribution
    These regions aren't linked but discuss overlapping problems
```

## Implementation Plan

### Phase A: Core analyzer (~300-400 lines Rust)

Add to `src-tauri/src/commands/spore.rs` (or a new `analyzer.rs` module).

**Data loading:** Single query to load all nodes and edges into memory. At 2,200 nodes and 7,600 edges, this fits comfortably in RAM. Build an adjacency list representation.

```rust
struct GraphSnapshot {
    nodes: HashMap<NodeId, NodeInfo>,
    adj: HashMap<NodeId, Vec<(NodeId, EdgeInfo)>>,  // adjacency list
    edges: Vec<EdgeInfo>,
}

impl GraphSnapshot {
    fn from_db(db: &Database) -> Self { /* single load */ }
}
```

**Topology computation:** Connected components via iterative DFS (not recursive — stack overflow on large graphs). Degree distribution is a count over the adjacency list. Orphan detection is nodes with degree ≤ 1.

**Staleness:** SQL query joining nodes and edges on timestamps. No graph algorithm needed — it's a filter.

**Bridge detection:** Tarjan's bridge-finding algorithm (well-documented, O(V+E)). Articulation point detection is the same algorithm with minor modification.

**CLI command:**

```bash
mycelica-cli spore analyze [--json] [--since <duration>] [--region <node-id>]
```

### Phase B: Agent activity metrics (~100-150 lines)

Group edges by `agent_id` and compute counts, ratios, temporal patterns. This is aggregation, not graph theory. SQL GROUP BY with some post-processing.

### Phase C: Cross-region analysis (~200-300 lines)

This is the most complex part because it requires embeddings.

**Option 1 (simpler):** Use the existing embeddings in the database. Compute region centroids by averaging node embeddings within each top-level category. Cosine similarity between centroids. Tag overlap is string intersection.

**Option 2 (more accurate):** Use the existing `edges-for-context` ranking to find which nodes in region A are most similar to nodes in region B. This piggybacks on your existing composite scoring.

### Phase D: Report generation (~100 lines)

Serialize `StructuralReport` to JSON (serde) and/or format as human-readable terminal output.

### Total estimated effort

600-900 lines of Rust. The algorithms are standard graph theory — no novel research needed. Tarjan's algorithm, DFS for components, cosine similarity for embeddings, SQL aggregation for temporal patterns. You've written more complex things in less time (Phase 1 was 394 lines across 14 files, Phase 2 added 513 lines to schema.rs alone).

The hard part isn't the code — it's deciding which metrics actually matter. Start with topology + staleness (Phase A) and validate that the output tells you something you didn't already know. If it does, add the rest.

## How It Fits Into the Current Spore Architecture

```
Current agents:  coder, verifier, summarizer
                    │         │          │
                    │    MCP  │          │
                    ▼         ▼          ▼
              ┌─────────────────────────────┐
              │     KNOWLEDGE GRAPH         │
              │        (SQLite)             │
              └──────────┬──────────────────┘
                         │
              ┌──────────▼──────────────────┐
              │    ANALYZER (Rust, no LLM)   │
              │    mycelica-cli spore analyze│
              │                              │
              │    Reads graph. Computes.     │
              │    Outputs structured report. │
              │    Never writes to graph.*    │
              └──────────┬──────────────────┘
                         │
                    JSON report
                         │
              ┌──────────▼──────────────────┐
              │    YOU (or LLM Layer 2)      │
              │                              │
              │    Read report.              │
              │    Decide what to act on.    │
              │    Direct agents accordingly.│
              └─────────────────────────────┘
```

*Exception: the analyzer COULD write computed metrics as meta-nodes (e.g., a "Graph Health Report" node with the analysis as content). But it should never create relationship edges — that's the Synthesizer's job. The analyzer detects that a connection might be missing; a human or agent decides whether to create it.

## What This Replaces

| Guide Agent (current) | Analyzer (proposed) |
|---|---|
| LLM tries to scan entire graph | Algorithm scans entire graph in <1s |
| Skims, misses structural issues | Systematic, complete coverage |
| Loops on same observations | Deterministic, runs once |
| Burns $2-5 per run in tokens | Zero cost (local computation) |
| Output varies wildly between runs | Same input → same output |
| Can't detect disconnected components | Tarjan's algorithm, O(V+E) |
| Can't compute centrality metrics | Standard graph theory |
| "Connects dots" by guessing | Connects dots by cosine similarity |
| Needs a CLAUDE.md prompt | Needs a function signature |
| When wrong, debug the prompt | When wrong, debug the code |
| Has no termination condition | Terminates when computation completes |

## What This Does NOT Replace

The analyzer doesn't replace the Summarizer. Summaries are language tasks — explaining what a region means in human terms. The analyzer tells you *that* region X has low coherence; the Summarizer explains *why* and *what to do about it*. They're complementary.

The analyzer also doesn't replace the Synthesizer. Creating new edges between nodes based on content understanding is still an LLM task (or a human task). The analyzer says "region A and region B have 0.87 semantic similarity but no edges" — the Synthesizer (or you) decides whether to create the connection and what type it should be.

The analyzer replaces the guide. The guide's job was "maintain big-picture awareness" — the analyzer does this computationally and reports findings. The guide's job was "connect dots" — the analyzer identifies candidate connections programmatically. The guide's job was "catch architectural drift" — the analyzer detects staleness, orphans, and structural anomalies.

## Decision

Start with Phase A (topology + staleness). Run it against the real graph. If the output surfaces something you didn't know — a disconnected component, a stale spec you forgot about, an orphan cluster — then the approach is validated and you build the rest. If the output is boring and tells you nothing new, the graph might not have enough structure yet for analysis to be useful.

This is the same "gate before building more" principle from the single-agent test, applied to the analyzer itself. Don't build all four phases before validating that Phase A produces useful output.
