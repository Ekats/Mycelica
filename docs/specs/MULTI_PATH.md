# Multi-Path Associations

## Core Principle

Traditional file systems force single-path hierarchies:
```
Documents/Work/Projects/Mycelica/notes.md
```

But knowledge doesn't work this way. The same insight connects to multiple contexts:
- A conversation about "Tauri performance" belongs to:
  - "Rust Development" (language)
  - "Mycelica Project" (context)
  - "Performance Optimization" (topic)
  - "Desktop Apps" (domain)

The brain doesn't use folders. A memory about "debugging Rust code" connects to "Rust", "programming", "problem-solving", and "that frustrating Tuesday" simultaneously. Multiple synapses reach the same neuron.

**Mycelica uses graph edges for associations, not folder trees.**

---

## Implementation

### Primary vs Secondary Associations

- **Primary**: Stored in `cluster_id` — used for hierarchy building (Universe → Topics → Items)
- **Secondary**: Stored as edges — used for graph traversal and discovery

### Data Model

```sql
-- Primary association (for hierarchy navigation)
nodes.cluster_id = 1
nodes.cluster_label = "Rust Development"

-- All associations (for graph traversal)
edges:
  item_uuid → cluster-1 (type: 'belongs_to', weight: 0.9)
  item_uuid → cluster-3 (type: 'belongs_to', weight: 0.7)
  item_uuid → cluster-5 (type: 'belongs_to', weight: 0.4)
```

### AI Clustering Output

Claude returns multiple categories per item with confidence scores:

```json
{
  "item_id": "uuid-123",
  "clusters": [
    {"id": 0, "label": "Rust Development", "strength": 0.9},
    {"id": 3, "label": "Mycelica Project", "strength": 0.7},
    {"id": 5, "label": "Performance", "strength": 0.4}
  ]
}
```

### Storage Strategy

```rust
for assignment in assignments {
    // Sort by strength, highest first
    let sorted = assignment.clusters.sorted_by_strength();

    // Primary: highest strength → stored in cluster_id for hierarchy
    let primary = &sorted[0];
    db.update_node_clustering(&assignment.item_id, primary.id, &primary.label)?;

    // All associations: create edges
    for cluster in &sorted {
        db.create_edge(&Edge {
            source_id: assignment.item_id.clone(),
            target_id: format!("cluster-{}", cluster.id),
            edge_type: EdgeType::BelongsTo,
            weight: Some(cluster.strength),
        })?;
    }
}
```

---

## Navigation Modes

| Mode | Uses | Purpose |
|------|------|---------|
| **Hierarchy** | `cluster_id` + `parent_id` | Linear drill-down: Universe → Topic → Item |
| **Graph** | Edges with weights | See all connections, traverse any path |
| **Discovery** | Edge queries | "Show me everything connected to 'Performance'" |

---

## Why This Matters

1. **No lost context**: Item about "Rust + Mycelica + Performance" isn't forced into one bucket
2. **Serendipitous discovery**: Following edges reveals unexpected connections
3. **Brain-like**: Mirrors how human memory actually works
4. **Future-proof**: Enables features like "related items", "knowledge paths", cross-topic insights

---

## Database

> See `SCHEMA.md` for edge table schema, `TYPES.md` for edge types.

Multi-path associations use `belongs_to` edges with a `weight` field (0.0-1.0) indicating association strength.

---

## API

### Get all associations for an item

```typescript
const edges = await invoke('get_edges_for_node', { nodeId: itemId });
const associations = edges
  .filter(e => e.type === 'belongs_to')
  .sort((a, b) => (b.weight || 0) - (a.weight || 0));
```

### Find related items across topics

```typescript
// Get items that share associations with this item
const related = await invoke('get_related_items', {
  nodeId: itemId,
  minWeight: 0.3
});
```

---

## Visual Representation

```
           ┌─────────────────────┐
           │  Rust Development   │
           │    (weight: 0.9)    │
           └──────────┬──────────┘
                      │
    ┌─────────────────┼─────────────────┐
    │                 │                 │
    ▼                 ▼                 ▼
┌───────┐       ┌───────────┐     ┌──────────┐
│ Item  │       │   Item    │     │   Item   │
│  A    │───────│"Tauri vs  │─────│    C     │
│       │       │ Electron" │     │          │
└───────┘       └─────┬─────┘     └──────────┘
                      │
           ┌──────────┼──────────┐
           │          │          │
           ▼          ▼          ▼
    ┌──────────┐ ┌─────────┐ ┌──────────┐
    │ Mycelica │ │ Perf    │ │ Desktop  │
    │ Project  │ │ Optim.  │ │ Apps     │
    │ (0.7)    │ │ (0.4)   │ │ (0.3)    │
    └──────────┘ └─────────┘ └──────────┘
```

The item "Tauri vs Electron" lives primarily in "Rust Development" but is also connected to three other topics with varying strengths.

---

## FOS-Based Edges (Papers)

For scientific papers imported from OpenAIRE, Field of Science (FOS) categories provide additional cross-topic associations:

- Papers can belong to multiple FOS categories
- FOS edges created during `cluster_with_fos_pregrouping()`
- Cross-FOS edges enable discovery across research domains

See [AI_CLUSTERING.md](AI_CLUSTERING.md) for FOS clustering details.

---

*Last updated: 2026-01-08*
