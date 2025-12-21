# Leaf View Mini-Clustering System

## The Problem

When you drill into a topic with 30+ items, the graph becomes cluttered. Not all items are equal:
- Some are **ideas and discussions** — the primary content you want to navigate
- Some are **code snippets** — supporting material
- Some are **error logs and debugging** — useful when needed, noise otherwise
- Some are **long pastes** — reference material

Currently they all render as equal nodes, creating visual chaos at scale.

---

## The Solution

**Separate content into two paths:**

```
                    ┌─────────────┐
                    │   TOPIC     │
                    └──────┬──────┘
                           │
              ┌────────────┴────────────┐
              │                         │
          GRAPH PATH                PANEL PATH
         (navigable)               (accessible)
              │                         │
        ┌─────┴─────┐            ┌──────┴──────┐
        │   IDEAS   │            │   CODE      │
        │   DOCS    │            │   DEBUG     │
        └───────────┘            │   PASTES    │
                                 └─────────────┘
```

- **Ideas/Docs** render as nodes in the graph
- **Code/Debug/Pastes** live in a panel, one click away
- Supporting items are **associated** with specific idea nodes by similarity + time

---

## Content Classification

### New Database Field

```sql
ALTER TABLE nodes ADD COLUMN content_type TEXT;
-- Values: "idea" | "code" | "debug" | "paste" | NULL
-- NULL = legacy/unclassified, treated as "idea"
```

### Classification Rules (Pattern Matching, No AI)

| Type | Detection Signals |
|------|-------------------|
| `code` | Triple backticks, file extensions (.rs, .tsx, .py), high indentation ratio, syntax keywords (fn, const, import, def) |
| `debug` | "error", "failed", "exception", "stack trace", "panic", troubleshooting language patterns |
| `paste` | Length > 1500 chars AND low conversation markers (few questions, few "I", "you") |
| `idea` | Everything else — conversational, questions, explanations, decisions |

### Classification Runs

- During import (new items)
- During rebuild (backfill existing)
- Fast pattern matching, no API calls

---

## Association Model

Supporting items aren't just grouped under a topic — they're **associated with specific idea nodes** using:

1. **Semantic similarity** — embedding cosine distance to nearby ideas
2. **Time proximity** — items from same conversation window

### Association Algorithm

```
FOR each supporting item (code/debug/paste):
    
    1. Get all idea nodes in same topic
    
    2. Score each idea by:
       - similarity_score = cosine_similarity(supporting.embedding, idea.embedding)
       - time_score = 1 / (1 + hours_between(supporting.created, idea.created))
       - combined_score = (similarity_score * 0.7) + (time_score * 0.3)
    
    3. Associate with highest-scoring idea (if score > threshold)
       OR mark as "topic-level" if no strong match
```

### Database Representation

```sql
ALTER TABLE nodes ADD COLUMN associated_idea_id TEXT REFERENCES nodes(id);
-- NULL = belongs to topic as a whole (no specific idea match)
```

### Example

```
Topic: "Mycelica Graph Development"

idea_1: "Graph traversal approach" (Dec 18, 10:30)
idea_2: "Edge weighting concept" (Dec 18, 14:00)
idea_3: "Node rendering strategy" (Dec 19, 09:00)

code_1: "traversal.rs snippet" (Dec 18, 10:45)
        → associated_idea_id: idea_1 (high similarity + 15 min apart)

debug_1: "edge weight NaN error" (Dec 18, 14:20)  
        → associated_idea_id: idea_2 (high similarity + 20 min apart)

paste_1: "full cargo build log" (Dec 17, 16:00)
        → associated_idea_id: NULL (no strong match, lives at topic level)
```

---

## Data Model Summary

```
nodes table:
┌────────┬──────────────────┬───────────┬──────────────┬───────────────────┐
│ id     │ title            │ parent_id │ content_type │ associated_idea_id│
├────────┼──────────────────┼───────────┼──────────────┼───────────────────┤
│ t1     │ Mycelica Dev     │ universe  │ NULL (topic) │ NULL              │
│ i1     │ Graph approach   │ t1        │ idea         │ NULL              │
│ i2     │ Edge concept     │ t1        │ idea         │ NULL              │
│ c1     │ traversal.rs     │ t1        │ code         │ i1                │
│ d1     │ Edge NaN error   │ t1        │ debug        │ i2                │
│ p1     │ Cargo build log  │ t1        │ paste        │ NULL              │
└────────┴──────────────────┴───────────┴──────────────┴───────────────────┘
```

---

## Query Changes

### Current

```rust
fn get_children(parent_id: &str) -> Vec<Node> {
    // Returns ALL children
    SELECT * FROM nodes WHERE parent_id = ?
}
```

### New

```rust
fn get_graph_children(parent_id: &str) -> Vec<Node> {
    // Returns only ideas for graph rendering
    SELECT * FROM nodes 
    WHERE parent_id = ? 
    AND (content_type = 'idea' OR content_type IS NULL)
    AND is_item = 1
}

fn get_supporting_items(parent_id: &str) -> SupportingItems {
    // Returns code/debug/paste grouped by type and association
    SELECT * FROM nodes 
    WHERE parent_id = ? 
    AND content_type IN ('code', 'debug', 'paste')
    ORDER BY content_type, created_at DESC
}

fn get_associated_items(idea_id: &str) -> Vec<Node> {
    // Returns supporting items linked to a specific idea
    SELECT * FROM nodes 
    WHERE associated_idea_id = ?
    ORDER BY content_type, created_at DESC
}
```

