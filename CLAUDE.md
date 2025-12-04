# CLAUDE.md - Project Context

## What is Mycelica?

A tool for externalizing associative thought networks. Built on principles of synaptic architecture - because knowledge organization should mirror how brains actually work.

> *Named after mycelium - the underground fungal network connecting everything together, just like your conversations.*

Mycelica imports Claude conversation exports and visualizes them as an interactive spatial mind map. Conversations are automatically clustered by topic with hierarchical zoom levels.

**Core Problem**: AI interfaces force linear conversations, but research is naturally non-linear. Users manually manage context switches between topics, which is exhausting.

**Solution**: Automatic topic detection, visual mind mapping, and smart context injection for continuing conversations.

## Project Status

**Current Phase**: Phase 1 Complete (December 2025)
**License**: AGPL-3.0 (copyleft)

### Phase 1 Features (Complete):

**Core Visualization:**
- Hierarchical Zoom: 5-level navigation (Universe → Galaxy → Cluster → Topic → Message)
- Semantic Zoom: Dynamic text/emoji transitions based on zoom level
- Sticky Note View: Topics displayed as colorful sticky notes with titles, keywords, and counts
- Galaxy View: Emoji-based glossy bubbles representing conversation clusters
- Detective-Style Connections: Visual similarity lines with strength-based thickness/color
- Universe Categories: Auto-generated + manual topic organization

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

## Architecture Overview

### Components:
1. **Frontend** (React + D3.js) - Interactive mind map visualization
2. **Backend** (FastAPI + Python) - Data processing, analysis, API
3. **Browser Extension** (Planned) - Bridge to claude.ai for seamless integration

### Tech Stack:
- **Backend**: Python 3.11+, FastAPI, SQLite
- **Frontend**: React, TypeScript, Vite, D3.js, TailwindCSS
- **Analysis**: TF-IDF with scikit-learn (free local processing)
- **Integration**: Manual export/import (browser extension planned)

### Project Structure:
```
Mycelica/
├── backend/
│   ├── app/
│   │   ├── main.py          # FastAPI server & endpoints
│   │   ├── analysis.py      # TF-IDF clustering & graph building
│   │   ├── db.py            # Unified database layer
│   │   ├── ai_client.py     # AI calling wrapper (Anthropic)
│   │   └── data_service.py  # High-level data operations
│   ├── requirements.txt
│   └── conversations.json   # Your exported data (not committed)
├── frontend/
│   ├── src/
│   │   ├── App.tsx          # Main visualization component
│   │   ├── emojiMatcher.ts  # Emoji keyword mapping
│   │   └── App.css          # Styles
│   └── package.json
├── planning/                # Design docs (gitignored)
├── CLAUDE.md               # This file
└── LICENSE                 # AGPL-3.0
```

### Backend Module Architecture:
- **db.py**: Context manager for SQLite, schema migration, CRUD operations
- **ai_client.py**: Single `call_ai()` function, JSON parsing, specialized analysis functions
- **data_service.py**: Cached data loaders, label resolution, stats aggregation, surfaced content

## Development Workflow

### Running the Stack:
```bash
# Backend
cd backend
source venv/bin/activate  # or venv\Scripts\activate on Windows
python -m uvicorn app.main:app --reload --port 8000

# Frontend
cd frontend
npm run dev

# After backend restart:
curl -X POST http://localhost:8000/analyze
```

Open http://localhost:5173

### Import Your Conversations:
1. Export from claude.ai (Settings > Export Data)
2. Place JSON as `backend/conversations.json`
3. POST to `/analyze`
4. Refresh frontend

### Known Issues:
- Uvicorn reload doesn't always work on Windows - kill and restart server
- Must re-run `/analyze` endpoint after backend restart

## Data Flow

1. **Import**: Upload claude.ai export → Parse conversations → TF-IDF clustering
2. **Analyze**: Extract keywords, assign emojis, write/retrieve titles
3. **Graph**: Build hierarchical structure with similarity connections
4. **Visualize**: D3.js force-directed layout with semantic zoom levels
5. **Continue**: Click "Open in Claude" → Copy context → Paste in new conversation

## UX Philosophy: "Wonder to Surf"

Mycelica should feel like **scanning your own brain** - not a mechanical file browser. The interface mirrors how we naturally compartmentalize and connect ideas.

