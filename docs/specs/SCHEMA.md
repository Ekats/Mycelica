# Database Schema

> Generated from `src-tauri/src/db/schema.rs`. This is the actual schema.

Mycelica uses **SQLite** with **rusqlite** (not sqlx). Database location:
- Linux: `~/.local/share/com.mycelica.app/mycelica.db`
- macOS: `~/Library/Application Support/com.mycelica.app/mycelica.db`
- Windows: `%APPDATA%\com.mycelica.app\mycelica.db`

---

## Tables Overview

| Table | Purpose |
|-------|---------|
| `nodes` | All content: items, categories, Universe |
| `edges` | Relationships between nodes |
| `tags` | Persistent tag definitions for clustering |
| `item_tags` | Item-to-tag assignments |
| `learned_emojis` | AI-learned emoji mappings |
| `db_metadata` | Pipeline state tracking |
| `nodes_fts` | Full-text search (FTS5 virtual table) |

---

## nodes

The primary table containing all graph content (32 columns).

```sql
CREATE TABLE nodes (
    -- Identity
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL,              -- 'conversation', 'note', 'bookmark', 'cluster'

    -- Display
    title TEXT NOT NULL,             -- Original/raw title
    ai_title TEXT,                   -- AI-generated clean title
    summary TEXT,                    -- AI-generated summary
    tags TEXT,                       -- JSON array: '["react", "api"]'
    emoji TEXT,                      -- Display emoji
    url TEXT,                        -- For bookmarks
    content TEXT,                    -- Full content

    -- Layout (ephemeral)
    position_x REAL NOT NULL DEFAULT 0,
    position_y REAL NOT NULL DEFAULT 0,

    -- Timestamps
    created_at INTEGER NOT NULL,     -- Unix timestamp (seconds)
    updated_at INTEGER NOT NULL,
    last_accessed_at INTEGER,        -- For recency in sidebar
    latest_child_date INTEGER,       -- Bubbled up from descendants

    -- Hierarchy
    depth INTEGER NOT NULL DEFAULT 0,      -- 0 = Universe, increases toward items
    is_item INTEGER NOT NULL DEFAULT 0,    -- 1 = leaf content, opens in Leaf mode
    is_universe INTEGER NOT NULL DEFAULT 0, -- 1 = root node (exactly one)
    parent_id TEXT,                        -- Structural parent
    child_count INTEGER NOT NULL DEFAULT 0, -- Direct children count

    -- Clustering
    cluster_id INTEGER,              -- Semantic group assignment
    cluster_label TEXT,              -- Human-readable cluster name
    needs_clustering INTEGER NOT NULL DEFAULT 1, -- 1 = pending clustering

    -- Processing
    is_processed INTEGER NOT NULL DEFAULT 0, -- 1 = AI has processed
    embedding BLOB,                  -- Float32 array for similarity

    -- Conversation context
    conversation_id TEXT,            -- Parent conversation ID
    sequence_index INTEGER,          -- Order in conversation (0-based)

    -- Quick access
    is_pinned INTEGER NOT NULL DEFAULT 0,

    -- Privacy (0.0 = private, 1.0 = public)
    privacy REAL,                    -- Continuous privacy score
    is_private INTEGER,              -- Legacy boolean (deprecated)
    privacy_reason TEXT,             -- Explanation of rating

    -- Content classification
    content_type TEXT,               -- 'insight', 'idea', 'debug', etc.
    associated_idea_id TEXT,         -- Links supporting item to idea

    -- Import tracking
    source TEXT                      -- 'claude', 'googlekeep', 'markdown'
);
```

### Key Columns

| Column | Type | Description |
|--------|------|-------------|
| `id` | TEXT | UUID primary key |
| `type` | TEXT | `conversation`, `note`, `bookmark`, `cluster` |
| `depth` | INTEGER | 0 = Universe (root), increases toward leaves |
| `is_item` | INTEGER | 1 = openable in Leaf mode |
| `is_universe` | INTEGER | 1 = root node (exactly one) |
| `parent_id` | TEXT | References parent node's `id` |
| `privacy` | REAL | 0.0-1.0 scale (0 = private, 1 = public) |
| `content_type` | TEXT | Classification tier |
| `embedding` | BLOB | 384-dimensional float32 vector |

### Content Types

```
VISIBLE (shown in graph):
  insight, idea, exploration, synthesis, question, planning

SUPPORTING (lazy-loaded):
  investigation, discussion, reference, creative

HIDDEN (excluded):
  debug, code, paste, trivial
```

