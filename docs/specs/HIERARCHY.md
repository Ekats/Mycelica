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

### Threshold-Based Level Creation

| Item Count | Structure | Example |
|------------|-----------|---------|
| < 30 | Universe → Items | Just show everything |
| 30-150 | Universe → Topics → Items | One clustering layer |
| 150-500 | Universe → Domains → Topics → Items | Two layers |
| 500-2000 | Universe → Galaxies → Domains → Topics → Items | Three layers |
| 2000+ | Add more as needed | Deep hierarchies for massive collections |

**The system should:**
1. Count items at each level
2. If a level has too many nodes (>30-50), cluster them
3. If a level has too few nodes (<5), consider flattening
4. Rebalance when imports significantly change the collection

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

```
For each cluster:
  └── Create a parent node (Topic, Region, whatever scale fits)
  └── Set items' parent_id to this new node
  └── New node gets depth = item_depth - 1

If too many clusters at one level:
  └── Cluster the clusters (meta-clustering)
  └── Create another parent level
  └── Repeat until manageable

Create Universe if not exists:
  └── Set top-level nodes' parent_id to Universe
```

### Step 4: Rebalance

```
If any level has >50 nodes:
  └── Cluster that level, add parents

If any level has <3 nodes:
  └── Consider merging up or flattening

If item count changes significantly:
  └── Re-run clustering from scratch
```

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