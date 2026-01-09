# Mycelica Types

> Generated from `src/types/graph.ts` and `src-tauri/src/db/models.rs`.

## Core Principle: Graph vs Leaf Separation

```
┌─────────────────────────────────────────────────────────────────┐
│  GRAPH MODE = Navigation                                        │
│                                                                 │
│  Uses these fields:                                             │
│    - isItem: boolean (opens in Leaf vs drill into children)     │
│    - childCount: number (has children to navigate?)             │
│    - clusterId: number (determines card color)                  │
│    - title/aiTitle, summary, emoji (display fields)             │
│    - contentType (determines visibility tier)                   │
│                                                                 │
│  Does NOT branch on node.type                                   │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│  LEAF MODE = Content Display                                    │
│                                                                 │
│  Uses node.type to decide HOW to render:                        │
│    - 'conversation' → chat bubble layout                        │
│    - 'note' → markdown renderer                                 │
│    - 'bookmark' → URL preview + content                         │
│    - 'cluster' → organizational, shouldn't open in Leaf         │
└─────────────────────────────────────────────────────────────────┘
```

---

## TypeScript Types (Frontend)

Source: `src/types/graph.ts`

### NodeType

```typescript
// Metadata for Leaf mode - NOT used in Graph rendering
type NodeType =
  | 'conversation'  // Imported chat (Claude, etc.) - renders as chat bubbles
  | 'note'          // User note - renders as markdown
  | 'bookmark'      // URL/webpage - renders with URL preview
  | 'cluster'       // Organizational grouping - not openable in Leaf
  | 'page'          // Legacy: treat as note
  | 'thought'       // Legacy: treat as note
  | 'context';      // Legacy: treat as note
```

### EdgeType

```typescript
type EdgeType =
  // General relationships
  | 'reference'    // Citation or link
  | 'because'      // Causal relationship
  | 'related'      // Semantic similarity
  | 'contains'     // Parent-child containment
  | 'belongs_to'   // Multi-path category membership
  // Code relationships
  | 'calls'        // Function calls function
  | 'uses_type'    // Function references struct/enum
  | 'implements'   // Impl implements trait
  | 'defined_in'   // Code item defined in module/file
  | 'imports'      // Module imports module
  | 'tests'        // Test function tests function
  | 'documents';   // Doc references code (backtick refs)
```

### ContentType

```typescript
// 15 content types across 3 visibility tiers
type ContentType =
  // VISIBLE (shown in graph)
  | 'insight'       // Realization, conclusion, crystallized understanding
  | 'exploration'   // Researching, thinking out loud
  | 'synthesis'     // Summarizing, connecting threads
  | 'question'      // Inquiry that frames investigation
  | 'planning'      // Roadmap, TODO, intentions
  | 'paper'         // Scientific paper (imported from OpenAIRE)
  | 'bookmark'      // Web capture from browser extension
  // SUPPORTING (lazy-loaded in Leaf view)
  | 'investigation' // Problem-solving focused on fixing
  | 'discussion'    // Back-and-forth Q&A without synthesis
  | 'reference'     // Factual lookup, definitions
  | 'creative'      // Fiction, poetry, roleplay
  // HIDDEN (excluded from graph)
  | 'debug'         // Error messages, stack traces
  | 'code'          // Code blocks, implementations
  | 'paste'         // Logs, terminal output, data dumps
  | 'trivial';      // Greetings, acknowledgments, fragments
```

Note: `idea` was previously used as an alias for `insight` but is deprecated. Code imports create `code_*` types (code_function, code_struct, code_enum, etc.) which are also visible.

### Node

