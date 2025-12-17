//! Application settings storage
//!
//! Stores configuration like API keys in a JSON file in the app data directory.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;

/// Global settings instance
static SETTINGS: RwLock<Option<Settings>> = RwLock::new(None);

/// Path to config file (set during init)
static CONFIG_PATH: RwLock<Option<PathBuf>> = RwLock::new(None);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProcessingStats {
    #[serde(default)]
    pub total_ai_processing_secs: f64,
    #[serde(default)]
    pub total_rebuild_secs: f64,
    #[serde(default)]
    pub last_ai_processing_secs: f64,
    #[serde(default)]
    pub last_rebuild_secs: f64,
    #[serde(default)]
    pub ai_processing_runs: u32,
    #[serde(default)]
    pub rebuild_runs: u32,
    #[serde(default)]
    pub total_anthropic_input_tokens: u64,
    #[serde(default)]
    pub total_anthropic_output_tokens: u64,
    #[serde(default)]
    pub total_openai_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub anthropic_api_key: Option<String>,
    #[serde(default)]
    pub openai_api_key: Option<String>,
    #[serde(default)]
    pub processing_stats: ProcessingStats,
    #[serde(default)]
    pub custom_db_path: Option<String>,
    #[serde(default = "default_true")]
    pub protect_recent_notes: bool,
    #[serde(default = "default_true")]
    pub use_local_embeddings: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            anthropic_api_key: None,
            openai_api_key: None,
            processing_stats: ProcessingStats::default(),
            custom_db_path: None,
            protect_recent_notes: true,
            use_local_embeddings: true, // Local embeddings are optimized for clustering
        }
    }
}

impl Settings {
    /// Load settings from disk or create default
    fn load(path: &PathBuf) -> Self {
        if path.exists() {
            match fs::read_to_string(path) {
                Ok(content) => {
                    serde_json::from_str(&content).unwrap_or_default()
                }
                Err(_) => Settings::default(),
            }
        } else {
            Settings::default()
        }
    }

    /// Save settings to disk
    fn save(&self, path: &PathBuf) -> Result<(), String> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }

        fs::write(path, content)
            .map_err(|e| format!("Failed to write settings: {}", e))?;

        Ok(())
    }
}

/// Initialize settings with the app data directory
pub fn init(app_data_dir: PathBuf) {
    let config_path = app_data_dir.join("settings.json");
    let settings = Settings::load(&config_path);

    println!("Settings loaded from: {:?}", config_path);

    *CONFIG_PATH.write().unwrap() = Some(config_path);
    *SETTINGS.write().unwrap() = Some(settings);
}

/// Get the current API key (checks env var first, then stored setting)
pub fn get_api_key() -> Option<String> {
    // Environment variable takes precedence
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            return Some(key);
        }
    }

    // Fall back to stored setting
    let guard = SETTINGS.read().ok()?;
    let settings = guard.as_ref()?;
    settings.anthropic_api_key.clone()
}

/// Check if API key is available
pub fn has_api_key() -> bool {
    get_api_key().map(|k| !k.is_empty()).unwrap_or(false)
}

/// Set and save the API key
pub fn set_api_key(key: String) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.anthropic_api_key = if key.is_empty() { None } else { Some(key) };

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("API key saved to settings");
    Ok(())
}

/// Get masked API key for display (shows first/last 4 chars)
pub fn get_masked_api_key() -> Option<String> {
    get_api_key().map(|key| {
        if key.len() > 12 {
            format!("{}...{}", &key[..8], &key[key.len()-4..])
        } else {
            "*".repeat(key.len())
        }
    })
}

// ==================== OpenAI API Key (for embeddings) ====================

/// Get the OpenAI API key (checks env var first, then stored setting)
pub fn get_openai_api_key() -> Option<String> {
    // Environment variable takes precedence
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        if !key.is_empty() {
            return Some(key);
        }
    }

    // Fall back to stored setting
    let guard = SETTINGS.read().ok()?;
    let settings = guard.as_ref()?;
    settings.openai_api_key.clone()
}

/// Check if OpenAI API key is available
pub fn has_openai_api_key() -> bool {
    get_openai_api_key().map(|k| !k.is_empty()).unwrap_or(false)
}

