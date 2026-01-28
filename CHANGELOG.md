# Changelog

All notable changes to Mycelica will be documented in this file.

## [0.9.1] - 2026-01-29

### Removed
- **Legacy clustering subsystem**: Deleted entire classic clustering pipeline (~2,570 lines)
  - Removed `clustering.rs` module (1,322 lines) - embedding-based agglomerative clustering with multi-path assignments
  - Removed clustering commands: `cluster run`, `cluster all`, `cluster name`, `cluster reset`, `cluster thresholds`
  - Removed hierarchy commands: `hierarchy rebuild-lite`
  - Removed Tauri commands: `get_clustering_status`, `name_clusters`, `reset_clustering`, `rebuild_lite`
  - Removed UI: "Rebuild Lite" and "Name Clusters (AI)" buttons from Settings panel
  - Removed from `hierarchy.rs` (683 lines):
    - K-means clustering system (`calculate_target_k`, `kmeans_cluster`, `kmeans_plusplus_init`, `recompute_centroids`, `cluster_items_by_embedding`)
    - Recursive parent level creation (`create_parent_level`)
    - Depth-based configuration (`max_children_for_depth`, `is_coherent_for_deep_split`)
    - Semantic level naming (`level_name`, `level_emoji` - Universe/Galaxy/Domain/Region)
    - AI progress tracking (`AiProgressEvent`, `emit_progress`)
  - Removed tag vocabulary system from `tags.rs` (346 lines):
    - `generate_tags_from_item_vocabulary` - tag clustering and hierarchy building
    - Tag-based similarity bonuses (`get_tag_similarity_bonus`)
  - Removed FOS edge cache system from `schema.rs` (110 lines):
    - `precompute_fos_edges`, `get_edges_for_fos`, `clear_fos_edges`, `delete_fos_edges_for_node`
    - `get_papers_by_fos` - FOS-based pre-grouping
  - Removed from other files:
    - `has_code_patterns` from `classification.rs` (55 lines)
    - `find_paper_in_subtree` helper from `dendrogram.rs` (13 lines)

### Added
- **Paper deduplication system**: Content-based duplicate detection for OpenAIRE imports
  - New `content_hash` column on papers table with index for O(1) lookups
  - `get_all_paper_dois`, `get_all_content_hashes` for batch import filtering
  - `find_duplicate_papers_by_title` for manual cleanup
  - `update_paper_content_hash` for backfilling existing papers
  - CLI commands: `maintenance clean-duplicates`, `maintenance backfill-hashes`
- **Implementation documentation**: `docs/implementation/DEAD_CODE_CLEANUP_2026-01.md` (979 lines)
  - Complete file-by-file breakdown of all deletions
  - Architectural analysis (before/after pipeline diagrams)
  - Migration guide for users and developers
  - Performance impact comparison (~50% faster rebuilds: 5-7 min → 2-3 min)

### Changed
- **Simplified architecture**: Two-stage pipeline (clustering → hierarchy) unified into single adaptive tree
  - Before: `Items → [Clustering] → Topics → [Hierarchy] → Categories`
  - After: `Items → [Embeddings] → Edges → [Adaptive Tree] → Categories`
- **Flat hierarchy**: 3-level structure (Universe → Topics → Items) replaces multi-tier recursive building
- **Direct edge-based**: Dendrogram reads similarity edges directly (no separate O(n²) clustering pass)
- **Adaptive thresholds**: Per-subtree threshold selection replaces fixed 0.75/0.60 global thresholds
- **Inline naming**: Categories named during tree construction (no separate naming pass)
- **Clear Structure** command now only clears hierarchy (removed clustering reset step)

### Migration Guide
**Old workflow (REMOVED):**
```bash
mycelica-cli cluster run          # Cluster new items
mycelica-cli cluster all --lite   # Recluster (keyword naming)
mycelica-cli cluster name         # AI naming pass
mycelica-cli hierarchy rebuild-lite  # Safe rebuild
```

**New workflow:**
```bash
mycelica-cli setup                # One-command: embeddings + hierarchy
mycelica-cli hierarchy build      # Rebuild hierarchy only
mycelica-cli hierarchy smart-add  # Add orphan items intelligently
```

### Technical
- Deleted modules: `clustering.rs`, tag vocabulary from `tags.rs`, FOS cache from `schema.rs`
- Deleted 4 Tauri command registrations from `lib.rs`
- Active commands: 147 Tauri, 20 CLI groups, 10 hierarchy subcommands
- Dendrogram replaces all clustering functionality:
  - Agglomerative clustering → Dendrogram merging
  - Multi-path assignment → Bridge detection
  - Fixed thresholds → Adaptive cuts
  - Recursive hierarchy → Flat 3-level structure
- Performance: ~50% faster full rebuilds (no separate O(n²) clustering phase)

