//! Batch OpenAIRE import script
//!
//! Runs multiple OpenAIRE imports sequentially.
//! Usage: cargo run --bin batch_import --release
//!
//! Queries are hardcoded below. Edit as needed.

use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::sync::Arc;

// Import from the library crate
use mycelica_lib::db::Database;
use mycelica_lib::import;
use mycelica_lib::settings;

/// Query configuration
struct ImportQuery {
    search: &'static str,
    max_papers: u32,
}

/// Estonia queries to import
const QUERIES: &[ImportQuery] = &[
    ImportQuery { search: "medical education", max_papers: 420 },
    ImportQuery { search: "psychology", max_papers: 400 },
    ImportQuery { search: "public health", max_papers: 350 },
    ImportQuery { search: "neuroscience", max_papers: 300 },
    ImportQuery { search: "physiology", max_papers: 250 },
    ImportQuery { search: "psychiatry", max_papers: 200 },
    ImportQuery { search: "infectious disease", max_papers: 150 },
];

const COUNTRY: &str = "EE"; // Estonia
const DOWNLOAD_PDFS: bool = true;
const MAX_PDF_SIZE_MB: u32 = 20;

#[tokio::main]
async fn main() {
    println!("==============================================");
    println!("  OpenAIRE Batch Import - Estonia Papers");
    println!("==============================================");
    println!();

    // Find the database
    let db_path = find_database();
    println!("[Batch] Using database: {:?}", db_path);

    let db = match Database::new(&db_path) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            eprintln!("[Batch] ERROR: Failed to open database: {}", e);
            std::process::exit(1);
        }
    };

    // Get API key from settings (if configured)
    let app_data_dir = dirs::data_dir()
        .map(|p| p.join("com.mycelica.dev"))
        .unwrap_or_else(|| PathBuf::from("."));
    settings::init(app_data_dir);
    let api_key = settings::get_openaire_api_key();
    println!("[Batch] OpenAIRE auth: {}", if api_key.is_some() { "yes" } else { "no (public API)" });
    println!();

    let total_queries = QUERIES.len();
    let total_papers: u32 = QUERIES.iter().map(|q| q.max_papers).sum();
    println!("[Batch] {} queries, ~{} papers total", total_queries, total_papers);
    println!();

    let start_time = Instant::now();
    let mut total_imported = 0usize;
    let mut total_pdfs = 0usize;
    let mut total_duplicates = 0usize;
    let mut failed_queries: Vec<&str> = Vec::new();

    for (i, query) in QUERIES.iter().enumerate() {
        println!("----------------------------------------------");
        println!("[{}/{}] Query: \"{}\" (max {} papers)",
            i + 1, total_queries, query.search, query.max_papers);
        println!("----------------------------------------------");

        let query_start = Instant::now();

        // Progress callback
        let on_progress = |current: usize, total: usize| {
            if current % 10 == 0 || current == total {
                println!("[{}/{}] Progress: {}/{}", i + 1, total_queries, current, total);
            }
        };

        match import::import_openaire_papers(
            &db,
            query.search.to_string(),
            Some(COUNTRY.to_string()),
            None, // fos
            query.max_papers,
            DOWNLOAD_PDFS,
            MAX_PDF_SIZE_MB,
            api_key.clone(),
            on_progress,
        ).await {
            Ok(result) => {
                let elapsed = query_start.elapsed();
                total_imported += result.papers_imported;
                total_pdfs += result.pdfs_downloaded;
                total_duplicates += result.duplicates_skipped;

                println!("[{}/{}] DONE: {} imported, {} PDFs, {} duplicates, {} errors ({:.1}s)",
                    i + 1, total_queries,
                    result.papers_imported, result.pdfs_downloaded,
                    result.duplicates_skipped, result.errors.len(),
                    elapsed.as_secs_f64());

                if !result.errors.is_empty() {
                    println!("[{}/{}] Errors:", i + 1, total_queries);
                    for (j, err) in result.errors.iter().take(5).enumerate() {
                        println!("  {}. {}", j + 1, err);
                    }
                    if result.errors.len() > 5 {
                        println!("  ... and {} more", result.errors.len() - 5);
                    }
                }
            }
            Err(e) => {
                eprintln!("[{}/{}] FAILED: {}", i + 1, total_queries, e);
                failed_queries.push(query.search);
            }
        }

        println!();

        // Small delay between queries
        if i < total_queries - 1 {
            println!("[Batch] Waiting 5 seconds before next query...");
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    let total_elapsed = start_time.elapsed();

    println!("==============================================");
    println!("  BATCH IMPORT COMPLETE");
    println!("==============================================");
    println!();
    println!("Total time: {:.1} minutes", total_elapsed.as_secs_f64() / 60.0);
    println!("Papers imported: {}", total_imported);
    println!("PDFs downloaded: {}", total_pdfs);
    println!("Duplicates skipped: {}", total_duplicates);

    if !failed_queries.is_empty() {
        println!();
        println!("Failed queries ({}):", failed_queries.len());
        for q in &failed_queries {
            println!("  - {}", q);
        }
    }

    println!();
}

/// Find the database path
fn find_database() -> PathBuf {
    // Check command line argument first
    if let Some(path) = std::env::args().nth(1) {
        let db_path = PathBuf::from(&path);
        if db_path.exists() {
            return db_path;
        }
        eprintln!("[Batch] WARNING: Specified path doesn't exist: {}", path);
    }

    // Check specific known paths
    let known_paths = [
        dirs::data_dir().map(|p| p.join("com.mycelica.app").join("mycelica-openAIRE-med.db")),
        dirs::data_dir().map(|p| p.join("com.mycelica.app").join("mycelica.db")),
        dirs::data_dir().map(|p| p.join("com.mycelica.dev").join("mycelica.db")),
        Some(PathBuf::from("data/mycelica.db")),
    ];

    for path_opt in known_paths.iter() {
        if let Some(path) = path_opt {
            if path.exists() {
                return path.clone();
            }
        }
    }

    // Fall back to local
    PathBuf::from("data/mycelica.db")
}
