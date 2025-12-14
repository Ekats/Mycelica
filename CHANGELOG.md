# Changelog

All notable changes to Mycelica will be documented in this file.

## [0.5.0] - 2025-12-14

### Settings Panel Redesign
- Reorganized into 5 tabs: Setup, API Keys, Maintenance, Privacy, Info
- Panel width increased for better readability
- Tab navigation with icons and active indicator
- All action buttons now use consistent compact row format with:
  - Colored icon circle on left
  - Description + API cost estimates in middle
  - Large square emoji button on right (48×48px)

### Maintenance Tab
- Operations section: Full Rebuild, Flatten, Consolidate Root, Tidy Database
- Each operation shows API call estimates and item counts
- Combined "Danger Zone" section with Reset Flags
- Reset buttons in 2×2 grid: Reset AI, Reset Clustering, Clear Embeddings, Clear Hierarchy
- "Delete All Data" moved to bottom with subtle danger styling

### Privacy Tab Improvements
- Added description explaining privacy scanning functionality
- Documents category inheritance behavior (hidden when ALL children private)
- Instructions for adjusting sensitivity: edit `PRIVACY_PROMPT` in `src-tauri/src/commands/privacy.rs` (lines 15-45)
- References lock button location for toggling private content visibility

### Recent Notes Protection
- New protection system for "Recent Notes" container and all descendants
- Protected from: AI processing, clustering, hierarchy building, consolidate root, tidy operations
- Toggle in Settings → Info tab: "Protect Recent Notes"
- Defaults to enabled

### Node Details Panel
- Added privacy toggle button (Lock/LockOpen icons) next to pin button
- Rose color when node is marked private
- New `set_node_privacy` Tauri command for manual privacy control
- Pin button now syncs with sidebar via Tauri events

### Quick Notes
- 📝 button in hamburger menu to add quick notes
- Notes saved under "Recent Notes" container (auto-created if missing)
- Modal with title and content fields
- Notes automatically added to current view after creation

### UI Polish
- X button in sidebar Recent section always visible (was hover-only)
- Pin and privacy buttons use consistent Lucide icons
- Larger, square action buttons throughout Settings panel

## [0.4.0] - 2025-12-13

### Date-Based Coloring
- Node dates colored with gradient: red (oldest) → yellow → blue → cyan (newest)
- Skips green for colorblind accessibility
- Edge connection lines use same gradient based on similarity weight
- Color legend in bottom-right shows "Age / Similarity" scale

### Latest Child Date Propagation
- New `latestChildDate` field on nodes
- Bottom-up propagation: leaves use `createdAt`, groups bubble up MAX from children
- "📅 Dates" button to manually propagate (fast, no AI)
- Auto-runs as Step 6 of "Full Rebuild"
- Groups now show "X items · Latest: date" with date colored by recency

### Node Footer Improvements
- Split footer: item count on left, latest date on right (for groups)
- Semi-transparent background behind footer text for readability
- Items show creation date (colored), groups show child count + latest date

## [0.3.0] - 2025-12-11

### AI Progress Indicator
- Floating progress panel in bottom-right during AI operations
- Shows current/total nodes with percentage
- Time remaining estimate with smart formatting (s/m/h)
- Progress bar that turns green on completion
- Auto-hides 3 seconds after completion
- Works for both "Process AI" and "Full Rebuild" (embedding step)

### Conversation Rendering (LeafView)
- Document-style layout instead of chat bubbles
- Section headers: "You" (amber) and "Claude" (gray) with underlines
- Auto-formatting: commands and paths wrapped in code blocks
- Code blocks: dark gray-950 background, green text, rounded borders
- Full markdown rendering with prose styling
- Removed excessive centering for better readability

### Import Pipeline
- Exchange pairing: human + assistant messages combined into single nodes
- Format: `Human: {question}\n\nAssistant: {response}`
- Conversation filtering by title (e.g., "mycelica" only)
- Exclude list for specific conversation UUIDs
- Python import script at `scripts/import_conversations.py`

### Hierarchy Build Logging
- Clear step headers with visual separators (═══, ───)
- Step numbering: STEP 1/4, STEP 2/4, etc.
- Time estimates during embedding generation
- Completion summary with iteration counts

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
