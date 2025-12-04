# Mycelica

> *Imagine all the people*  
> *Livin' life in peace*
> 
> *You may say I'm a dreamer*  
> *But I'm not the only one*  
> *I hope someday you'll join us*  
> *And the world will be as one*
> 
> Imagine all the people...  
> **sharing cognitive architectures**  
> instead of just  
> **shouting conclusions at each other.**

## What is this?

A tool for externalizing associative thought networks. Built on principles of synaptic architecture - because knowledge organization should mirror how brains actually work.

> *Named after mycelium - the underground fungal network connecting everything together, just like your conversations.*

## What is this again?

Mycelica imports your Claude conversation export and visualizes it as an interactive spatial mind map. Conversations are automatically clustered by topic with hierarchical zoom levels, letting you:

- Navigate from universe-level categories down to individual messages
- See connections between related topics with visual similarity lines
- Find any discussion in seconds through spatial memory
- Explore your research as sticky notes and emoji bubbles
- Continue conversations with context in Claude

## Features

**Phase 1 Complete (December 2025):**

**Core Visualization:**
- **Hierarchical Zoom**: 5-level navigation (Universe → Galaxy → Cluster → Topic → Message)
- **Semantic Zoom**: Dynamic text/emoji transitions based on zoom level
- **Sticky Note View**: Topics displayed as colorful sticky notes with titles, keywords, and counts
- **Galaxy View**: Emoji-based glossy bubbles representing conversation clusters
- **Detective-Style Connections**: Visual similarity lines with strength-based thickness/color
- **Universe Categories**: Auto-generated + manual topic organization

**Analysis & Organization:**
- Import Claude conversation exports (JSON)
- TF-IDF clustering with automatic topic detection
- 2362+ manual topic titles (human-written, not algorithmic)
- Emoji matching with 900+ keyword mappings
- Concentric ring layout with importance-based tiering

**Interaction:**
- Smooth zoom with cursor-following behavior
- Click to select, double-click to drill down
- Focus Mode - highlight only connected nodes
- Breadcrumb navigation between zoom levels
- Real-time search
- "Open in Claude" integration
- Expandable cluster legend

**Tech Stack:**
- Backend: Python 3.11+ + FastAPI + SQLite
- Frontend: React + TypeScript + Vite + D3.js + TailwindCSS
- Analysis: TF-IDF with scikit-learn (free local processing)

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
- Scroll to zoom between hierarchy levels
- Drag to pan around the space
- Click cluster labels or category bubbles to zoom in
- Double-click topics to drill down to messages
- Use breadcrumbs to navigate back up

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
4. **Analyze**: Extract keywords, assign emojis, write/retrieve titles
5. **Graph**: Build hierarchical structure with similarity connections
6. **Visualize**: D3.js force-directed layout with semantic zoom levels

## UX Philosophy: "Wonder to Surf"

Mycelica should feel like **scanning your own brain** - not a mechanical file browser. The interface mirrors how we naturally compartmentalize and connect ideas.

**Core Principles:**
- **Notes everywhere**: Every level uses readable notes, not abstract bubbles
- **Dynamic depth**: Complex topics get more levels; simple topics stay shallow
- **Surface the gems**: Significant findings, quotes, and insights visible at higher levels
- **Show connections**: Notes explain relationships, not just content
- **Preview the next level**: Each note hints at what's inside before you drill down

The goal is ease of accessing research through exploratory browsing, not mechanical navigation to raw data.

## Development

**Backend reload issues on Windows:**
Uvicorn's `--reload` sometimes detects changes but doesn't actually restart. If your changes aren't taking effect:
1. Kill the server (Ctrl+C)
2. Restart fresh
3. Re-run `/analyze`
4. Hard refresh the frontend

## Roadmap

**Near-term enhancements:**
- [ ] Stats bars showing findings/insights at each level
- [ ] AI-generated connection text between related topics
- [ ] Dynamic depth adjustment based on content complexity
- [ ] Automatic significance scoring for surfacing insights

**Phase 2-3:**
- [ ] Browser extension for real-time sync (no manual export)
- [ ] Automatic branch detection within conversations
- [ ] Context injection when opening in Claude
- [ ] Semantic search across all conversations

**Long-term vision:**
- [ ] Multi-user shared knowledge substrate
- [ ] Research community collaboration features
- [ ] Export/publish curated knowledge graphs

## The Vision

**Phase 1**: Individual tool for externalizing associative thought (✅ complete)

**Phase 2**: Collect usage patterns → reveal how humans organize knowledge → enable cognitive research

**Phase 3**: Multi-brain knowledge web where research communities build interconnected thought networks

**Phase 4+**: Infrastructure for sharing cognitive architectures across fields and disciplines

Mycelica started as a tool for managing AI conversations. It's evolving into a platform for understanding and connecting human thought itself.

See `planning/VISION.md` and `CLAUDE.md` for technical details.

## License

MIT

---

*Built for exploratory researchers who think in webs, not lists - because knowledge organization should mirror how brains actually work.*