//! Privacy filtering commands for generating shareable databases
//!
//! Uses Claude Haiku to analyze nodes for sensitive content and allows
//! exporting a database with private nodes removed.

use crate::commands::graph::AppState;
use crate::settings;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::fs;
use tauri::{AppHandle, Emitter, State};

pub static CANCEL_PRIVACY_SCAN: AtomicBool = AtomicBool::new(false);

/// Showcase mode - much stricter, for demo databases
const PRIVACY_PROMPT_SHOWCASE: &str = r#"Analyze this content for a PUBLIC SHOWCASE database. Be VERY strict. Return JSON only.

Content:
- Title: {title}
- Summary: {summary}
- Tags: {tags}
- Content snippet: {content}

Mark as PRIVATE (is_private: true) - filter OUT - if it contains ANY of:
- ANY personal information, names, locations, employers, schools
- Health, medical, relationships, family, dating
- Financial details of any kind
- Opinions about specific people or companies
- Personal projects (unless it's Mycelica itself)
- Career discussions, job searching, interviews
- Personal struggles, emotions, venting
- Specific usernames, accounts, file paths with names
- "I want", "I need", "I feel" personal statements
- Daily life, routines, personal preferences
- Anything that reveals identity or personal context

Mark as SAFE (is_private: false) - KEEP for showcase - ONLY if it's:
- Mycelica development (this project itself - always keep!)
- Pure philosophy, epistemology, consciousness discussions
- Abstract technical architecture discussions
- Interesting AI/ML concepts and discussions
- Pure code examples with no personal context
- Educational explanations of universal concepts
- Meta discussions about knowledge organization

The goal is a CLEAN demo database with only universally interesting content.
When in doubt, mark PRIVATE. Be aggressive about filtering.

Respond with ONLY: {"is_private": true, "reason": "brief explanation"} or {"is_private": false, "reason": null}"#;

const PRIVACY_PROMPT: &str = r#"Analyze this content for privacy sensitivity. Return JSON only.

Content:
- Title: {title}
- Summary: {summary}
- Tags: {tags}
- Content snippet: {content}

Mark as PRIVATE (is_private: true) ONLY if it contains:
- Health, medical, mental health topics (conditions, symptoms, medications)
- Relationship, dating, or family discussions
- Financial details (salaries, debts, specific amounts)
- Complaints or negative opinions about specific people, employers, or coworkers
- Personal emotional struggles, venting, or crisis moments
- Specific home addresses or personal location data
- Personal file paths with usernames (e.g., /home/username/, C:\Users\name\)
- Non-public names of people (friends, family, coworkers)

Mark as SAFE (is_private: false) if it's:
- Technical discussions, even with "I think" or "I want to implement"
- Code, debugging, or software architecture discussions
- Project planning or feature discussions (even for personal projects)
- Educational or factual content
- Public figure discussions in professional context

Technical first-person statements like "I implemented", "I think this approach",
"I want to add" are SAFE — these are normal developer narration, not personal disclosure.

When genuinely uncertain, mark PRIVATE.

Respond with ONLY: {"is_private": true, "reason": "brief explanation"} or {"is_private": false, "reason": null}"#;

/// Progress event for privacy scanning
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyProgressEvent {
    pub current: usize,
    pub total: usize,
    pub node_title: String,
    pub is_private: bool,
    pub reason: Option<String>,
    pub status: String, // "processing", "success", "error", "complete", "cancelled"
    pub error_message: Option<String>,
}

/// Privacy statistics
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyStats {
    pub total: usize,
    pub scanned: usize,
    pub unscanned: usize,
    pub private: usize,
    pub safe: usize,
    // Category stats
    pub total_categories: usize,
    pub scanned_categories: usize,
}

/// Result of analyzing a single node
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyResult {
    pub is_private: bool,
    pub reason: Option<String>,
}

/// Summary report after batch analysis
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyReport {
    pub total: usize,
    pub private_count: usize,
    pub safe_count: usize,
    pub error_count: usize,
    pub cancelled: bool,
}

/// Summary report after category scanning (includes propagation)
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryPrivacyReport {
    pub categories_scanned: usize,
    pub categories_private: usize,
    pub categories_safe: usize,
    pub items_propagated: usize,  // Items marked private via inheritance
    pub error_count: usize,
    pub cancelled: bool,
}

/// Anthropic API structures
#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: String,
}