/// Set and save the OpenAI API key
pub fn set_openai_api_key(key: String) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.openai_api_key = if key.is_empty() { None } else { Some(key) };

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("OpenAI API key saved to settings");
    Ok(())
}

/// Get masked OpenAI API key for display (shows first/last 4 chars)
pub fn get_masked_openai_api_key() -> Option<String> {
    get_openai_api_key().map(|key| {
        if key.len() > 12 {
            format!("{}...{}", &key[..8], &key[key.len()-4..])
        } else {
            "*".repeat(key.len())
        }
    })
}

// ==================== Processing Stats ====================

/// Get processing stats
pub fn get_processing_stats() -> ProcessingStats {
    let guard = SETTINGS.read().ok();
    guard
        .as_ref()
        .and_then(|g| g.as_ref())
        .map(|s| s.processing_stats.clone())
        .unwrap_or_default()
}

/// Add AI processing time (additive)
pub fn add_ai_processing_time(elapsed_secs: f64) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.processing_stats.total_ai_processing_secs += elapsed_secs;
    settings.processing_stats.last_ai_processing_secs = elapsed_secs;
    settings.processing_stats.ai_processing_runs += 1;

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("AI processing time saved: {:.1}s (total: {:.1}s, runs: {})",
        elapsed_secs,
        settings.processing_stats.total_ai_processing_secs,
        settings.processing_stats.ai_processing_runs);
    Ok(())
}

/// Set rebuild time (replaces previous - rebuild replaces hierarchy each time)
pub fn add_rebuild_time(elapsed_secs: f64) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);

    // Rebuild replaces hierarchy, so replace time instead of adding
    settings.processing_stats.total_rebuild_secs = elapsed_secs;
    settings.processing_stats.last_rebuild_secs = elapsed_secs;
    settings.processing_stats.rebuild_runs += 1;

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("Rebuild time saved: {:.1}s (runs: {})",
        elapsed_secs,
        settings.processing_stats.rebuild_runs);
    Ok(())
}

/// Add Anthropic API token usage
pub fn add_anthropic_tokens(input_tokens: u64, output_tokens: u64) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.processing_stats.total_anthropic_input_tokens += input_tokens;
    settings.processing_stats.total_anthropic_output_tokens += output_tokens;

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;
    Ok(())
}

/// Add OpenAI API token usage
pub fn add_openai_tokens(tokens: u64) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.processing_stats.total_openai_tokens += tokens;

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;
    Ok(())
}

// ==================== Custom Database Path ====================

/// Get custom database path (if set)
pub fn get_custom_db_path() -> Option<String> {
    let guard = SETTINGS.read().ok()?;
    let settings = guard.as_ref()?;
    settings.custom_db_path.clone()
}

/// Set custom database path
pub fn set_custom_db_path(path: Option<String>) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.custom_db_path = path.clone();

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("Custom DB path saved: {:?}", path);
    Ok(())
}

// ==================== Recent Notes Protection ====================

/// Check if Recent Notes protection is enabled (default: true)
pub fn is_recent_notes_protected() -> bool {
    let guard = SETTINGS.read().ok();
    guard
        .as_ref()
        .and_then(|g| g.as_ref())
        .map(|s| s.protect_recent_notes)
        .unwrap_or(true) // Default to protected
}

/// Set Recent Notes protection
pub fn set_protect_recent_notes(protected: bool) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.protect_recent_notes = protected;

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("Recent Notes protection set to: {}", protected);
    Ok(())
}

/// The fixed ID for the Recent Notes container
pub const RECENT_NOTES_CONTAINER_ID: &str = "container-recent-notes";

// ==================== Local Embeddings ====================

/// Check if local embeddings are enabled (default: false)
pub fn use_local_embeddings() -> bool {
    let guard = SETTINGS.read().ok();
    guard
        .as_ref()
        .and_then(|g| g.as_ref())
        .map(|s| s.use_local_embeddings)
        .unwrap_or(false) // Default to OpenAI
}

/// Set local embeddings preference
pub fn set_use_local_embeddings(enabled: bool) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.use_local_embeddings = enabled;

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("Local embeddings set to: {}", enabled);
    Ok(())
}
