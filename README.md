# Mycelica

**Visual knowledge graph for connected thinking**

Turn scattered conversations and notes into a navigable knowledge graph with semantic edges. Named after mycelium, the underground fungal network that connects everything.

https://github.com/user-attachments/assets/149d3241-1b93-4269-94c2-10edb9153db3

---

## Why

Knowledge tools mimic file systems: folders, hierarchies, categories.
But thinking is both hierarchical *and* associative — and current tools only show one or the other.

Every insight links to others. Every question branches into more questions. Every concept echoes across domains. Traditional tools bury these connections in separate folders, separate apps, separate contexts.

Mycelica shows structure you can navigate, plus connections that cross category boundaries. Reasoning becomes visible. Your knowledge becomes a living network you can explore, not a graveyard of files you'll never reopen.

Currently handles 3700+ nodes with 14,000+ semantic connections.

---

## Demo

A sample database is included for testing: `mycelica-openAIRE-preview.db`

Switch to it via Settings → Select Database to explore ~2300 medical research papers from OpenAIRE.

NB! DEMO DATABASES ARE ALREADY FULLY PREPROCESSED, NO NEED TO RUN ANY SETUP/PROCESSING IN SETTINGS!

---

## Features

- **Visual Graph Navigation** — Zoomable, pannable D3 canvas with dynamic hierarchy levels
- **AI-Powered Analysis** — Claude generates titles, summaries, tags, and emojis for imported content
- **Smart Clustering** — Multi-method clustering (AI + TF-IDF fallback) organizes items into semantic topics
- **Dynamic Hierarchy** — Auto-creates navigable structure with 8-15 children per level
- **Semantic Connections** — OpenAI embeddings create "Related" edges between similar content:
  ```
  "Rust async debugging"    ←─ 0.89 ─→  "Tokio runtime errors"
  "Consciousness research"  ←─ 0.76 ─→  "Philosophy of mind"
  ```
- **Leaf Reader** — Full-screen reader for conversations (chat bubbles) and notes (markdown)
- **Privacy Filtering** — Showcase/normal modes for safe database exports
- **Import** — Claude conversations JSON, Markdown files
- **Local-First** — SQLite database stays on your machine

---

## Quick Start

```bash
# Install dependencies
npm install

# Run development server
npm run tauri dev
```

### API Keys

Set via **Settings panel** or environment variables:

| Key | Required | Purpose |
|-----|----------|---------|
| `ANTHROPIC_API_KEY` | Yes | AI analysis, clustering, privacy scanning |
| `OPENAI_API_KEY` | No | Semantic embeddings for similarity edges |

---

## Architecture

```
┌─────────────────────────────────────────┐
│   React Frontend                        │
│   TypeScript + D3 + Tailwind + Zustand  │
└──────────────┬──────────────────────────┘
               │ Tauri invoke()
┌──────────────▼──────────────────────────┐
│   Rust Backend                          │
│   Tauri 2 + Tokio + rusqlite            │
└──────────────┬──────────────────────────┘
               │
┌──────────────▼──────────────────────────┐
│   SQLite Database                       │
│   Nodes + Edges + Embeddings + FTS5     │
└─────────────────────────────────────────┘
```

---

## Core Concepts

### Hierarchy

```
Universe (root)
└── Categories (dynamic depth)
    └── Topics
        └── Items (imported content)
```

- **Universe** — Single root node, always exists
- **Categories/Topics** — AI-generated groupings, depth adjusts to content size
- **Items** — Importable content, click to open in full-screen reader

### Processing Pipeline

1. **Import** — Claude conversations or Markdown files
2. **AI Analysis** — Generate titles, summaries, tags, emojis
3. **Clustering** — Group items into semantic topics
4. **Hierarchy Build** — Create navigable structure (8-15 children per level)
5. **Embeddings** — Generate vectors for semantic similarity edges

---

## Development

### Prerequisites

- [Rust toolchain](https://rustup.rs/) (stable)
- Node.js 16+
- Platform build tools (Xcode on macOS, build-essential on Linux)

### Commands

```bash
npm run tauri dev    # Development with hot reload
npm run tauri build  # Production build
```

### Project Structure

```
mycelica/
├── src/                    # React frontend
│   ├── components/
│   │   ├── graph/          # D3 visualization
│   │   ├── leaf/           # Content reader
│   │   ├── sidebar/        # Quick access
│   │   └── settings/       # Configuration
│   ├── stores/             # Zustand state
│   └── hooks/              # Data fetching
│
├── src-tauri/              # Rust backend
│   └── src/
│       ├── commands/       # Tauri command handlers
│       ├── db/             # SQLite layer
│       ├── ai_client.rs    # Anthropic integration
│       ├── hierarchy.rs    # Hierarchy algorithms
│       └── clustering.rs   # Topic clustering
│
└── CLAUDE.md               # Detailed developer docs
```

See [CLAUDE.md](./CLAUDE.md) for comprehensive architecture documentation.

---

## Database Locations

| Environment | Path |
|-------------|------|
| Development | `./data/mycelica.db` |
| macOS | `~/Library/Application Support/com.mycelica.app/` |
| Linux | `~/.local/share/com.mycelica.app/` |
| Windows | `%APPDATA%\Mycelica\` |

---

## Privacy

Mycelica includes AI-powered privacy scanning:

- **Normal mode** — Filters health, relationships, financials, personal complaints
- **Showcase mode** — Strict filtering for demo databases (keeps only technical/philosophical content)

Export shareable databases with private content removed via Settings → Privacy → Export. (I suggest manual checking of nodes after filtering)

---

## License

[AGPL-3.0](LICENSE) — Copyleft. Derivatives must share source.