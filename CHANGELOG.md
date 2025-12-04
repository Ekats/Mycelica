# Changelog

All notable changes to Mycelica will be documented in this file.

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