/// Emit privacy progress event
fn emit_progress(app: &AppHandle, event: PrivacyProgressEvent) {
    let _ = app.emit("privacy-progress", event);
}

/// Cancel ongoing privacy scan
#[tauri::command]
pub fn cancel_privacy_scan() -> Result<(), String> {
    CANCEL_PRIVACY_SCAN.store(true, Ordering::SeqCst);
    Ok(())
}

/// Reset all privacy flags (to re-scan with different settings)
#[tauri::command]
pub fn reset_privacy_flags(state: State<AppState>) -> Result<usize, String> {
    state.db.reset_all_privacy_flags()
        .map_err(|e| e.to_string())
}

/// Get privacy statistics
#[tauri::command]
pub fn get_privacy_stats(state: State<AppState>) -> Result<PrivacyStats, String> {
    let (total, scanned, unscanned, private, safe, total_categories, scanned_categories) = state.db
        .get_privacy_stats_extended()
        .map_err(|e| e.to_string())?;

    Ok(PrivacyStats {
        total,
        scanned,
        unscanned,
        private,
        safe,
        total_categories,
        scanned_categories,
    })
}

/// Manually set a node's privacy status
#[tauri::command]
pub fn set_node_privacy(
    state: State<'_, AppState>,
    node_id: String,
    is_private: bool,
) -> Result<(), String> {
    let reason = if is_private {
        Some("Manually marked as private")
    } else {
        None
    };
    state.db.update_node_privacy(&node_id, is_private, reason)
        .map_err(|e| e.to_string())
}

/// Analyze a single node for privacy
#[tauri::command]
pub async fn analyze_node_privacy(
    state: State<'_, AppState>,
    node_id: String,
) -> Result<PrivacyResult, String> {
    let api_key = settings::get_api_key().ok_or("ANTHROPIC_API_KEY not set")?;

    // Get node from db
    let node = state.db.get_node(&node_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Node {} not found", node_id))?;

    // Build prompt with node content
    let title = node.ai_title.as_deref().unwrap_or(&node.title);
    let summary = node.summary.as_deref().unwrap_or("");
    let tags = node.tags.as_deref().unwrap_or("[]");
    // Include content (truncated to ~2000 chars to stay under token limits)
    let content = node.content.as_deref().unwrap_or("");
    let content_snippet = if content.len() > 2000 {
        // Find a valid UTF-8 boundary near 2000 bytes
        let truncate_at = content.char_indices()
            .take_while(|(i, _)| *i < 2000)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        format!("{}...", &content[..truncate_at])
    } else {
        content.to_string()
    };

    let prompt = PRIVACY_PROMPT
        .replace("{title}", title)
        .replace("{summary}", summary)
        .replace("{tags}", tags)
        .replace("{content}", &content_snippet);

    // Call Claude Haiku API
    let request = AnthropicRequest {
        model: "claude-3-5-haiku-20241022".to_string(),
        max_tokens: 200,
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: prompt,
        }],
    };

    let client = reqwest::Client::new();
    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("API error {}: {}", status, body));
    }

    let api_response: AnthropicResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    // Track token usage
    if let Some(usage) = &api_response.usage {
        let _ = settings::add_anthropic_tokens(usage.input_tokens, usage.output_tokens);
    }

    // Parse response
    let text = api_response
        .content
        .first()
        .map(|c| c.text.clone())
        .unwrap_or_default();

    let (is_private, reason) = parse_privacy_response(&text)?;

    // Update node in database
    state.db.update_node_privacy(&node_id, is_private, reason.as_deref())
        .map_err(|e| e.to_string())?;

    Ok(PrivacyResult { is_private, reason })
}

