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
    get_nodes, get_node, create_node, update_node, delete_node,
    get_edges, get_edges_for_node, create_edge, delete_edge,
    search_nodes,
    // Clustering commands
    run_clustering, recluster_all, get_clustering_status,
    // AI processing commands
    process_nodes, get_ai_status,
    get_api_key_status, save_api_key, clear_api_key,
    get_learned_emojis, save_learned_emoji,
    // Hierarchy commands
    get_nodes_at_depth, get_children, get_universe, get_items, get_max_depth,
    build_hierarchy, build_full_hierarchy, cluster_hierarchy_level, get_children_flat,
    // Multi-path association commands
    get_item_associations, get_related_items, get_category_items,
    // Conversation context commands
    get_conversation_context,
    // Import commands
    import_claude_conversations,
    // Quick access commands (Sidebar)
    set_node_pinned, touch_node, get_pinned_nodes, get_recent_nodes, clear_recent,
    // Semantic similarity commands
    get_similar_nodes, get_embedding_status,
    // OpenAI API key commands
    get_openai_api_key_status, save_openai_api_key, clear_openai_api_key,
};
use db::Database;
use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Get app data directory for settings
            let app_data_dir = app.path().app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            std::fs::create_dir_all(&app_data_dir).ok();

            // Initialize settings
            settings::init(app_data_dir.clone());

            // In development, use local data/mycelica.db if it exists
            let local_db = std::path::PathBuf::from("data/mycelica.db");
            let db_path = if local_db.exists() {
                println!("Using local database: {:?}", local_db);
                local_db
            } else {
                let path = app_data_dir.join("mycelica.db");
                println!("Using app data database: {:?}", path);
                path
            };

            let db = Database::new(&db_path).expect("Failed to initialize database");

            // Auto-build hierarchy if no Universe exists
            if db.get_universe().ok().flatten().is_none() {
                println!("No Universe found, building hierarchy...");
                if let Err(e) = hierarchy::build_hierarchy(&db) {
                    eprintln!("Failed to build hierarchy on startup: {}", e);
                }
            }

            app.manage(AppState { db: Arc::new(db) });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_nodes,
            get_node,
            create_node,
            update_node,
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
            // Multi-path associations
            get_item_associations,
            get_related_items,
            get_category_items,
            // Conversation context
            get_conversation_context,
            // Import
            import_claude_conversations,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
