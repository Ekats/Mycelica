# CLAUDE.md

> Read this first. Read `docs/specs/HIERARCHY.md` for the mental model.

## What Is Mycelica?

A visual knowledge graph. Replace browser tabs with a node graph — pages, thoughts, contexts exist as connected nodes with explicit reasoning edges. Named after mycelium — the underground network connecting everything.

**Core insight**: Knowledge tools should mirror synaptic architecture, not file systems.

---

## Quick Start

```bash
npm run tauri dev      # Development (frontend + Rust backend)
npm run tauri build    # Production build
```

**Running dev:**
1. Clean first: `pkill -f "mycelica|tauri|vite"`
2. Use `run_in_background: true` — never shell `&`
3. One dev session at a time

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        React Frontend                                │
│  ┌─────────────────────────┐    ┌─────────────────────────────┐     │
│  │      Graph Mode         │◄──►│       Leaf Mode             │     │
│  │   (navigation)          │    │   (content reader)          │     │
│  └────────────┬────────────┘    └──────────────┬──────────────┘     │
└───────────────┼─────────────────────────────────┼───────────────────┘
                │ invoke()                        │
┌───────────────▼─────────────────────────────────▼───────────────────┐
│              Rust Backend (Tauri) + SQLite                          │
└─────────────────────────────────────────────────────────────────────┘
```

**Phase 1 (current):** Tauri app. Local-first, proves the UX.
**Phase 2 (future):** Firefox fork. Leaf becomes Gecko viewport.

### Stack

- **Frontend:** React + TypeScript + Tailwind + Zustand
- **Backend:** Rust (Tauri) + SQLite + rusqlite
- **Rendering:** SVG-based graph (D3.js)
- **Icons:** Lucide React

---

## Hierarchy (CRITICAL)

**Two things are fixed. Everything else is dynamic.**

```
┌─────────────────────────────────────────────────────────┐
│  UNIVERSE — Single root node. Always exists.            │
└────────────────────────┬────────────────────────────────┘
                         │
              ┌──────────▼──────────┐
              │   DYNAMIC LEVELS    │
              │   (as many as the   │
              │   collection needs) │
              └──────────┬──────────┘
                         │
┌────────────────────────▼────────────────────────────────┐
│  ITEMS — Imported content. Conversations, notes, etc.  │
└────────────────────────┬────────────────────────────────┘
                         │ click
                         ▼
┌─────────────────────────────────────────────────────────┐
│  LEAF — Reader mode. NOT a graph level.                │
│  Integrated viewer for any content type.               │
│  Future: Firefox/Gecko viewport.                       │
└─────────────────────────────────────────────────────────┘
```

### Dynamic Level Creation

| Item Count | Structure |
|------------|-----------|
| < 30 | Universe → Items (no middle levels) |
| 30-150 | Universe → Topics → Items |
| 150-500 | Universe → Domains → Topics → Items |
| 500+ | Add more levels as needed |

### Key Database Fields

```sql
depth INTEGER        -- 0 = Universe, increases toward Items
parent_id TEXT       -- Structural parent
cluster_id TEXT      -- Semantic grouping (for clustering)
is_universe BOOLEAN  -- True for single root
is_item BOOLEAN      -- True = openable in Leaf reader
```

### Workflow

```
1. Import → Creates Items (is_item=true, no parent)
2. Cluster → Assigns cluster_id via AI/TF-IDF (fine-grained topics, e.g., 206 topics)
3. Build Hierarchy → Creates intermediate levels, sets parent_id, creates Universe
4. Recursive Grouping → AI groups topics into 8-15 parent categories until navigable
```

**Key constraint:** Each level should have 8-15 children for navigability.

**Read `docs/specs/HIERARCHY.md` for full details.**

---

## Project Structure

```
mycelica/
├── CLAUDE.md                    # THIS FILE
├── docs/specs/                  # Implementation specs
│   ├── HIERARCHY.md             # ← Read this for mental model
│   ├── TYPES.md                 # Rust/TS types
│   ├── COMMANDS.md              # Tauri commands
│   ├── SCHEMA.md                # SQLite schema
│   ├── ALGORITHMS.md            # Connection discovery, search, clustering
│   └── IMPORT.md                # Import pipeline
│
├── src/                         # React frontend
│   ├── components/
│   │   ├── graph/               # Canvas, Node, Edge
│   │   └── leaf/                # Leaf reader components
│   ├── stores/graphStore.ts     # Zustand state
│   └── types/graph.ts           # TypeScript types
│
├── src-tauri/                   # Rust backend
│   ├── src/
│   │   ├── lib.rs
│   │   ├── commands/            # Tauri commands
│   │   └── db/                  # SQLite layer
│   └── Cargo.toml
│
└── archive/                     # Old Python codebase (reference only)
```

---

## Tauri Commands

```typescript
import { invoke } from "@tauri-apps/api/core";