/// Parse the AI response for privacy classification
fn parse_privacy_response(text: &str) -> Result<(bool, Option<String>), String> {
    // Try to extract JSON from the response
    let json_text = if text.starts_with("```") {
        text.lines()
            .skip(1)
            .take_while(|l| !l.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.to_string()
    };

    match serde_json::from_str::<serde_json::Value>(&json_text) {
        Ok(json) => {
            let is_private = json
                .get("is_private")
                .and_then(|v| v.as_bool())
                .unwrap_or(true); // Default to private if parsing fails (cautious)

            let reason = json
                .get("reason")
                .and_then(|v| v.as_str())
                .map(String::from);

            Ok((is_private, reason))
        }
        Err(_) => {
            // If JSON parsing fails, be cautious and mark as private
            Ok((true, Some("Failed to parse AI response - marked private for safety".to_string())))
        }
    }
}

/// Analyze all unscanned nodes for privacy
#[tauri::command]
pub async fn analyze_all_privacy(
    state: State<'_, AppState>,
    app: AppHandle,
    showcase_mode: Option<bool>,
) -> Result<PrivacyReport, String> {
    CANCEL_PRIVACY_SCAN.store(false, Ordering::SeqCst);

    let api_key = settings::get_api_key().ok_or("ANTHROPIC_API_KEY not set")?;
    let prompt_template = if showcase_mode.unwrap_or(false) {
        PRIVACY_PROMPT_SHOWCASE
    } else {
        PRIVACY_PROMPT
    };

    // Get all items that haven't been scanned
    let nodes = state.db.get_items_needing_privacy_scan()
        .map_err(|e| e.to_string())?;

    let total = nodes.len();
    let mut private_count = 0;
    let mut safe_count = 0;
    let mut error_count = 0;

    if total == 0 {
        emit_progress(&app, PrivacyProgressEvent {
            current: 0,
            total: 0,
            node_title: String::new(),
            is_private: false,
            reason: None,
            status: "complete".to_string(),
            error_message: None,
        });
        return Ok(PrivacyReport {
            total: 0,
            private_count: 0,
            safe_count: 0,
            error_count: 0,
            cancelled: false,
        });
    }

    let client = reqwest::Client::new();

    for (i, node) in nodes.iter().enumerate() {
        // Check for cancellation
        if CANCEL_PRIVACY_SCAN.load(Ordering::SeqCst) {
            emit_progress(&app, PrivacyProgressEvent {
                current: i,
                total,
                node_title: String::new(),
                is_private: false,
                reason: None,
                status: "cancelled".to_string(),
                error_message: None,
            });
            return Ok(PrivacyReport {
                total,
                private_count,
                safe_count,
                error_count,
                cancelled: true,
            });
        }

        let title = node.ai_title.as_deref().unwrap_or(&node.title);

        // Emit processing event
        emit_progress(&app, PrivacyProgressEvent {
            current: i + 1,
            total,
            node_title: title.to_string(),
            is_private: false,
            reason: None,
            status: "processing".to_string(),
            error_message: None,
        });

        // Build prompt
        let summary = node.summary.as_deref().unwrap_or("");
        let tags = node.tags.as_deref().unwrap_or("[]");
        // Include content (truncated to ~2000 chars to stay under token limits)
        let content = node.content.as_deref().unwrap_or("");
        let content_snippet = if content.len() > 2000 {
            // Find a valid UTF-8 boundary near 2000 bytes
            let truncate_at = content.char_indices()
                .take_while(|(i, _)| *i < 2000)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            format!("{}...", &content[..truncate_at])
        } else {
            content.to_string()
        };

        let prompt = prompt_template
            .replace("{title}", title)
            .replace("{summary}", summary)
            .replace("{tags}", tags)
            .replace("{content}", &content_snippet);

        let request = AnthropicRequest {
            model: "claude-3-5-haiku-20241022".to_string(),
            max_tokens: 200,
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: prompt,
            }],
        };

        // Make API call
        match client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    if let Ok(api_response) = response.json::<AnthropicResponse>().await {
                        // Track tokens
                        if let Some(usage) = &api_response.usage {
                            let _ = settings::add_anthropic_tokens(usage.input_tokens, usage.output_tokens);
                        }

                        let text = api_response
                            .content
                            .first()
                            .map(|c| c.text.clone())
                            .unwrap_or_default();

                        let (is_private, reason) = parse_privacy_response(&text)
                            .unwrap_or((true, Some("Parse error".to_string())));

                        // Update database
                        if let Err(e) = state.db.update_node_privacy(&node.id, is_private, reason.as_deref()) {
                            eprintln!("Failed to update node {}: {}", node.id, e);
                            error_count += 1;
                        } else {
                            if is_private {
                                private_count += 1;
                                println!("[{}/{}] 🔒 PRIVATE: \"{}\" - {}", i + 1, total, title, reason.as_deref().unwrap_or("no reason"));
                            } else {
                                safe_count += 1;
                                println!("[{}/{}] ✓ SAFE: \"{}\"", i + 1, total, title);
                            }

                            emit_progress(&app, PrivacyProgressEvent {
                                current: i + 1,
                                total,
                                node_title: title.to_string(),
                                is_private,
                                reason: reason.clone(),
                                status: "success".to_string(),
                                error_message: None,
                            });
                        }
                    } else {
                        error_count += 1;
                        eprintln!("[{}/{}] ❌ ERROR: \"{}\" - Failed to parse API response", i + 1, total, title);
                        emit_progress(&app, PrivacyProgressEvent {
                            current: i + 1,
                            total,
                            node_title: title.to_string(),
                            is_private: false,
                            reason: None,
                            status: "error".to_string(),
                            error_message: Some("Failed to parse API response".to_string()),
                        });
                    }
                } else {
                    error_count += 1;
                    let status_code = response.status();
                    eprintln!("[{}/{}] ❌ ERROR: \"{}\" - API error: {}", i + 1, total, title, status_code);
                    emit_progress(&app, PrivacyProgressEvent {
                        current: i + 1,
                        total,
                        node_title: title.to_string(),
                        is_private: false,
                        reason: None,
                        status: "error".to_string(),
                        error_message: Some(format!("API error: {}", status_code)),
                    });
                }
            }
            Err(e) => {
                error_count += 1;
                eprintln!("[{}/{}] ❌ ERROR: \"{}\" - Request failed: {}", i + 1, total, title, e);
                emit_progress(&app, PrivacyProgressEvent {
                    current: i + 1,
                    total,
                    node_title: title.to_string(),
                    is_private: false,
                    reason: None,
                    status: "error".to_string(),
                    error_message: Some(format!("Request failed: {}", e)),
                });
            }
        }

        // Small delay to avoid rate limits (100ms)
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // Emit completion event
    emit_progress(&app, PrivacyProgressEvent {
        current: total,
        total,
        node_title: String::new(),
        is_private: false,
        reason: None,
        status: "complete".to_string(),
        error_message: None,
    });

    Ok(PrivacyReport {
        total,
        private_count,
        safe_count,
        error_count,
        cancelled: false,
    })
}

