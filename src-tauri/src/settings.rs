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
    pub openaire_api_key: Option<String>,
    #[serde(default)]
    pub unpaywall_email: Option<String>,
    #[serde(default)]
    pub core_api_key: Option<String>,
    #[serde(default)]
    pub processing_stats: ProcessingStats,
    #[serde(default)]
    pub custom_db_path: Option<String>,
    #[serde(default = "default_true")]
    pub protect_recent_notes: bool,
    #[serde(default = "default_true")]
    pub use_local_embeddings: bool,
    #[serde(default = "default_cache_ttl")]
    pub similarity_cache_ttl_secs: u64,
    /// Manual override for primary clustering threshold (None = use adaptive)
    #[serde(default)]
    pub clustering_primary_threshold: Option<f32>,
    /// Manual override for secondary clustering threshold (None = use adaptive)
    #[serde(default)]
    pub clustering_secondary_threshold: Option<f32>,
    /// Privacy threshold - items below this go to Personal category (default: 0.5)
    #[serde(default = "default_privacy_threshold")]
    pub privacy_threshold: f32,
    /// Show tips/hints in the UI (default: true)
    #[serde(default = "default_true")]
    pub show_tips: bool,
    /// LLM backend: "anthropic" or "ollama" (default: "anthropic")
    #[serde(default = "default_llm_backend")]
    pub llm_backend: String,
    /// Ollama model name (default: "qwen2.5:7b")
    #[serde(default = "default_ollama_model")]
    pub ollama_model: String,
    /// Author name for team mode attribution
    #[serde(default)]
    pub author: Option<String>,
    /// Remote server URL for team sync (Phase 3)
    #[serde(default)]
    pub remote_url: Option<String>,
    /// API key for browser extension authentication
    #[serde(default)]
    pub extension_api_key: Option<String>,
}

fn default_llm_backend() -> String {
    "anthropic".to_string()
}

fn default_ollama_model() -> String {
    "qwen2.5:7b".to_string()
}

fn default_cache_ttl() -> u64 {
    300 // 5 minutes
}

fn default_true() -> bool {
    true
}

fn default_privacy_threshold() -> f32 {
    0.5 // Items below this go to Personal category
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            anthropic_api_key: None,
            openai_api_key: None,
            openaire_api_key: None,
            unpaywall_email: None,
            core_api_key: None,
            processing_stats: ProcessingStats::default(),
            custom_db_path: None,
            protect_recent_notes: true,
            use_local_embeddings: true, // Local embeddings are optimized for clustering
            similarity_cache_ttl_secs: 300, // 5 minutes
            clustering_primary_threshold: Some(0.75), // Tighter clusters for better accuracy
            clustering_secondary_threshold: Some(0.60), // Secondary assignment threshold
            privacy_threshold: 0.5, // Items below this go to Personal category
            show_tips: true, // Show tips by default
            llm_backend: "anthropic".to_string(), // Use Anthropic by default
            ollama_model: "qwen2.5:7b".to_string(), // Default Ollama model
            author: None,
            remote_url: None,
            extension_api_key: None,
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

// ==================== OpenAIRE API Key ====================

/// Get the OpenAIRE API key (checks env var first, then stored setting)
pub fn get_openaire_api_key() -> Option<String> {
    // Environment variable takes precedence
    if let Ok(key) = std::env::var("OPENAIRE_API_KEY") {
        if !key.is_empty() {
            return Some(key);
        }
    }

    // Fall back to stored setting
    let guard = SETTINGS.read().ok()?;
    let settings = guard.as_ref()?;
    settings.openaire_api_key.clone()
}

/// Check if OpenAIRE API key is available
pub fn has_openaire_api_key() -> bool {
    get_openaire_api_key().map(|k| !k.is_empty()).unwrap_or(false)
}

/// Set and save the OpenAIRE API key
pub fn set_openaire_api_key(key: String) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.openaire_api_key = if key.is_empty() { None } else { Some(key) };

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("OpenAIRE API key saved to settings");
    Ok(())
}

/// Get masked OpenAIRE API key for display (shows first/last 4 chars)
pub fn get_masked_openaire_api_key() -> Option<String> {
    get_openaire_api_key().map(|key| {
        if key.len() > 12 {
            format!("{}...{}", &key[..8], &key[key.len()-4..])
        } else {
            "*".repeat(key.len())
        }
    })
}

// ==================== Unpaywall Email ====================

/// Get the Unpaywall email (checks env var first, then stored setting)
pub fn get_unpaywall_email() -> Option<String> {
    // Environment variable takes precedence
    if let Ok(email) = std::env::var("UNPAYWALL_EMAIL") {
        if !email.is_empty() {
            return Some(email);
        }
    }

    // Fall back to stored setting
    let guard = SETTINGS.read().ok()?;
    let settings = guard.as_ref()?;
    settings.unpaywall_email.clone()
}

