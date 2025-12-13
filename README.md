# Mycelica

A visual knowledge graph where everything is a node. Conversations, thoughts, and concepts exist as connected nodes with semantic edges discovered by AI.

Named after mycelium — the underground network that connects everything.

## Why

Knowledge tools mimic file systems: folders, hierarchies, tags. 
But thinking is associative, not hierarchical. You don't file a thought — you connect it.

Mycelica makes connections explicit and discoverable.

https://github.com/user-attachments/assets/149d3241-1b93-4269-94c2-10edb9153db3

## Current State

- **Graph Navigation**: Zoomable node graph with tiered layout
- **Semantic Connections**: AI-generated embeddings find related content across categories
- **Smart Clustering**: Imports organize into meaningful groups, preserving project namespaces
- **Everything is a Node**: Items and categories have embeddings, edges, connections
- **Semantic Edges**: Automatic "Related" connections between similar nodes (>70% similarity)

## What It Does Now

1. **Import** Claude conversations (more sources coming)
2. **Process** with AI — generates titles, summaries, tags, and embeddings
3. **Cluster** into semantic groups — AI preserves project names as namespaces
4. **Build hierarchy** — dynamic levels based on collection size
5. **Create edges** — semantic similarity edges connect related content
6. **Navigate** — drill into categories, click ↔ to find similar nodes anywhere

## Stack

- **Frontend**: React + TypeScript + D3 + Zustand
- **Backend**: Rust (Tauri) + SQLite
- **AI**: Anthropic (clustering, titles) + OpenAI (embeddings)
- **Local-first**: Your data stays on your machine

## Quick Start
```bash
# Install dependencies
npm install

# Set API keys (or configure in Settings)
export ANTHROPIC_API_KEY=sk-ant-...
export OPENAI_API_KEY=sk-...

# Run development
npm run tauri dev
```

Requires [Rust](https://rustup.rs/) and [Node.js](https://nodejs.org/).

## Architecture
```
┌─────────────────────────────────────────────────────────────┐
│                     React Frontend                          │
│  ┌───────────────────┐      ┌───────────────────┐          │
│  │    Graph View     │◄────►│    Leaf View      │          │
│  │   (navigation)    │      │  (content reader) │          │
│  └─────────┬─────────┘      └─────────┬─────────┘          │
└────────────┼──────────────────────────┼────────────────────┘
             │ Tauri invoke()           │
┌────────────▼──────────────────────────▼────────────────────┐
│                  Rust Backend + SQLite                      │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐   │
│  │ Hierarchy│  │Clustering│  │Embeddings│  │  Edges   │   │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘   │
└────────────────────────────────────────────────────────────┘
```

## Hierarchy Model

Two things are fixed. Everything else is dynamic.
```
UNIVERSE          ← Single root (always exists)
    │
DYNAMIC LEVELS    ← AI-organized (8-12 children per level)
    │
TOPICS            ← Semantic clusters from content
    │
ITEMS             ← Your actual content
    │
    ▼ click
LEAF VIEW         ← Reader (conversations, notes)
```

Project namespaces preserved:
```
Universe
├── Mycelica          ← Project stays together
│   ├── Architecture
│   └── UX Design
├── AI Research
└── Personal Development
```

## Semantic Connections

Every node has a 1536-dimensional embedding. Similar nodes get "Related" edges automatically.
```
"Rust async debugging" ←──0.89──→ "Tokio runtime errors"
"DPDR research"        ←──0.76──→ "Consciousness studies"
```

Click ↔ on any node to see connections across the entire graph.

## Vision

Make reasoning visible through structure.

- **Current**: Personal knowledge graph with semantic discovery
- **Next**: Import from more sources (bookmarks, notes, emails)
- **Future**: Share nodes with reasoning history intact

Information shouldn't scatter. Context shouldn't die. Connections shouldn't be invisible.

## License

[AGPL-3.0](LICENSE) — Copyleft. Derivatives must share source.
