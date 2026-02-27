use crate::db::Database;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::RwLock;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use std::path::Path;
use serde::{Serialize, Deserialize};
use instant_distance::{Builder, HnswMap, Search};
use instant_distance::Point as HnswPoint;

// Global cancellation flags
pub static CANCEL_PROCESSING: AtomicBool = AtomicBool::new(false);
pub static CANCEL_REBUILD: AtomicBool = AtomicBool::new(false);

/// Cache for similarity search results with TTL
pub struct SimilarityCache {
    results: HashMap<String, (Vec<(String, f32)>, Instant)>,
    ttl: Duration,
}

impl SimilarityCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            results: HashMap::new(),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    pub fn get(&self, node_id: &str) -> Option<Vec<(String, f32)>> {
        self.results.get(node_id)
            .filter(|(_, time)| time.elapsed() < self.ttl)
            .map(|(results, _)| results.clone())
    }

    pub fn insert(&mut self, node_id: String, results: Vec<(String, f32)>) {
        self.results.insert(node_id, (results, Instant::now()));
    }

    pub fn invalidate(&mut self) {
        self.results.clear();
    }
}

/// In-memory cache for all embeddings - loaded once, avoids repeated SQLite reads
/// ~80MB for 55k nodes × 384 floats × 4 bytes
pub struct EmbeddingsCache {
    embeddings: Option<HashMap<String, Vec<f32>>>,
    loaded_at: Option<Instant>,
}

impl EmbeddingsCache {
    pub fn new() -> Self {
        Self {
            embeddings: None,
            loaded_at: None,
        }
    }

    /// Check if cache is loaded
    pub fn is_loaded(&self) -> bool {
        self.embeddings.is_some()
    }

    /// Get all embeddings as Vec for similarity search
    pub fn get_all(&self) -> Option<Vec<(String, Vec<f32>)>> {
        self.embeddings.as_ref().map(|map| {
            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        })
    }

    /// Get a single embedding
    pub fn get(&self, node_id: &str) -> Option<&Vec<f32>> {
        self.embeddings.as_ref()?.get(node_id)
    }

    /// Load all embeddings from database
    pub fn load(&mut self, db: &Database) -> Result<usize, String> {
        let start = Instant::now();
        let all = db.get_nodes_with_embeddings().map_err(|e| e.to_string())?;
        let count = all.len();
        let map: HashMap<String, Vec<f32>> = all.into_iter().collect();
        self.embeddings = Some(map);
        self.loaded_at = Some(Instant::now());
        println!("[PERF] EmbeddingsCache: loaded {} embeddings in {}ms", count, start.elapsed().as_millis());
        Ok(count)
    }

    /// Invalidate cache (call when embeddings are added/updated/deleted)
    pub fn invalidate(&mut self) {
        self.embeddings = None;
        self.loaded_at = None;
        println!("[PERF] EmbeddingsCache: invalidated");
    }

    /// Update a single embedding in cache (avoids full reload)
    pub fn update(&mut self, node_id: &str, embedding: Vec<f32>) {
        if let Some(ref mut map) = self.embeddings {
            map.insert(node_id.to_string(), embedding);
        }
    }

    /// Remove a single embedding from cache
    pub fn remove(&mut self, node_id: &str) {
        if let Some(ref mut map) = self.embeddings {
            map.remove(node_id);
        }
    }
}

/// Embedding point wrapper for HNSW
/// Implements distance as Euclidean (smaller = closer)
#[derive(Clone, Serialize, Deserialize)]
pub struct EmbeddingPoint(pub Vec<f32>);

impl HnswPoint for EmbeddingPoint {
    fn distance(&self, other: &Self) -> f32 {
        // Euclidean distance (instant-distance expects smaller = closer)
        // For normalized embeddings, this is equivalent to sqrt(2*(1 - cosine_sim))
        self.0.iter()
            .zip(other.0.iter())
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            .sqrt()
    }
}

/// HNSW index for fast approximate nearest neighbor search
/// Provides O(log n) queries instead of O(n) brute-force
/// Can be serialized to disk for fast loading on app startup
pub struct HnswIndex {
    index: Option<HnswMap<EmbeddingPoint, String>>,  // Point -> node_id
    built_at: Option<Instant>,
    node_count: usize,
    building: std::sync::atomic::AtomicBool,  // true if background build in progress
}