// Graph navigation
await invoke("get_universe");           // Get root node
await invoke("get_children", { parentId }); // Get children of any node
await invoke("get_children_flat", { parentId }); // Skip single-child chains
await invoke("get_items");              // Get all leaf-openable content

// CRUD
await invoke("create_node", { node });
await invoke("update_node", { node });
await invoke("delete_node", { id });

// Hierarchy building (step-by-step)
await invoke("run_clustering", { useAi: true }); // Assign cluster_id to items
await invoke("build_hierarchy");        // Create initial intermediate levels

// Hierarchy building (full recursive - RECOMMENDED)
await invoke("build_full_hierarchy", { runClustering: true });
// Runs: clustering → hierarchy → recursive AI grouping until navigable

// Manual grouping (for targeted hierarchy adjustment)
await invoke("cluster_hierarchy_level", { parentId }); // Group children into 8-15 categories

// Leaf reader
await invoke("get_leaf_content", { itemId }); // Full content for reader

// Search
await invoke("search", { query, mode }); // mode: 'semantic' | 'keyword' | 'combined'
```

---

## UI Modes

### Graph Mode
- Canvas with nodes and edges
- Click non-item node → zoom into children
- Click item node → switch to Leaf mode

### Leaf Mode
- Full content display
- Conversation → chat transcript
- Note → rendered markdown
- Bookmark → webview (future: Gecko)
- Back → return to graph

```typescript
type ViewMode = 'graph' | 'leaf';

interface ViewState {
  mode: ViewMode;
  focusedNodeId: string;    // Graph: which node we're inside
  openItemId?: string;      // Leaf: which item is open
}
```

---

## UI/UX Principles

**Philosophy:** Remove, don't add.

- **Colors:** white/gray-50/gray-100 bg, gray-700/500 text, amber-600 accent
- **Borders:** Near-invisible `rgba(0,0,0,0.06)`
- **Shadows:** `shadow-sm` default
- **Typography:** Inter/system-ui, `text-sm`
- **Motion:** `transition-all duration-200`
- **Icons:** Lucide React, 18-20px

---

## When To Read Which Spec

| Task | Read |
|------|------|
| Understanding hierarchy | `docs/specs/HIERARCHY.md` |
| Data types | `docs/specs/TYPES.md` |
| Tauri commands (85+) | `docs/specs/COMMANDS.md` |
| Database schema (6 tables) | `docs/specs/SCHEMA.md` |
| Privacy & content classification | `docs/specs/PRIVACY.md` |
| Full architecture overview | `docs/ARCHITECTURE.md` |

---

## Code Style

- **Rust:** `thiserror` for errors, `rusqlite` for DB, type everything
- **TypeScript:** Strict mode, functional components, Zustand
- **Both:** Small functions, minimal comments

---

## Common Mistakes

❌ Treating Leaf as a graph level (it's a view mode)
❌ Fixed L0/L1/L2/L3 levels (depth is dynamic)
❌ Building hierarchy before clustering
❌ Importing to wrong depth (items have `is_item=true`)
❌ Using `is_private` boolean (use `privacy` float 0.0-1.0)
❌ Ignoring content classification (affects graph visibility)

---

## Key Systems

### Content Classification
- 13 content types across 3 visibility tiers
- **Visible**: insight, exploration, synthesis, question, planning
- **Supporting**: investigation, discussion, reference, creative
- **Hidden**: debug, code, paste, trivial

### Privacy Scoring
- Scale: 0.0 (private) → 1.0 (public)
- Propagates from categories to descendants
- Used for shareable exports

### Multi-Path Associations
- Items can belong to multiple categories via `belongs_to` edges
- Weight field indicates association strength

### Recent Notes Protection
- Notes in "Recent Notes" can be excluded from AI processing
- Toggle via `set_protect_recent_notes()`

### Local Embeddings
- Option to generate embeddings on-device vs OpenAI API
- Toggle via `set_use_local_embeddings()`

### Dev Console
- Built-in frontend dev console for debugging
- Logs hierarchy operations, AI progress, errors