> "It shouldn't be mechanical, it should be a wonder to surf"

### Core Principles:

#### 1. Notes Everywhere (No Bubbles)
- **Every level uses sticky notes**, not abstract bubbles
- Emoji appears **in the titlebar** of each note (not as the main content)
- Notes feel tangible and readable at every zoom level
- Visual consistency from Universe down to Message

#### 2. Dynamic Depth Based on Content
- **NOT uniform hierarchy** - depth adapts to content type
- Complex topics (coding, philosophy, theories) → **more levels** to organize
- Simple topics (fitness, recipes) → **fewer levels**, don't over-fragment
- The system should recognize content complexity and adjust automatically

#### 3. Surface the Gems (Stats Bar)
- **Colored stats bar** next to each note showing:
  - Number of items inside
  - Key findings/insights count
  - Quotes, poems, significant content flags
- Significant content **bubbles up** - you see findings at higher levels
- Don't hide the good stuff at the bottom of the hierarchy

#### 4. Connection Text, Not Just Transcription
- Notes show **how things connect** via synthesized text
- Not just "accurate text I wrote" but relationships between ideas
- Original notes are scattered and conversations are long/hard to read
- Higher-level notes are **synthesis**, not summary
- Always can drill down to see full original if desired

#### 5. Preview the Next Level
- Each note shows **text about what's inside** the next container
- Stats about child items (counts, types, highlights)
- Makes navigation predictive - know what you'll find before clicking

### Implementation Notes
- AI analysis generates: titles, summaries, tags, **and significance scores**
- Findings/quotes/poems get flagged during analysis for surfacing
- Connection text generated by looking at related items, not just content
- Depth algorithm considers: keyword diversity, conversation length, topic complexity

The goal is ease of accessing research through exploratory browsing, not mechanical navigation to raw data.

## Roadmap

**Near-term:**
- [ ] Stats bars showing findings/insights at each level
- [ ] AI-generated connection text between related topics
- [ ] Dynamic depth adjustment based on content complexity
- [ ] Automatic significance scoring for surfacing insights

**Phase 2-3:**
- [ ] Browser extension for real-time sync (no manual export)
- [ ] Automatic branch detection within conversations
- [ ] Context injection when opening in Claude
- [ ] Semantic search across all conversations

**Long-term Vision:**
- [ ] Multi-user shared knowledge substrate
- [ ] Research community collaboration features
- [ ] Export/publish curated knowledge graphs
- [ ] Infrastructure for sharing cognitive architectures

## The Vision

**Phase 1**: Individual tool for externalizing associative thought (complete)
**Phase 2**: Collect usage patterns → reveal how humans organize knowledge → enable cognitive research
**Phase 3**: Multi-brain knowledge web where research communities build interconnected thought networks
**Phase 4+**: Infrastructure for sharing cognitive architectures across fields and disciplines

---

## API Endpoints

### Core:
- `GET /health` - Health check
- `GET /stats` - Database statistics
- `POST /import` - Import Claude export
- `POST /analyze` - Run clustering analysis

### Graph:
- `GET /graph/zoom/{level}` - Get nodes for hierarchy level (universe/galaxy/topic/message)
- `GET /graph/surfaced` - Get findings/quotes/poems for message IDs
- `GET /graph/findings` - Get all significant findings
- `GET /graph/stats` - Get analysis statistics

### API Key:
- `POST /api-key` - Set Anthropic API key (for AI naming)
- `GET /api-key/status` - Check if key is set
- `DELETE /api-key` - Clear API key

### Analysis:
- `POST /regenerate-tags` - Regenerate tags with AI
- `GET /analyze-messages/status` - Check AI analysis progress
- `POST /analyze-messages` - Run AI analysis on all messages

---

## Recent Technical Changes (2025-12-04)

### Modular Backend Architecture
Created "cute internal APIs" to consolidate scattered logic:
- **db.py**: Unified database layer with context manager, schema migration
- **ai_client.py**: Single AI calling wrapper with JSON parsing
- **data_service.py**: Cached data loaders, label resolution, stats aggregation

### Surfaced Content Endpoints
New endpoints for "surfacing gems":
- `/graph/surfaced` - Get findings, quotes, poems for specific messages
- `/graph/findings` - Get all findings across messages
- `/graph/stats` - Overall analysis statistics

