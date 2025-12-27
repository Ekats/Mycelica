# AI-Powered Clustering Specification

## Overview

Production clustering system using Claude API for semantic categorization with TF-IDF fallback.

---

## Database Changes

Add to `nodes` table:
```sql
needs_clustering INTEGER DEFAULT 1
```

- When items are imported → `needs_clustering = 1`
- After clustering → `needs_clustering = 0`

---

## Clustering Flow

### Entry Points

1. `get_items_needing_clustering()` — returns items where `needs_clustering = 1`
2. `cluster_with_ai(items, existing_clusters)` — main clustering function

### AI Clustering Process

```
Items needing clustering
    ↓
Batch (10-20 per API call)
    ↓
Build prompt with:
  - Item titles + content (1000 chars)
  - Existing cluster names
    ↓
Claude returns JSON assignments
    ↓
Update DB: cluster_id, cluster_label
    ↓
Set needs_clustering = 0
```

### Fallback to TF-IDF

Use TF-IDF when:
- No API key configured
- API call fails
- User selects offline mode

---

## Claude Prompt

```
You are organizing a knowledge base. Given these items and existing categories, assign each item to the best category or suggest a new one.

Existing categories:
- [id: 0] "Mycelica Development"
- [id: 1] "Browser Tech"
(or "None yet" if empty)

Items to categorize:
1. [uuid-1] "Building a branching Claude UI..." | Content: ...
2. [uuid-2] "Tauri vs Electron..." | Content: ...
...

Return JSON only:
[
  {"item_id": "uuid-1", "cluster_id": 0, "cluster_label": "Mycelica Development"},
  {"item_id": "uuid-2", "cluster_id": 2, "cluster_label": "Performance Comparison", "is_new_cluster": true}
]
```

---

## Response Format

```json
[
  {
    "item_id": "uuid-1",
    "cluster_id": 0,
    "cluster_label": "Mycelica Development"
  },
  {
    "item_id": "uuid-2",
    "cluster_id": 2,
    "cluster_label": "Performance Comparison",
    "is_new_cluster": true
  }
]
```

---

## Outlier Handling

- If Claude can't categorize → assign to "Miscellaneous" (cluster_id: -1)
- Prompt should encourage finding a home for everything
- Miscellaneous is a last resort

---

## UX Considerations

1. **Badge**: Show "X items need processing" in UI
2. **Progress**: Show indicator during clustering
3. **Incremental**: "Cluster" button only processes unclustered items (fast)
4. **Instant rebuild**: "Rebuild Hierarchy" uses cached cluster_id (no API)