---

## [0.9.0] - 2026-01-24

### Added
- **Adaptive tree algorithm**: New hierarchy building that auto-configures from data
  - Parameters derive from edge statistics (IQR, density)
  - Works on both sparse and dense graphs
  - 100% item coverage with nearest-neighbor orphan assignment
  - Replaces manual threshold tuning
- **LLM category naming**: Categories named by LLM (Ollama/Anthropic) by default
  - Uses parent category as context for better subcategory names
  - `--keywords-only` flag for faster keyword extraction fallback
  - Bottom-up naming: deepest categories first, parents use child names
- **Category embeddings**: Setup pipeline generates embeddings for categories
  - Similar nodes feature now works for categories
  - HNSW index includes both items and categories
- **Edge type display**: Similar nodes panel shows "calls", "documents" etc. in blue

### Changed
- `setup` command uses adaptive algorithm by default (`--algorithm adaptive`)
- Setup pipeline now 8 steps (0-7) with category embeddings before HNSW build
- `hierarchy rebuild --algorithm adaptive --auto` for auto-configured rebuild

### Fixed
- Edge loading for universe view (was skipping when at root level)
- Missing `edgeType` field in SimilarNode TypeScript interface
- Bottom-up category naming (was only naming depth-1 categories)
- Standardized universe node ID to 'universe' (fixes dual-universe bug)
- Progressive min_size caps tree depth (~9 instead of 27 on large databases)
- Live name deduplication prevents duplicate category names
- Compressed depth brackets for min_size (0-2/3-4/5-6/7/8+) for shallower trees
- Non-English text detection: local LLMs (Ollama) fall back to TF-IDF keywords for non-ASCII titles
- Consolidate Root now generates embeddings and sibling edges for uber-categories
- Added Unconsolidate Root command to reverse consolidation (flatten uber-categories back to Universe)

### Documentation
- **ALGORITHMS.md rewrite**: Replaced old hierarchy building docs with adaptive tree algorithm
  - Removed deprecated sections (depth limits, edge-based grouping, coherence refinement, full trace)
  - Added: dendrogram construction, split quality metrics, bridge detection, centroid bisection fallback
  - Added: category naming with live deduplication, 8-step setup pipeline
  - Updated constants table with dendrogram.rs values (EDGE_FLOOR, TIGHT_THRESHOLD, COHESION_THRESHOLD)
- **README.md**: Fixed language support (C not C/C++, added JavaScript), labeled Processing Pipeline as "Conceptual Overview"
- **CLAUDE.md**: Added key files (dendrogram.rs, clustering.rs, similarity.rs, openaire.rs), reorganized edge types into Auto-Generated/Manual/Future sections
- **ARCHITECTURE.md**: Updated date

### Technical
- `rebuild_hierarchy_adaptive()` with auto-config from `EdgeIndex` statistics
- `build_adaptive_tree()` in dendrogram lib with cohesion-based splitting
- `name_cluster_with_parent()` for context-aware LLM naming
- O(E) SQL joins replace O(T²) edge-based grouping

---

## [0.8.4] - 2026-01-19

### Changed
- **Edge-based hierarchy building**: Structural decisions now use paper connectivity instead of AI
  - Uber-category grouping: Topics group by cross-edge counts between their papers
  - Recursive grouping: Subcategories form from connected components in paper edge graph
  - Deterministic and free (no API calls for structure, only for naming)
- **Ollama-first naming**: Cluster naming prefers local Ollama when available
  - Falls back to Anthropic API only if Ollama not running
  - Simpler prompt that works for both models
- **Disabled project detection**: Phase 4 skipped (edge-based grouping handles organization better)

### Added
- **Sibling category edges**: New `sibling` edge type between categories
  - Weight derived from paper cross-edge counts (normalized by smaller category size)
  - Created after hierarchy build for graph visualization
  - Query with `edges WHERE type = 'sibling'`
- `delete_edges_by_type()` helper in schema

### Technical
- `create_category_edges_from_cross_counts()` creates edges between sibling categories
- `group_topics_by_paper_connectivity()` finds connected components for recursive grouping
- `map_topics_to_components()` assigns topics to dominant paper component
- Pipeline reordered: semantic edges created before hierarchy build (enables edge-based grouping on first run)

---

## [0.8.3] - 2026-01-16

### Added
- **HNSW similarity index**: O(log n) approximate nearest neighbor search replaces O(n) brute-force
  - 50-100x speedup for similarity queries (~870ms → ~10ms)
  - Index auto-built during `mycelica-cli setup` (Step 5/6)
  - Background build on app startup if index missing
  - Background build on database switch
  - Saved to disk as `{db-name}-hnsw.bin` (~100MB for 55k embeddings)
