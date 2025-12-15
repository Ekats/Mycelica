# Changelog

All notable changes to Mycelica will be documented in this file.

## [Unreleased]

### Added
- **Edit functionality in Leaf View**: End nodes (items) can now be edited directly in the Leaf reader with Edit/Save/Cancel buttons. Changes persist to the database.
- **Database hot-swapping**: Switching databases in Settings now works without requiring an app restart. The backend uses `RwLock<Arc<Database>>` to enable live connection swapping.
- **Database sanitization for sharing**: Created tooling to analyze and sanitize databases for public sharing, removing PII patterns (usernames, emails, paths).
- **Split/Unsplit hierarchy controls**:
  - Split button creates max 5 sub-categories from a node's children using AI grouping
  - Unsplit button flattens intermediate categories back into parent
- **Preview database**: Added `mycelica-testing-preview.db` as a sanitized sample database for demos

### Changed
- **AppState architecture**: Changed from `Arc<Database>` to `RwLock<Arc<Database>>` to support hot-swapping database connections
- **Database switching UX**: No longer requires page reload - navigates to Universe root and refreshes data automatically
- **Details panel**: Shows Split/Unsplit buttons for all nodes with children (highlighted blue when >5 children)

### Fixed
- **Details panel stale state**: Fixed stale closure issue where clicking some nodes showed previous selection's details
- **Leaf View save error**: Fixed "invalid type: sequence, expected a string" error by creating dedicated `update_node_content` command that bypasses complex Node serialization

### Technical
- Added `update_node_content` command for simple content-only updates
- Updated all 98 `state.db.` usages across `graph.rs` and `privacy.rs` to use `RwLock` read guards
- Async commands now clone the `Arc<Database>` before await points to avoid holding locks across suspension points
