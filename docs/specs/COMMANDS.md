# Tauri Commands Reference

> Generated from `src-tauri/src/commands/*.rs`. This is the actual API.

All backend operations exposed via `invoke()`. Frontend calls these through `@tauri-apps/api/core`.

---

## Quick Reference

| Category | Commands |
|----------|----------|
| [Node CRUD](#node-crud) | 7 commands |
| [Edge Operations](#edge-operations) | 6 commands |
| [Hierarchy Navigation](#hierarchy-navigation) | 8 commands |
| [Hierarchy Building](#hierarchy-building) | 9 commands |
| [Clustering](#clustering) | 4 commands |
| [AI Processing](#ai-processing) | 5 commands |
| [Content Classification](#content-classification) | 3 commands |
| [Rebuild Lite](#rebuild-lite) | 4 commands |
| [Import](#import) | 8 commands |
| [Paper Operations](#paper-operations) | 9 commands |
| [Quick Access](#quick-access-sidebar) | 5 commands |
| [Semantic Similarity](#semantic-similarity) | 3 commands |
| [Privacy](#privacy) | 10 commands |
| [Settings & State](#settings--state) | 27 commands |
| [Database Management](#database-management) | 11 commands |
| [Cancellation](#cancellation) | 4 commands |

**Total: ~120+ commands**

> See also: [CLI.md](../CLI.md) for command-line interface reference

---

## Node CRUD

### get_nodes
Get all nodes in the database.

```rust
fn get_nodes(state: State<AppState>) -> Result<Vec<Node>, String>
```

```typescript
const nodes = await invoke<Node[]>('get_nodes');
```

---

### get_node
Get a single node by ID.

```rust
fn get_node(state: State<AppState>, id: String) -> Result<Option<Node>, String>
```

```typescript
const node = await invoke<Node | null>('get_node', { id: 'abc123' });
```

---

### create_node
Insert a new node.

```rust
fn create_node(state: State<AppState>, node: Node) -> Result<(), String>
```

```typescript
await invoke('create_node', { node: { id: 'new-id', title: 'My Node', ... } });
```

---

### add_note
Create a quick note under "Recent Notes" container. Auto-creates container if needed.

```rust
fn add_note(state: State<AppState>, title: String, content: String) -> Result<String, String>
```

**Returns:** ID of created note

```typescript
const noteId = await invoke<string>('add_note', {
  title: 'Quick thought',
  content: 'Some content here'
});
```

---

### update_node
Update all fields of a node.

```rust
fn update_node(state: State<AppState>, node: Node) -> Result<(), String>
```

```typescript
await invoke('update_node', { node: updatedNode });
```

---

### update_node_content
Update only the content field (simpler API for editing).

```rust
fn update_node_content(state: State<AppState>, node_id: String, content: String) -> Result<(), String>
```

```typescript
await invoke('update_node_content', { nodeId: 'abc123', content: 'New content' });
```

---

### delete_node
Delete a node by ID.

```rust
fn delete_node(state: State<AppState>, id: String) -> Result<(), String>
```

```typescript
await invoke('delete_node', { id: 'abc123' });
```

---

## Edge Operations

### get_edges
Get all edges in the database.

```rust
fn get_edges(state: State<AppState>) -> Result<Vec<Edge>, String>
```

```typescript
const edges = await invoke<Edge[]>('get_edges');
```

---

### get_edges_for_node
Get all edges connected to a node (both directions).

```rust
fn get_edges_for_node(state: State<AppState>, node_id: String) -> Result<Vec<Edge>, String>
```

```typescript
const edges = await invoke<Edge[]>('get_edges_for_node', { nodeId: 'abc123' });
```

---

### create_edge
Insert a new edge.

```rust
fn create_edge(state: State<AppState>, edge: Edge) -> Result<(), String>
```

```typescript
await invoke('create_edge', { edge: { id: 'edge-1', sourceId: 'a', targetId: 'b', type: 'related' } });
```

---

### delete_edge
Delete an edge by ID.

```rust
fn delete_edge(state: State<AppState>, id: String) -> Result<(), String>
```

```typescript
await invoke('delete_edge', { id: 'edge-1' });
```

---

### get_edges_for_view
Get edges for a specific view (where both endpoints share the same parent).

```rust
fn get_edges_for_view(state: State<AppState>, parent_id: String) -> Result<Vec<Edge>, String>
```

Uses indexed lookup on `(source_parent_id, target_parent_id)` for O(1) performance.

```typescript
const edges = await invoke<Edge[]>('get_edges_for_view', { parentId: 'topic-id' });
```

---

### get_edges_for_fos
Get edges for a Field of Science category (papers).

```rust
fn get_edges_for_fos(state: State<AppState>, fos_id: String) -> Result<Vec<Edge>, String>
```

```typescript
const edges = await invoke<Edge[]>('get_edges_for_fos', { fosId: 'computer-science-id' });
```

---

## Hierarchy Navigation

### get_universe
Get the root Universe node (`is_universe = true`).

```rust
fn get_universe(state: State<AppState>) -> Result<Option<Node>, String>
```

```typescript
const universe = await invoke<Node | null>('get_universe');
```

---

### get_children
Get direct children of a parent node.

```rust
fn get_children(state: State<AppState>, parent_id: String) -> Result<Vec<Node>, String>
```

```typescript
const children = await invoke<Node[]>('get_children', { parentId: 'universe-id' });
```

---

### get_children_flat
Get children, skipping single-child intermediate levels.

```rust
fn get_children_flat(state: State<AppState>, parent_id: String) -> Result<Vec<Node>, String>
```

```typescript
// If parent has only 1 child which also has children, returns grandchildren
const children = await invoke<Node[]>('get_children_flat', { parentId: 'universe-id' });
```

---

### get_graph_children
Get visible children for graph display (excludes hidden content types).

```rust
fn get_graph_children(state: State<AppState>, parent_id: String) -> Result<Vec<Node>, String>
```

```typescript
const visibleChildren = await invoke<Node[]>('get_graph_children', { parentId: 'topic-id' });
```

---

### get_nodes_at_depth
Get all nodes at a specific depth level.

```rust
fn get_nodes_at_depth(state: State<AppState>, depth: i32) -> Result<Vec<Node>, String>
```

```typescript
const topicsAtDepth1 = await invoke<Node[]>('get_nodes_at_depth', { depth: 1 });
```

---

### get_items
Get all leaf items (`is_item = true`).

```rust
fn get_items(state: State<AppState>) -> Result<Vec<Node>, String>
```

```typescript
const items = await invoke<Node[]>('get_items');
```

---

### get_max_depth
Get the maximum depth in the hierarchy.

```rust
fn get_max_depth(state: State<AppState>) -> Result<i32, String>
```

```typescript
const maxDepth = await invoke<number>('get_max_depth');
```

---

### get_conversation_context
Get all messages in a conversation thread.

```rust
fn get_conversation_context(state: State<AppState>, conversation_id: String) -> Result<Vec<Node>, String>
```

```typescript
const messages = await invoke<Node[]>('get_conversation_context', { conversationId: 'conv-123' });
```

---

## Hierarchy Building

### build_hierarchy
Build initial hierarchy structure from clustered items.

```rust
fn build_hierarchy(state: State<AppState>) -> Result<HierarchyResult, String>
```

**Returns:** `{ created: number, depth: number }`

```typescript
const result = await invoke<{ created: number; depth: number }>('build_hierarchy');
```

---

### build_full_hierarchy
Complete pipeline: clustering → hierarchy → recursive AI grouping.

```rust
async fn build_full_hierarchy(
    app: AppHandle,
    state: State<AppState>,
    run_clustering: Option<bool>,  // default true
) -> Result<HierarchyResult, String>
```

**Emits:** `hierarchy-log` events during processing

```typescript
const result = await invoke<{ created: number; depth: number }>('build_full_hierarchy', {
  runClustering: true
});
```

---

### cluster_hierarchy_level
Group children of a node into 8-15 AI-generated categories.

```rust
async fn cluster_hierarchy_level(
    app: AppHandle,
    state: State<AppState>,
    parent_id: String,
) -> Result<usize, String>
```

**Returns:** Number of categories created

```typescript
const categoriesCreated = await invoke<number>('cluster_hierarchy_level', { parentId: 'node-id' });
```

---

### unsplit_node
Flatten a node's single-child intermediate levels.

```rust
fn unsplit_node(state: State<AppState>, parent_id: String) -> Result<usize, String>
```

**Returns:** Number of nodes moved up

```typescript
const moved = await invoke<number>('unsplit_node', { parentId: 'node-id' });
```

---

### flatten_hierarchy
Remove empty intermediate levels globally.

```rust
fn flatten_hierarchy(state: State<AppState>) -> Result<usize, String>
```

```typescript
const removed = await invoke<number>('flatten_hierarchy');
```

---

### consolidate_root
AI-powered consolidation of Universe children into balanced categories.

```rust
async fn consolidate_root(state: State<AppState>) -> Result<ConsolidateResult, String>
```

**Returns:** `{ categories_created: number, items_moved: number }`

```typescript
const result = await invoke<{ categoriesCreated: number; itemsMoved: number }>('consolidate_root');
```

---

### quick_add_to_hierarchy
Find best parent for uncategorized items and assign them.

```rust
async fn quick_add_to_hierarchy(state: State<AppState>) -> Result<QuickAddResult, String>
```

```typescript
const result = await invoke('quick_add_to_hierarchy');
```

---

### smart_add_to_hierarchy
Smart placement of orphan items using embedding similarity.

```rust
async fn smart_add_to_hierarchy(state: State<AppState>) -> Result<SmartAddResult, String>
```

```typescript
const result = await invoke<{ itemsPlaced: number; categoriesCreated: number }>('smart_add_to_hierarchy');
```

---

### propagate_latest_dates
Bubble up `latest_child_date` from descendants to ancestors.

```rust
fn propagate_latest_dates(state: State<AppState>) -> Result<(), String>
```

```typescript
await invoke('propagate_latest_dates');
```

---

### tidy_database
Clean up orphan edges, recount children, fix inconsistencies.

```rust
fn tidy_database(state: State<AppState>) -> Result<TidyReport, String>
```

```typescript
const report = await invoke<TidyReport>('tidy_database');
```

---

## Clustering

### run_clustering
Assign `cluster_id` to items needing clustering.

```rust
async fn run_clustering(state: State<AppState>, use_ai: Option<bool>) -> Result<ClusterResult, String>
```

**Parameters:**
- `use_ai`: Use AI clustering (default `true`), falls back to TF-IDF

```typescript
const result = await invoke<ClusterResult>('run_clustering', { useAi: true });
```

---

### recluster_all
Force re-clustering of all items.

```rust
async fn recluster_all(state: State<AppState>, use_ai: Option<bool>) -> Result<ClusterResult, String>
```

```typescript
const result = await invoke<ClusterResult>('recluster_all', { useAi: true });
```

---

### get_clustering_status
Get clustering statistics.

```rust
fn get_clustering_status(state: State<AppState>) -> Result<ClusteringStatus, String>
```

**Returns:**
```typescript
interface ClusteringStatus {
  itemsNeedingClustering: number;
  totalItems: number;
  aiAvailable: boolean;
}
```

```typescript
const status = await invoke<ClusteringStatus>('get_clustering_status');
```

---

## AI Processing

### process_nodes
AI-analyze unprocessed nodes: generates titles, summaries, tags, content_type.

```rust
async fn process_nodes(app: AppHandle, state: State<AppState>) -> Result<ProcessingResult, String>
```

**Emits:** `ai-progress` events with:
```typescript
interface AiProgressEvent {
  current: number;
  total: number;
  nodeTitle: string;
  newTitle: string;
  contentType: string | null;
  status: 'processing' | 'success' | 'error' | 'complete' | 'cancelled';
  errorMessage: string | null;
  elapsedSecs: number | null;
  estimateSecs: number | null;
  remainingSecs: number | null;
}
```

```typescript
const result = await invoke<ProcessingResult>('process_nodes');
```

---

### get_ai_status
Get AI processing statistics.

```rust
fn get_ai_status(state: State<AppState>) -> Result<AiStatus, String>
```

```typescript
interface AiStatus {
  available: boolean;
  totalNodes: number;
  processedNodes: number;
  unprocessedNodes: number;
}
```

---

### get_learned_emojis
Get emoji mappings learned by AI.

```rust
fn get_learned_emojis(state: State<AppState>) -> Result<HashMap<String, String>, String>
```

```typescript
const emojis = await invoke<Record<string, string>>('get_learned_emojis');
// { "rust": "🦀", "react": "⚛️", ... }
```

---

### save_learned_emoji
Save an emoji mapping.

```rust
fn save_learned_emoji(state: State<AppState>, keyword: String, emoji: String) -> Result<(), String>
```

```typescript
await invoke('save_learned_emoji', { keyword: 'typescript', emoji: '🔷' });
```

---

### regenerate_all_embeddings
Regenerate embeddings for all items.

```rust
async fn regenerate_all_embeddings(
    app: AppHandle,
    state: State<AppState>,
    use_local: Option<bool>,
) -> Result<EmbeddingResult, String>
```

**Emits:** `embedding-progress` events

```typescript
const result = await invoke('regenerate_all_embeddings', { useLocal: true });
```

---

## Content Classification

### classify_and_associate
Classify all items by content type and create associations.

```rust
fn classify_and_associate(state: State<AppState>) -> Result<(usize, usize), String>
```

**Returns:** `(classified_count, associated_count)`

```typescript
const [classified, associated] = await invoke<[number, number]>('classify_and_associate');
```

---

### classify_and_associate_children
Classify items under a specific parent.

```rust
fn classify_and_associate_children(state: State<AppState>, parent_id: String) -> Result<(usize, usize), String>
```

```typescript
const [classified, associated] = await invoke<[number, number]>('classify_and_associate_children', { parentId: 'topic-id' });
```

---

### get_supporting_counts
Get counts of supporting content types under a parent.

```rust
fn get_supporting_counts(state: State<AppState>, parent_id: String) -> Result<SupportingCounts, String>
```

```typescript
interface SupportingCounts { code: number; debug: number; paste: number; }
const counts = await invoke<SupportingCounts>('get_supporting_counts', { parentId: 'topic-id' });
```

---

## Rebuild Lite

Fast hierarchy refresh without full AI processing.

### reclassify_pattern
Pattern-based content type classification.

```rust
fn reclassify_pattern(state: State<AppState>) -> Result<usize, String>
```

```typescript
const classified = await invoke<number>('reclassify_pattern');
```

---

### reclassify_ai
AI-powered content type reclassification.

```rust
async fn reclassify_ai(app: AppHandle, state: State<AppState>) -> Result<usize, String>
```

**Emits:** `reclassify-progress` events

```typescript
const classified = await invoke<number>('reclassify_ai');
```

---

### rebuild_lite
Quick rebuild: pattern classification + hierarchy refresh.

```rust
async fn rebuild_lite(app: AppHandle, state: State<AppState>) -> Result<RebuildLiteResult, String>
```

```typescript
const result = await invoke('rebuild_lite');
```

---

### rebuild_hierarchy_only
Rebuild just the hierarchy (no clustering or AI).

```rust
async fn rebuild_hierarchy_only(app: AppHandle, state: State<AppState>) -> Result<HierarchyResult, String>
```

```typescript
const result = await invoke('rebuild_hierarchy_only');
```

---

## Import

### import_claude_conversations
Import Claude conversation JSON export.

```rust
fn import_claude_conversations(state: State<AppState>, json_content: String) -> Result<ImportResult, String>
```

**Returns:**
```typescript
interface ImportResult {
  imported: number;
  skipped: number;
  errors: string[];
}
```

```typescript
const result = await invoke<ImportResult>('import_claude_conversations', { jsonContent: rawJson });
```

---

### import_markdown_files
Import markdown files.

```rust
fn import_markdown_files(state: State<AppState>, file_paths: Vec<String>) -> Result<ImportResult, String>
```

```typescript
const result = await invoke<ImportResult>('import_markdown_files', {
  filePaths: ['/path/to/file1.md', '/path/to/file2.md']
});
```

---

### import_google_keep
Import Google Keep archive (Takeout zip).

```rust
fn import_google_keep(state: State<AppState>, zip_path: String) -> Result<GoogleKeepImportResult, String>
```

```typescript
const result = await invoke('import_google_keep', { zipPath: '/path/to/takeout.zip' });
```

---

### import_openaire
Import scientific papers from OpenAIRE API.

```rust
async fn import_openaire(
    state: State<AppState>,
    app: AppHandle,
    query: String,
    max_results: Option<usize>,
) -> Result<OpenAireImportResult, String>
```

**Emits:** `openaire-progress` events

```typescript
const result = await invoke('import_openaire', { query: 'machine learning', maxResults: 100 });
```

---

### count_openaire_papers
Count papers that would match an OpenAIRE query.

```rust
async fn count_openaire_papers(query: String) -> Result<usize, String>
```

---

### cancel_openaire
Cancel ongoing OpenAIRE import.

```rust
fn cancel_openaire(state: State<AppState>) -> Result<(), String>
```

---

### get_imported_paper_count
Get count of imported papers.

```rust
fn get_imported_paper_count(state: State<AppState>) -> Result<usize, String>
```

---

### import_code
Import source code from a directory. Parses Rust, TypeScript, and Markdown files. Respects .gitignore.

```rust
fn import_code(
    state: State<AppState>,
    path: String,
    language: Option<String>,
) -> Result<CodeImportResult, String>
```

**Returns:**
```typescript
interface CodeImportResult {
  functions: number;
  structs: number;
  enums: number;
  traits: number;
  impls: number;
  modules: number;
  macros: number;
  docs: number;
  files_processed: number;
  files_skipped: number;
  edges_created: number;
  doc_edges: number;
  errors: string[];
}
```

```typescript
const result = await invoke<CodeImportResult>('import_code', {
  path: '/path/to/project',
  language: null  // or 'rust', 'typescript'
});
```

---

### analyze_code_edges
Analyze code and create "Calls" edges between functions.

```rust
fn analyze_code_edges(
    state: State<AppState>,
    path_filter: Option<String>,
) -> Result<CodeEdgesResult, String>
```

**Returns:**
```typescript
interface CodeEdgesResult {
  functions_analyzed: number;
  edges_found: number;
  edges_created: number;
}
```

```typescript
const result = await invoke<CodeEdgesResult>('analyze_code_edges', {
  pathFilter: 'src/'  // optional path filter
});
```

---

## Paper Operations

### get_paper_metadata
Get metadata for a paper by node ID.

```rust
fn get_paper_metadata(state: State<AppState>, node_id: String) -> Result<Option<Paper>, String>
```

---

### get_paper_pdf
Get stored PDF blob for a paper.

```rust
fn get_paper_pdf(state: State<AppState>, node_id: String) -> Result<Option<Vec<u8>>, String>
```

---

### has_paper_pdf
Check if a paper has a stored PDF.

```rust
fn has_paper_pdf(state: State<AppState>, node_id: String) -> Result<bool, String>
```

---

### download_paper_on_demand
Download and store PDF for a paper.

```rust
async fn download_paper_on_demand(state: State<AppState>, node_id: String) -> Result<bool, String>
```

---

### open_paper_external
Open paper PDF in external viewer.

```rust
fn open_paper_external(state: State<AppState>, node_id: String) -> Result<(), String>
```

---

### get_paper_document
Get paper as Document struct for Leaf view.

```rust
fn get_paper_document(state: State<AppState>, node_id: String) -> Result<Option<Document>, String>
```

---

### reformat_paper_abstracts
AI-format abstracts with section headers.

```rust
async fn reformat_paper_abstracts(state: State<AppState>, app: AppHandle) -> Result<usize, String>
```

---

### sync_paper_pdf_status
Sync `pdf_available` flag with actual storage.

```rust
fn sync_paper_pdf_status(state: State<AppState>) -> Result<usize, String>
```

---

### sync_paper_dates
Sync node dates with paper publication dates.

```rust
fn sync_paper_dates(state: State<AppState>) -> Result<usize, String>
```

---

## Quick Access (Sidebar)

### set_node_pinned
Pin/unpin a node for quick access.

```rust
fn set_node_pinned(state: State<AppState>, node_id: String, pinned: bool) -> Result<(), String>
```

```typescript
await invoke('set_node_pinned', { nodeId: 'abc123', pinned: true });
```

---

### touch_node
Update `last_accessed_at` timestamp (for recency tracking).

```rust
fn touch_node(state: State<AppState>, node_id: String) -> Result<(), String>
```

```typescript
await invoke('touch_node', { nodeId: 'abc123' });
```

---

### get_pinned_nodes
Get all pinned nodes.

```rust
fn get_pinned_nodes(state: State<AppState>) -> Result<Vec<Node>, String>
```

```typescript
const pinned = await invoke<Node[]>('get_pinned_nodes');
```

---

### get_recent_nodes
Get recently accessed nodes.

```rust
fn get_recent_nodes(state: State<AppState>, limit: Option<i32>) -> Result<Vec<Node>, String>
```

```typescript
const recent = await invoke<Node[]>('get_recent_nodes', { limit: 15 });
```

---

### clear_recent
Clear a node's last_accessed_at timestamp.

```rust
fn clear_recent(state: State<AppState>, node_id: String) -> Result<(), String>
```

```typescript
await invoke('clear_recent', { nodeId: 'abc123' });
```

---

## Semantic Similarity

### get_similar_nodes
Find nodes semantically similar to a given node.

```rust
fn get_similar_nodes(
    state: State<AppState>,
    node_id: String,
    limit: Option<usize>,  // default 10
    min_similarity: Option<f32>,  // default 0.3
) -> Result<Vec<SimilarNode>, String>
```

**Returns:**
```typescript
interface SimilarNode {
  node: Node;
  similarity: number;  // 0.0-1.0
}
```

```typescript
const similar = await invoke<SimilarNode[]>('get_similar_nodes', {
  nodeId: 'abc123',
  limit: 5,
  minSimilarity: 0.5
});
```

---

### get_embedding_status
Get embedding generation statistics.

```rust
fn get_embedding_status(state: State<AppState>) -> Result<EmbeddingStatus, String>
```

```typescript
interface EmbeddingStatus {
  nodesWithEmbeddings: number;
  totalItems: number;
  coverage: number;  // percentage
}
```

---

### search_nodes
Full-text search using FTS5.

```rust
fn search_nodes(state: State<AppState>, query: String) -> Result<Vec<Node>, String>
```

```typescript
const results = await invoke<Node[]>('search_nodes', { query: 'react hooks' });
```

---

## Privacy

### get_privacy_stats
Get privacy scanning statistics.

```rust
fn get_privacy_stats(state: State<AppState>) -> Result<PrivacyStats, String>
```

```typescript
interface PrivacyStats {
  total: number;
  scanned: number;
  unscanned: number;
  private: number;
  safe: number;
  totalCategories: number;
  scannedCategories: number;
}
```

---

### analyze_node_privacy
AI-analyze a single node for privacy.

```rust
async fn analyze_node_privacy(state: State<AppState>, node_id: String) -> Result<PrivacyResult, String>
```

```typescript
interface PrivacyResult {
  isPrivate: boolean;
  reason: string | null;
}
```

---

### analyze_all_privacy
Batch analyze all unscanned items.

```rust
async fn analyze_all_privacy(
    state: State<AppState>,
    app: AppHandle,
    showcase_mode: Option<bool>,  // stricter filtering
) -> Result<PrivacyReport, String>
```

**Emits:** `privacy-progress` events

```typescript
const report = await invoke<PrivacyReport>('analyze_all_privacy', { showcaseMode: false });
```

---

### analyze_categories_privacy
Analyze category nodes, propagate to descendants.

```rust
async fn analyze_categories_privacy(
    state: State<AppState>,
    app: AppHandle,
    showcase_mode: Option<bool>,
) -> Result<CategoryPrivacyReport, String>
```

---

### score_privacy_all_items
Batch score items on 0.0-1.0 privacy scale.

```rust
async fn score_privacy_all_items(
    state: State<AppState>,
    app: AppHandle,
    force_rescore: bool,
) -> Result<PrivacyScoringResult, String>
```

**Emits:** `privacy-scoring-progress` events

---

### set_node_privacy
Manually set a node's privacy status.

```rust
fn set_node_privacy(state: State<AppState>, node_id: String, is_private: bool) -> Result<SetPrivacyResult, String>
```

**Returns:** `{ affectedIds: string[] }` — includes propagated descendants

---

### reset_privacy_flags
Clear all privacy flags to re-scan.

```rust
fn reset_privacy_flags(state: State<AppState>) -> Result<usize, String>
```

---

### cancel_privacy_scan
Cancel ongoing privacy scan.

```rust
fn cancel_privacy_scan() -> Result<(), String>
```

---

### get_export_preview
Preview export counts at given threshold.

```rust
fn get_export_preview(
    state: State<AppState>,
    min_privacy: f64,
    include_tags: Option<Vec<String>>,
) -> Result<ExportPreview, String>
```

```typescript
interface ExportPreview {
  included: number;
  excluded: number;
  unscored: number;
}
```

---

### export_shareable_db
Export database with private content removed.

```rust
fn export_shareable_db(
    state: State<AppState>,
    min_privacy: f64,  // 0.0-1.0 threshold
    include_tags: Option<Vec<String>>,  // optional tag whitelist
) -> Result<String, String>  // returns path
```

```typescript
const exportPath = await invoke<string>('export_shareable_db', {
  minPrivacy: 0.7,
  includeTags: ['mycelica', 'public']
});
```

---

## Settings & State

### get_api_key_status
Get Anthropic API key status.

```rust
fn get_api_key_status() -> ApiKeyStatus
```

```typescript
interface ApiKeyStatus {
  hasKey: boolean;
  source: 'env' | 'settings' | null;
  masked: string | null;  // "sk-...abc"
}
```

---

### save_api_key
Save Anthropic API key to settings.

```rust
fn save_api_key(key: String) -> Result<(), String>
```

---

### clear_api_key
Remove saved API key.

```rust
fn clear_api_key() -> Result<(), String>
```

---

### get_openai_api_key_status
Get OpenAI API key status (for embeddings).

```rust
fn get_openai_api_key_status() -> Result<Option<String>, String>
```

---

### save_openai_api_key
Save OpenAI API key.

```rust
fn save_openai_api_key(key: String) -> Result<(), String>
```

---

### clear_openai_api_key
Remove OpenAI API key.

```rust
fn clear_openai_api_key() -> Result<(), String>
```

---

### get_openaire_api_key_status
Get OpenAIRE API key status.

```rust
fn get_openaire_api_key_status() -> Result<Option<String>, String>
```

---

### save_openaire_api_key
Save OpenAIRE API key.

```rust
fn save_openaire_api_key(key: String) -> Result<(), String>
```

---

### clear_openaire_api_key
Remove OpenAIRE API key.

```rust
fn clear_openaire_api_key() -> Result<(), String>
```

---

### get_pipeline_state
Get current pipeline processing state.

```rust
fn get_pipeline_state(state: State<AppState>) -> Result<String, String>
```

**States:** `fresh`, `imported`, `processed`, `clustered`, `hierarchized`, `complete`

---

### set_pipeline_state
Set pipeline state.

```rust
fn set_pipeline_state(state: State<AppState>, pipeline_state: String) -> Result<(), String>
```

---

### get_db_metadata
Get all metadata key-value pairs.

```rust
fn get_db_metadata(state: State<AppState>) -> Result<Vec<(String, String, i64)>, String>
```

---

### get_processing_stats
Get cumulative processing time stats.

```rust
fn get_processing_stats() -> ProcessingStats
```

```typescript
interface ProcessingStats {
  totalAiProcessingSecs: number;
  totalRebuildSecs: number;
}
```

---

### add_ai_processing_time / add_rebuild_time
Track processing times.

```rust
fn add_ai_processing_time(elapsed_secs: f64) -> Result<(), String>
fn add_rebuild_time(elapsed_secs: f64) -> Result<(), String>
```

---

### get_use_local_embeddings / set_use_local_embeddings
Toggle local vs API embeddings.

```rust
fn get_use_local_embeddings() -> bool
fn set_use_local_embeddings(enabled: bool) -> Result<(), String>
```

---

### get_protect_recent_notes / set_protect_recent_notes
Toggle protection of Recent Notes from AI processing.

```rust
fn get_protect_recent_notes() -> bool
fn set_protect_recent_notes(protected: bool) -> Result<(), String>
```

---

### get_clustering_thresholds / set_clustering_thresholds
Get/set clustering similarity thresholds.

```rust
fn get_clustering_thresholds() -> (Option<f32>, Option<f32>)
fn set_clustering_thresholds(primary: Option<f32>, secondary: Option<f32>) -> Result<(), String>
```

```typescript
const [primary, secondary] = await invoke<[number | null, number | null]>('get_clustering_thresholds');
await invoke('set_clustering_thresholds', { primary: 0.7, secondary: 0.5 });
```

---

### get_privacy_threshold / set_privacy_threshold
Get/set privacy score threshold for filtering.

```rust
fn get_privacy_threshold() -> f32
fn set_privacy_threshold(threshold: f32) -> Result<(), String>
```

```typescript
const threshold = await invoke<number>('get_privacy_threshold');
await invoke('set_privacy_threshold', { threshold: 0.7 });
```

---

### get_show_tips / set_show_tips
Toggle UI tips display.

```rust
fn get_show_tips() -> bool
fn set_show_tips(enabled: bool) -> Result<(), String>
```

---

## Database Management

### get_db_path
Get current database file path.

```rust
fn get_db_path(state: State<AppState>) -> Result<String, String>
```

---

### switch_database
Switch to a different database file.

```rust
fn switch_database(app: AppHandle, state: State<AppState>, db_path: String) -> Result<DbStats, String>
```

---

### get_db_stats
Get database statistics.

```rust
fn get_db_stats(state: State<AppState>) -> Result<DbStats, String>
```

```typescript
interface DbStats {
  totalNodes: number;
  totalItems: number;
  processedItems: number;
  itemsWithEmbeddings: number;
  unprocessedItems: number;
  unclusteredItems: number;
  orphanItems: number;
  topicsCount: number;
}
```

---

### delete_all_data
Delete all nodes and edges.

```rust
fn delete_all_data(state: State<AppState>) -> Result<DeleteResult, String>
```

---

### reset_ai_processing
Mark all nodes as unprocessed.

```rust
fn reset_ai_processing(state: State<AppState>) -> Result<usize, String>
```

---

### reset_clustering
Mark all items as needing clustering.

```rust
fn reset_clustering(state: State<AppState>) -> Result<usize, String>
```

---

### clear_embeddings
Remove all embeddings and semantic edges.

```rust
fn clear_embeddings(state: State<AppState>) -> Result<usize, String>
```

---

### clear_hierarchy
Delete non-item nodes, clear parent_id on items.

```rust
fn clear_hierarchy(state: State<AppState>) -> Result<usize, String>
```

---

### clear_tags
Delete all tags and item_tags.

```rust
fn clear_tags(state: State<AppState>) -> Result<usize, String>
```

---

### delete_empty_nodes
Remove items with no content.

```rust
fn delete_empty_nodes(state: State<AppState>) -> Result<usize, String>
```

---

### export_trimmed_database
Export database without PDF blobs (smaller file size).

```rust
fn export_trimmed_database(state: State<AppState>, output_path: String) -> Result<String, String>
```

```typescript
const exportPath = await invoke<string>('export_trimmed_database', {
  outputPath: '/path/to/export.db'
});
```

---

### get_leaf_content
Get full content for Leaf view.

```rust
fn get_leaf_content(state: State<AppState>, node_id: String) -> Result<String, String>
```

---

## Cancellation

### cancel_processing
Cancel AI processing.

```rust
fn cancel_processing() -> Result<(), String>
```

---

### cancel_rebuild
Cancel rebuild/clustering operations.

```rust
fn cancel_rebuild() -> Result<(), String>
```

---

### cancel_all
Cancel all operations (AI, clustering, hierarchy).

```rust
fn cancel_all() -> Result<(), String>
```

```typescript
await invoke('cancel_all');
```

---

## Multi-Path Associations

### get_item_associations
Get `belongs_to` edges for an item (multi-category membership).

```rust
fn get_item_associations(state: State<AppState>, item_id: String) -> Result<Vec<Edge>, String>
```

---

### get_related_items
Get items related via edges.

```rust
fn get_related_items(state: State<AppState>, item_id: String, min_weight: Option<f64>) -> Result<Vec<Node>, String>
```

---

### get_category_items
Get items in a cluster via edges.

```rust
fn get_category_items(state: State<AppState>, cluster_id: i32, min_weight: Option<f64>) -> Result<Vec<Node>, String>
```

---

### get_supporting_items
Get supporting (hidden) items under a parent.

```rust
fn get_supporting_items(state: State<AppState>, parent_id: String) -> Result<Vec<Node>, String>
```

---

### get_associated_items
Get items associated with an idea.

```rust
fn get_associated_items(state: State<AppState>, idea_id: String) -> Result<Vec<Node>, String>
```

---

## Event Listeners

Commands emit events for real-time progress:

```typescript
import { listen } from '@tauri-apps/api/event';

// AI processing progress
listen<AiProgressEvent>('ai-progress', (event) => {
  console.log(`${event.payload.current}/${event.payload.total}: ${event.payload.nodeTitle}`);
});

// Hierarchy building logs
listen<HierarchyLogEvent>('hierarchy-log', (event) => {
  console.log(`[${event.payload.level}] ${event.payload.message}`);
});

// Privacy scanning progress
listen<PrivacyProgressEvent>('privacy-progress', (event) => {
  console.log(`Privacy: ${event.payload.current}/${event.payload.total}`);
});

// Embedding generation progress
listen<EmbeddingProgressEvent>('embedding-progress', (event) => {
  console.log(`Embeddings: ${event.payload.current}/${event.payload.total}`);
});
```

---

## Error Handling

All commands return `Result<T, String>`. Frontend handles:

```typescript
try {
  const node = await invoke<Node>('get_node', { id: 'abc123' });
} catch (error) {
  console.error('Command failed:', error);  // error is the String from Rust
}
```

---

*Last updated: 2026-01-10*