/// Check if Unpaywall email is available
pub fn has_unpaywall_email() -> bool {
    get_unpaywall_email().map(|e| !e.is_empty()).unwrap_or(false)
}

/// Set and save the Unpaywall email
pub fn set_unpaywall_email(email: String) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.unpaywall_email = if email.is_empty() { None } else { Some(email) };

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("Unpaywall email saved to settings");
    Ok(())
}

/// Get masked Unpaywall email for display (shows first 3 chars + domain)
pub fn get_masked_unpaywall_email() -> Option<String> {
    get_unpaywall_email().map(|email| {
        if let Some(at_pos) = email.find('@') {
            if at_pos > 3 {
                format!("{}...{}", &email[..3], &email[at_pos..])
            } else {
                format!("***{}", &email[at_pos..])
            }
        } else {
            "*".repeat(email.len())
        }
    })
}

// ==================== CORE API Key ====================

/// Get the CORE API key (checks env var first, then stored setting)
pub fn get_core_api_key() -> Option<String> {
    // Environment variable takes precedence
    if let Ok(key) = std::env::var("CORE_API_KEY") {
        if !key.is_empty() {
            return Some(key);
        }
    }

    // Fall back to stored setting
    let guard = SETTINGS.read().ok()?;
    let settings = guard.as_ref()?;
    settings.core_api_key.clone()
}

/// Check if CORE API key is available
pub fn has_core_api_key() -> bool {
    get_core_api_key().map(|k| !k.is_empty()).unwrap_or(false)
}

/// Set and save the CORE API key
pub fn set_core_api_key(key: String) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.core_api_key = if key.is_empty() { None } else { Some(key) };

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("CORE API key saved to settings");
    Ok(())
}