```typescript
interface Node {
  id: string;

  // --- Metadata for Leaf mode (NOT used in Graph rendering) ---
  type: NodeType;           // Determines Leaf render mode
  url?: string;             // For bookmarks
  content?: string;         // Raw content from import

  // --- Display fields (used in Graph) ---
  title: string;            // Raw title from import
  aiTitle?: string;         // AI-generated clean title
  summary?: string;         // AI-generated summary
  tags?: string[];          // Parsed tags array
  emoji?: string;           // Topic emoji (AI-suggested or matched)

  // --- Graph navigation fields ---
  depth: number;            // 0 = Universe, increases toward items
  isItem: boolean;          // true = opens in Leaf, false = drill into children
  isUniverse: boolean;      // true = root node (exactly one)
  parentId?: string;        // Parent node ID (null for Universe)
  childCount: number;       // Number of direct children
  clusterId?: number;       // Semantic group ID (determines card color)
  clusterLabel?: string;    // Human-readable cluster name

  // --- Graph layout (not persisted) ---
  position: { x: number; y: number };

  // --- Timestamps & processing state ---
  createdAt: number;        // Unix timestamp (milliseconds)
  updatedAt: number;
  isProcessed: boolean;     // Whether AI has processed this node

  // --- Quick access (Sidebar) ---
  isPinned: boolean;        // User-pinned favorite
  lastAccessedAt?: number;  // For recency tracking in sidebar

  // --- Hierarchy date propagation ---
  latestChildDate?: number; // MAX(children's created_at), bubbled up from leaves

  // --- Conversation context (for message Leafs) ---
  conversationId?: string;  // ID of parent conversation this message belongs to
  sequenceIndex?: number;   // Position in original conversation (0, 1, 2...)

  // --- Privacy filtering ---
  isPrivate?: boolean;      // DEPRECATED: legacy boolean (undefined = not scanned)
  privacy?: number;         // 0.0 = private, 1.0 = public, undefined = unscored
  privacyReason?: string;   // Why it was marked private (for review)

  // --- Content classification ---
  contentType?: ContentType;     // Determines visibility tier
  associatedIdeaId?: string;     // Links supporting item to specific idea node

  // --- Import tracking ---
  source?: string;          // 'claude', 'googlekeep', 'markdown', 'openaire'
}
```

### Edge

```typescript
interface Edge {
  id: string;
  source: string;           // Source node ID
  target: string;           // Target node ID
  type: EdgeType;
  label?: string;           // Human-readable label
  weight?: number;          // Association strength (0.0-1.0)
  edgeSource?: string;      // 'ai', 'user', or null (origin tracking)
  evidenceId?: string;      // Node ID that explains WHY this edge exists
  confidence?: number;      // Certainty (0.0-1.0), distinct from weight
  createdAt: number;
}
```

### Supporting Types

```typescript
interface Viewport {
  x: number;
  y: number;
  zoom: number;
}

interface Graph {
  nodes: Map<string, Node>;
  edges: Map<string, Edge>;
  viewport: Viewport;
  activeNodeId?: string;
}
```

---

## Rust Types (Backend)

Source: `src-tauri/src/db/models.rs`

### NodeType (Enum)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Page,      // Legacy
    Thought,   // User-created note
    Context,   // Legacy
    Cluster,   // Organizational grouping
}
```

Note: The Rust enum is smaller than the TypeScript type. Additional types like `conversation`, `note`, `bookmark` are stored as strings in the database.

### EdgeType (Enum)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EdgeType {
    // General relationships
    Reference,   // Citation or link
    Because,     // Causal relationship
    Related,     // Semantic similarity
    Contains,    // Parent-child containment
    BelongsTo,   // Multi-path category membership
    // Code relationships
    Calls,       // Function calls function
    UsesType,    // Function references struct/enum
    Implements,  // Impl implements trait
    DefinedIn,   // Code item defined in module/file
    Imports,     // Module imports module
    Tests,       // Test function tests function
    Documents,   // Doc references code (backtick refs)
}
```

