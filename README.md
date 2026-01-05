# Mycelica

**Visual knowledge graph for connected thinking**

Turn scattered conversations and notes into a navigable knowledge graph with semantic edges. Named after mycelium, the underground fungal network that connects everything.

https://github.com/user-attachments/assets/149d3241-1b93-4269-94c2-10edb9153db3

---

## Try It Now

**5,236 research papers. 25,703 semantic connections. Zero setup.**

1. Download [latest release](https://github.com/Ekats/Mycelica/releases)
2. Grab `mycelica-somatic-neuroscience-trimmed.db` (88 MB)
3. Open Mycelica → Settings → Open database
4. Navigate

Imported from OpenAIRE using:
```
(peripheral neuroplasticity) OR (somatic mutation accumulation) OR (interoception) OR (DNA repair aging)
```

From raw papers to 7-level hierarchy with 25,703 semantic edges in 33 minutes. No manual categorization. Structure emerged from embeddings.

![Graph visualization](https://github.com/user-attachments/assets/695f699f-9902-4d50-a8bb-13371818c96d)

---

## Why

Knowledge tools mimic file systems: folders, hierarchies, categories.
But thinking is both hierarchical *and* associative — and current tools only show one or the other.

Every insight links to others. Every question branches into more questions. Every concept echoes across domains. Traditional tools bury these connections in separate folders, separate apps, separate contexts.

Mycelica shows structure you can navigate, plus connections that cross category boundaries. Reasoning becomes visible. Your knowledge becomes a living network you can explore, not a graveyard of files you'll never reopen.


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
- **Import** — Claude conversations, Markdown files, OpenAIRE papers, Google Keep
- **OpenAIRE Integration** — Query EU Open Research Graph with country/field/year filters, optional PDF download
- **CLI & TUI** — 18 command categories, interactive terminal UI, BibTeX/JSON/Markdown/DOT export
- **Local-First** — SQLite database stays on your machine

---

## Installation

### Pre-built Binaries

Download from [Releases](https://github.com/Ekats/Mycelica/releases):

| Format | Install |
|--------|---------|
| `.deb` | `sudo dpkg -i Mycelica_*.deb` |
| `.rpm` | `sudo rpm -i Mycelica-*.rpm` |
| `.AppImage` | `chmod +x Mycelica_*.AppImage && ./Mycelica_*.AppImage` |
| `.tar.gz` | `tar -xzf Mycelica_*.tar.gz && ./mycelica` |

### Build from Source

```bash
# Prerequisites: Rust toolchain, Node.js 18+, platform build tools

git clone https://github.com/Ekats/Mycelica.git
cd Mycelica
npm install
npm run tauri dev    # Development
npm run tauri build  # Production
```

### API Keys

Set via **Settings panel** or environment variables:

| Key | Required | Purpose |
|-----|----------|---------|
| `ANTHROPIC_API_KEY` | Yes | AI analysis, clustering, privacy scanning |
| `OPENAI_API_KEY` | No | Semantic embeddings for similarity edges |

---

## CLI & TUI

Mycelica includes a headless CLI for scripting, automation, and server use.

### Build the CLI

```bash
cd src-tauri
cargo build --release --bin mycelica-cli
# Binary at: target/release/mycelica-cli
```

### CLI Usage

```bash
mycelica-cli [OPTIONS] <COMMAND>

# Global options
--db <PATH>     # Use specific database
--json          # Output JSON for scripting
-q, --quiet     # Suppress progress output
-v, --verbose   # Detailed logging
```

### Commands

<details>
<summary>**Top-level:**</summary>

| Command | Description |
|---------|-------------|
| `setup` | Interactive first-time setup wizard |
| `tui` | Interactive TUI mode |
| `search <query>` | Global search across all nodes |
| `db` | Database operations |
| `import` | Import data |
| `export` | Export data |
| `node` | Node operations |
| `hierarchy` | Hierarchy operations |
| `process` | AI processing |
| `cluster` | Clustering |
| `embeddings` | Embedding operations |
| `privacy` | Privacy analysis |
| `paper` | Paper operations |
| `config` | Configuration |
| `recent` | Recent nodes |
| `pinned` | Pinned nodes |
| `nav` | Graph navigation |
| `maintenance` | Database maintenance |
| `completions` | Shell completions |

</details>

<details>
<summary>**Import subcommands:**</summary>
| Command | Description |
|---------|-------------|
| `import openaire -q "..."` | Import from OpenAIRE |
| `import claude <file>` | Import Claude JSON |
| `import markdown <path>` | Import Markdown |
| `import keep <zip>` | Import Google Keep |
</details>

<details>
<summary>**Export subcommands:**</summary>
| Command | Description |
|---------|-------------|
| `export bibtex -o file.bib` | BibTeX format |
| `export markdown -o file.md` | Markdown format |
| `export json -o file.json` | JSON format |
| `export graph -o file.dot` | DOT graph |
| `export subgraph <id>` | Export subtree |
</details>

<details>
<summary>OpenAIRE import options</summary>

| Option | Description |
|--------|-------------|
| `-q, --query` | Search query (required) |
| `-c, --country` | Country code (EE, US, etc.) |
| `--fos` | Field of science |
| `--from-year` | Start year |
| `--to-year` | End year |
| `-m, --max` | Max papers [default: 100] |
| `--download-pdfs` | Download PDFs |
| `--max-pdf-size` | Max PDF MB [default: 20] |

</details>

### Examples

```bash
# First-time setup wizard
mycelica-cli setup

# Interactive database picker
mycelica-cli db select

# Import papers from OpenAIRE
mycelica-cli import openaire --query "machine learning" --country EE --max 500

# Import with PDF download
mycelica-cli import openaire --query "neural" --download-pdfs --max-pdf-size 10

# Global search
mycelica-cli search "interoception" --limit 20

# Export as BibTeX
mycelica-cli export bibtex -o ~/papers.bib

# JSON output for scripting
mycelica-cli --json search "neural" | jq '.[].title'

# Launch TUI
mycelica-cli tui

# Generate shell completions
mycelica-cli completions bash >> ~/.bashrc
```

### TUI Mode

Interactive terminal UI with 3-column layout:
```bash
mycelica-cli tui
```

**Layout:** Tree (50%) | Pins + Recents (25%) | Preview (25%)

<details>
<summary>**Hierarchy Navigation:**</summary>
| Key | Action |
|-----|--------|
| `j/k` | Navigate up/down |
| `Enter` | Enter cluster / open item |
| `Backspace` / `-` | Go up one level |
| `Tab` | Cycle panes |
| `/` | Search mode |
| `g/G` | Jump to top/bottom |
| `r` | Reload |
| `q` | Quit |
</details>

<details>
<summary>**Leaf View:**</summary>
| Key | Action |
|-----|--------|
| `Tab` | Cycle: Content → Similar → Edges |
| `n/N` | Next/prev similar node |
| `e` | Edit mode |
| `v` | Open PDF externally |
| `o` | Open URL in browser |
| `Backspace` | Back to hierarchy |
</details>

<details>
<summary>**Edit Mode:**</summary>
| Key | Action |
|-----|--------|
| Arrow keys | Move cursor |
| `Ctrl+S` | Save |
| `Esc` | Cancel |
</details>
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

1. **Import** — Claude conversations, Markdown, OpenAIRE papers, or Google Keep
2. **AI Analysis** — Generate titles, summaries, tags, emojis
3. **Clustering** — Group items into semantic topics
4. **Hierarchy Build** — Create navigable structure (8-15 children per level)
5. **Embeddings** — Generate vectors for semantic similarity edges

---

## Development

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


```


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