# Mycelica Architecture

> Comprehensive map of the codebase. Last updated: 2026-01-08

## Overview

**Mycelica** is a visual knowledge graph that replaces browser tabs with a connected node graph. Named after mycelium — the underground network connecting everything.

**Tech Stack:**
- Frontend: React 19, TypeScript, Tailwind CSS 4, Zustand 5, D3.js 7
- Backend: Rust, Tauri 2, rusqlite, Tokio
- Database: SQLite with FTS5 full-text search

```
┌─────────────────────────────────────────────────────────────────┐
│                      FRONTEND (React + D3.js)                    │
│  • D3 SVG graph rendering with zoom/pan                         │
│  • Zustand state management                                      │
│  • Dynamic hierarchy navigation                                  │
│  • Error boundaries for graceful degradation                     │
└──────────────────────────────┬──────────────────────────────────┘
                               │ invoke()
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                       BACKEND (Tauri + Rust)                     │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐              │
│  │ commands/   │  │ clustering  │  │ hierarchy   │              │
│  │ graph.rs    │  │ .rs         │  │ .rs         │              │
│  └─────────────┘  └─────────────┘  └─────────────┘              │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐              │
│  │ commands/   │  │ classifi-   │  │ local_      │              │
│  │ privacy.rs  │  │ cation.rs   │  │ embeddings  │              │
│  └─────────────┘  └─────────────┘  └─────────────┘              │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐              │
│  │ ai_client   │  │ similarity  │  │ db/         │              │
│  │ .rs         │  │ .rs         │  │ schema.rs   │              │
│  └─────────────┘  └─────────────┘  └─────────────┘              │
│                         │                                        │
│                         ▼                                        │
│              ┌───────────────────┐                               │
│              │      SQLite       │                               │
│              │  8 tables + FTS5  │                               │
│              └───────────────────┘                               │
└─────────────────────────────────────────────────────────────────┘
```

---

## Hierarchy Model (DYNAMIC)

**Two things are fixed. Everything else is dynamic.**

```
UNIVERSE (depth=0)     ← Single root, always exists
    │
    ▼
DYNAMIC LEVELS         ← As many as the collection needs (tiered limits: 10/25/50/100/150)
    │
    ▼
ITEMS (is_item=true)   ← Imported content (conversations, notes)
    │
    ▼ click
LEAF MODE              ← NOT a graph level. Reader view for content.
```

| Item Count | Structure |
|------------|-----------|
| < 30 | Universe → Items |
| 30-150 | Universe → Topics → Items |
| 150-500 | Universe → Domains → Topics → Items |
| 500+ | Add more levels as needed |

**Navigation:** Click topic → drill into children. Click item → open in Leaf reader. Back → go up.

---

## Data Flow

### Processing Pipeline

```
Import → Privacy Scan → AI Processing → Clustering → Hierarchy → Embeddings
   │          │              │              │            │           │
   ▼          ▼              ▼              ▼            ▼           ▼
is_item    privacy      is_processed    cluster_id   parent_id   embedding
source     privacy_reason  ai_title                  depth       (BLOB)
           (0.0-1.0)       summary
                           content_type
```

### IPC Pattern

```
React → invoke('command', params) → Tauri IPC → Rust Handler → SQLite → JSON → Zustand → Re-render
```

### Navigation

```
Graph: Click Topic → navigateToNode() → get_children() → deeper
Graph: Click Item → openLeaf() → get_leaf_content() → Leaf Mode
Leaf: Back → closeLeaf() → Graph Mode
```

### Edge Loading (View-Based)

```
View Change → loadEdgesForView(parentId) → get_edges_for_view() → edges for current view only
```

Edges are NOT loaded at startup. Instead, `useGraph.ts` loads edges per-view:
- `loadEdgesForView(parentId)` fetches edges where both endpoints share the parent
- Called on navigation via `Graph.tsx:385`
- Uses indexed lookup on `(source_parent_id, target_parent_id)`

### Hierarchy Building

```
run_clustering() → assign cluster_id
build_hierarchy() → create topics from clusters
cluster_hierarchy_level() → recursively group to 8-15 children
```

---

## Frontend (`src/`)

### Components

#### `components/graph/Graph.tsx` (~3300 lines)
Main graph visualization - the core of the application.

**Rendering:**
- D3.js SVG with zoom/pan via d3-zoom
- Two modes: cards (zoomed in) and bubbles (zoomed out)
- Semantic zoom transitions

