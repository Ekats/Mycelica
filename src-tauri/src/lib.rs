pub mod db;
mod commands;
pub mod utils;
mod http_server;
pub mod clustering;
pub mod ai_client;
pub mod settings;
pub mod hierarchy;
pub mod import;
pub mod similarity;
mod local_embeddings;
pub mod classification;
mod tags;
pub mod openaire;
mod format_abstract;
pub mod code;

use commands::{
    AppState,
    get_nodes, get_node, create_node, add_note, update_node, update_node_content, delete_node,
    get_edges, get_edges_for_node, get_edges_for_fos, get_edges_for_view, create_edge, delete_edge,
    search_nodes,
    // Clustering commands
    run_clustering, recluster_all, get_clustering_status,
    // AI processing commands
    process_nodes, get_ai_status, cancel_processing, cancel_rebuild, cancel_all,
    get_api_key_status, save_api_key, clear_api_key,
    get_learned_emojis, save_learned_emoji,
    // Pipeline state commands
    get_pipeline_state, set_pipeline_state, get_db_metadata,
    // Hierarchy commands
    get_nodes_at_depth, get_children, get_universe, get_items, get_max_depth,
    build_hierarchy, build_full_hierarchy, cluster_hierarchy_level, unsplit_node, get_children_flat,
    propagate_latest_dates, smart_add_to_hierarchy,
    // Multi-path association commands
    get_item_associations, get_related_items, get_category_items,
    // Mini-clustering commands
    get_graph_children, get_supporting_items, get_associated_items, get_supporting_counts,
    classify_and_associate, classify_and_associate_children,
    // Rebuild Lite commands
    preclassify_items, reclassify_pattern, reclassify_ai, rebuild_lite, rebuild_hierarchy_only,
    // Conversation context commands
    get_conversation_context,
    // Import commands
    import_claude_conversations, import_chatgpt_conversations, import_markdown_files, import_google_keep, import_openaire, count_openaire_papers, cancel_openaire, get_imported_paper_count,
    // Code import commands
    import_code, analyze_code_edges,
    // Paper retrieval commands
    get_paper_metadata, get_paper_pdf, get_paper_document, has_paper_pdf, open_paper_external, reformat_paper_abstracts, sync_paper_pdf_status, sync_paper_dates, download_paper_on_demand,
    // Quick access commands (Sidebar)
    set_node_pinned, touch_node, get_pinned_nodes, get_recent_nodes, clear_recent,
    // Semantic similarity commands
    get_similar_nodes, get_embedding_status,
    // OpenAI API key commands
    get_openai_api_key_status, save_openai_api_key, clear_openai_api_key,
    // OpenAIRE API key commands
    get_openaire_api_key_status, save_openaire_api_key, clear_openaire_api_key,
    // Leaf view commands
    get_leaf_content,
    // Settings panel commands
    delete_all_data, reset_ai_processing, reset_clustering, clear_embeddings, clear_hierarchy, clear_tags, delete_empty_nodes, flatten_hierarchy, consolidate_root, get_db_stats,
    get_db_path, switch_database, tidy_database, export_trimmed_database,
    // Processing stats commands
    get_processing_stats, add_ai_processing_time, add_rebuild_time,
    // Privacy filtering commands
    analyze_node_privacy, analyze_all_privacy, analyze_categories_privacy, cancel_privacy_scan, reset_privacy_flags, get_privacy_stats, export_shareable_db, set_node_privacy, score_privacy_all_items, get_export_preview,
    // Recent Notes protection commands
    get_protect_recent_notes, set_protect_recent_notes,
    // Local embeddings commands
    get_use_local_embeddings, set_use_local_embeddings, regenerate_all_embeddings,
    // Clustering thresholds commands
    get_clustering_thresholds, set_clustering_thresholds,
    // Privacy threshold commands
    get_privacy_threshold, set_privacy_threshold,
    // Tips commands
    get_show_tips, set_show_tips,
};
use db::Database;
use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            // Get app data directory for settings
            let app_data_dir = app.path().app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            std::fs::create_dir_all(&app_data_dir).ok();

            // Initialize settings
            settings::init(app_data_dir.clone());

            // Check for custom database path in settings first
            let db_path = if let Some(custom_path) = settings::get_custom_db_path() {
                let path = std::path::PathBuf::from(&custom_path);
                if path.exists() {
                    println!("Using custom database from settings: {:?}", path);
                    path
                } else {
                    // Custom path no longer exists, clear it and fall back to default
                    eprintln!("Custom database not found: {:?}, reverting to default", path);
                    let _ = settings::set_custom_db_path(None);
                    app_data_dir.join("mycelica.db")
                }
            } else {
                // In development, use local data/mycelica.db if it exists
                let local_db = std::path::PathBuf::from("data/mycelica.db");
                if local_db.exists() {
                    println!("Using local database: {:?}", local_db);
                    local_db
                } else {
                    let default_path = app_data_dir.join("mycelica.db");

                    // First run: copy bundled sample database if no database exists
                    if !default_path.exists() {
                        println!("First run detected: no database at {:?}", default_path);
                        match app.path().resource_dir() {
                            Ok(resource_dir) => {
                                println!("Resource dir: {:?}", resource_dir);
                                // Try both possible paths (with and without resources/ prefix)
                                let bundled_db = resource_dir.join("resources/mycelica-openAIRE-med-trimmed.db");
                                let bundled_db_alt = resource_dir.join("mycelica-openAIRE-med-trimmed.db");

                                let source = if bundled_db.exists() {
                                    Some(bundled_db)
                                } else if bundled_db_alt.exists() {
                                    Some(bundled_db_alt)
                                } else {
                                    println!("Bundled database not found at {:?} or {:?}", bundled_db, bundled_db_alt);
                                    None
                                };

                                if let Some(src) = source {
                                    println!("First run: copying bundled sample database from {:?}", src);
                                    if let Err(e) = std::fs::copy(&src, &default_path) {
                                        eprintln!("Failed to copy bundled database: {}", e);
                                    } else {
                                        println!("Sample database copied to {:?}", default_path);
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("Failed to get resource dir: {}", e);
                            }
                        }
                    }

                    println!("Using app data database: {:?}", default_path);
                    default_path
                }
            };

            let db = Database::new(&db_path).expect("Failed to initialize database");

            // Auto-build hierarchy if no Universe exists
            if db.get_universe().ok().flatten().is_none() {
                println!("No Universe found, building hierarchy...");
                if let Err(e) = hierarchy::build_hierarchy(&db) {
                    eprintln!("Failed to build hierarchy on startup: {}", e);
                }
            }

            // Wrap in Arc for sharing between Tauri and HTTP server
            let db = Arc::new(db);

            // Start HTTP server for browser extension (localhost:9876)
            http_server::start(db.clone());

            // Use configurable cache TTL from settings
            let cache_ttl = settings::similarity_cache_ttl_secs();
            app.manage(AppState {
                db: std::sync::RwLock::new(db),
                similarity_cache: std::sync::RwLock::new(commands::SimilarityCache::new(cache_ttl)),
                openaire_cancel: std::sync::atomic::AtomicBool::new(false),
            });

            // Set window title and handle HiDPI scaling
            // Note: On Wayland, title updates taskbar but not header bar (upstream GTK issue)
            let db_path_clone = db_path.clone();
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                let path_str = db_path_clone.to_string_lossy();
                let home = std::env::var("HOME").unwrap_or_default();
                let display_path = if !home.is_empty() && path_str.starts_with(&home) {
                    path_str.replacen(&home, "~", 1)
                } else {
                    path_str.to_string()
                };
                let title = format!("Mycelica — {}", display_path);

                // Multiple attempts for window readiness
                for delay in [100, 500, 1000] {
                    std::thread::sleep(std::time::Duration::from_millis(delay));
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.set_title(&title);

                        // HiDPI fix: DISABLED - was causing double-scaling
                        // let detected_scale = window.scale_factor().unwrap_or(1.0);
                        // let forced_scale = std::env::var("MYCELICA_SCALE")
                        //     .or_else(|_| std::env::var("GDK_SCALE"))
                        //     .ok()
                        //     .and_then(|s| s.parse::<f64>().ok());
                        //
                        // let scale = forced_scale.unwrap_or(detected_scale);
                        // println!("[HiDPI] Detected: {}, Forced: {:?}, Using: {}", detected_scale, forced_scale, scale);
                        //
                        // if scale > 1.0 {
                        //     if let Err(e) = window.set_zoom(scale) {
                        //         eprintln!("[HiDPI] Failed to set zoom: {}", e);
                        //     } else {
                        //         println!("[HiDPI] Set webview zoom to {}", scale);
                        //     }
                        // }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_nodes,
            get_node,
            create_node,
            add_note,
            update_node,
            update_node_content,
            delete_node,
            get_edges,
            get_edges_for_node,
            get_edges_for_fos,
            get_edges_for_view,
            create_edge,
            delete_edge,
            search_nodes,
            // Clustering
            run_clustering,
            recluster_all,
            get_clustering_status,
            // AI processing
            process_nodes,
            get_ai_status,
            cancel_processing,
            cancel_rebuild,
            cancel_all,
            get_api_key_status,
            save_api_key,
            clear_api_key,
            get_learned_emojis,
            save_learned_emoji,
            // Pipeline state
            get_pipeline_state,
            set_pipeline_state,
            get_db_metadata,
            // Hierarchy
            get_nodes_at_depth,
            get_children,
            get_children_flat,
            get_universe,
            get_items,
            get_max_depth,
            build_hierarchy,
            build_full_hierarchy,
            cluster_hierarchy_level,
            unsplit_node,
            propagate_latest_dates,
            smart_add_to_hierarchy,
            // Multi-path associations
            get_item_associations,
            get_related_items,
            get_category_items,
            // Mini-clustering
            get_graph_children,
            get_supporting_items,
            get_associated_items,
            get_supporting_counts,
            classify_and_associate,
            classify_and_associate_children,
            // Rebuild Lite
            preclassify_items,
            reclassify_pattern,
            reclassify_ai,
            rebuild_lite,
            rebuild_hierarchy_only,
            // Conversation context
            get_conversation_context,
            // Import
            import_claude_conversations,
            import_chatgpt_conversations,
            import_markdown_files,
            import_google_keep,
            import_openaire,
            count_openaire_papers,
            cancel_openaire,
            get_imported_paper_count,
            // Code import
            import_code,
            analyze_code_edges,
            // Paper retrieval
            get_paper_metadata,
            get_paper_pdf,
            get_paper_document,
            has_paper_pdf,
            open_paper_external,
            reformat_paper_abstracts,
            sync_paper_pdf_status,
            sync_paper_dates,
            download_paper_on_demand,
            // Quick access (Sidebar)
            set_node_pinned,
            touch_node,
            get_pinned_nodes,
            get_recent_nodes,
            clear_recent,
            // Semantic similarity
            get_similar_nodes,
            get_embedding_status,
            // OpenAI API key
            get_openai_api_key_status,
            save_openai_api_key,
            clear_openai_api_key,
            // OpenAIRE API key
            get_openaire_api_key_status,
            save_openaire_api_key,
            clear_openaire_api_key,
            // Leaf view
            get_leaf_content,
            // Settings panel
            delete_all_data,
            reset_ai_processing,
            reset_clustering,
            clear_embeddings,
            clear_hierarchy,
            clear_tags,
            delete_empty_nodes,
            flatten_hierarchy,
            consolidate_root,
            tidy_database,
            get_db_stats,
            get_db_path,
            switch_database,
            export_trimmed_database,
            // Processing stats
            get_processing_stats,
            add_ai_processing_time,
            add_rebuild_time,
            // Privacy filtering
            analyze_node_privacy,
            analyze_all_privacy,
            analyze_categories_privacy,
            cancel_privacy_scan,
            reset_privacy_flags,
            get_privacy_stats,
            export_shareable_db,
            set_node_privacy,
            score_privacy_all_items,
            get_export_preview,
            // Recent Notes protection
            get_protect_recent_notes,
            set_protect_recent_notes,
            // Local embeddings
            get_use_local_embeddings,
            set_use_local_embeddings,
            regenerate_all_embeddings,
            // Clustering thresholds
            get_clustering_thresholds,
            set_clustering_thresholds,
            get_privacy_threshold,
            set_privacy_threshold,
            get_show_tips,
            set_show_tips,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
