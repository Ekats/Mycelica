# Changelog

All notable changes to Mycelica will be documented in this file.

## [0.7.4] - 2026-01-10

### Added
- **HTTP server for browser extension** (localhost:9876): Enables Firefox extension integration
  - `POST /capture`: Create bookmark nodes from web content with auto-generated embeddings
  - `GET /search?q=<query>`: Full-text search across nodes
  - `GET /status`: Connection check and version info
  - CORS headers for browser extension access
- **Bookmark node type**: New `bookmark` type for web captures (source: "firefox", content_type: "reference")

### Fixed
- TUI leaf view: Similar and Calls panels now use dynamic width to fill available space

---

## [0.7.3] - 2026-01-09

### Added
- **Incremental code import** (`--update` flag): Surgically update single files/directories instead of full reimport. Deletes old nodes, reimports, regenerates embeddings, refreshes Calls edges.
- **Auto-discovery**: CLI finds `.mycelica.db` by walking up directories (like `.git`). No config needed.
- **New CLAUDE.md**: Instructs AI agents to use `mycelica-cli` instead of grep for code exploration.

### Fixed
- Search results now show full node IDs instead of truncated 8-character IDs
- Edge deletion uses correct column names (`source_id`/`target_id`)

### Changed
- Code items skip AI processing entirely (keep signatures as titles, only generate embeddings)
- Code-only databases skip project detection in Phase 4

---

## [0.7.0] - 2026-01-04

### Added
- **Full CLI interface**: Headless command-line tool (`mycelica-cli`) for scripting, automation, and server use
  - 70+ commands across 16 categories: db, import, export, node, hierarchy, process, cluster, embeddings, privacy, paper, config, recent, pinned, nav, maintenance, completions
  - `--json` flag for machine-readable output
  - `--db <path>` to specify database
  - `-q/--quiet` and `-v/--verbose` flags
- **Interactive TUI mode**: Terminal UI for browsing the knowledge graph (`mycelica-cli tui`)
  - **CD-style navigation**: Enter=cd into cluster, Backspace/- = go up one level
  - **Breadcrumb bar**: Shows path from Universe to current location
  - **3-column layout**: Tree (50%) | Pins+Recents (25%) | Preview (25%)
  - **Pane interactivity**: Tab cycles focus between Tree → Pins → Recents (cyan border = focused)
    - j/k navigates within focused pane, Enter jumps to selected node
  - **Leaf View mode**: Full-screen content viewer for items
    - Tab cycles focus: Content → Similar → Edges
    - j/k scrolls content or navigates sidebar based on focus
    - Enter on Similar/Edges jumps to that node
  - **Edit mode**: Full-screen text editor with line numbers, cursor, Ctrl+S save, Esc cancel
  - Vim-style keybindings (j/k/h/l, g/G, n/N for similar navigation)
  - Full-text search with `/`
  - Pin/unpin nodes with `p` key
  - Colored dates in tree (red=old → yellow → cyan=recent)
- **Global search command**: `mycelica-cli search <query>` searches across all nodes
  - Filter by type: `--type item|category|paper|all`
  - Limit results: `--limit 20`
- **Maintenance commands**: 12 dangerous operations with interactive confirmation
  - `maintenance wipe` - Delete all nodes and edges
  - `maintenance reset-ai` - Clear AI-generated titles/summaries
  - `maintenance reset-clusters` - Remove clustering assignments
  - `maintenance reset-privacy` - Clear privacy scores
  - `maintenance clear-embeddings` - Remove all embeddings
  - `maintenance clear-hierarchy` - Flatten hierarchy to Universe
  - `maintenance clear-tags` - Remove tag assignments
  - `maintenance delete-empty` - Remove nodes without content
  - `maintenance vacuum` - Compact database
  - `maintenance fix-counts` - Recalculate child counts
  - `maintenance fix-depths` - Recalculate node depths
  - `maintenance prune-edges` - Remove orphaned edges
  - All require `--force` flag or interactive "yes" confirmation
- **Export commands**: 5 export formats for data portability
  - `export bibtex -o papers.bib` - BibTeX citation format for papers
  - `export markdown -o output.md` - Markdown with hierarchy
  - `export json -o output.json` - Full node data as JSON
  - `export graph -o graph.dot -f dot|graphml` - Graph formats with optional similarity edges
  - `export subgraph <node_id> -o subtree.json -d 3` - Extract subtree to depth
  - All support `--node <id>` to limit to specific subtree
- **Database selector**: `mycelica-cli db select` scans common locations and lets you pick interactively
  - Saves selection to settings for persistence
  - Checks repo directory, app data, Downloads for .db files
- **Shell completions**: `mycelica-cli completions bash|zsh|fish` generates shell completions
- **OpenAIRE import via CLI**: `mycelica-cli import openaire --query "..." --country EE --fos "medical"` with year filters
- **Navigation commands**: `nav ls`, `nav tree`, `nav path`, `nav edges`, `nav similar` for non-interactive exploration
- **Paper commands**: `paper list`, `paper get`, `paper download`, `paper open`, `paper sync-pdfs`, `paper reformat-abstracts`

