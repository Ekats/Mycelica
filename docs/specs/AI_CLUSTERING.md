# Embedding-Based Clustering Specification

> **Note:** This document reflects the actual implementation as of 2026-01-08.
> Previous documentation described AI-batch clustering which is no longer used.

## Overview

Mycelica uses **embedding-based cosine similarity clustering** (deterministic, local) as the primary clustering method. AI is only used for:
1. **Naming clusters** after they're formed (optional)
2. **Project detection** via capitalized word analysis
3. **Uber-category grouping** when too many topics exist

---

## Clustering Flow

### Entry Points

| Function | Description |
|----------|-------------|
| `run_clustering(db, use_ai)` | Main entry: clusters items needing clustering |
| `cluster_with_embeddings(db, items)` | Direct embedding-based clustering |
| `cluster_with_embeddings_lite(db)` | Fast variant without AI naming |
| `cluster_with_fos_pregrouping(db)` | For papers: FOS-based grouping first |

### Core Algorithm

```
Items needing clustering (needs_clustering = 1)
    ↓
Ensure embeddings exist (generate if missing)
    ↓
Calculate cosine similarity matrix
    ↓
Cluster using adaptive thresholds:
  - primary_threshold: 0.72-0.78 (adaptive to collection size)
  - secondary_threshold: 0.65-0.70
    ↓
Merge small clusters (< 3 items) into nearest neighbor
    ↓
[Optional] AI names clusters in batches
    ↓
Update DB: cluster_id, cluster_label
    ↓
Set needs_clustering = 0
```

---

## Embedding Generation

Embeddings are 384-dimensional float32 vectors from OpenAI's `text-embedding-3-small` model (or local fallback).

```rust
// From ai_client.rs
pub async fn generate_embedding(text: &str) -> Result<Vec<f32>, String>
```

- Truncates input to ~1000 bytes for embedding
- Cached in `nodes.embedding` BLOB column
- Falls back to local embeddings if OPENAI_API_KEY not set

---

## Adaptive Thresholds

Thresholds adjust based on collection size:

| Collection Size | Primary | Secondary |
|-----------------|---------|-----------|
| < 50 items | 0.78 | 0.70 |
| 50-200 items | 0.75 | 0.67 |
| 200-500 items | 0.73 | 0.65 |
| > 500 items | 0.72 | 0.64 |

Users can override via settings: `get_clustering_thresholds()` / `set_clustering_thresholds()`.

---

## Cluster Naming (AI-Assisted)

After clusters form, AI names them in batches:

```
NAMING_BATCH_SIZE = 30 clusters per API call
```

### Naming Prompt Pattern

```
Given these item titles grouped into clusters, suggest a 2-4 word name for each cluster:

Cluster 0:
- "Building Rust Backend..."
- "Tauri vs Electron..."
- "Cross-platform Development..."

Cluster 1:
- "React Component Patterns..."
- "TypeScript Generics..."

Return JSON:
[
  {"cluster_id": 0, "name": "Desktop App Development"},
  {"cluster_id": 1, "name": "Frontend TypeScript"}
]
```

---

## FOS Pre-Grouping (Papers)

For scientific papers imported from OpenAIRE, FOS (Field of Science) pre-grouping:

1. **Create FOS parent nodes** - One per field (e.g., "Computer Science", "Medicine")
2. **Assign papers to FOS** - Via `papers.subjects` metadata
3. **Cluster within each FOS** - Preserves field boundaries
4. **Generate cross-FOS edges** - For related papers across fields

```rust
pub async fn cluster_with_fos_pregrouping(db: &Database) -> Result<ClusteringResult, String>
```

---

## Project Detection

Identifies named projects via capitalized word analysis:

```rust
pub fn collect_capitalized_words(db: &Database) -> Vec<CandidateWord>
pub async fn detect_projects_with_ai(candidates: Vec<CandidateWord>) -> Result<Vec<String>, String>
```

Finds patterns like "Mycelica", "ObsidianMD", "TauriApp" across item titles/content.

---

## Uber-Category Grouping

When too many topics exist (> 150), groups them into higher-level categories:

```rust
BATCH_SIZE_FOR_GROUPING = 150 topics per API call

pub async fn group_into_uber_categories(
    topics: Vec<String>,
) -> Result<Vec<(String, Vec<String>)>, String>
```

---

## Edge Generation

Clustering also creates `related` edges between semantically similar items:

```rust
// From clustering.rs
fn generate_edges_for_cluster(
    db: &Database,
    cluster_items: &[(String, Vec<f32>)],
    cluster_id: i32,
) -> Result<usize, String>
```

- Edge weight = cosine similarity
- Only creates edges above secondary threshold
- Edges limited per item to prevent explosion

---

## Database State

### nodes table columns

| Column | Description |
|--------|-------------|
| `cluster_id` | Assigned cluster (INTEGER) |
| `cluster_label` | AI-generated cluster name |
| `needs_clustering` | 1 = pending, 0 = done |
| `embedding` | 384-dim float32 BLOB |

### Pipeline state

```
fresh → imported → processed → clustered → hierarchized → complete
```

---

## UX Considerations

1. **Deterministic**: Same embeddings produce same clusters (reproducible)
2. **Fast**: No API calls for clustering itself, only for optional naming
3. **Incremental**: Only processes items with `needs_clustering = 1`
4. **Offline capable**: Works without API (no cluster names, but clustering works)

---

## Comparison: Old vs New

| Aspect | Old (Documented) | New (Actual) |
|--------|------------------|--------------|
| Method | AI batch classification | Embedding cosine similarity |
| API calls | 10-20 items per call | 0 for clustering, 30 clusters per naming call |
| Determinism | Non-deterministic | Deterministic |
| Offline | TF-IDF fallback | Full clustering, no names |
| Speed | Slow (many API calls) | Fast (local computation) |

---

*Last updated: 2026-01-08*
