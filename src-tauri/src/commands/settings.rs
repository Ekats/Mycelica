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
