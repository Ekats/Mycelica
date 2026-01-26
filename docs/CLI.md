# Mycelica CLI Reference

> Command-line interface for Mycelica knowledge graph operations.

## Installation

### For Development (with GUI)

The CLI must be installed globally AND copied to the sidecar location for the GUI to use it:

```bash
cd src-tauri

# Install globally (requires nightly for CUDA)
cargo +nightly install --path . --bin mycelica-cli --features cuda --force

# Copy to sidecar location (required for GUI to spawn CLI)
cp ~/.cargo/bin/mycelica-cli binaries/mycelica-cli-x86_64-unknown-linux-gnu
```

Platform-specific sidecar names:
- Linux: `mycelica-cli-x86_64-unknown-linux-gnu`
- macOS Intel: `mycelica-cli-x86_64-apple-darwin`
- macOS ARM: `mycelica-cli-aarch64-apple-darwin`
- Windows: `mycelica-cli-x86_64-pc-windows-msvc.exe`

### CLI-Only (no GUI)

```bash
cd src-tauri
cargo +nightly build --release --bin mycelica-cli --features cuda
# Binary at: target/release/mycelica-cli
```

Or install globally without sidecar:
```bash
cargo +nightly install --path src-tauri --bin mycelica-cli --features cuda
```

---

## Global Options

```
mycelica-cli [OPTIONS] <COMMAND>

Options:
  -d, --db <PATH>     Use specific database file
  -j, --json          Output JSON format (for scripting)
  -q, --quiet         Suppress progress output
  -v, --verbose       Verbose output
  -h, --help          Print help
  -V, --version       Print version
```

---

## Command Categories

### db - Database Operations

```bash
mycelica-cli db stats              # Show database statistics
mycelica-cli db path               # Print database file path
mycelica-cli db select             # Select database interactively
mycelica-cli db new <PATH>         # Create a new database
mycelica-cli db export <PATH>      # Export trimmed database (no PDFs)
mycelica-cli db tidy               # Vacuum, fix counts, prune edges
```

### import - Data Import

```bash
# Import Claude conversations (JSON export)
mycelica-cli import claude <FILE>

# Import ChatGPT conversations (JSON export)
mycelica-cli import chatgpt <FILE>

# Import markdown files
mycelica-cli import markdown <FILE>...
mycelica-cli import markdown ./notes/*.md

# Import Google Keep Takeout archive
mycelica-cli import keep <ZIP_FILE>

# Import scientific papers from OpenAIRE
mycelica-cli import openaire <QUERY> [--max-results N]
mycelica-cli import openaire "machine learning" --max-results 100

# Import source code (Rust, TypeScript, JavaScript, Python, C, Markdown, RST)
mycelica-cli import code <PATH>              # Import codebase
mycelica-cli import code src/ --update       # Incremental update (delete + reimport + embeddings + edges)
mycelica-cli import code . --language rust   # Filter by language
```

### node - Node Operations

```bash
mycelica-cli node get <ID>                    # Get node details
mycelica-cli node search <QUERY>              # Full-text search
mycelica-cli node list [--items-only]         # List all nodes
mycelica-cli node set-private <ID> <true|false>  # Set privacy status
```

### hierarchy - Hierarchy Operations

```bash
mycelica-cli hierarchy build       # Build hierarchy from clusters
mycelica-cli hierarchy rebuild     # Full rebuild (clustering + hierarchy)
mycelica-cli hierarchy clear       # Clear hierarchy (keep items)
mycelica-cli hierarchy status      # Show hierarchy statistics
```

### process - AI Processing

```bash
mycelica-cli process run           # AI-analyze unprocessed nodes
mycelica-cli process status        # Show processing statistics
mycelica-cli process reset         # Mark all nodes as unprocessed
```

### cluster - Clustering

```bash
mycelica-cli cluster run           # Cluster items needing clustering
mycelica-cli cluster recluster     # Force recluster all items
mycelica-cli cluster status        # Show clustering statistics
```

### embeddings - Embedding Operations

```bash
mycelica-cli embeddings status                # Show embedding statistics
mycelica-cli embeddings regenerate            # Regenerate all embeddings
mycelica-cli embeddings clear                 # Clear all embeddings
mycelica-cli embeddings local                 # Show local embeddings status
mycelica-cli embeddings local on              # Enable local embeddings (default)
mycelica-cli embeddings local off             # Disable local embeddings
```

### privacy - Privacy Operations

```bash
mycelica-cli privacy scan-items [--force]  # AI score all items (25/batch)
mycelica-cli privacy status                 # Show privacy statistics
mycelica-cli privacy set <ID> <0.0-1.0>     # Set node privacy score
mycelica-cli privacy reset                  # Clear all privacy scores
```

### paper - Paper Operations

```bash
mycelica-cli paper search <QUERY>      # Search OpenAIRE (preview)
mycelica-cli paper list                # List imported papers
mycelica-cli paper get <ID>            # Get paper details
mycelica-cli paper download <ID>       # Download PDF for paper
mycelica-cli paper open <ID>           # Open PDF in external viewer
```

### config - Configuration

```bash
mycelica-cli config list                   # List all settings
mycelica-cli config get <KEY>              # Get setting value
mycelica-cli config set anthropic-key <KEY>  # Set Anthropic API key
mycelica-cli config set openai-key <KEY>   # Set OpenAI API key
mycelica-cli config set local-embeddings <true|false>
```