- **Edge parent indexing**: Added Step 6/6 to CLI setup for fast view-based edge loading
- **HNSW status API**: `get_hnsw_status` command returns `{ isBuilt, isBuilding, nodeCount }`
- **Building indicator**: Bottom-right UI shows spinner + message while HNSW index builds
- **Loading screen**: Dark background with spinner on app startup (replaces white flash)

### Changed
- `get_similar_nodes` returns empty array if HNSW index not ready (no blocking)
- Dev builds: Added `opt-level = 3` for `instant-distance` crate (faster dev testing)

### Technical
- `instant-distance` crate with `serde` + `serde-big-array` features for index serialization
- `HnswIndex` struct with `building` AtomicBool to prevent concurrent builds
- `HnswBuildingIndicator` React component polls status every 1s until built

---

## [0.8.2] - 2026-01-14

### Added
- **Holerabbit browser extension backend**: Full session tracking for Firefox extension
  - `POST /holerabbit/visit` - Record page visits with navigation edges
  - `GET /holerabbit/sessions` - List all browsing sessions
  - `GET /holerabbit/session/{id}` - Session detail with items and edges
  - `GET /holerabbit/live` - Query current live session
  - Session controls: pause/resume/rename/merge/delete
  - Single live session enforcement (resume auto-pauses others)
  - `init()` pauses all sessions on startup
- **Sessions panel in sidebar**: View and manage browsing sessions
  - Real-time updates via Tauri events (no polling)
  - Inline rename with edit button
  - Delete with confirmation
  - Pause/resume controls
- **New edge types**: `Clicked`, `Backtracked`, `SessionItem` for navigation tracking
- **New node types**: `web` (pages) and `session` (containers)

### Technical
- `holerabbit.rs` module (~1100 lines) for session management
- HTTP server emits `holerabbit:visit` event for real-time UI updates
- `SessionsPanel.tsx` (~470 lines) with Tauri event listener

---

## [0.8.1] - 2026-01-13

### Added
- **LLM Backend Toggle**: Settings → API Keys now has Claude/Ollama toggle
  - Shows Ollama model input and running status when Ollama selected
  - Note: Only affects AI Processing; hierarchy build still uses Claude
- **Call edges in Similar Nodes**: Shows "calls" / "called by" labels instead of percentages
  - Works in both graph details panel and leaf view similar nodes
  - Group headers show edge type when all items share the same type

### Changed
- **Unified color system**: Single `getHeatColor()` source of truth for all value-based colors
  - Removed duplicate `getDateColor`, `getEdgeColor` functions
  - Simplified gradient: Red → Yellow → Cyan (removed blue step)
  - Fixed inconsistent 100% colors (removed hardcoded green)

### Fixed
- Similar nodes panel: All 100% entries now display same color

---

## [0.8.0] - 2026-01-12

### Added
- **Multi-language code import**: Full parsing support for Python, C, TypeScript, and RST
  - `import code <path>` now handles `.py`, `.c`, `.h`, `.ts`, `.tsx`, `.rst` files
  - Extracts functions, classes, structs, methods with proper signatures
  - RST parser extracts documentation sections and code blocks
- **Ollama backend**: Local LLM alternative to Claude for AI processing
  - `config set-backend ollama` / `config set-backend anthropic`
  - `config set-ollama-model <model>` (default: qwen2.5:7b)
  - `check_ollama_status` command to verify Ollama is running
- **CUDA embeddings**: GPU-accelerated local embeddings via candle
  - Build with `cargo +nightly build --features cuda`
  - Separate CPU and CUDA binaries in releases
- **Call graph analysis**: `analyze code-edges` extracts function call relationships
  - Creates `Calls` edges between functions
  - Supports cross-file call detection
- **CLI file logging**: All CLI output now logged to `~/.local/share/com.mycelica.app/logs/`
- **Maintenance repair-code-tags**: Repairs corrupted code node metadata without re-running AI

### Changed
- Code nodes skip AI title generation (keep function signatures as titles)
- Embeddings generated for all code items for similarity search

### Fixed
- Code node tags preserved during AI processing (was overwriting file_path metadata)
- Cluster nodes filtered from similar nodes (can't open in leaf view)

---

## [0.7.4] - 2026-01-10

### Added
- **HTTP server for browser extension** (localhost:9876): Enables Firefox extension integration
  - `POST /capture`: Create bookmark nodes from web content with auto-generated embeddings
  - `GET /search?q=<query>`: Full-text search across nodes
  - `GET /status`: Connection check and version info
  - CORS headers for browser extension access
- **Bookmark node type**: New `bookmark` type for web captures (source: "firefox", content_type: "bookmark")

### Fixed
- TUI leaf view: Similar and Calls panels now use dynamic width to fill available space
- Bookmarks preserve their content_type ("bookmark") during AI processing and reclassification

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