### Changed
- Settings initialization moved earlier in CLI startup to support custom database path loading
- README updated with CLI & TUI documentation and Quick Start section
- TUI now uses modal design: Navigation mode → Leaf View → Edit mode

### Technical
- Added dependencies: clap 4, clap_complete 4, ratatui 0.26, crossterm 0.27, tui-tree-widget 0.19, nucleo 0.5, arboard 3
- New binary target `mycelica-cli` in Cargo.toml
- Made modules public for CLI access: clustering, ai_client, settings, hierarchy, import, similarity, openaire
- TUI uses parent_id tracking for proper tree expansion
- Fixed `search_nodes` SQL query missing 5 columns (source, pdf_available, content_type, associated_idea_id, privacy)
- TUI state machine with `TuiMode` enum: Navigation, LeafView, Edit, Search, Maintenance, Settings, Jobs
- Pane focus tracking with `NavFocus` (Tree, Pins, Recents) and `LeafFocus` (Content, Similar, Edges) enums
- Edit mode implements full text buffer with UTF-8 aware cursor positioning
- Export functions: `export_dot()`, `export_graphml()`, `get_export_nodes()`, `collect_descendants()`
- Maintenance commands use interactive confirmation pattern with `--force` override

---

## [Unreleased]

### Fixed (2026-01-08)
- **CLI embedding generation**: `process run` and `setup` commands now generate embeddings after AI analysis, matching GUI behavior
- **CLI tags format**: Changed from CSV (`tags.join(",")`) to JSON array (`serde_json::to_string(&tags)`), fixing frontend compatibility
- **CLI privacy scoring**: Implemented full batched AI privacy scoring (25 items/batch) using Haiku model - was previously a stub

### Added (2026-01-08)
- **Edge view architecture**: Edges now loaded per-view instead of all at startup
  - New columns: `edges.source_parent_id`, `edges.target_parent_id`
  - New index: `idx_edges_view(source_parent_id, target_parent_id)` for O(1) lookups
  - Backend: `get_edges_for_view(parent_id)` command
  - Frontend: `loadEdgesForView(parentId)` in `useGraph.ts:204`, called on view change in `Graph.tsx:385`
- **docs/CLI.md**: Complete CLI reference with all commands, examples, and workflows

### Changed (2026-01-08)
- **Documentation overhaul**: All specs updated to match actual implementation
  - `AI_CLUSTERING.md`: Complete rewrite - now describes embedding-based cosine similarity clustering (not AI batch)
  - `SCHEMA.md`: Added `papers` table, `fos_edges` (deprecated), edge view columns, frontend integration
  - `HIERARCHY.md`: Added tiered child limits (10/25/50/100/150 by depth)
  - `COMMANDS.md`: Added Paper Operations, OpenAIRE import, edge view commands (~120+ total)
  - `TYPES.md`: Added `paper` content type (14 total)
  - `PRIVACY.md`: Added CLI availability table
  - `ARCHITECTURE.md`: Added CLI section, edge loading section, updated counts
  - `MULTI_PATH.md`: Added FOS-based edges section

### Deprecated (2026-01-08)
- `fos_edges` table: Superseded by view-based edge loading with parent columns on `edges` table

---

### Added (Previous)
- **Persistent tags system**: Tags that survive hierarchy rebuilds and guide clustering
  - Tags generated from existing `nodes.tags` JSON field (AI-assigned item tags)
  - Similar tag strings clustered (cosine > 0.75) and canonicalized (e.g., "rust", "Rust", "rust-lang" → "rust")
  - Controlled vocabulary: only tags appearing in 10+ items (ln-scaled threshold)
  - Tag hierarchy built from semantic similarity between canonical tags
  - +0.08 similarity bonus per shared tag\ during clustering
  - Tag anchors passed to uber-category AI as preferred category names
  - "Clear Tags" button in Settings → Maintenance to regenerate
- **Conversation clustering bonus**: Items from the same conversation get +0.1 similarity boost during clustering, keeping related exchanges together
- **Privacy scoring system**: Continuous 0.0-1.0 privacy scores for items (0.0=private, 1.0=public) scored by AI via Haiku
  - `score_privacy_all_items` command processes items in batches of 25
  - "Score Privacy" button in Settings → Maintenance with cost estimation
  - Privacy propagation: categories inherit minimum privacy of their children
  - Private items (< 0.3) automatically moved to "Personal" category during hierarchy build
  - `get_shareable_items(min_privacy)` export filter for sharing sanitized databases
