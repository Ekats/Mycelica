//! Settings-related Tauri commands

use crate::settings;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyStatus {
    pub has_key: bool,
    pub masked_key: Option<String>,
    pub source: String, // "env", "settings", or "none"
}

#[tauri::command]
pub fn get_api_key_status() -> ApiKeyStatus {
    let env_key = std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.is_empty());

    if env_key.is_some() {
        ApiKeyStatus {
            has_key: true,
            masked_key: settings::get_masked_api_key(),
            source: "env".to_string(),
        }
    } else if settings::has_api_key() {
        ApiKeyStatus {
            has_key: true,
            masked_key: settings::get_masked_api_key(),
            source: "settings".to_string(),
        }
    } else {
        ApiKeyStatus {
            has_key: false,
            masked_key: None,
            source: "none".to_string(),
        }
    }
}

#[tauri::command]
pub fn save_api_key(key: String) -> Result<(), String> {
    settings::set_api_key(key)
}

#[tauri::command]
pub fn clear_api_key() -> Result<(), String> {
    settings::set_api_key(String::new())
}

// ==================== Processing Stats ====================

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingStats {
    pub total_ai_processing_secs: f64,
    pub total_rebuild_secs: f64,
    pub last_ai_processing_secs: f64,
    pub last_rebuild_secs: f64,
    pub ai_processing_runs: u32,
    pub rebuild_runs: u32,
}

#[tauri::command]
pub fn get_processing_stats() -> ProcessingStats {
    let stats = settings::get_processing_stats();
    ProcessingStats {
        total_ai_processing_secs: stats.total_ai_processing_secs,
        total_rebuild_secs: stats.total_rebuild_secs,
        last_ai_processing_secs: stats.last_ai_processing_secs,
        last_rebuild_secs: stats.last_rebuild_secs,
        ai_processing_runs: stats.ai_processing_runs,
        rebuild_runs: stats.rebuild_runs,
    }
}

#[tauri::command]
pub fn add_ai_processing_time(elapsed_secs: f64) -> Result<(), String> {
    settings::add_ai_processing_time(elapsed_secs)
}

#[tauri::command]
pub fn add_rebuild_time(elapsed_secs: f64) -> Result<(), String> {
    settings::add_rebuild_time(elapsed_secs)
}

// ==================== Local Embeddings ====================

#[tauri::command]
pub fn get_use_local_embeddings() -> bool {
    settings::use_local_embeddings()
}

#[tauri::command]
pub fn set_use_local_embeddings(enabled: bool) -> Result<(), String> {
    settings::set_use_local_embeddings(enabled)
}

// ==================== Pipeline State ====================

use tauri::State;
use crate::app_state::AppState;

/// Get the current database pipeline state
/// Returns: fresh, imported, processed, clustered, hierarchized, complete
#[tauri::command]
pub fn get_pipeline_state(state: State<AppState>) -> Result<String, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
    Ok(db.get_pipeline_state())
}

/// Set the database pipeline state
#[tauri::command]
pub fn set_pipeline_state(state: State<AppState>, pipeline_state: String) -> Result<(), String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
    db.set_pipeline_state(&pipeline_state).map_err(|e| e.to_string())
}

/// Get all database metadata
#[tauri::command]
pub fn get_db_metadata(state: State<AppState>) -> Result<Vec<(String, String, i64)>, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
    db.get_all_metadata().map_err(|e| e.to_string())
}

// ==================== Clustering Thresholds ====================

/// Get clustering thresholds (primary, secondary)
/// Returns (None, None) for adaptive defaults
#[tauri::command]
pub fn get_clustering_thresholds() -> (Option<f32>, Option<f32>) {
    settings::get_clustering_thresholds()
}

/// Set clustering thresholds
/// Pass None for either to use adaptive defaults
#[tauri::command]
pub fn set_clustering_thresholds(primary: Option<f32>, secondary: Option<f32>) -> Result<(), String> {
    settings::set_clustering_thresholds(primary, secondary)
}

// ==================== Privacy Threshold ====================

/// Get privacy threshold (items below this go to Personal category)
#[tauri::command]
pub fn get_privacy_threshold() -> f32 {
    settings::get_privacy_threshold()
}

/// Set privacy threshold (0.0 to 1.0)
#[tauri::command]
pub fn set_privacy_threshold(threshold: f32) -> Result<(), String> {
    settings::set_privacy_threshold(threshold)
}

// ==================== Show Tips ====================

#[tauri::command]
pub fn get_show_tips() -> bool {
    settings::show_tips()
}

#[tauri::command]
pub fn set_show_tips(enabled: bool) -> Result<(), String> {
    settings::set_show_tips(enabled)
}

// ==================== LLM Backend (Ollama) ====================

/// Ollama status response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaStatus {
    pub available: bool,
    pub models: Vec<String>,
}

/// Check if Ollama is available and list models
#[tauri::command]
pub async fn check_ollama_status() -> OllamaStatus {
    use crate::ai_client;

    let available = ai_client::ollama_available().await;
    let models = if available {
        ai_client::ollama_list_models()
            .await
            .map(|m| m.into_iter().map(|model| model.name).collect())
            .unwrap_or_default()
    } else {
        vec![]
    };

    OllamaStatus { available, models }
}

/// Get current LLM backend ("anthropic" or "ollama")
#[tauri::command]
pub fn get_llm_backend() -> String {
    settings::get_llm_backend()
}

/// Set LLM backend ("anthropic" or "ollama")
#[tauri::command]
pub fn set_llm_backend(backend: String) -> Result<(), String> {
    settings::set_llm_backend(backend)
}

/// Get current Ollama model name
#[tauri::command]
pub fn get_ollama_model() -> String {
    settings::get_ollama_model()
}

/// Set Ollama model name
#[tauri::command]
pub fn set_ollama_model(model: String) -> Result<(), String> {
    settings::set_ollama_model(model)
}
