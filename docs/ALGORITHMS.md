# Mycelica Algorithms Specification

Mathematical and algorithmic specification for clustering, hierarchy building, and semantic processing.

---

## Table of Contents

1. [Similarity Functions](#1-similarity-functions)
2. [Initial Clustering](#2-initial-clustering)
3. [Semantic Edges](#3-semantic-edges)
4. [TF-IDF Keyword Extraction](#4-tf-idf-keyword-extraction)
5. [Content Type Tiers](#5-content-type-tiers)
6. [Privacy Filtering](#6-privacy-filtering)
7. [Adaptive Tree Algorithm](#7-adaptive-tree-algorithm)
8. [Category Naming](#8-category-naming)
9. [Setup Pipeline](#9-setup-pipeline)
10. [Constants Summary](#10-constants-summary)
11. [File References](#11-file-references)

---

## 1. Similarity Functions

### Cosine Similarity

```
cos(a, b) = (a · b) / (||a|| × ||b||)
         = Σ(aᵢ × bᵢ) / (√Σaᵢ² × √Σbᵢ²)
```

- **Range:** [-1, 1]
- **Interpretation:** 1 = identical, 0 = orthogonal, -1 = opposite
- **Implementation:** `src-tauri/src/similarity.rs`

### Centroid (L2-normalized mean)

```
centroid(E) = normalize(Σeᵢ / |E|)
normalize(v) = v / ||v||₂
```

Where `||v||₂ = √Σvᵢ²`

- **Implementation:** `src-tauri/src/similarity.rs`

---

## 2. Semantic Edges

Created by `setup` step 2 to enable hierarchy edge-based grouping:

```
min_similarity = 0.50
max_edges_per_node = 5
```

```python
def create_semantic_edges(db, min_similarity, max_edges):
    embeddings = get_all_embeddings()

    for node_id, embedding in embeddings:
        similar = find_similar(embedding, embeddings,
                               exclude=node_id,
                               top_n=max_edges,
                               min_similarity=min_similarity)

        for (neighbor_id, sim) in similar:
            create_edge(source=node_id, target=neighbor_id,
                       type="Related", weight=sim)
```

---

## 4. TF-IDF Keyword Extraction

Used for fallback cluster naming:

```python
def extract_keywords(text, top_n):
    words = tokenize(text.lower())

    # Filter
    words = [w for w in words
             if 3 <= len(w) <= 25
             and w not in STOP_WORDS
             and not w.isdigit()]

    # Count frequencies
    word_counts = Counter(words)
    total_words = len(words)

    # Calculate TF with boost
    scored = []
    for word, count in word_counts.items():
        tf = count / total_words
        boost = 1.0 + log(count) if count > 1 else 1.0
        scored.append((word, tf * boost))

    # Sort by score descending
    scored.sort(key=lambda x: -x[1])
    return scored[:top_n]
```

- **Implementation:** `src-tauri/src/clustering.rs`

---

## 5. Content Type Tiers

| Tier | Types | Clustered? | In Hierarchy? |
|------|-------|------------|---------------|
| **VISIBLE** | insight (alias: idea), exploration, synthesis, question, planning, paper, bookmark | Yes | Yes |
| **SUPPORTING** | investigation, discussion, reference, creative | No | No |
| **HIDDEN** | code_*, debug, paste, trivial | No | No |
| **SPECIAL** | session | No | No |

- **Implementation:** `src-tauri/src/clustering.rs`

---

## 6. Privacy Filtering

```python
privacy_threshold = settings.get_privacy_threshold()

# Partition items
private_items = [i for i in items if i.privacy < privacy_threshold]
public_items = [i for i in items if i.privacy >= privacy_threshold]

# Private items → "Personal" category (excluded from clustering)
# Public items → Normal hierarchy
```

---

## 7. Adaptive Tree Algorithm

### Core Insight

Edge weights already encode hierarchical structure. The algorithm reads this structure rather than computing it twice.

```
edges → dendrogram → adaptive cuts → tree
```

### Data Model

```
Papers (leaves)     ──edges──►  Dendrogram (merge tree)  ──cuts──►  Tree (nested groups)
     │                                │                                   │
 similarity                      union-find                          validated
  scores                         on sorted                            splits
                                  edges
```

### Edge Threshold vs Cut Thresholds

**Problem**: At 0.5 edge threshold, most papers merge into one giant blob early. The dendrogram lacks internal structure to find.

**Solution**: Decouple edge creation from hierarchy cuts.

```
EDGE_FLOOR = 0.3    ← Store edges down to here (more granularity)
CUT_RANGE = 0.3-1.0 ← Hierarchy cuts happen anywhere in this range
```

**Why this works**:
- Edges at 0.4-0.5 may represent real (weaker) relationships
- The giant blob at 0.5 might have sub-structure at 0.45, 0.42, etc.
- More edges = more merge events = finer dendrogram resolution

### Similarity Metrics

**Intra-group similarity** (cohesion within a group):
```
intra(G) = mean{ sim(pᵢ, pⱼ) | pᵢ, pⱼ ∈ G, i ≠ j }
```

**Inter-group similarity** (separation between sibling groups):
```
inter(A, B) = mean{ sim(pᵢ, pⱼ) | pᵢ ∈ A, pⱼ ∈ B }
```

**Cohesion ratio** (validity of a split):
```
                     intra(A) + intra(B)
cohesion_ratio(A,B) = ───────────────────
                        2 × inter(A, B)
```

Valid split requires: `cohesion_ratio > 1.2`

### Split Quality Metric

For a split producing children C₁, C₂, ..., Cₙ with sizes s₁, s₂, ..., sₙ:

**Balance score**:
```
balance = min(s₁..sₙ) / mean(s₁..sₙ)
```

**Split quality**:
```
quality = n × balance
```

Where `n` = number of children.

**Examples** (1000 papers):

| Split | n | balance | quality |
|-------|---|---------|---------|
| 500/500 | 2 | 1.0 | 2.0 |
| 400/350/250 | 3 | 0.75 | 2.25 |
| 250/250/250/250 | 4 | 1.0 | 4.0 |
| 200×5 | 5 | 1.0 | 5.0 |
| 800/100/100 | 3 | 0.3 | 0.9 |

Higher quality = more structure found with better balance.

### Adaptive Balance Threshold

Minimum acceptable child ratio scales with parent size:

```
                    ⎧ 0.05  if parent_size > 500
min_ratio(size) =   ⎨ 0.08  if parent_size > 200
                    ⎪ 0.12  if parent_size > 50
                    ⎩ 0.25  otherwise
```

| Parent | Min child | Ratio |
|--------|-----------|-------|
| 1000 | 50 | 5% |
| 200 | 16 | 8% |
| 50 | 6 | 12% |
| 20 | 5 | 25% |

### Minimum Similarity Gap (Δ)

**Problem**: Cuts at 0.71 and 0.72 are noise, not structure.

**Solution**: Enforce minimum gap between cuts.

```
Δ_min = 0.03
```

**Rules**:
1. Child's cut must be ≥ Δ_min away from parent's cut
2. Child's similarity range must be ≥ Δ_min wide to recurse
3. If range too narrow → become leaf (already as tight as meaningful)

### Bridge Detection

**Problem**: Papers near the cut threshold belong to both sides. Forcing single membership loses information.

**Solution**: Soft membership for papers within Δ of the cut.

```
BRIDGE_ZONE(p, τ, Δ):
    max_edge_above = max{ e.weight | e connects p to component above τ }
    max_edge_below = max{ e.weight | e connects p to component below τ }

    if |max_edge_above - τ| < Δ AND |max_edge_below - τ| < Δ:
        return BRIDGE  → assign to BOTH children
    else:
        return SINGLE  → assign to one child
```

**Result**: Tree becomes DAG at the edges. Deep papers belong to one group. Bridge papers belong to siblings.

### Sibling Edge Metadata

Bridge papers explain how siblings relate:

```
SIBLING_EDGE(A, B, τ):
    return {
        weight: inter(A, B),                           # average similarity
        threshold: τ,                                   # where split occurred
        bridges: [p | p.parents contains both A and B], # shared papers
        bridge_count: |bridges|
    }
```

### Algorithm Phases

#### Phase 1: Build Dendrogram

```
DENDROGRAM(papers, edges):
    sorted_edges ← SORT(edges, by: weight, order: DESC)

    parent[p] ← p  ∀p ∈ papers
    rank[p] ← 0    ∀p ∈ papers

    merges ← []

    for (p₁, p₂, w) in sorted_edges:
        r₁ ← FIND(p₁)
        r₂ ← FIND(p₂)

        if r₁ ≠ r₂:
            merge ← {
                left: r₁,
                right: r₂,
                weight: w,
                size: |r₁| + |r₂|
            }
            merges.append(merge)
            UNION(r₁, r₂)

    return merges
```

**Complexity**: O(E log E) for sort + O(E α(N)) for union-find ≈ **O(E log E)**

#### Phase 2: Adaptive Tree Building

```
BUILD_TREE(group, range, parent_threshold, depth):
    ─────────────────────────────────────
    │ STOPPING CONDITIONS               │
    ─────────────────────────────────────
    if |group| < MIN_SIZE:
        return LEAF(group)

    if σ²(group.edges) < TIGHT_THRESHOLD:
        return LEAF(group)

    if (range.max - range.min) < Δ_min:
        return LEAF(group)  # range too narrow

    ─────────────────────────────────────
    │ FIND VALID SPLITS                 │
    ─────────────────────────────────────
    splits ← FIND_VALID_SPLITS(group, range, parent_threshold)

    if splits = ∅:
        # FALLBACK: Centroid bisection
        splits = CENTROID_BISECTION(group)

    ─────────────────────────────────────
    │ SELECT & RECURSE                  │
    ─────────────────────────────────────
    best ← argmax(splits, by: quality)

    ─────────────────────────────────────
    │ DETECT BRIDGES                    │
    ─────────────────────────────────────
    bridges ← FIND_BRIDGES(group, best.threshold, Δ_min)

    children ← []
    for child_group in best.children:
        # Add bridges to child
        child_group ← child_group ∪ relevant_bridges

        child_range ← (
            min(child_group.internal_edges),
            max(child_group.internal_edges)
        )
        children.append(
            BUILD_TREE(child_group, child_range, best.threshold, depth + 1)
        )

    ─────────────────────────────────────
    │ CREATE SIBLING EDGES              │
    ─────────────────────────────────────
    for (A, B) in PAIRS(children):
        CREATE_SIBLING_EDGE(A, B, best.threshold, bridges)

    return NODE(group, children, depth)
```

#### Phase 3: Find Valid Splits

```
FIND_VALID_SPLITS(group, range, parent_threshold):
    edges ← INTERNAL_EDGES(group) ∩ range
    thresholds ← UNIQUE(edges.weights) sorted DESC

    valid ← []

    for τ in thresholds:
        # Gap check
        if parent_threshold ≠ NULL AND |τ - parent_threshold| < Δ_min:
            continue

        components ← CUT(group, τ)
        components ← { c ∈ components : |c| ≥ MIN_SIZE }

        if |components| < 2:
            continue

        sizes ← [|c| for c in components]

        # Balance check
        if min(sizes) / max(sizes) < min_ratio(|group|):
            continue

        # Cohesion check
        if ¬VALID_COHESION(components):
            continue

        # Quality
        balance ← min(sizes) / mean(sizes)
        quality ← |components| × balance

        valid.append((τ, components, quality))

    return valid
```

### Progressive Min_Size Depth Capping

`min_size` increases with depth, naturally capping tree depth:

```rust
fn min_size_at_depth(depth: usize) -> usize {
    match depth {
        0..=2 => 5,     // 5 at depths 0-2
        3..=4 => 10,    // 10 at depths 3-4
        5..=6 => 20,    // 20 at depths 5-6
        7 => 40,        // 40 at depth 7
        _ => 100,       // 100 at depth 8+
    }
}
```

**Effect**: Tree depth naturally caps at ~9 levels after uber-consolidation.

### Centroid Bisection Fallback

When no valid threshold splits exist:

```
CENTROID_BISECTION(group):
    1. Find centroid paper: p* = argmax_p Σ edge_weight(p, q) for all q in group
    2. Sort all papers by similarity to centroid (descending)
    3. Split at midpoint: top half = "close to centroid", bottom half = "far from centroid"
    4. Return two groups of approximately equal size
```

**Guarantee**: Even with disconnected or uniform-similarity graphs, groups will be bisected until they reach `min_size`.

### Mathematical Summary

**Core Formula**:
```
SPLIT(G) = argmax   { n × min(sizes)/mean(sizes) }
           τ ∈ edges(G)

           where children(τ) satisfy:
               |children| ≥ 2
               ∀c: |c| ≥ MIN_SIZE
               min(sizes)/max(sizes) ≥ f(|G|)
               ∀(A,B): intra(A)+intra(B) > 2×inter(A,B)
               |τ - τ_parent| ≥ Δ_min
```

**Recursion termination**:
```
LEAF(group) ⟺
    |group| < MIN_SIZE
    ∨ σ²(internal_edges) < TIGHT_THRESHOLD
    ∨ (range_max - range_min) < Δ_min
    ∨ VALID_SPLITS(group) = ∅
```

### Properties

| Property | Guarantee |
|----------|-----------|
| **Deterministic** | Same edges → same tree |
| **Complexity** | O(E log E) dendrogram + O(N²) cohesion checks |
| **Graceful** | No valid splits → centroid bisection fallback |
| **Adaptive** | Different branches, different thresholds |
| **Multi-way** | Not forced binary; finds natural seams |
| **Validated** | Every split passes balance + cohesion + gap |
| **Bridge-aware** | Near-threshold papers get multi-membership |

- **Implementation:** `src-tauri/src/dendrogram.rs`

---

## 8. Category Naming

### Bottom-up Traversal

Categories are named from leaves to root, so parent categories can reference child names for context.

### Live Name Deduplication

**Problem**: Sibling categories often get identical names (e.g., three "Machine Learning" categories).

**Solution**: Track used names globally and pass forbidden names to the AI prompt.

```rust
let mut forbidden_names: HashSet<String> =
    db.get_all_category_names().into_iter().collect();

for category in categories_to_name {
    let prompt = format!(
        "Name this category. Do NOT use these names: {:?}",
        forbidden_names
    );
    let name = ai_client.generate_name(category, &prompt).await?;

    // LIVE UPDATE: Add to forbidden set immediately
    forbidden_names.insert(name.clone());

    db.update_category_name(category.id, &name)?;
}
```

**Key insight**: The `forbidden_names` set is updated *during* the naming loop, not just before.

- **Implementation:** `src-tauri/src/bin/cli.rs` (setup command)

---

## 9. Setup Pipeline

The `mycelica-cli setup` command runs a complete pipeline for newly imported items.

### Adaptive Tree Pipeline (Default)

```
Step 0: Pattern Classification (FREE)
    - Classify items by content patterns (code, debug, paste, etc.)
    - Hidden items skip AI processing

Step 1: AI Processing + Embeddings
    [1a] AI process items (title, summary, tags via Claude/Ollama)
    [1b] Generate embeddings (local all-MiniLM-L6-v2, no API)
         - Papers: title + abstract
         - Code: signature + body

Step 2: Clustering & Hierarchy
    [2a] Generate semantic edges at 0.30 threshold
         - Brute-force O(n²) pairwise cosine similarity
         - Lower threshold = more edges = finer dendrogram resolution
    [2b] Build hierarchy with adaptive tree algorithm
         - Edges → Dendrogram → Adaptive cuts → Tree
         - NO pre-clustering step (edges define structure directly)
         - Includes category naming (bottom-up, LLM or keyword extraction)
         - Live dedup via forbidden name set

Step 3: Code Analysis (if code nodes exist)
    - Create Calls edges between functions

Step 4: Flatten Hierarchy
    - Remove sparse intermediate levels

Step 5: Category Embeddings
    - Compute centroid embeddings for all categories

Step 6: Build HNSW Index
    - Build approximate nearest-neighbor index for GUI similarity panel
    - Includes all items + category centroids

Step 7: Index Edge Parents
    - Precompute parent_id for edges (faster view loading)
```

**Total time**: ~2-5 minutes for 1000 items (depends on AI backend speed).

- **Implementation:** `src-tauri/src/bin/cli.rs`

---

## 10. Constants Summary

### Adaptive Tree Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| `EDGE_FLOOR` | 0.3 | Lowest similarity to store as edge |
| `MIN_SIZE` | 5 | Minimum papers for a group |
| `TIGHT_THRESHOLD` | 0.001 | Variance below which group is cohesive |
| `COHESION_THRESHOLD` | 1.2 | Min intra/inter ratio for valid split |
| `DELTA_MIN` | 0.03 | Minimum gap between cuts |

### Semantic Edge Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| `semantic_min_similarity` | 0.50 | Semantic edge threshold |
| `semantic_max_edges` | 5 | Max edges per node |

---

## 11. File References

*Line numbers are approximate and may drift as code evolves.*

| Algorithm | File |
|-----------|------|
| Cosine similarity | `src-tauri/src/similarity.rs` |
| Centroid computation | `src-tauri/src/similarity.rs` |
| Union-Find clustering | `src-tauri/src/clustering.rs` |
| TF-IDF keywords | `src-tauri/src/clustering.rs` |
| Content type tiers | `src-tauri/src/clustering.rs` |
| Adaptive tree (dendrogram) | `src-tauri/src/dendrogram.rs` |
| Bridge detection | `src-tauri/src/dendrogram.rs` |
| Centroid bisection | `src-tauri/src/dendrogram.rs` |
| Category naming | `src-tauri/src/bin/cli.rs` |
| Setup pipeline | `src-tauri/src/bin/cli.rs` |