---

## UI Surfaces

### 1. Graph View (Cleaned Up)

Only idea nodes render. Topic nodes show badges indicating hidden content.

```
┌──────────────────────────────────────────────────┐
│                                                  │
│         ○ Graph approach                         │
│            [📝 1]           ← badge shows 1 code │
│                                                  │
│    ○ Edge concept      ○ Node rendering          │
│       [🐛 1]                                     │
│                                                  │
│              ○ Clustering                        │
│                strategy                          │
│                                                  │
└──────────────────────────────────────────────────┘
```

### 2. Node Hover/Select — Associated Items

When you hover or select an idea node, its associated supporting items appear:

```
┌─────────────────────────────────────┐
│  ○ Edge concept                     │
│                                     │
│  ┌───────────────────────────────┐  │
│  │ 🐛 edge weight NaN error      │  │
│  │    Dec 18, 14:20              │  │
│  │    [Open]                     │  │
│  └───────────────────────────────┘  │
│                                     │
└─────────────────────────────────────┘
```

### 3. Leaf View Panel — Full Access

When viewing a topic's leaf view, tabs provide full access:

```
┌─────────────────────────────────────────────────────────┐
│  LEAF VIEW: Mycelica Development                        │
│                                                         │
│  [💡 Ideas (5)]  [📝 Code (3)]  [🐛 Debug (2)]  [📋 1]  │
│                                                         │
│  ┌─────────────────────────────────────────────────┐   │
│  │ ● Graph approach                    Dec 18      │   │
│  │   └─ 📝 traversal.rs (associated)               │   │
│  │                                                  │   │
│  │ ● Edge concept                      Dec 18      │   │
│  │   └─ 🐛 edge NaN error (associated)             │   │
│  │                                                  │   │
│  │ ● Node rendering                    Dec 19      │   │
│  │                                                  │   │
│  └─────────────────────────────────────────────────┘   │
│                                                         │
│  UNASSOCIATED:                                          │
│  ┌─────────────────────────────────────────────────┐   │
│  │ 📋 Cargo build log                  Dec 17      │   │
│  └─────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

### 4. Topic Badge — Quick Stats

Topic nodes in the graph show what's hidden:

```
┌─────────────────────────┐
│  Mycelica Development   │
│        (12 ideas)       │
│   📝 3  🐛 2  📋 1      │  ← click opens panel
└─────────────────────────┘
```

---

## Sorting Within Views

### Ideas in Graph
- Time-based layout: recent toward center, older toward edges
- Or by importance (child count, connection count)

### Supporting Items in Panel
- Primary sort: by associated idea (grouped)
- Secondary sort: by time (recent first)
- Unassociated items appear at bottom

### Tab Views
- Each content type sorted by time
- Shows association inline: "↳ linked to: Graph approach"

---

## Implementation Phases

### Phase 1: Classification Infrastructure
- [ ] Add `content_type` column to nodes table
- [ ] Create `classify_content(text: &str) -> ContentType` function
- [ ] Run classification during import
- [ ] Backfill command for existing nodes

### Phase 2: Association System
- [ ] Add `associated_idea_id` column
- [ ] Create `compute_association(item, topic_ideas) -> Option<idea_id>`
- [ ] Run association after classification
- [ ] Integrate with rebuild process

### Phase 3: Query Layer
- [ ] `get_graph_children()` — ideas only
- [ ] `get_supporting_items()` — by topic
- [ ] `get_associated_items()` — by idea
- [ ] `get_supporting_counts()` — for badges

### Phase 4: Graph UI
- [ ] Filter graph to only render idea nodes
- [ ] Add badges to idea nodes showing associated item counts
- [ ] Add badges to topic nodes showing total supporting counts

### Phase 5: Panel UI
- [ ] Add tabs to leaf view panel
- [ ] Show associations inline with ideas
- [ ] Add "unassociated" section
- [ ] Click-through to supporting items

### Phase 6: Node Interaction
- [ ] Hover/select shows associated items
- [ ] Expand/collapse supporting items inline
- [ ] Quick navigation between idea ↔ supporting item

---

## Performance Considerations

### Classification
- Pattern matching only, no AI calls
- O(n) where n = content length
- Run incrementally during import

### Association
- Requires embeddings (already computed)
- O(ideas × supporting) per topic, but topics are small
- Cache associations, recompute only on change

### Graph Rendering
- Fewer nodes = faster render
- From 30 nodes to ~10 nodes per topic = 3x speedup
- Supporting items loaded on demand (panel open, node hover)

---

## Future Enhancements

### Smart Association
- Use conversation thread to group related items
- Items from same chat session auto-associate

### Cross-Topic References
- Code snippet used in multiple topics
- Show "also appears in: Topic X, Topic Y"

### Inline Preview
- Hover code badge → syntax-highlighted preview
- Hover debug badge → error summary

### Search Integration
- "Find all code related to clustering"
- Searches supporting items, shows parent idea context

---

## Summary

**The leaf view system solves the "1000s of nodes" problem by:**

1. Classifying content by type (idea, code, debug, paste)
2. Associating supporting items with specific ideas (similarity + time)
3. Rendering only ideas in the graph
4. Providing quick access to supporting items via badges and panels
5. Keeping the data model simple (two new columns, same hierarchy)

The graph becomes a clean **idea map**. Everything else is one click away.