/// Analyze category nodes (topics/domains) for privacy, then propagate to descendants
/// This is much faster than scanning individual items - if a category is private,
/// all its children inherit that status automatically
#[tauri::command]
pub async fn analyze_categories_privacy(
    state: State<'_, AppState>,
    app: AppHandle,
    showcase_mode: Option<bool>,
) -> Result<CategoryPrivacyReport, String> {
    CANCEL_PRIVACY_SCAN.store(false, Ordering::SeqCst);

    let api_key = settings::get_api_key().ok_or("ANTHROPIC_API_KEY not set")?;
    let prompt_template = if showcase_mode.unwrap_or(false) {
        PRIVACY_PROMPT_SHOWCASE
    } else {
        PRIVACY_PROMPT
    };

    // Get all category nodes (non-items with children) that haven't been scanned
    let categories = state.db.get_category_nodes_needing_privacy_scan()
        .map_err(|e| e.to_string())?;

    let total = categories.len();
    let mut categories_private = 0;
    let mut categories_safe = 0;
    let mut items_propagated = 0;
    let mut error_count = 0;

    if total == 0 {
        emit_progress(&app, PrivacyProgressEvent {
            current: 0,
            total: 0,
            node_title: String::new(),
            is_private: false,
            reason: None,
            status: "complete".to_string(),
            error_message: None,
        });
        return Ok(CategoryPrivacyReport {
            categories_scanned: 0,
            categories_private: 0,
            categories_safe: 0,
            items_propagated: 0,
            error_count: 0,
            cancelled: false,
        });
    }

    let client = reqwest::Client::new();

    for (i, category) in categories.iter().enumerate() {
        // Check for cancellation
        if CANCEL_PRIVACY_SCAN.load(Ordering::SeqCst) {
            emit_progress(&app, PrivacyProgressEvent {
                current: i,
                total,
                node_title: String::new(),
                is_private: false,
                reason: None,
                status: "cancelled".to_string(),
                error_message: None,
            });
            return Ok(CategoryPrivacyReport {
                categories_scanned: i,
                categories_private,
                categories_safe,
                items_propagated,
                error_count,
                cancelled: true,
            });
        }

        let title = category.ai_title.as_deref()
            .or(category.cluster_label.as_deref())
            .unwrap_or(&category.title);

        // Emit processing event
        emit_progress(&app, PrivacyProgressEvent {
            current: i + 1,
            total,
            node_title: format!("{} ({} children)", title, category.child_count),
            is_private: false,
            reason: None,
            status: "processing".to_string(),
            error_message: None,
        });

        // Build prompt - for categories, use cluster label and child count context
        let summary = category.summary.as_deref().unwrap_or("");
        let tags = category.tags.as_deref().unwrap_or("[]");
        // Include content (truncated to ~2000 chars to stay under token limits)
        // Categories may not have content, so empty string is fine
        let content = category.content.as_deref().unwrap_or("");
        let content_snippet = if content.len() > 2000 {
            // Find a valid UTF-8 boundary near 2000 bytes
            let truncate_at = content.char_indices()
                .take_while(|(i, _)| *i < 2000)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            format!("{}...", &content[..truncate_at])
        } else {
            content.to_string()
        };

        let prompt = prompt_template
            .replace("{title}", title)
            .replace("{summary}", summary)
            .replace("{tags}", tags)
            .replace("{content}", &content_snippet);

        let request = AnthropicRequest {
            model: "claude-3-5-haiku-20241022".to_string(),
            max_tokens: 200,
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: prompt,
            }],
        };

        // Make API call
        match client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    if let Ok(api_response) = response.json::<AnthropicResponse>().await {
                        // Track tokens
                        if let Some(usage) = &api_response.usage {
                            let _ = settings::add_anthropic_tokens(usage.input_tokens, usage.output_tokens);
                        }

                        let text = api_response
                            .content
                            .first()
                            .map(|c| c.text.clone())
                            .unwrap_or_default();

                        let (is_private, reason) = parse_privacy_response(&text)
                            .unwrap_or((true, Some("Parse error".to_string())));

                        // Update the category node itself
                        if let Err(e) = state.db.update_node_privacy(&category.id, is_private, reason.as_deref()) {
                            eprintln!("Failed to update category {}: {}", category.id, e);
                            error_count += 1;
                            continue;
                        }

                        if is_private {
                            categories_private += 1;

                            // PROPAGATE to all descendants!
                            let propagation_reason = format!("Inherited from private category: {}", title);
                            match state.db.propagate_privacy_to_descendants(&category.id, &propagation_reason) {
                                Ok(count) => {
                                    items_propagated += count;
                                    println!("Category '{}' marked private, propagated to {} descendants", title, count);
                                }
                                Err(e) => {
                                    eprintln!("Failed to propagate privacy for {}: {}", category.id, e);
                                }
                            }

                            emit_progress(&app, PrivacyProgressEvent {
                                current: i + 1,
                                total,
                                node_title: format!("{} → {} items", title, category.child_count),
                                is_private: true,
                                reason: reason.clone(),
                                status: "success".to_string(),
                                error_message: None,
                            });
                        } else {
                            categories_safe += 1;
                            emit_progress(&app, PrivacyProgressEvent {
                                current: i + 1,
                                total,
                                node_title: title.to_string(),
                                is_private: false,
                                reason: None,
                                status: "success".to_string(),
                                error_message: None,
                            });
                        }
                    } else {
                        error_count += 1;
                        emit_progress(&app, PrivacyProgressEvent {
                            current: i + 1,
                            total,
                            node_title: title.to_string(),
                            is_private: false,
                            reason: None,
                            status: "error".to_string(),
                            error_message: Some("Failed to parse API response".to_string()),
                        });
                    }
                } else {
                    error_count += 1;
                    let status_code = response.status();
                    emit_progress(&app, PrivacyProgressEvent {
                        current: i + 1,
                        total,
                        node_title: title.to_string(),
                        is_private: false,
                        reason: None,
                        status: "error".to_string(),
                        error_message: Some(format!("API error: {}", status_code)),
                    });
                }
            }
            Err(e) => {
                error_count += 1;
                emit_progress(&app, PrivacyProgressEvent {
                    current: i + 1,
                    total,
                    node_title: title.to_string(),
                    is_private: false,
                    reason: None,
                    status: "error".to_string(),
                    error_message: Some(format!("Request failed: {}", e)),
                });
            }
        }

        // Small delay to avoid rate limits (100ms)
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // Emit completion event
    emit_progress(&app, PrivacyProgressEvent {
        current: total,
        total,
        node_title: String::new(),
        is_private: false,
        reason: None,
        status: "complete".to_string(),
        error_message: None,
    });

    println!("Category scan complete: {} categories ({} private, {} safe), {} items propagated",
             total, categories_private, categories_safe, items_propagated);

    Ok(CategoryPrivacyReport {
        categories_scanned: total,
        categories_private,
        categories_safe,
        items_propagated,
        error_count,
        cancelled: false,
    })
}

