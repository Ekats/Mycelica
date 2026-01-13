# Mycelica Hierarchy

Two things are fixed: **Universe** (the root) and **Leaf** (the reader). Everything in between is dynamic.

---

## The Two Constants

```
┌─────────────────────────────────────────────────────────────────────┐
│  UNIVERSE                                                            │
│  The root. Entry point. Always exactly one.                         │
│  "All your knowledge starts here"                                    │
└───────────────────────────────┬─────────────────────────────────────┘
                                │
                                ▼
                    ┌───────────────────────┐
                    │                       │
                    │   DYNAMIC LEVELS      │
                    │                       │
                    │   As many or as few   │
                    │   as your collection  │
                    │   needs               │
                    │                       │
                    └───────────┬───────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│  ITEMS                                                               │
│  The actual content nodes. Conversations, notes, bookmarks, docs.   │
│  What you imported. Clickable to open in Leaf.                      │
└───────────────────────────────┬─────────────────────────────────────┘
                                │
                                │ click
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│  LEAF (Reader Mode)                                                  │
│  NOT a graph level. A different view entirely.                      │
│  Integrated reader for any content type.                            │
│  Future: Firefox/Gecko viewport for web content.                    │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Dynamic Middle Levels

The middle levels exist to reduce cognitive overload. They're created based on **collection size**, not a fixed schema.

### Tiered Child Limits

The hierarchy enforces tiered limits on children per level:

| Depth | Max Children | Purpose |
|-------|--------------|---------|
| 0-1 (Universe + L1) | 10 | Strict navigation, clean top level |
| 2 | 25 | Buffer layer |
| 3 | 50 | Topic groupings |
| 4 | 100 | Normal max depth |
| 5+ | 150 | Coherent mega-clusters only |

```rust
// From hierarchy.rs:156-164
fn max_children_for_depth(depth: i32) -> usize {
    match depth {
        0 | 1 => 10,   // Universe + L1: strict navigation
        2 => 25,       // L2: buffer layer
        3 => 50,       // L3: topic groupings
        4 => 100,      // L4: normal max depth
        _ => 150,      // L5+: coherent mega-clusters only
    }
}
```

### Safety Limits

**MAX_HIERARCHY_DEPTH = 15** — Prevents runaway recursion during intermediate node creation. If grouping would exceed this depth, the operation stops and logs a warning.

```rust
// From hierarchy.rs:1358
const MAX_HIERARCHY_DEPTH: i32 = 15;
```

### Garbage Name Filtering

AI-generated category names are filtered against a list of meaningless terms. A name is rejected if >50% of its words match garbage terms (substring matching).

```rust
// From hierarchy.rs:201-206
const GARBAGE_NAMES: &[&str] = &[
    "empty", "cluster", "misc", "other", "general", "various",
    "uncategorized", "miscellaneous", "group", "collection",
    "related", "topics", "items", "content", "stuff", "things",
    "mixed", "assorted", "combined", "merged", "grouped", "sorted",
];
```

When a garbage name is detected, the system falls back to keyword-based naming or skips the grouping.

### Threshold-Based Level Creation

| Item Count | Structure | Example |
|------------|-----------|---------|
| < 30 | Universe → Items | Just show everything |
| 30-150 | Universe → Topics → Items | One clustering layer |
| 150-500 | Universe → Domains → Topics → Items | Two layers |
| 500-2000 | Universe → Galaxies → Domains → Topics → Items | Three layers |
| 2000+ | Add more as needed | Deep hierarchies for massive collections |

**The system:**
1. Counts children at each level
2. If a level exceeds its max (tiered by depth), creates intermediate clusters
3. Merges up small clusters (< 3 children)
4. Uses `cluster_hierarchy_level()` for AI-assisted grouping when needed

### Level Names Are Semantic

Don't think "L0, L1, L2" — think about what the level *means*:

| Name | Scale | Contains |
|------|-------|----------|
| Universe | Everything | All top-level groupings |
| Galaxy | Life domain | Work, Personal, Learning, Creative... |
| World | Major area | "Programming", "Health", "Music"... |
| Continent | Sub-area | "Rust", "Web Dev", "Backend"... |
| Region | Topic cluster | "Async patterns", "Error handling"... |
| Topic | Tight group | 5-15 closely related items |
| Item | Single piece | One conversation, note, bookmark |

**You don't need all of these.** A small collection might just be:
```
Universe → Topics → Items
```

A massive knowledge base might need:
```
Universe → Galaxies → Worlds → Regions → Topics → Items
```

### Skip Levels When Sparse

If a clustering pass produces only 3 clusters, don't create a whole level for them — attach directly to parent or merge up.

```
# Bad: Sparse intermediate level
Universe
  └── Domains (only 2 nodes here... why?)
        ├── Tech
        └── Life

# Better: Skip it
Universe
  ├── Tech stuff (was Domain)
  │     └── [topics]
  └── Life stuff (was Domain)
        └── [topics]