### Sticky Notes at All Levels
Universe/Galaxy levels now render as sticky notes with:
- Emoji in titlebar
- Title text
- Stats bar with colored dots (blue for item count, green for keywords)
- Preview text showing what's inside
- Level indicator badge (🌌/🌟/💫)
- Counter-scaling for zoom consistency

---

## Technical Implementation Details

This section contains specific formulas, values, and bug fixes for reference.

### Sticky Note Visualization (Topic Level)
- **Dimensions**: 140x100px base size for topic notes
- **Colors**: 12 pastel colors cycling based on cluster_id:
  - Yellow, orange, pink, purple, blue, teal, green variants
- **Paper Styling**: Shadow, slight rotation per note, folded corner effect
- **Zoom Behavior**: Notes counter-scale with zoom (0.5x to 2x) to stay readable
- **Interaction**: Click to select, double-click to drill down to messages
- **Layout**: Concentric ring layout, distributed by importance tier

### Detective-Style Connection Lines
- **Similarity Calculation**: Same cluster (+0.6) + shared keywords (+0.2 each, capped at 0.6)
- **Curved Paths**: Quadratic bezier curves with varying curvature for visual interest
- **Line Thickness Formula**: `0.3 + weight^2.5 * 28` (gives hairline 1px to chunky 28px+)
- **Color Gradient**: Dark burgundy (#450a0a) for weak → pale glowing red (#fca5a5) for strong
- **Opacity Scaling**: 15% to 90% opacity based on connection strength
- **Zoom Counter-Scaling**: Lines scale up when zoomed out to stay visible at 0.1x

### Concentric Ring Layout
- Ring 0: 1 node at center
- Ring 1: 6 nodes
- Ring 2: 12 nodes
- Pattern continues with hexagonal packing
- 3 tiers total: large nodes center, medium middle ring, small outer ring
- Similar topics grouped by cluster_id sorting within tiers

### Zoom & Navigation
- **Smooth Map Transitions**: CSS transition (150ms cubic-bezier) on SVG transform
- **Cursor-Following Drift**: When zooming in, viewport drifts towards cursor (15% pull strength)
- **SVG-Relative Calculation**: Uses `getBoundingClientRect()` to account for legend panel offset
- **Bubble Scaling**: Grows from scale 0.2→1.0, stays constant past 1.0
- **Text Counter-Scaling**: Labels stay readable at all zoom levels
- **Wheel Sensitivity**: Reduced delta (0.8x) for finer control

### Bug Fixes (Historical)

**Click-to-Zoom Centering**
- Problem: Clicking a node didn't center properly due to position spreading during zoom
- Solution: Pre-calculate node's position at target scale before zooming
  - Get current spread factor from zoom level
  - Calculate target spread at scale 2.5
  - Scale node's offset from graph center proportionally
  - Zoom to predicted final position

**Galaxy View Node Spacing**
- Problem: Large emoji nodes appeared crowded/touching
- Solution: Multi-layer approach:
  1. D3 Force Collision: `d3.forceCollide()` for collision detection
  2. Tiered Ring Layout: 3 importance tiers with organized rings
  3. Zoom-Responsive Sizing: Bubble radius and emoji font scale together
  4. Dynamic Spreading: Positions scale with `sqrt(1/scale)` when zoomed out, capped at 2.5x

### Emoji Library
- File: `emojiMatcher.ts`
- Package: `unicode-emoji-json` with 900+ keyword mappings
- Custom mappings: Python→🐍, React→⚛️, Docker→🐳, GPT→🤖, Claude→🧠
- Categories: Tech/programming, AI/ML, business, science, design, general
- Fallback: Intelligent word matching with emoji name database

### Database Schema
Tables in `mycelica.db`:
- `message_analysis`: message_id, conversation_id, title, summary, tags, significance_score, is_finding, is_quote, is_poem
- `topic_analysis`: topic_id, conversation_id, title, summary, tags, message_count, child_count, significance_score, findings_count
- `topic_tags`: topic_id, tags, is_ai_generated
- `cluster_names`: cluster_id, level, name, keywords, is_manual
- `message_titles`: message_id, title, is_manual

Schema migration handled via ALTER TABLE with try/except for backwards compatibility.

---

*Last Updated: 2025-12-04*