- **Content classification system**: Items are now classified by type (idea/code/debug/paste) using pattern matching. This separates conversational content from supporting artifacts.
- **Mini-clustering associations**: Supporting items (code/debug/paste) are automatically associated with related idea nodes using embedding similarity (70%) + time proximity (30%).
- **Associated items sidebar in Leaf View**: When viewing an idea, a right sidebar shows associated code snippets, debug logs, and pastes with clickable links.
- **Classify Content button**: Added to Settings → Maintenance to manually trigger classification and association.
- **Incomplete conversation cleanup**: Hierarchy rebuild now automatically deletes items that only have a human query with no Claude response.
- **Edit functionality in Leaf View**: End nodes (items) can now be edited directly in the Leaf reader with Edit/Save/Cancel buttons. Changes persist to the database.
- **Database hot-swapping**: Switching databases in Settings now works without requiring an app restart. The backend uses `RwLock<Arc<Database>>` to enable live connection swapping.
- **Database sanitization for sharing**: Created tooling to analyze and sanitize databases for public sharing, removing PII patterns (usernames, emails, paths).
- **Split/Unsplit hierarchy controls**:
  - Split button creates max 5 sub-categories from a node's children using AI grouping
  - Unsplit button flattens intermediate categories back into parent
- **Preview database**: Added `mycelica-testing-preview.db` as a sanitized sample database for demos

### Changed
- **Tag generation approach**: Refactored from cluster-based to vocabulary-based
  - Old: Promoted large clusters to tags, assigned items via centroid similarity (>0.35)
  - New: Extracts tags from `nodes.tags` field, clusters similar strings, maps items directly to canonical tags
  - More accurate: items get the tags they actually have, just deduplicated and canonicalized
- **Graph view filtering**: Only idea nodes are now shown in the graph. Code/debug/paste items are hidden and accessible via the Leaf View sidebar.
- **Clustering optimization**: Clustering now only processes idea items, skipping code/debug/paste. This produces cleaner topic groupings focused on conversational content.
- **Hierarchy build order**: Classification now runs before clustering (Step 1/7), associations run after hierarchy is built (Step 7/7).
- **Import filtering**: Claude conversation import now skips human messages without responses instead of creating placeholder nodes.
- **AppState architecture**: Changed from `Arc<Database>` to `RwLock<Arc<Database>>` to support hot-swapping database connections
- **Database switching UX**: No longer requires page reload - navigates to Universe root and refreshes data automatically
- **Details panel**: Shows Split/Unsplit buttons for all nodes with children (highlighted blue when >5 children)

### Fixed
- **Details panel stale state**: Fixed stale closure issue where clicking some nodes showed previous selection's details
- **Leaf View save error**: Fixed "invalid type: sequence, expected a string" error by creating dedicated `update_node_content` command that bypasses complex Node serialization
- **Hierarchy depth explosion**: Added MAX_HIERARCHY_DEPTH (15) safety limit to prevent runaway recursive grouping
- **Duplicate node count bug**: Fixed 21/19 impossible count by deduplicating matching children by node ID using HashSet
- **Garbage names in uber-categories**: Step 4.5 consolidation now filters garbage names (Empty, Cluster, Mixed, etc.) - same pattern as recursive grouping
- **Projects buried in hierarchy**: Project umbrella nodes now protected from uber-category reparenting. Projects stay at depth 1 under Universe
- **Expanded garbage word list**: Added "mixed", "assorted", "combined", "merged", "grouped", "sorted" to filter meaningless AI-generated names

### Technical
- Added `tags` and `item_tags` tables for persistent tag storage with centroids (BLOB)
- Added `tags.rs` module with `generate_tags_from_item_vocabulary()` - extracts, embeds, clusters, and canonicalizes item tags
- Tag embedding uses local model (`local_embeddings::generate_batch`) for fast batch processing
- Union-find algorithm for clustering similar tag strings
- Pre-loaded `item_tags` map in `clustering.rs` for O(1) tag bonus lookup during O(n²) similarity calculation
- Added `clear_tags` Tauri command and Settings button
- Added `privacy REAL` column to nodes table (0.0=private, 1.0=public, NULL=unscored)
- Added `propagate_privacy_scores()` in hierarchy.rs - bottom-up propagation where category privacy = min(children)
- Privacy filter in clustering.rs excludes items with privacy < 0.3 from main clustering
- "Personal" category (🔒) auto-created during hierarchy build for private items
- JSON parser for AI responses now handles trailing commas (common Haiku issue)
- Added `GARBAGE_NAMES` constant and `is_garbage_name()` filter function in `hierarchy.rs` to reject meaningless AI-generated category names
- Added `MAX_HIERARCHY_DEPTH` (15) constant to prevent runaway depth during recursive grouping
- Uber-category consolidation (Step 4.5) now filters garbage names and protects project nodes from reparenting
- Classification diagnostic logging shows why 0 items classified (already classified by AI vs empty content)
- Added `classification.rs` module with pattern-based content type detection and embedding-based association algorithm
- Added `content_type` and `associated_idea_id` columns to nodes table with migration
- Added Tauri commands: `get_graph_children`, `get_supporting_items`, `get_associated_items`, `get_supporting_counts`, `classify_and_associate`
- Added `delete_incomplete_conversations()` database method for cleanup
- Cluster naming now batches API calls (50 clusters per batch) to prevent response truncation
- Embeddings now generated for ALL items (not just those with AI titles) to enable association matching
- Added `update_node_content` command for simple content-only updates
- Updated all 98 `state.db.` usages across `graph.rs` and `privacy.rs` to use `RwLock` read guards
- Async commands now clone the `Arc<Database>` before await points to avoid holding locks across suspension points