```

---

## Leaf: The Integrated Reader

Leaf is **not a graph level**. It's a completely different mode.

### What Leaf Does

When you click an Item in the graph, Leaf opens to display its content. Leaf is a universal reader that handles:

| Content Type | Leaf Rendering |
|--------------|----------------|
| conversation | Chat transcript with user/assistant styling |
| note | Markdown → formatted HTML |
| bookmark | Webview (future: Gecko) |
| document | PDF viewer, Word rendering |
| snippet | Syntax-highlighted code |
| image | Image viewer with zoom |
| audio | Audio player with transcript |

### Leaf Has Internal Structure

Complex content doesn't need more graph levels — Leaf handles it:

```
┌─────────────────────────────────────────┐
│  LEAF READER                            │
│                                         │
│  ┌─────────────────────────────────┐   │
│  │ Table of Contents / Sections    │   │
│  │ ├── Introduction                │   │
│  │ ├── Main Discussion             │   │
│  │ └── Conclusions                 │   │
│  └─────────────────────────────────┘   │
│                                         │
│  ┌─────────────────────────────────┐   │
│  │ Content viewport                │   │
│  │                                 │   │
│  │ [The actual content renders     │   │
│  │  here, scrollable, searchable]  │   │
│  │                                 │   │
│  └─────────────────────────────────┘   │
│                                         │
│  ┌─────────────────────────────────┐   │
│  │ Related nodes sidebar           │   │
│  │ Connections to other Items      │   │
│  └─────────────────────────────────┘   │
│                                         │
└─────────────────────────────────────────┘
```

**For conversations:** Leaf can show thread branches, collapse long responses, highlight key moments.

**For documents:** Leaf can show chapters, headings, page numbers.

**For webpages (future):** Leaf IS the Firefox viewport. Full Gecko rendering.

### Back to Graph

From Leaf, user can:
- Press back / escape → return to graph at same position
- Click a related node → open that in Leaf
- Click "show in graph" → highlight this item's position in hierarchy

---

## Database Model

> See `SCHEMA.md` for full schema.

**Key fields:**
- `depth`: Numeric depth from Universe (0). Items are at max depth.
- `is_universe`: True for the single root node.
- `is_item`: True for content nodes (clickable to open Leaf).
- `parent_id`: Structural parent in hierarchy.
- `cluster_id`: Semantic grouping (used during clustering, may differ from parent).

### Why `depth` Instead of `level`

`level` implies fixed meanings (L0 = Universe, L1 = Galaxy...).

`depth` is just "how many hops from root" — works for any number of intermediate levels.

---

## Clustering → Hierarchy Flow

### Step 1: Import Creates Items

```
Import conversations.json
  └── Create nodes with is_item=1, depth=MAX, no parent
```

Items exist but aren't organized yet.

### Step 2: Clustering Groups Items

```
Run TF-IDF clustering on items
  └── Assign cluster_id to each item
  └── Items with similar content get same cluster_id
```

### Step 3: Build Hierarchy From Clusters

Two functions available:

| Function | Description |
|----------|-------------|
| `build_hierarchy(db)` | Basic: Creates parent nodes from clusters, enforces tiered limits |
| `build_full_hierarchy(app, db)` | Full 7-step pipeline with AI: uber-categories, FOS grouping, edge indexing |

**Basic build_hierarchy:**
```
For each cluster:
  └── Create a parent node (Topic, Region, whatever scale fits)
  └── Set items' parent_id to this new node
  └── New node gets depth = item_depth - 1

If too many clusters at one level:
  └── Apply tiered limits (10/25/50/100/150)
  └── Create intermediate groupings
  └── Repeat until manageable

Create Universe if not exists:
  └── Set top-level nodes' parent_id to Universe
```

**Full build_full_hierarchy (7 steps):**
1. Clear existing hierarchy (keep items)
2. Create/verify Universe node
3. Reparent items to Universe
4. Build hierarchy from clusters
5. Create uber-categories if Universe has >10 children
6. Apply tiered limits at each depth
7. Update edge parent columns for fast view loading

### Step 4: Rebalance

```
If any level has >50 nodes:
  └── Cluster that level, add parents

If any level has <3 nodes:
  └── Consider merging up or flattening

If item count changes significantly:
  └── Re-run clustering from scratch
```

### Step 5: Privacy Propagation

After hierarchy is built, privacy scores propagate bottom-up from items to categories.

**Algorithm (from hierarchy.rs:2947-2988):**
1. Start from deepest level (one above items)
2. Work upward toward Universe
3. For each category: `privacy = min(children's privacy scores)`
4. Most restrictive score wins (0.0 = private, 1.0 = public)

```
Items at depth 5:
  Item A (privacy: 0.8)
  Item B (privacy: 0.3)  ← most restrictive
  Item C (privacy: 0.9)
    ↓
Parent category gets privacy: 0.3
```

**Protected nodes:** Project umbrellas (`project-*`) and Personal category (`category-personal`) stay at depth 1 and are excluded from uber-category grouping.

---

## UI View States

```typescript
type ViewMode = 'graph' | 'leaf';

interface ViewState {
  mode: ViewMode;
  
  // Graph mode
  focusedNodeId: string;      // Which node we're "inside" (showing children)
  selectedNodeId?: string;    // Which node is highlighted
  zoomLevel: number;          // Visual zoom on canvas
  
  // Leaf mode
  openItemId?: string;        // Which item is open in reader
  scrollPosition?: number;    // Where in the content
}

// Transitions
function openItem(itemId: string) {
  setState({ mode: 'leaf', openItemId: itemId });
}

function backToGraph() {
  setState({ mode: 'graph', openItemId: undefined });
}

function zoomInto(nodeId: string) {
  // If it's an item, open Leaf
  if (node.is_item) {
    openItem(nodeId);
  } else {
    // Otherwise, show its children in graph
    setState({ focusedNodeId: nodeId });
  }
}
```

---

## Summary

| Concept | Fixed? | Description |
|---------|--------|-------------|
| Universe | ✅ Yes | Single root, always exists |
| Middle levels | ❌ Dynamic | Created by clustering, count varies |
| Items | ✅ Yes (content) | Imported content, openable in Leaf |
| Leaf | ✅ Yes (reader) | Universal reader, future Gecko viewport |

**The graph is for navigation. The Leaf is for reading.**

Middle levels are just scaffolding to make navigation manageable. Small collection? Few levels. Massive collection? Many levels. The system adapts.

---

*Last updated: 2026-01-13*