/// Export a shareable database with private nodes removed
#[tauri::command]
pub fn export_shareable_db(state: State<AppState>) -> Result<String, String> {
    use rusqlite::Connection;

    // Get current db path
    let db_path = state.db.get_path();

    // Generate shareable path
    let shareable_path = if db_path.ends_with(".db") {
        db_path.replace(".db", "-shareable.db")
    } else {
        format!("{}-shareable.db", db_path)
    };

    // Copy the file
    fs::copy(&db_path, &shareable_path)
        .map_err(|e| format!("Failed to copy database: {}", e))?;

    // Open the copy and remove private nodes
    let conn = Connection::open(&shareable_path)
        .map_err(|e| format!("Failed to open shareable db: {}", e))?;

    // Delete private nodes
    let deleted_nodes = conn.execute("DELETE FROM nodes WHERE is_private = 1", [])
        .map_err(|e| format!("Failed to delete private nodes: {}", e))?;

    // Delete orphaned edges
    let deleted_edges = conn.execute(
        "DELETE FROM edges WHERE source_id NOT IN (SELECT id FROM nodes) OR target_id NOT IN (SELECT id FROM nodes)",
        []
    ).map_err(|e| format!("Failed to delete orphaned edges: {}", e))?;

    // Vacuum to reclaim space
    conn.execute("VACUUM", [])
        .map_err(|e| format!("Failed to vacuum database: {}", e))?;

    println!("Exported shareable database: {} (removed {} nodes, {} edges)",
             shareable_path, deleted_nodes, deleted_edges);

    Ok(shareable_path)
}
