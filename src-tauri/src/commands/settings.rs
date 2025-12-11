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
