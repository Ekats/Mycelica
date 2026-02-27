//! Privacy filtering commands for generating shareable databases
//!
//! Uses Claude Haiku to analyze nodes for sensitive content and allows
//! exporting a database with private nodes removed.

use crate::app_state::AppState;
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
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.reset_all_privacy_flags()
        .map_err(|e| e.to_string())
}

/// Get privacy statistics
#[tauri::command]
pub fn get_privacy_stats(state: State<AppState>) -> Result<PrivacyStats, String> {
    let (total, scanned, unscanned, private, safe, total_categories, scanned_categories) = state.db.read().map_err(|e| format!("DB lock error: {}", e))?
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

/// Preview counts for export at a given threshold
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreview {
    pub included: usize,
    pub excluded: usize,
    pub unscored: usize,
}

/// Get preview of how many items would be included/excluded at a threshold
/// include_tags: optional whitelist - if provided, also filters by tag
#[tauri::command]
pub fn get_export_preview(
    state: State<AppState>,
    min_privacy: f64,
    include_tags: Option<Vec<String>>,
) -> Result<ExportPreview, String> {
    let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;

    // Get base counts from privacy threshold
    let (mut included, mut excluded, unscored) = db
        .get_export_preview(min_privacy)
        .map_err(|e| e.to_string())?;

    // If tag filtering, adjust counts
    if let Some(ref tags) = include_tags {
        if !tags.is_empty() {
            let tagged_items = db.get_items_with_any_tags(tags)
                .map_err(|e| e.to_string())?;

            // Get all items that pass privacy threshold
            let items = db.get_items().map_err(|e| e.to_string())?;
            let passing_privacy: Vec<_> = items.iter()
                .filter(|n| n.privacy.map(|p| p >= min_privacy).unwrap_or(false))
                .collect();

            // Count how many also have required tags
            let passing_both = passing_privacy.iter()
                .filter(|n| tagged_items.contains(&n.id))
                .count();

            // Items excluded = those passing privacy but not tags + those failing privacy
            excluded = passing_privacy.len() - passing_both + excluded;
            included = passing_both;
        }
    }

    Ok(ExportPreview {
        included,
        excluded,
        unscored,
    })
}

/// Result of setting node privacy - includes affected node IDs for frontend update
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPrivacyResult {
    pub affected_ids: Vec<String>,
}

/// Manually set a node's privacy status
#[tauri::command]
pub fn set_node_privacy(
    state: State<'_, AppState>,
    node_id: String,
    is_private: bool,
) -> Result<SetPrivacyResult, String> {
    let reason = if is_private {
        Some("Manually marked as private")
    } else {
        None
    };

    // Update the node itself
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.update_node_privacy(&node_id, is_private, reason)
        .map_err(|e| e.to_string())?;

    // Propagate to all descendants
    let descendant_ids = if is_private {
        let propagation_reason = "Inherited from manually marked private parent";
        state.db.read().map_err(|e| format!("DB lock error: {}", e))?.force_propagate_privacy_to_descendants(&node_id, propagation_reason)
            .map_err(|e| e.to_string())?
    } else {
        state.db.read().map_err(|e| format!("DB lock error: {}", e))?.clear_privacy_from_descendants(&node_id)
            .map_err(|e| e.to_string())?
    };

    println!("[Privacy] Marked '{}' {}, propagated to {} descendants",
             node_id,
             if is_private { "private" } else { "public" },
             descendant_ids.len());

    let mut all_ids = vec![node_id];
    all_ids.extend(descendant_ids);
    Ok(SetPrivacyResult { affected_ids: all_ids })
}

/// Analyze a single node for privacy
#[tauri::command]
pub async fn analyze_node_privacy(
    state: State<'_, AppState>,
    node_id: String,
) -> Result<PrivacyResult, String> {
    let api_key = settings::get_api_key().ok_or("ANTHROPIC_API_KEY not set")?;

    // Get node from db
    let node = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_node(&node_id)
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
        model: "claude-haiku-4-5-20251001".to_string(),
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
    state.db.read().map_err(|e| format!("DB lock error: {}", e))?.update_node_privacy(&node_id, is_private, reason.as_deref())
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
    let nodes = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_items_needing_privacy_scan()
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
            model: "claude-haiku-4-5-20251001".to_string(),
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
                        if let Err(e) = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.update_node_privacy(&node.id, is_private, reason.as_deref()) {
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
    let categories = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_category_nodes_needing_privacy_scan()
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
            model: "claude-haiku-4-5-20251001".to_string(),
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
                        if let Err(e) = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.update_node_privacy(&category.id, is_private, reason.as_deref()) {
                            eprintln!("Failed to update category {}: {}", category.id, e);
                            error_count += 1;
                            continue;
                        }

                        if is_private {
                            categories_private += 1;

                            // PROPAGATE to all descendants!
                            let propagation_reason = format!("Inherited from private category: {}", title);
                            match state.db.read().map_err(|e| format!("DB lock error: {}", e))?.propagate_privacy_to_descendants(&category.id, &propagation_reason) {
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
/// min_privacy: threshold for inclusion (0.0 = private, 1.0 = public)
/// include_tags: optional whitelist - if provided, items must have at least one matching tag
/// Nodes with privacy < min_privacy are removed
/// If include_tags provided: items must ALSO have at least one of those tags
#[tauri::command]
pub fn export_shareable_db(
    state: State<AppState>,
    min_privacy: f64,
    include_tags: Option<Vec<String>>,
) -> Result<String, String> {
    use rusqlite::Connection;

    // Get current db path
    let db_path = state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_path();

    // Generate shareable path with threshold in name
    let threshold_str = format!("{:.1}", min_privacy).replace(".", "");
    let tag_suffix = if let Some(ref tags) = include_tags {
        if !tags.is_empty() {
            format!("-tags-{}", tags.len())
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    let shareable_path = if db_path.ends_with(".db") {
        db_path.replace(".db", &format!("-shareable-{}{}.db", threshold_str, tag_suffix))
    } else {
        format!("{}-shareable-{}{}.db", db_path, threshold_str, tag_suffix)
    };

    // If tag filtering requested, get the allowed item IDs first
    let allowed_items: Option<std::collections::HashSet<String>> = match &include_tags {
        Some(tags) if !tags.is_empty() => {
            Some(state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_items_with_any_tags(tags)
                .map_err(|e| format!("Failed to get tagged items: {}", e))?)
        }
        _ => None,
    };

    // Copy the file
    fs::copy(&db_path, &shareable_path)
        .map_err(|e| format!("Failed to copy database: {}", e))?;

    // Open the copy and remove private nodes
    let conn = Connection::open(&shareable_path)
        .map_err(|e| format!("Failed to open shareable db: {}", e))?;

    // Delete nodes below privacy threshold (privacy < min_privacy OR unscored with old is_private = 1)
    let deleted_privacy = conn.execute(
        "DELETE FROM nodes WHERE (privacy IS NOT NULL AND privacy < ?1) OR (privacy IS NULL AND is_private = 1)",
        [min_privacy]
    ).map_err(|e| format!("Failed to delete private nodes: {}", e))?;

    // If tag filtering, also delete items that don't have required tags
    let deleted_tags = if let Some(ref allowed) = allowed_items {
        // Build list of allowed IDs for SQL
        if allowed.is_empty() {
            // No items have the tags - delete all items
            conn.execute(
                "DELETE FROM nodes WHERE is_item = 1",
                []
            ).map_err(|e| format!("Failed to delete non-tagged items: {}", e))?
        } else {
            // Delete items NOT in the allowed set
            let placeholders: String = allowed.iter().enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect::<Vec<_>>()
                .join(", ");
            let query = format!(
                "DELETE FROM nodes WHERE is_item = 1 AND id NOT IN ({})",
                placeholders
            );
            let params: Vec<&str> = allowed.iter().map(|s| s.as_str()).collect();
            let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter()
                .map(|s| s as &dyn rusqlite::ToSql)
                .collect();
            conn.execute(&query, param_refs.as_slice())
                .map_err(|e| format!("Failed to delete non-tagged items: {}", e))?
        }
    } else {
        0
    };

    // Delete orphaned edges
    let deleted_edges = conn.execute(
        "DELETE FROM edges WHERE source_id NOT IN (SELECT id FROM nodes) OR target_id NOT IN (SELECT id FROM nodes)",
        []
    ).map_err(|e| format!("Failed to delete orphaned edges: {}", e))?;

    // Vacuum to reclaim space
    conn.execute("VACUUM", [])
        .map_err(|e| format!("Failed to vacuum database: {}", e))?;

    let tag_info = if let Some(ref tags) = include_tags {
        format!(", tags {:?} filtered {} items", tags, deleted_tags)
    } else {
        String::new()
    };
    println!("Exported shareable database: {} (threshold {:.1}, removed {} private nodes, {} edges{})",
             shareable_path, min_privacy, deleted_privacy, deleted_edges, tag_info);

    Ok(shareable_path)
}

// ===== Privacy Scoring (continuous 0.0-1.0 scale) =====

/// Result of privacy scoring operation
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyScoringResult {
    pub items_scored: usize,
    pub batches_processed: usize,
    pub error_count: usize,
    pub cancelled: bool,
}

/// Progress event for privacy scoring
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyScoringProgress {
    pub current_batch: usize,
    pub total_batches: usize,
    pub items_scored: usize,
    pub total_items: usize,
    pub status: String, // "processing", "complete", "error", "cancelled"
}

fn emit_scoring_progress(app: &AppHandle, event: PrivacyScoringProgress) {
    let _ = app.emit("privacy-scoring-progress", event);
}

const PRIVACY_SCORING_PROMPT: &str = r#"Score each item 0.0-1.0 for public shareability:

0.0-0.2: Highly private — real names, health/mental state, finances, relationships, personal struggles, private contact info
0.3-0.4: Personal — work grievances, emotional venting, private project details, identifiable personal situations
0.5-0.6: Semi-private — named companies/projects in neutral context, work discussions, some identifiable context
0.7-0.8: Low risk — technical content with minor project context, professional discussions
0.9-1.0: Public — generic concepts, public knowledge, tutorials, no identifying context

When content spans multiple levels, use the LOWEST applicable score.

Items to score:
{items_json}

Return ONLY a JSON array:
[{"id": "...", "privacy": 0.7}, {"id": "...", "privacy": 0.3}]"#;

/// Score privacy for all unscored items using AI
#[tauri::command]
pub async fn score_privacy_all_items(
    state: State<'_, AppState>,
    app: AppHandle,
    force_rescore: bool,
) -> Result<PrivacyScoringResult, String> {
    CANCEL_PRIVACY_SCAN.store(false, Ordering::SeqCst);

    let api_key = settings::get_api_key().ok_or("ANTHROPIC_API_KEY not set")?;

    // Get items needing scoring
    let items = if force_rescore {
        // Get ALL items if force rescore
        state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_all_nodes(false)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|n| n.is_item)
            .collect::<Vec<_>>()
    } else {
        state.db.read().map_err(|e| format!("DB lock error: {}", e))?.get_items_needing_privacy_scoring()
            .map_err(|e| e.to_string())?
    };

    let total_items = items.len();
    if total_items == 0 {
        emit_scoring_progress(&app, PrivacyScoringProgress {
            current_batch: 0,
            total_batches: 0,
            items_scored: 0,
            total_items: 0,
            status: "complete".to_string(),
        });
        return Ok(PrivacyScoringResult {
            items_scored: 0,
            batches_processed: 0,
            error_count: 0,
            cancelled: false,
        });
    }

    const BATCH_SIZE: usize = 25;
    let batches: Vec<_> = items.chunks(BATCH_SIZE).collect();
    let total_batches = batches.len();

    let mut items_scored = 0;
    let mut error_count = 0;
    let client = reqwest::Client::new();

    println!("[Privacy Scoring] Starting scoring for {} items in {} batches", total_items, total_batches);

    for (batch_idx, batch) in batches.iter().enumerate() {
        // Check for cancellation
        if CANCEL_PRIVACY_SCAN.load(Ordering::SeqCst) {
            emit_scoring_progress(&app, PrivacyScoringProgress {
                current_batch: batch_idx,
                total_batches,
                items_scored,
                total_items,
                status: "cancelled".to_string(),
            });
            return Ok(PrivacyScoringResult {
                items_scored,
                batches_processed: batch_idx,
                error_count,
                cancelled: true,
            });
        }

        println!("[Privacy Scoring] Processing batch {}/{} ({} items)", batch_idx + 1, total_batches, batch.len());

        emit_scoring_progress(&app, PrivacyScoringProgress {
            current_batch: batch_idx + 1,
            total_batches,
            items_scored,
            total_items,
            status: "processing".to_string(),
        });

        // Build items JSON for prompt
        let items_for_prompt: Vec<serde_json::Value> = batch.iter().map(|item| {
            let title = item.ai_title.as_deref().unwrap_or(&item.title);
            let summary = item.summary.as_deref().unwrap_or("");
            let content = item.content.as_deref().unwrap_or("");
            // Truncate content to 500 chars
            let content_preview = if content.len() > 500 {
                let truncate_at = content.char_indices()
                    .take_while(|(i, _)| *i < 500)
                    .last()
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(0);
                format!("{}...", &content[..truncate_at])
            } else {
                content.to_string()
            };

            serde_json::json!({
                "id": item.id,
                "title": title,
                "summary": summary,
                "content_preview": content_preview
            })
        }).collect();

        let items_json = serde_json::to_string_pretty(&items_for_prompt)
            .unwrap_or_else(|_| "[]".to_string());

        let prompt = PRIVACY_SCORING_PROMPT.replace("{items_json}", &items_json);

        let request = AnthropicRequest {
            model: "claude-haiku-4-5-20251001".to_string(),
            max_tokens: 2000,
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

                        // Parse JSON array response
                        match parse_scoring_response(&text) {
                            Ok(scores) => {
                                let db = state.db.read().map_err(|e| format!("DB lock error: {}", e))?;
                                for score in scores {
                                    if let Err(e) = db.update_privacy_score(&score.id, score.privacy) {
                                        eprintln!("  Failed to update privacy for {}: {}", score.id, e);
                                        error_count += 1;
                                    } else {
                                        items_scored += 1;
                                    }
                                }
                                println!("  Batch {}: scored {} items", batch_idx + 1, batch.len());
                            }
                            Err(e) => {
                                eprintln!("  Batch {} parse error: {}", batch_idx + 1, e);
                                eprintln!("  Raw response: {}", text);
                                error_count += batch.len();
                            }
                        }
                    } else {
                        eprintln!("  Batch {} failed to parse API response", batch_idx + 1);
                        error_count += batch.len();
                    }
                } else {
                    let status = response.status();
                    eprintln!("  Batch {} API error: {}", batch_idx + 1, status);
                    error_count += batch.len();
                }
            }
            Err(e) => {
                eprintln!("  Batch {} request failed: {}", batch_idx + 1, e);
                error_count += batch.len();
            }
        }

        // Small delay to avoid rate limits
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    emit_scoring_progress(&app, PrivacyScoringProgress {
        current_batch: total_batches,
        total_batches,
        items_scored,
        total_items,
        status: "complete".to_string(),
    });

    println!("[Privacy Scoring] Complete: scored {} items, {} errors", items_scored, error_count);

    Ok(PrivacyScoringResult {
        items_scored,
        batches_processed: total_batches,
        error_count,
        cancelled: false,
    })
}

/// Parse scoring response JSON array
fn parse_scoring_response(text: &str) -> Result<Vec<PrivacyScore>, String> {
    // Try to find JSON array in response
    let text = text.trim();

    // Handle markdown code blocks
    let json_text = if text.starts_with("```") {
        text.lines()
            .skip(1)
            .take_while(|line| !line.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.to_string()
    };

    // Find array start/end
    let start = json_text.find('[').ok_or("No JSON array found")?;
    let end = json_text.rfind(']').ok_or("No JSON array end found")?;
    let array_text = &json_text[start..=end];

    // Fix common AI JSON issues: trailing commas before ]
    // e.g., {"id": "...", "privacy": 0.5},\n] -> {"id": "...", "privacy": 0.5}\n]
    let mut cleaned = array_text.to_string();
    // Remove trailing comma with various whitespace patterns
    while cleaned.contains(",]") || cleaned.contains(", ]") || cleaned.contains(",\n]") || cleaned.contains(",\r\n]") {
        cleaned = cleaned
            .replace(",\r\n]", "]")
            .replace(",\n]", "]")
            .replace(", ]", "]")
            .replace(",]", "]");
    }
    // Also handle comma followed by multiple whitespace/newlines before ]
    // Find pattern: , followed by whitespace until ]
    if let Some(last_comma) = cleaned.rfind(',') {
        let after_comma = &cleaned[last_comma + 1..];
        if after_comma.trim() == "]" {
            cleaned = format!("{}]", &cleaned[..last_comma]);
        }
    }

    serde_json::from_str::<Vec<PrivacyScore>>(&cleaned)
        .map_err(|e| format!("JSON parse error: {}", e))
}

#[derive(Deserialize)]
struct PrivacyScore {
    id: String,
    privacy: f64,
}