/// Get masked CORE API key for display (shows first/last 4 chars)
pub fn get_masked_core_api_key() -> Option<String> {
    get_core_api_key().map(|key| {
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

/// The fixed ID for the Holerabbit sessions container
pub const HOLERABBIT_CONTAINER_ID: &str = "holerabbit-sessions";

// ==================== Code Import Containers ====================

/// Container IDs for code imports by language
pub const RUST_IMPORT_CONTAINER_ID: &str = "import-rust";
pub const TYPESCRIPT_IMPORT_CONTAINER_ID: &str = "import-typescript";
pub const JAVASCRIPT_IMPORT_CONTAINER_ID: &str = "import-javascript";
pub const PYTHON_IMPORT_CONTAINER_ID: &str = "import-python";
pub const C_IMPORT_CONTAINER_ID: &str = "import-c";
pub const DOCS_IMPORT_CONTAINER_ID: &str = "import-docs";

/// All import container IDs for protection during hierarchy rebuilds
pub const IMPORT_CONTAINER_IDS: &[&str] = &[
    RUST_IMPORT_CONTAINER_ID,
    TYPESCRIPT_IMPORT_CONTAINER_ID,
    JAVASCRIPT_IMPORT_CONTAINER_ID,
    PYTHON_IMPORT_CONTAINER_ID,
    C_IMPORT_CONTAINER_ID,
    DOCS_IMPORT_CONTAINER_ID,
];

// ==================== Local Embeddings ====================

/// Check if local embeddings are enabled (default: true)
pub fn use_local_embeddings() -> bool {
    let guard = SETTINGS.read().ok();
    guard
        .as_ref()
        .and_then(|g| g.as_ref())
        .map(|s| s.use_local_embeddings)
        .unwrap_or(true) // Default to local (matches struct default)
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

// ==================== Similarity Cache ====================

/// Get similarity cache TTL in seconds (default: 300 = 5 minutes)
pub fn similarity_cache_ttl_secs() -> u64 {
    let guard = SETTINGS.read().ok();
    guard
        .as_ref()
        .and_then(|g| g.as_ref())
        .map(|s| s.similarity_cache_ttl_secs)
        .unwrap_or(300)
}

// ==================== Clustering Thresholds ====================

/// Get clustering thresholds (primary, secondary) - None means use adaptive
pub fn get_clustering_thresholds() -> (Option<f32>, Option<f32>) {
    let guard = SETTINGS.read().ok();
    guard
        .as_ref()
        .and_then(|g| g.as_ref())
        .map(|s| (s.clustering_primary_threshold, s.clustering_secondary_threshold))
        .unwrap_or((None, None))
}

/// Set clustering thresholds (None = use adaptive defaults)
pub fn set_clustering_thresholds(primary: Option<f32>, secondary: Option<f32>) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.clustering_primary_threshold = primary;
    settings.clustering_secondary_threshold = secondary;

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("Clustering thresholds set to: primary={:?}, secondary={:?}", primary, secondary);
    Ok(())
}

// ==================== Privacy Threshold ====================

/// Get privacy threshold (items below this go to Personal category)
pub fn get_privacy_threshold() -> f32 {
    let guard = SETTINGS.read().ok();
    guard
        .as_ref()
        .and_then(|g| g.as_ref())
        .map(|s| s.privacy_threshold)
        .unwrap_or(0.5)
}

/// Set privacy threshold
pub fn set_privacy_threshold(threshold: f32) -> Result<(), String> {
    // Validate range
    if threshold < 0.0 || threshold > 1.0 {
        return Err("Privacy threshold must be between 0.0 and 1.0".to_string());
    }

    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.privacy_threshold = threshold;

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("Privacy threshold set to: {}", threshold);
    Ok(())
}

// ==================== Show Tips ====================

/// Check if tips are enabled (default: true)
pub fn show_tips() -> bool {
    let guard = SETTINGS.read().ok();
    guard
        .as_ref()
        .and_then(|g| g.as_ref())
        .map(|s| s.show_tips)
        .unwrap_or(true)
}

/// Set show tips preference
pub fn set_show_tips(enabled: bool) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.show_tips = enabled;

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("Show tips set to: {}", enabled);
    Ok(())
}

// ==================== LLM Backend ====================

/// Get LLM backend: "anthropic" or "ollama" (default: "anthropic")
pub fn get_llm_backend() -> String {
    let guard = SETTINGS.read().ok();
    guard
        .as_ref()
        .and_then(|g| g.as_ref())
        .map(|s| s.llm_backend.clone())
        .unwrap_or_else(|| "anthropic".to_string())
}

/// Set LLM backend: "anthropic" or "ollama"
pub fn set_llm_backend(backend: String) -> Result<(), String> {
    // Validate backend
    if backend != "anthropic" && backend != "ollama" {
        return Err(format!("Invalid LLM backend: {}. Must be 'anthropic' or 'ollama'", backend));
    }

    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.llm_backend = backend.clone();

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("LLM backend set to: {}", backend);
    Ok(())
}

/// Get Ollama model name (default: "qwen2.5:7b")
pub fn get_ollama_model() -> String {
    let guard = SETTINGS.read().ok();
    guard
        .as_ref()
        .and_then(|g| g.as_ref())
        .map(|s| s.ollama_model.clone())
        .unwrap_or_else(|| "qwen2.5:7b".to_string())
}

/// Set Ollama model name
pub fn set_ollama_model(model: String) -> Result<(), String> {
    if model.is_empty() {
        return Err("Ollama model name cannot be empty".to_string());
    }

    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.ollama_model = model.clone();

    // Save to disk
    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;

    println!("Ollama model set to: {}", model);
    Ok(())
}

// ==================== Team Mode Author ====================

/// Get configured author name. Checks MYCELICA_AUTHOR env var first, then settings.
pub fn get_author() -> Option<String> {
    if let Ok(author) = std::env::var("MYCELICA_AUTHOR") {
        if !author.is_empty() {
            return Some(author);
        }
    }
    let guard = SETTINGS.read().ok()?;
    let settings = guard.as_ref()?;
    settings.author.clone()
}

/// Get author, falling back to system username if not configured.
pub fn get_author_or_default() -> String {
    get_author().unwrap_or_else(|| {
        std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "anonymous".to_string())
    })
}

/// Set author name for team mode attribution
pub fn set_author(name: String) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.author = if name.is_empty() { None } else { Some(name) };

    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;
    Ok(())
}

// ==================== Remote URL (Phase 3) ====================

/// Get configured remote server URL
pub fn get_remote_url() -> Option<String> {
    let guard = SETTINGS.read().ok()?;
    let settings = guard.as_ref()?;
    settings.remote_url.clone()
}

/// Set remote server URL for team mode
pub fn set_remote_url(url: String) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.remote_url = if url.is_empty() { None } else { Some(url) };

    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;
    Ok(())
}

// ==================== Extension API Key ====================

/// Get the extension API key for browser extension authentication
pub fn get_extension_api_key() -> Option<String> {
    let guard = SETTINGS.read().ok()?;
    let settings = guard.as_ref()?;
    settings.extension_api_key.clone()
}

/// Set and save the extension API key
pub fn set_extension_api_key(key: String) -> Result<(), String> {
    let mut settings_guard = SETTINGS.write()
        .map_err(|_| "Failed to acquire settings lock")?;

    let settings = settings_guard.get_or_insert_with(Settings::default);
    settings.extension_api_key = if key.is_empty() { None } else { Some(key) };

    let config_path = CONFIG_PATH.read()
        .map_err(|_| "Failed to acquire config path lock")?
        .clone()
        .ok_or("Settings not initialized")?;

    settings.save(&config_path)?;
    Ok(())
}

/// Get masked extension API key for display (shows first 8/last 4 chars)
pub fn get_masked_extension_api_key() -> Option<String> {
    get_extension_api_key().map(|key| {
        if key.len() > 12 {
            format!("{}...{}", &key[..8], &key[key.len()-4..])
        } else {
            "*".repeat(key.len())
        }
    })
}