**Node Visuals:**
- Violet (#5b21b6) base shadow = "notes exist here"
- Cluster-colored stacking shadows by child count
- "NOTE" badge for items

**Connection Coloring (on select/hover):**
- BFS traversal computes hop distances from active node
- Direct (1 hop): red→green based on edge weight, opacity 0.9
- Chain (2+ hops): darker red, opacity 0.7
- Unconnected: muted gray, opacity 0.15-0.3

**Key Patterns:**
- `connectionMap`: Map<nodeId, {weight, distance}> via BFS
- `activeNodeIdRef`/`connectionMapRef`: refs for event handler access
- `.attr('opacity')` not `.style('opacity')` for SVG reliability
- `interrupt()` before transitions to prevent cancellation

#### `components/leaf/LeafView.tsx`
Content reader mode.
- Fetches full content via `get_leaf_content`
- Renders markdown or conversation format

#### `components/leaf/ConversationRenderer.tsx`
Chat message formatting with Human/Assistant bubbles.

#### `components/sidebar/Sidebar.tsx`
- Pinned nodes (favorites)
- Recent nodes (touch on click)
- Search

#### `components/settings/SettingsPanel.tsx`
Admin panel: API keys, DB stats, hierarchy operations, privacy filtering, tidy database.

#### `components/ErrorBoundary.tsx`
Reusable error boundary for graceful component failures.

### State (`stores/graphStore.ts`)

```typescript
// Core
nodes: Map<string, Node>
edges: Map<string, Edge>
viewport: { x, y, zoom }
activeNodeId: string | null

// View mode
viewMode: 'graph' | 'leaf'
leafNodeId: string | null

// Navigation
currentDepth: number
maxDepth: number
currentParentId: string | null
breadcrumbs: BreadcrumbEntry[]

// Processing status
aiProgress: { current, total, status }

// Actions
navigateToNode(), navigateBack(), navigateToRoot(), jumpToNode()
openLeaf(), closeLeaf()
```

### Types (`types/graph.ts`)

```typescript
Node {
  id, type, title, aiTitle, summary, emoji, tags
  depth, isItem, isUniverse, parentId, childCount, clusterId
  position: { x, y }
  isPinned, lastAccessedAt, latestChildDate
  conversationId, sequenceIndex
  privacy, privacyReason, isPrivate (deprecated)
  contentType, associatedIdeaId
  source
}

Edge { id, source, target, type, weight, edgeSource, evidenceId, confidence }
```

---

## Backend (`src-tauri/src/`)

### Database (`db/`)

**schema.rs** - SQLite tables (8 tables):

```sql
nodes (32 columns) - All content
edges (12 columns) - Relationships (includes source_parent_id, target_parent_id for view-based loading)
papers - Scientific paper metadata and PDFs
tags - Persistent tag definitions
item_tags - Item-to-tag assignments
learned_emojis - AI emoji mappings
db_metadata - Pipeline state tracking
fos_edges (DEPRECATED) - Field of Science edge grouping
nodes_fts - FTS5 virtual table
```

See `docs/specs/SCHEMA.md` for full schema.

**models.rs** - Rust structs: `Node`, `Edge`, `Tag`, `ItemTag`, `NodeType`, `EdgeType`

### Commands

**`commands/graph.rs`** (~2800 lines, ~90 commands)
- Node/Edge CRUD
- Hierarchy navigation
- AI processing
- Import operations (Claude, Markdown, Keep, OpenAIRE)
- Paper operations
- Quick access (pinned/recent)
- Database management

**`commands/privacy.rs`** (~1300 lines, ~12 commands)
- Privacy scoring (0.0-1.0)
- Category-level analysis with propagation
- Shareable database export

**`commands/settings.rs`** (~120 lines, ~12 commands)
- API key management
- Pipeline state
- Processing statistics
- Local embeddings toggle

See `docs/specs/COMMANDS.md` for full API reference.

### Processing Modules

**ai_client.rs** - Claude API
- Generates: aiTitle, summary, tags, emoji, contentType
- Uses Claude Haiku (claude-haiku-4-5-20251001)
- Token usage tracking

**clustering.rs**
- Embedding-based cosine similarity clustering (deterministic, local)
- AI only for naming clusters (NAMING_BATCH_SIZE=30)
- FOS pre-grouping for papers
- Multi-path via BelongsTo edges
- Cancellation support

**hierarchy.rs**
- Phase 1: Cluster items into topics
- Phase 2: Recursively group using tiered limits (10/25/50/100/150 by depth)
- Cancellation support, progress events

**classification.rs**
- Pattern-based content type classification
- 14 content types across 3 visibility tiers (added: paper)
- No AI required (fast, consistent)

**openaire.rs**
- Scientific paper import from OpenAIRE API
- PDF download and storage
- FOS (Field of Science) extraction

**similarity.rs** - Embedding-based semantic search
- Cosine similarity comparison
- Caching with configurable TTL

**local_embeddings.rs** - Local embedding generation
- Candle ML for on-device inference
- Sentence transformers model

**import.rs**
- `import_claude_conversations()` - Claude JSON export
- `import_markdown_files()` - Markdown files
- `import_google_keep()` - Google Keep Takeout

**settings.rs** - Configuration persistence
- API keys (Anthropic, OpenAI)
- Custom database path
- Processing statistics
- Recent Notes protection

**tags.rs** - Persistent tag system for clustering guidance

### Entry Points

**lib.rs** - Tauri setup, command registration, AppState initialization
**main.rs** - Minimal wrapper

### CLI (`bin/cli.rs`)

Standalone command-line interface (~5000 lines, 77 subcommands).

```
mycelica-cli [OPTIONS] <COMMAND>

Commands:
  db          Database operations (stats, path, switch)
  import      Import data (claude, markdown, keep, openaire)
  node        Node operations (get, search, set-private)
  hierarchy   Hierarchy operations (build, rebuild, clear)
  process     AI processing (run, status, reset)
  cluster     Clustering (run, recluster, fos)
  embeddings  Embedding operations (regenerate, clear, status)
  privacy     Privacy operations (scan-items, status, set, reset)
  paper       Paper operations (search, list, get, download)
  config      Configuration (get, set, list)
  nav         Navigation (recent, pinned)
  maintenance Database maintenance (tidy, clear-*)
  export      Export data (json, markdown, bibtex, graph)
  setup       Setup wizard
  tui         Interactive TUI
  search      Full-text search
  completions Shell completions
```

See [CLI.md](CLI.md) for complete reference.

---

## Database Locations

| Platform | Path |
|----------|------|
| Linux | `~/.local/share/com.mycelica.app/mycelica.db` |
| macOS | `~/Library/Application Support/com.mycelica.app/mycelica.db` |
| Windows | `%APPDATA%\com.mycelica.app\mycelica.db` |

Settings stored in `settings.json` alongside the database.

---

## API Key Handling

```
1. Check ANTHROPIC_API_KEY environment variable
2. Fall back to settings.json stored key
3. User can configure in Settings panel
```

OpenAI key optional (for embeddings API vs local).

---

## File Tree

```
src/
├── App.tsx                     # Root layout + error boundary
├── main.tsx                    # React bootstrap
├── components/
│   ├── graph/
│   │   ├── Graph.tsx           # Main visualization (3300+ lines)
│   │   └── SimilarNodesPanel.tsx
│   ├── leaf/
│   │   ├── LeafView.tsx        # Content reader
│   │   └── ConversationRenderer.tsx
│   ├── sidebar/Sidebar.tsx
│   ├── settings/SettingsPanel.tsx
│   └── ErrorBoundary.tsx       # Reusable error boundary
├── stores/graphStore.ts        # Zustand state
├── types/graph.ts              # TypeScript interfaces
└── hooks/useGraph.ts           # Data fetching

src-tauri/src/
├── lib.rs                      # Tauri setup, AppState
├── main.rs                     # Entry point
├── commands/
│   ├── graph.rs                # ~70 commands
│   ├── privacy.rs              # Privacy commands
│   ├── settings.rs             # Settings commands
│   └── mod.rs
├── db/
│   ├── schema.rs               # SQLite tables + migrations
│   ├── models.rs               # Rust structs
│   └── mod.rs                  # DB interface
├── ai_client.rs                # Claude API
├── clustering.rs               # TF-IDF + AI clustering
├── classification.rs           # Content type classification
├── hierarchy.rs                # Dynamic levels
├── similarity.rs               # Semantic search
├── local_embeddings.rs         # On-device embeddings
├── import.rs                   # Data import
├── settings.rs                 # Config persistence
└── tags.rs                     # Persistent tag system
```

---

## Key Patterns

1. **Dynamic Hierarchy** - depth-based, not fixed L0/L1/L2
2. **BFS Connection Coloring** - hop distance → opacity/color
3. **Refs for Handlers** - `activeNodeIdRef`, `connectionMapRef`
4. **SVG Attr Opacity** - `.attr('opacity')` not `.style()`
5. **Interrupt-Before-Transition** - prevent animation cancellation
6. **Multi-path Associations** - items can belong to multiple clusters via `belongs_to` edges
7. **Progress Events** - Tauri emitter for long operations (`ai-progress`, `hierarchy-log`, `privacy-progress`)
8. **Error Boundaries** - Component-level failure isolation
9. **Privacy Scoring** - Continuous 0.0-1.0 scale for granular filtering
10. **Content Classification** - 3 visibility tiers (visible/supporting/hidden)

---

## Event System

Frontend listens to backend events for real-time updates:

```typescript
import { listen } from '@tauri-apps/api/event';

// Long-running operations
listen('ai-progress', handler);       // AI processing progress
listen('hierarchy-log', handler);     // Hierarchy building logs
listen('privacy-progress', handler);  // Privacy scanning progress
listen('embedding-progress', handler); // Embedding generation
listen('reclassify-progress', handler); // Reclassification progress
```

---

*See other docs in `docs/specs/` for detailed specifications.*
