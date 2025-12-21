# Changelog

All notable changes to Mycelica will be documented in this file.

## [Unreleased]

### Added
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
