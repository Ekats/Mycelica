# Mycelica

A visual knowledge graph for exploring and managing your Claude AI conversation history.

> *Named after mycelium - the underground fungal network connecting everything together, just like your conversations.*

## What is this?

Mycelica imports your Claude conversation export and visualizes it as an interactive spatial graph. Conversations are automatically clustered by topic, letting you:

- See all your research at a glance
- Find related discussions visually
- Navigate from high-level overview to individual messages
- Click to continue any conversation in Claude

## Features

**Implemented:**
- Import Claude conversation exports (JSON)
- TF-IDF clustering with automatic topic detection
- Interactive D3.js force-directed graph
- Prezi-style semantic zoom (cluster labels when zoomed out, node labels when zoomed in)
- Counter-scaled text (readable at any zoom level)
- Focus Mode - click a node to highlight only its connections
- Topic-based emoji indicators
- Expandable cluster legend with conversation lists
- Search bar
- "Open in Claude" integration

**Tech Stack:**
- Backend: Python + FastAPI + SQLite
- Frontend: React + TypeScript + D3.js
- Analysis: TF-IDF embeddings with agglomerative clustering

## Quick Start

### Prerequisites
- Python 3.10+
- Node.js 18+
- A Claude conversation export (JSON file from claude.ai settings)

### Backend Setup

```bash
cd backend
python -m venv venv
venv\Scripts\activate  # Windows
# or: source venv/bin/activate  # Mac/Linux

pip install -r requirements.txt

# Start the server
python -m uvicorn app.main:app --reload --port 8000
```

### Frontend Setup

```bash
cd frontend
npm install
npm run dev
```

Open http://localhost:5173

### Import Your Conversations

1. Export your Claude conversations from claude.ai (Settings > Export Data)
2. Place the JSON file as `backend/conversations.json`
3. POST to `/analyze` to process:
   ```bash
   curl -X POST http://localhost:8000/analyze
   ```
4. Refresh the frontend

## Usage

**Navigation:**
- Scroll to zoom in/out
- Drag to pan
- Click cluster labels to zoom to cluster
- Click nodes to select and view details

**Focus Mode:**
- Select a node
- Click "Focus Mode" in the detail panel
- Only connected nodes remain visible
- Great for seeing what relates to a specific conversation

**Legend:**
- Click cluster names to expand/collapse
- Shows all conversations in each cluster sorted by size
- Click a conversation to select it

## Project Structure

```
Mycelica/
├── backend/
│   ├── app/
│   │   ├── main.py        # FastAPI server
│   │   └── analysis.py    # TF-IDF clustering & graph building
│   ├── requirements.txt
│   └── conversations.json # Your exported data (not committed)
├── frontend/
│   ├── src/
│   │   ├── App.tsx        # Main visualization component
│   │   └── App.css        # Styles
│   └── package.json
└── planning/              # Design docs (gitignored)
```

## How It Works

1. **Import**: Parse Claude's JSON export
2. **Embed**: Generate TF-IDF vectors from conversation content
3. **Cluster**: Agglomerative clustering based on cosine similarity
4. **Keywords**: Extract top terms per cluster for labeling
5. **Graph**: Build nodes (conversations) and edges (similar pairs)
6. **Visualize**: D3.js force-directed layout with semantic zoom

## Development

**Backend reload issues on Windows:**
Uvicorn's `--reload` sometimes detects changes but doesn't actually restart. If your changes aren't taking effect:
1. Kill the server (Ctrl+C)
2. Restart fresh
3. Re-run `/analyze`
4. Hard refresh the frontend

See `planning/reload-troubleshooting.md` for details.

## Roadmap

- [ ] Browser extension for real-time sync (no manual export)
- [ ] Automatic branch detection within conversations
- [ ] Semantic search
- [ ] Context injection (Claude knows your history)
- [ ] Timeline view
- [ ] 3D visualization option

## The Vision

Mycelica is Phase 1 of a larger goal: a unified knowledge workspace combining AI conversations, quick clipboard captures, and manual notes - all in one spatial mind map.

See `planning/VISION.md` for the full picture.

## License

MIT

---

*Built for the 10% of AI users who research non-linearly and need their branching thoughts visible and navigable.*