impl HnswIndex {
    pub fn new() -> Self {
        Self {
            index: None,
            built_at: None,
            node_count: 0,
            building: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn is_built(&self) -> bool {
        self.index.is_some()
    }

    pub fn is_building(&self) -> bool {
        self.building.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_building(&self, value: bool) {
        self.building.store(value, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn count(&self) -> usize {
        self.node_count
    }

    /// Build index from embeddings
    /// Uses tuned HNSW parameters for 50k+ vectors:
    /// - ef_construction=100: build speed vs quality tradeoff
    /// - ef_search=50: fast queries with good recall
    pub fn build(&mut self, embeddings: &[(String, Vec<f32>)]) {
        self.set_building(true);
        let start = Instant::now();
        println!("[PERF] HnswIndex: starting build with {} points...", embeddings.len());

        let points: Vec<EmbeddingPoint> = embeddings.iter()
            .map(|(_, emb)| EmbeddingPoint(emb.clone()))
            .collect();
        let values: Vec<String> = embeddings.iter()
            .map(|(id, _)| id.clone())
            .collect();

        // Use tuned parameters for large datasets
        // ef_construction=100 (default is higher, causing slow builds)
        // ef_search=50 (fast queries, ~95% recall)
        self.index = Some(
            Builder::default()
                .ef_construction(100)
                .ef_search(50)
                .build(points, values)
        );
        self.built_at = Some(Instant::now());
        self.node_count = embeddings.len();
        self.set_building(false);
        println!("[PERF] HnswIndex: built with {} points in {}ms",
            embeddings.len(), start.elapsed().as_millis());
    }

    /// Save index to disk for fast loading on next startup
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let Some(ref index) = self.index else {
            return Err("No index to save".to_string());
        };

        let start = Instant::now();
        let bytes = bincode::serialize(index)
            .map_err(|e| format!("Failed to serialize HNSW index: {}", e))?;

        std::fs::write(path, &bytes)
            .map_err(|e| format!("Failed to write HNSW index to {:?}: {}", path, e))?;

        println!("[PERF] HnswIndex: saved {} points ({} bytes) to {:?} in {}ms",
            self.node_count, bytes.len(), path, start.elapsed().as_millis());
        Ok(())
    }

    /// Load pre-built index from disk
    pub fn load(&mut self, path: &Path) -> Result<usize, String> {
        let start = Instant::now();

        let bytes = std::fs::read(path)
            .map_err(|e| format!("Failed to read HNSW index from {:?}: {}", path, e))?;

        let index: HnswMap<EmbeddingPoint, String> = bincode::deserialize(&bytes)
            .map_err(|e| format!("Failed to deserialize HNSW index: {}", e))?;

        // HnswMap doesn't expose length, estimate from file size
        // ~1.5KB per point for 384-dim embeddings + graph structure
        let estimated_count = bytes.len() / 2000;

        self.index = Some(index);
        self.built_at = Some(Instant::now());
        self.node_count = estimated_count;

        println!("[PERF] HnswIndex: loaded from {:?} ({} bytes) in {}ms",
            path, bytes.len(), start.elapsed().as_millis());
        Ok(estimated_count)
    }

    /// Search for k nearest neighbors
    /// Returns (node_id, similarity) pairs - converts Euclidean distance to cosine similarity
    pub fn search(&self, query: &[f32], k: usize, exclude_id: &str) -> Vec<(String, f32)> {
        let Some(ref index) = self.index else { return vec![]; };

        let query_point = EmbeddingPoint(query.to_vec());
        let mut search = Search::default();

        // Get k+1 results to account for potential self-match
        let results: Vec<_> = index.search(&query_point, &mut search)
            .take(k + 1)
            .filter(|item| item.value != exclude_id)
            .take(k)
            .map(|item| {
                // Convert Euclidean distance to cosine similarity approximation
                // For normalized vectors: cos_sim ≈ 1 - (dist^2 / 2)
                let dist = item.distance;
                let sim = 1.0 - (dist * dist / 2.0);
                (item.value.clone(), sim.clamp(0.0, 1.0))
            })
            .collect();

        results
    }

    pub fn invalidate(&mut self) {
        self.index = None;
        self.built_at = None;
        self.node_count = 0;
        println!("[PERF] HnswIndex: invalidated");
    }
}

/// Get the HNSW index file path for a given database path
/// e.g., /path/to/mycelica.db -> /path/to/mycelica-hnsw.bin
pub fn hnsw_index_path(db_path: &Path) -> std::path::PathBuf {
    let stem = db_path.file_stem().unwrap_or_default().to_string_lossy();
    let parent = db_path.parent().unwrap_or(Path::new("."));
    parent.join(format!("{}-hnsw.bin", stem))
}

/// Delete the HNSW index file if it exists
pub fn delete_hnsw_index(db_path: &Path) {
    let index_path = hnsw_index_path(db_path);
    if index_path.exists() {
        if let Err(e) = std::fs::remove_file(&index_path) {
            eprintln!("[HNSW] Failed to delete index file {:?}: {}", index_path, e);
        } else {
            println!("[HNSW] Deleted stale index file {:?}", index_path);
        }
    }
}

pub struct AppState {
    pub db: RwLock<Arc<Database>>,
    pub db_path: std::path::PathBuf,
    pub similarity_cache: RwLock<SimilarityCache>,
    pub embeddings_cache: RwLock<EmbeddingsCache>,
    pub hnsw_index: RwLock<HnswIndex>,
    pub openaire_cancel: std::sync::atomic::AtomicBool,
}
