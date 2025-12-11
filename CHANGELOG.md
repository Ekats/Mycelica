# Changelog

All notable changes to Mycelica will be documented in this file.

## [0.2.0] - 2025-12-11

### Similar Nodes Panel
- Made scrollable with more items displayed
- Added green dot markers for nodes in same view
- Color gradient for similarity percentages (red→yellow→green)
- Jump navigation with breadcrumb tracking
- Panel made resizable with drag handle

### Edge Rendering
- Made edges thicker and more direct (curve radius 0.4→1.5)
- Thickness varies by semantic similarity weight (2-16px)
- Color gradient from red (low) to green (high similarity)
- Normalized weights based on min/max in current view
- Added weight field to TypeScript Edge interface

### Zen Mode
- New ☯ button (48x48px) in bottom-right of node cards
- Click to fade other nodes/edges by relevance
- Connected nodes fade based on edge weight
- Unconnected nodes fade to 15% opacity
- Click another node to switch zen focus, empty space to exit

### Node Card Styling
- Card height increased: 240px→320px
- Synopsis background with rgba(0,0,0,0.2)
- Text clipped to exactly 5 lines (no partial line visible)
- Nested div approach: outer for background, inner for text clipping
- Title clipped to 2 lines with -webkit-line-clamp
- Titlebar height: 72px→80px with more padding
- Summary text: 18px→20px, medium bold (font-weight: 500)
- Title text: 20px→22px
- Footer text: 14px→16px
- Font family: Inter, SF Pro Display, system fallbacks
- Title vertically centered in titlebar using flexbox

## [0.1.0] - 2025-11-23

### Added
- **Backend**
  - FastAPI server with conversation import endpoint
  - TF-IDF embeddings for conversation clustering
  - Agglomerative clustering with cosine similarity
  - Keyword extraction per cluster (with SEO-buried brand names)
  - Auto-title generation for untitled conversations
  - Empty conversation filtering (<50 chars)

- **Frontend**
  - Interactive D3.js force-directed graph
  - Prezi-style semantic zoom (cluster labels → node labels)
  - Counter-scaled text (readable at any zoom level)
  - Focus Mode - highlight only connected nodes/edges
  - Topic-based emoji indicators (70+ mappings)
  - Expandable cluster legend with conversation lists
  - Detail panel with "Open in Claude" button
  - Search bar
  - Stats footer (conversations, connections, clusters)

- **Visualization**
  - Spiral layout by cluster
  - Color-coded clusters
  - Node sizes based on message count
  - Edge weights based on similarity
  - Outline shadows on cluster labels

- **Documentation**
  - README.md with setup and usage guide
  - Reload troubleshooting guide (Windows uvicorn issues)

### Technical Details
- Backend: Python 3.10+, FastAPI, SQLite, scikit-learn
- Frontend: React 18, TypeScript, D3.js, Vite
- Analysis: TF-IDF vectors, agglomerative clustering, cosine similarity