### nav - Graph Navigation

```bash
mycelica-cli nav ls <ID>               # List children of a node (use "root" for Universe)
mycelica-cli nav ls <ID> --long        # Long format with details
mycelica-cli nav tree <ID>             # Show subtree
mycelica-cli nav tree <ID> --depth 5   # Show subtree with custom depth
mycelica-cli nav path <FROM> <TO>      # Find path between nodes
mycelica-cli nav edges <ID>            # Show edges for a node
mycelica-cli nav edges <ID> --type calls --direction incoming  # Filter edges
mycelica-cli nav similar <ID>          # Find similar nodes by embedding
mycelica-cli nav folder <PATH>         # Browse code by file path
```

### recent - Recent Nodes

```bash
mycelica-cli recent list [--limit N]   # Show recently accessed nodes
mycelica-cli recent clear              # Clear recent history
```

### pinned - Pinned Nodes

```bash
mycelica-cli pinned list               # Show pinned nodes
mycelica-cli pinned add <ID>           # Pin a node
mycelica-cli pinned remove <ID>        # Unpin a node
```

### maintenance - Database Maintenance

```bash
mycelica-cli maintenance wipe              # Delete ALL data
mycelica-cli maintenance reset-ai          # Reset AI processing
mycelica-cli maintenance reset-clusters    # Reset clustering data
mycelica-cli maintenance reset-privacy     # Reset privacy scores
mycelica-cli maintenance clear-embeddings  # Clear embeddings
mycelica-cli maintenance clear-hierarchy   # Clear hierarchy
mycelica-cli maintenance clear-tags        # Clear tags
mycelica-cli maintenance delete-empty      # Delete empty nodes
mycelica-cli maintenance vacuum            # Vacuum database
mycelica-cli maintenance fix-counts        # Fix child counts
mycelica-cli maintenance fix-depths        # Fix node depths
mycelica-cli maintenance prune-edges       # Prune dead edges
mycelica-cli maintenance index-edges          # Index edges by parent
mycelica-cli maintenance merge-small-categories  # Merge small siblings by embedding similarity
mycelica-cli maintenance repair-code-tags <PATH>  # Restore file_path metadata to code nodes
```

### export - Data Export

```bash
# Export to JSON
mycelica-cli export json [--output FILE]

# Export to Markdown
mycelica-cli export markdown [--output DIR]

# Export BibTeX (papers only)
mycelica-cli export bibtex [--output FILE]

# Export graph structure
mycelica-cli export graph [--output FILE]
```

### analyze - Code Analysis

```bash
# Create/refresh Calls edges between functions
mycelica-cli analyze code-edges
mycelica-cli analyze code-edges --dry-run    # Preview without writing
mycelica-cli analyze code-edges --path src/  # Limit to path
```

### code - Code Intelligence

```bash
# View source code for a code node (reads actual file)
mycelica-cli code show <ID>
```

### Special Commands

```bash
# Interactive setup wizard
mycelica-cli setup

# Non-interactive setup (auto-confirm, skip prompts)
mycelica-cli setup --yes

# Interactive TUI
mycelica-cli tui

# Full-text search
mycelica-cli search <QUERY>

# Generate shell completions
mycelica-cli completions <bash|zsh|fish|powershell>
```

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `ANTHROPIC_API_KEY` | Anthropic API key (for AI processing) |
| `OPENAI_API_KEY` | OpenAI API key (for embeddings) |
| `MYCELICA_DB` | Default database path |

---

## Examples

### Complete Setup Workflow

```bash
# 1. Import conversations
mycelica-cli import claude ~/Downloads/conversations.json

# 2. AI processing (generates titles, summaries, tags)
mycelica-cli process run

# 3. Generate embeddings
mycelica-cli embeddings regenerate

# 4. Cluster similar items
mycelica-cli cluster run

# 5. Build hierarchy
mycelica-cli hierarchy build

# 6. Check status
mycelica-cli db stats
```

### Using the Setup Wizard

```bash
# Interactive setup (does all of the above)
mycelica-cli setup
```

### Scientific Paper Workflow

```bash
# Import papers
mycelica-cli import openaire "neural networks" --max-results 200

# Build hierarchy
mycelica-cli hierarchy rebuild

# Export BibTeX
mycelica-cli export bibtex --output papers.bib
```

### Scripting with JSON Output

```bash
# Get stats as JSON
mycelica-cli --json db stats | jq '.totalItems'

# Search and process
mycelica-cli --json node search "rust" | jq '.[].title'
```

---

## CLI vs GUI Features

### Fully Implemented in Both
- Import (all 5 types: claude, markdown, keep, openaire, code)
- Clustering (run, recluster)
- Hierarchy build/rebuild
- Embeddings (regenerate, clear, status)
- Settings/config management
- Privacy scoring

### CLI-Only Features
- BibTeX export
- JSON/Markdown/Graph export
- Interactive TUI
- Setup wizard
- Shell completions
- Code call graph analysis (`analyze code-edges`)
- Folder-based code navigation (`nav folder`)

### GUI-Only Features
- Visual graph navigation
- Leaf content viewer
- Real-time progress events
- Manual hierarchy editing (unsplit_node, cluster_hierarchy_level)
- Paper PDF viewer integration
- Hide private nodes toggle

---

## Cancellation

CLI operations can be cancelled with Ctrl+C. This is different from GUI which uses cancellation flags.

---

*Last updated: 2026-01-25*