### Privacy Tiers

```
0.0-0.2: Highly private (names, health, finances)
0.3-0.4: Personal (work grievances, venting)
0.5-0.6: Semi-private (named projects, neutral context)
0.7-0.8: Low risk (technical content)
0.9-1.0: Public (generic concepts, tutorials)
```

---

## edges

Relationships between nodes.

```sql
CREATE TABLE edges (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    target_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    type TEXT NOT NULL,              -- Edge type
    label TEXT,                      -- Human-readable label
    weight REAL,                     -- Association strength (0.0-1.0)
    edge_source TEXT,                -- 'ai', 'user', or NULL
    evidence_id TEXT,                -- Node ID explaining reasoning
    confidence REAL,                 -- Certainty (0.0-1.0)
    created_at INTEGER NOT NULL
);
```

### Edge Types

| Type | Description |
|------|-------------|
| `reference` | Citation or link |
| `because` | Causal relationship |
| `related` | Semantic similarity |
| `contains` | Parent-child containment |
| `belongs_to` | Multi-path category membership |

---

## tags

Persistent tag definitions that survive hierarchy rebuilds.

```sql
CREATE TABLE tags (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    parent_tag_id TEXT REFERENCES tags(id),
    depth INTEGER NOT NULL DEFAULT 0,
    centroid BLOB,                   -- Embedding for matching
    item_count INTEGER NOT NULL DEFAULT 0,
    pinned INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

---

## item_tags

Many-to-many junction between items and tags.

```sql
CREATE TABLE item_tags (
    item_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    confidence REAL NOT NULL DEFAULT 1.0,
    source TEXT NOT NULL DEFAULT 'ai',  -- 'ai' or 'user'
    PRIMARY KEY (item_id, tag_id)
);
```

---

## learned_emojis

AI-learned emoji mappings for topics.

```sql
CREATE TABLE learned_emojis (
    keyword TEXT PRIMARY KEY,
    emoji TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
```

---

## db_metadata

Key-value store for pipeline state.

```sql
CREATE TABLE db_metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
```

### Known Keys

| Key | Values |
|-----|--------|
| `pipeline_state` | `fresh`, `imported`, `processed`, `clustered`, `hierarchized`, `complete` |

---

## Indexes

```sql
-- Edge traversal
CREATE INDEX idx_edges_source ON edges(source_id);
CREATE INDEX idx_edges_target ON edges(target_id);
CREATE INDEX idx_edges_type ON edges(type);

-- Node queries
CREATE INDEX idx_nodes_type ON nodes(type);
CREATE INDEX idx_nodes_parent_id ON nodes(parent_id);
CREATE INDEX idx_nodes_depth ON nodes(depth);
CREATE INDEX idx_nodes_is_item ON nodes(is_item);
CREATE INDEX idx_nodes_cluster_id ON nodes(cluster_id);
CREATE INDEX idx_nodes_content_type ON nodes(content_type);
CREATE INDEX idx_nodes_associated_idea ON nodes(associated_idea_id);
CREATE INDEX idx_nodes_privacy ON nodes(privacy);

-- Tag system
CREATE INDEX idx_tags_parent ON tags(parent_tag_id);
CREATE INDEX idx_tags_depth ON tags(depth);
CREATE INDEX idx_item_tags_item ON item_tags(item_id);
CREATE INDEX idx_item_tags_tag ON item_tags(tag_id);
```

---

## Full-Text Search (FTS5)

```sql
CREATE VIRTUAL TABLE nodes_fts USING fts5(
    title,
    content,
    content='nodes',
    content_rowid='rowid'
);
```

Kept in sync via triggers: `nodes_ai`, `nodes_ad`, `nodes_au`

**Query:**
```sql
SELECT n.* FROM nodes n
JOIN nodes_fts f ON n.rowid = f.rowid
WHERE nodes_fts MATCH ?1;
```

---

## Rust Usage (rusqlite)

```rust
use rusqlite::{Connection, params};

let conn = Connection::open(db_path)?;

// Query
let node: Node = conn.query_row(
    "SELECT * FROM nodes WHERE id = ?1",
    params![id],
    |row| Ok(Node { ... })
)?;

// Insert
conn.execute(
    "INSERT INTO nodes (id, type, title, ...) VALUES (?1, ?2, ?3, ...)",
    params![node.id, node.type, node.title, ...],
)?;
```

---

*Last updated: 2025-12-26*
