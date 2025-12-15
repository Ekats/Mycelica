mod db;
mod commands;
mod clustering;
mod ai_client;
mod settings;
mod hierarchy;
mod import;
mod similarity;

use commands::{
    AppState,
    get_nodes, get_node, create_node, add_note, update_node, update_node_content, delete_node,
    get_edges, get_edges_for_node, create_edge, delete_edge,
    search_nodes,
    // Clustering commands
    run_clustering, recluster_all, get_clustering_status,
    // AI processing commands
    process_nodes, get_ai_status, cancel_processing, cancel_rebuild,
    get_api_key_status, save_api_key, clear_api_key,
    get_learned_emojis, save_learned_emoji,
    // Hierarchy commands
    get_nodes_at_depth, get_children, get_universe, get_items, get_max_depth,
    build_hierarchy, build_full_hierarchy, cluster_hierarchy_level, unsplit_node, get_children_flat,
    propagate_latest_dates, quick_add_to_hierarchy,
    // Multi-path association commands
    get_item_associations, get_related_items, get_category_items,
    // Conversation context commands
    get_conversation_context,
    // Import commands
    import_claude_conversations, import_markdown_files,
    // Quick access commands (Sidebar)
    set_node_pinned, touch_node, get_pinned_nodes, get_recent_nodes, clear_recent,
    // Semantic similarity commands
    get_similar_nodes, get_embedding_status,
    // OpenAI API key commands
    get_openai_api_key_status, save_openai_api_key, clear_openai_api_key,
    // Leaf view commands
    get_leaf_content,
    // Settings panel commands
    delete_all_data, reset_ai_processing, reset_clustering, clear_embeddings, clear_hierarchy, delete_empty_nodes, flatten_hierarchy, consolidate_root, get_db_stats,
    get_db_path, switch_database, tidy_database,
    // Processing stats commands
    get_processing_stats, add_ai_processing_time, add_rebuild_time,
    // Privacy filtering commands
    analyze_node_privacy, analyze_all_privacy, analyze_categories_privacy, cancel_privacy_scan, reset_privacy_flags, get_privacy_stats, export_shareable_db, set_node_privacy,
    // Recent Notes protection commands
    get_protect_recent_notes, set_protect_recent_notes,
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
                    let path = app_data_dir.join("mycelica.db");
                    println!("Using app data database: {:?}", path);
                    path
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

            app.manage(AppState { db: std::sync::RwLock::new(Arc::new(db)) });

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
            get_api_key_status,
            save_api_key,
            clear_api_key,
            get_learned_emojis,
            save_learned_emoji,
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
            quick_add_to_hierarchy,
            // Multi-path associations
            get_item_associations,
            get_related_items,
            get_category_items,
            // Conversation context
            get_conversation_context,
            // Import
            import_claude_conversations,
            import_markdown_files,
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
            // Leaf view
            get_leaf_content,
            // Settings panel
            delete_all_data,
            reset_ai_processing,
            reset_clustering,
            clear_embeddings,
            clear_hierarchy,
            delete_empty_nodes,
            flatten_hierarchy,
            consolidate_root,
            tidy_database,
            get_db_stats,
            get_db_path,
            switch_database,
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
            // Recent Notes protection
            get_protect_recent_notes,
            set_protect_recent_notes,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