### Node (Struct)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub title: String,
    pub url: Option<String>,
    pub content: Option<String>,
    pub position: Position,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
    #[serde(rename = "clusterId")]
    pub cluster_id: Option<i32>,
    #[serde(rename = "clusterLabel")]
    pub cluster_label: Option<String>,

    // Dynamic hierarchy
    pub depth: i32,
    #[serde(rename = "isItem")]
    pub is_item: bool,
    #[serde(rename = "isUniverse")]
    pub is_universe: bool,
    #[serde(rename = "parentId")]
    pub parent_id: Option<String>,
    #[serde(rename = "childCount")]
    pub child_count: i32,

    // AI-processed
    #[serde(rename = "aiTitle")]
    pub ai_title: Option<String>,
    pub summary: Option<String>,
    pub tags: Option<String>,    // JSON array string
    pub emoji: Option<String>,
    #[serde(rename = "isProcessed")]
    pub is_processed: bool,

    // Conversation context
    #[serde(rename = "conversationId")]
    pub conversation_id: Option<String>,
    #[serde(rename = "sequenceIndex")]
    pub sequence_index: Option<i32>,

    // Quick access
    #[serde(rename = "isPinned")]
    pub is_pinned: bool,
    #[serde(rename = "lastAccessedAt")]
    pub last_accessed_at: Option<i64>,

    // Hierarchy date propagation
    #[serde(rename = "latestChildDate")]
    pub latest_child_date: Option<i64>,

    // Privacy
    #[serde(rename = "isPrivate")]
    pub is_private: Option<bool>,
    #[serde(rename = "privacyReason")]
    pub privacy_reason: Option<String>,
    pub privacy: Option<f64>,    // 0.0-1.0 continuous scale

    // Import tracking
    pub source: Option<String>,

    // Content classification
    #[serde(rename = "contentType")]
    pub content_type: Option<String>,
    #[serde(rename = "associatedIdeaId")]
    pub associated_idea_id: Option<String>,
}
```

### Edge (Struct)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(rename = "type")]
    pub edge_type: EdgeType,
    pub label: Option<String>,
    pub weight: Option<f64>,
    #[serde(rename = "edgeSource")]
    pub edge_source: Option<String>,
    #[serde(rename = "evidenceId")]
    pub evidence_id: Option<String>,
    pub confidence: Option<f64>,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
}
```

### Tag (Struct)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: String,
    pub title: String,
    #[serde(rename = "parentTagId")]
    pub parent_tag_id: Option<String>,
    pub depth: i32,
    // Note: centroid stored in DB as BLOB, not in struct
    #[serde(rename = "itemCount")]
    pub item_count: i32,
    pub pinned: bool,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
}
```

### ItemTag (Struct)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemTag {
    #[serde(rename = "itemId")]
    pub item_id: String,
    #[serde(rename = "tagId")]
    pub tag_id: String,
    pub confidence: f64,
    pub source: String,  // "ai" or "user"
}
```

---

## GraphNode (Internal to Graph.tsx)

Extended node type with layout position for D3 rendering:

```typescript
interface GraphNode extends Node {
  x: number;              // Computed layout position
  y: number;
  renderClusterId: number; // The cluster for card coloring
  displayTitle: string;    // aiTitle || title
  displayContent: string;  // summary || content
  displayEmoji: string;    // emoji or matched fallback
}
```

---

## Dynamic Hierarchy

No fixed L0/L1/L2/L3 levels. Depth is dynamic based on collection size.

| Field | Description |
|-------|-------------|
| `depth` | 0 = Universe (root), increases toward Items |
| `isUniverse` | Exactly one node has this = true |
| `isItem` | true = openable content, false = organizational |
| `parentId` | Structural parent (null for Universe only) |
| `childCount` | Number of direct children |
| `clusterId` | Semantic grouping (for coloring, not navigation) |

See `docs/specs/HIERARCHY.md` for the full mental model.

---

## Visibility Tiers

Content types map to visibility tiers:

| Tier | Content Types | Behavior |
|------|---------------|----------|
| **Visible** (7+) | insight, exploration, synthesis, question, planning, paper, bookmark, code_* | Shown in graph |
| **Supporting** (4) | investigation, discussion, reference, creative | Lazy-loaded in Leaf view |
| **Hidden** (4) | debug, code, paste, trivial | Excluded from graph entirely |

See `docs/specs/PRIVACY.md` for privacy classification details.

---

## Field Comparison: TS vs Rust vs DB

| Field | TypeScript | Rust | SQLite |
|-------|------------|------|--------|
| `tags` | `string[]` | `Option<String>` (JSON) | `TEXT` (JSON) |
| `privacy` | `number` | `Option<f64>` | `REAL` |
| `isPrivate` | `boolean` | `Option<bool>` | `INTEGER` |
| `embedding` | (not exposed) | (not in struct) | `BLOB` |
| `needsClustering` | (not exposed) | (not in struct) | `INTEGER` |

Embeddings are stored in the database but not loaded into Node structs due to size. Use `get_similar_nodes()` for similarity queries.

---

*Last updated: 2026-01-10*
