//! Import pipeline for external data sources.
//!
//! Creates exchange nodes (human + assistant paired) for better clustering.

use crate::db::{Database, Node, NodeType, Position};
use crate::openaire::{OpenAireClient, OpenAireQuery, OpenAirePaper};
use crate::format_abstract::{format_abstract, strip_html_tags};
use serde::{Deserialize, Serialize};

/// Claude conversation export format
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ClaudeConversation {
    pub uuid: String,
    pub name: Option<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub chat_messages: Vec<ClaudeMessage>,
    pub model: Option<String>,
    pub project_uuid: Option<String>,
}

/// Claude message format
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ClaudeMessage {
    pub uuid: String,
    pub sender: String,
    pub text: String,
    pub created_at: String,
}

/// Import result summary
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub conversations_imported: usize,
    pub exchanges_imported: usize,  // Renamed: human+assistant pairs
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// Parse ISO timestamp to Unix milliseconds
fn parse_timestamp(ts: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|dt| dt.timestamp_millis())
        .unwrap_or_else(|_| {
            // Try without timezone
            chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%.fZ")
                .map(|dt| dt.and_utc().timestamp_millis())
                .unwrap_or_else(|_| chrono::Utc::now().timestamp_millis())
        })
}

/// Create title from human question (first line, truncated)
fn create_exchange_title(human_text: &str) -> String {
    let clean_text = human_text.trim();
    // Take first line or first 60 chars
    let first_line = clean_text.lines().next().unwrap_or(clean_text);
    let preview: String = first_line.chars().take(60).collect();
    let suffix = if first_line.len() > 60 { "..." } else { "" };
    format!("{}{}", preview, suffix)
}

/// Import Claude conversations from JSON string.
///
/// Creates exchange nodes: each human message paired with its assistant response.
/// This keeps related Q&A together for better clustering.
///
/// Format: "Human: {question}\n\nAssistant: {response}"
pub fn import_claude_conversations(db: &Database, json_content: &str) -> Result<ImportResult, String> {
    let conversations: Vec<ClaudeConversation> = serde_json::from_str(json_content)
        .map_err(|e| format!("Failed to parse conversations JSON: {}", e))?;

    let mut result = ImportResult {
        conversations_imported: 0,
        exchanges_imported: 0,
        skipped: 0,
        errors: Vec::new(),
    };

    // Layout conversations in a circle
    let n_convos = conversations.len();
    let radius = 300.0;

    for (i, conv) in conversations.into_iter().enumerate() {
        let conv_id = conv.uuid.clone();

        // Check if conversation already exists
        if let Ok(Some(_)) = db.get_node(&conv_id) {
            result.skipped += 1;
            continue;
        }

        // Calculate position in circle
        let angle = (2.0 * std::f64::consts::PI * i as f64) / n_convos.max(1) as f64;
        let x = radius * angle.cos();
        let y = radius * angle.sin();

        let created_at = parse_timestamp(&conv.created_at);
        let updated_at = conv.updated_at.as_ref()
            .map(|ts| parse_timestamp(ts))
            .unwrap_or(created_at);

        // Pair messages: human + assistant = one exchange
        let exchanges = pair_messages(&conv.chat_messages);
        let exchange_count = exchanges.len();

        // 1. Create conversation container node (is_item = false)
        let container = Node {
            id: conv_id.clone(),
            node_type: NodeType::Context,
            title: conv.name.clone().unwrap_or_else(|| "Untitled".to_string()),
            url: None,
            content: Some(format!("{} exchanges", exchange_count)),
            position: Position { x, y },
            created_at,
            updated_at,
            cluster_id: None,
            cluster_label: None,
            depth: 0, // Will be set by hierarchy builder
            is_item: false, // Container, not a leaf - won't be clustered
            is_universe: false,
            parent_id: None, // Will be set by hierarchy builder
            child_count: exchange_count as i32,
            ai_title: None,
            summary: None,
            tags: None,
            emoji: Some("💬".to_string()),
            is_processed: false,
            conversation_id: None, // Container doesn't belong to a conversation
            sequence_index: None,
            is_pinned: false,
            last_accessed_at: None,
            latest_child_date: None,
            is_private: None,
            privacy_reason: None,
            source: Some("claude".to_string()),
            pdf_available: None,
            content_type: None,
            associated_idea_id: None,
            privacy: None,
        };

        if let Err(e) = db.insert_node(&container) {
            result.errors.push(format!("Failed to insert conversation {}: {}", conv_id, e));
            continue;
        }

        result.conversations_imported += 1;

        // 2. Create exchange nodes (paired human + assistant)
        let exchange_radius = 80.0;

        for (index, exchange) in exchanges.into_iter().enumerate() {
            let exchange_id = format!("{}-ex-{}", conv_id, index);

            // Position exchanges around their conversation container
            let ex_angle = (2.0 * std::f64::consts::PI * index as f64) / exchange_count.max(1) as f64;
            let ex_x = x + exchange_radius * ex_angle.cos();
            let ex_y = y + exchange_radius * ex_angle.sin();

            let ex_created = exchange.created_at;

            let exchange_node = Node {
                id: exchange_id.clone(),
                node_type: NodeType::Thought,
                title: exchange.title,
                url: None,
                content: Some(exchange.content),
                position: Position { x: ex_x, y: ex_y },
                created_at: ex_created,
                updated_at: ex_created,
                cluster_id: None,
                cluster_label: None,
                depth: 0, // Will be set by hierarchy builder
                is_item: true, // This IS a leaf - will be clustered
                is_universe: false,
                parent_id: None, // Will be set by hierarchy builder (not conversation container)
                child_count: 0,
                ai_title: None,
                summary: None,
                tags: None,
                emoji: Some("💬".to_string()), // Conversation emoji for exchanges
                is_processed: false,
                conversation_id: Some(conv_id.clone()), // Links to parent conversation
                sequence_index: Some(index as i32), // Order in conversation
                is_pinned: false,
                last_accessed_at: None,
                latest_child_date: None,
                is_private: None,
                privacy_reason: None,
                source: Some("claude".to_string()),
                pdf_available: None,
                content_type: None,
                associated_idea_id: None,
                privacy: None,
            };

            if let Err(e) = db.insert_node(&exchange_node) {
                result.errors.push(format!("Failed to insert exchange {}: {}", exchange_id, e));
                continue;
            }

            result.exchanges_imported += 1;
        }
    }

    Ok(result)
}

/// Import markdown files as notes.
///
/// Each .md file becomes a note under "Recent Notes" container.
/// Title is extracted from first # heading or filename.
pub fn import_markdown_files(db: &Database, file_paths: &[String]) -> Result<ImportResult, String> {
    use std::fs;
    use std::path::Path;
    use uuid::Uuid;

    let mut result = ImportResult {
        conversations_imported: 0,
        exchanges_imported: 0,
        skipped: 0,
        errors: Vec::new(),
    };

    if file_paths.is_empty() {
        return Ok(result);
    }

    let now = chrono::Utc::now().timestamp_millis();

    // Ensure "Recent Notes" container exists
    let container_id = crate::settings::RECENT_NOTES_CONTAINER_ID;
    if db.get_node(container_id).ok().flatten().is_none() {
        let container = Node {
            id: container_id.to_string(),
            node_type: NodeType::Cluster,
            title: "Recent Notes".to_string(),
            url: None,
            content: None,
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: Some("Recent Notes".to_string()),
            depth: 1,
            is_item: false,
            is_universe: false,
            parent_id: Some("universe".to_string()),
            child_count: 0,
            ai_title: None,
            summary: None,
            tags: None,
            emoji: Some("📝".to_string()),
            is_processed: true,
            conversation_id: None,
            sequence_index: None,
            is_pinned: false,
            last_accessed_at: None,
            latest_child_date: None,
            is_private: None,
            privacy_reason: None,
            source: None,
            pdf_available: None,
            content_type: None,
            associated_idea_id: None,
            privacy: None,
        };
        if let Err(e) = db.insert_node(&container) {
            result.errors.push(format!("Failed to create Recent Notes container: {}", e));
        }
    }

    for file_path in file_paths {
        let path = Path::new(file_path);

        // Read file content
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                result.errors.push(format!("Failed to read {}: {}", file_path, e));
                result.skipped += 1;
                continue;
            }
        };

        if content.trim().is_empty() {
            result.skipped += 1;
            continue;
        }

        // Extract title: first # heading or filename
        let title = extract_markdown_title(&content)
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Untitled")
                    .to_string()
            });

        let note_id = format!("note-{}", Uuid::new_v4());

        let note = Node {
            id: note_id.clone(),
            node_type: NodeType::Thought,
            title: title.clone(),
            url: None,
            content: Some(content),
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: None,
            depth: 2,
            is_item: true,
            is_universe: false,
            parent_id: Some(container_id.to_string()),
            child_count: 0,
            ai_title: None,
            summary: None,
            tags: None,
            emoji: Some("📄".to_string()),
            is_processed: false,
            conversation_id: None,
            sequence_index: None,
            is_pinned: false,
            last_accessed_at: None,
            latest_child_date: Some(now),
            is_private: None,
            privacy_reason: None,
            source: Some("markdown".to_string()),
            pdf_available: None,
            content_type: None,
            associated_idea_id: None,
            privacy: None,
        };

        if let Err(e) = db.insert_node(&note) {
            result.errors.push(format!("Failed to import {}: {}", title, e));
            result.skipped += 1;
            continue;
        }

        result.exchanges_imported += 1; // Reusing this field for notes count
    }

    // Update container child count
    if let Ok(Some(mut container)) = db.get_node(container_id) {
        let children = db.get_children(container_id).unwrap_or_default();
        container.child_count = children.len() as i32;
        let _ = db.update_node(&container);
    }

    Ok(result)
}

/// Extract title from markdown: first # heading
fn extract_markdown_title(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("# ") {
            return Some(trimmed[2..].trim().to_string());
        }
    }
    None
}

// =============================================================================
// Google Keep Import
// =============================================================================

/// Google Keep note format from Takeout export
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GoogleKeepNote {
    title: Option<String>,
    text_content: Option<String>,
    list_content: Option<Vec<GoogleKeepListItem>>,
    labels: Option<Vec<GoogleKeepLabel>>,
    is_pinned: Option<bool>,
    is_trashed: Option<bool>,
    is_archived: Option<bool>,
    color: Option<String>,
    created_timestamp_usec: Option<i64>,
    user_edited_timestamp_usec: Option<i64>,
    attachments: Option<Vec<GoogleKeepAttachment>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GoogleKeepListItem {
    text: String,
    is_checked: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GoogleKeepLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GoogleKeepAttachment {
    file_path: String,
}

/// Import result for Google Keep
#[derive(Debug, Serialize)]
pub struct GoogleKeepImportResult {
    pub notes_imported: i32,
    pub skipped: i32,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

/// Import Google Keep notes from a Google Takeout zip file.
///
/// Parses JSON files from Takeout/Keep/ directory in the zip.
/// Creates thought nodes with is_item=true and source="googlekeep".
pub fn import_google_keep(db: &Database, zip_path: &str) -> Result<GoogleKeepImportResult, String> {
    use std::io::Read;
    use uuid::Uuid;
    use zip::ZipArchive;

    let mut result = GoogleKeepImportResult {
        notes_imported: 0,
        skipped: 0,
        warnings: Vec::new(),
        errors: Vec::new(),
    };

    // Open the zip file
    let file = std::fs::File::open(zip_path)
        .map_err(|e| format!("Failed to open zip file: {}", e))?;

    let mut archive = ZipArchive::new(file)
        .map_err(|e| format!("Failed to read zip archive: {}", e))?;

    // Collect existing notes for duplicate detection (title + created_at)
    let existing_nodes = db.get_items().unwrap_or_default();
    let existing_keys: std::collections::HashSet<(String, i64)> = existing_nodes
        .iter()
        .filter(|n| n.source.as_deref() == Some("googlekeep"))
        .map(|n| (n.title.clone(), n.created_at))
        .collect();

    let now = chrono::Utc::now().timestamp_millis();

    // Iterate through all files in the archive
    for i in 0..archive.len() {
        let mut file = match archive.by_index(i) {
            Ok(f) => f,
            Err(e) => {
                result.errors.push(format!("Failed to read archive entry {}: {}", i, e));
                continue;
            }
        };

        let file_name = file.name().to_string();

        // Only process JSON files in Takeout/Keep/
        if !file_name.contains("Keep/") || !file_name.ends_with(".json") {
            continue;
        }

        // Skip label definitions (Labels/*.json)
        if file_name.contains("/Labels/") {
            continue;
        }

        // Read the JSON content
        let mut content = String::new();
        if let Err(e) = file.read_to_string(&mut content) {
            result.errors.push(format!("Failed to read {}: {}", file_name, e));
            continue;
        }

        // Parse the note
        let note: GoogleKeepNote = match serde_json::from_str(&content) {
            Ok(n) => n,
            Err(e) => {
                result.errors.push(format!("Failed to parse {}: {}", file_name, e));
                continue;
            }
        };

        // Skip trashed notes
        if note.is_trashed.unwrap_or(false) {
            result.skipped += 1;
            continue;
        }

        // Build content from textContent and/or listContent
        let text_content = note.text_content.clone().unwrap_or_default();
        let list_content = note.list_content.as_ref().map(|items| {
            items.iter()
                .map(|item| {
                    let checkbox = if item.is_checked.unwrap_or(false) { "[x]" } else { "[ ]" };
                    format!("- {} {}", checkbox, item.text)
                })
                .collect::<Vec<_>>()
                .join("\n")
        }).unwrap_or_default();

        // Concatenate both if present
        let full_content = match (text_content.is_empty(), list_content.is_empty()) {
            (true, true) => {
                result.skipped += 1;
                result.warnings.push(format!("Skipped empty note: {}", file_name));
                continue;
            }
            (false, true) => text_content,
            (true, false) => list_content,
            (false, false) => format!("{}\n\n{}", text_content, list_content),
        };

        // Extract title (use "Untitled Note" if empty)
        let title = note.title.clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "Untitled Note".to_string());

        // Convert timestamps (microseconds to milliseconds)
        let created_at = note.created_timestamp_usec
            .map(|usec| usec / 1000)
            .unwrap_or(now);
        let updated_at = note.user_edited_timestamp_usec
            .map(|usec| usec / 1000)
            .unwrap_or(created_at);

        // Duplicate check
        if existing_keys.contains(&(title.clone(), created_at)) {
            result.skipped += 1;
            continue;
        }

        // Warn about attachments
        if let Some(attachments) = &note.attachments {
            if !attachments.is_empty() {
                result.warnings.push(format!(
                    "Note '{}' has {} attachment(s) - text imported only",
                    title, attachments.len()
                ));
            }
        }

        // Extract tags from labels
        let tags = note.labels.as_ref().map(|labels| {
            let tag_list: Vec<String> = labels.iter().map(|l| l.name.clone()).collect();
            serde_json::to_string(&tag_list).unwrap_or_else(|_| "[]".to_string())
        });

        // Create the node
        let note_id = format!("keep-{}", Uuid::new_v4());

        let node = Node {
            id: note_id.clone(),
            node_type: NodeType::Thought,
            title,
            url: None,
            content: Some(full_content),
            position: Position { x: 0.0, y: 0.0 },
            created_at,
            updated_at,
            cluster_id: None,
            cluster_label: None,
            depth: 0, // Will be set by hierarchy builder
            is_item: true, // This is a leaf - will be clustered
            is_universe: false,
            parent_id: None, // Will be set by hierarchy builder
            child_count: 0,
            ai_title: None,
            summary: None,
            tags,
            emoji: Some("📝".to_string()), // Note emoji for Keep notes
            is_processed: false,
            conversation_id: None,
            sequence_index: None,
            is_pinned: note.is_pinned.unwrap_or(false),
            last_accessed_at: None,
            latest_child_date: Some(created_at),
            is_private: None,
            privacy_reason: None,
            source: Some("googlekeep".to_string()),
            pdf_available: None,
            content_type: None,
            associated_idea_id: None,
            privacy: None,
        };

        if let Err(e) = db.insert_node(&node) {
            result.errors.push(format!("Failed to insert note: {}", e));
            continue;
        }

        result.notes_imported += 1;
    }

    eprintln!(
        "[Google Keep Import] Imported {} notes, skipped {}, {} warnings, {} errors",
        result.notes_imported, result.skipped, result.warnings.len(), result.errors.len()
    );

    Ok(result)
}

// =============================================================================
// Claude Import Helpers
// =============================================================================

/// Paired exchange: human question + assistant response
struct Exchange {
    title: String,
    content: String,
    created_at: i64,
}

/// Pair consecutive human + assistant messages into exchanges
fn pair_messages(messages: &[ClaudeMessage]) -> Vec<Exchange> {
    let mut exchanges = Vec::new();
    let mut i = 0;

    while i < messages.len() {
        let msg = &messages[i];

        if msg.sender == "human" {
            let human_text = msg.text.clone();
            let human_time = parse_timestamp(&msg.created_at);

            // Look for following assistant response
            if i + 1 < messages.len() && messages[i + 1].sender == "assistant" {
                i += 1; // Consume the assistant message
                let assistant_text = messages[i].text.clone();

                exchanges.push(Exchange {
                    title: create_exchange_title(&human_text),
                    content: format!("Human: {}\n\nAssistant: {}", human_text, assistant_text),
                    created_at: human_time,
                });
            }
            // Skip human messages without responses - they're incomplete
        } else {
            // Orphan assistant message (no preceding human) - include it solo
            exchanges.push(Exchange {
                title: create_exchange_title(&msg.text),
                content: format!("Assistant: {}", msg.text),
                created_at: parse_timestamp(&msg.created_at),
            });
        }

        i += 1;
    }

    exchanges
}

// =============================================================================
// ChatGPT Import
// =============================================================================

use std::collections::HashMap;

/// ChatGPT conversation export format
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ChatGPTConversation {
    pub id: Option<String>,
    pub conversation_id: Option<String>,
    pub title: Option<String>,
    pub create_time: Option<f64>,
    pub update_time: Option<f64>,
    pub mapping: HashMap<String, ChatGPTNode>,
    pub current_node: Option<String>,
    pub is_archived: Option<bool>,
    pub gizmo_id: Option<String>,
    pub default_model_slug: Option<String>,
}

/// ChatGPT tree node in mapping
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ChatGPTNode {
    pub id: String,
    pub message: Option<ChatGPTMessage>,
    pub parent: Option<String>,
    #[serde(default)]
    pub children: Vec<String>,
}

/// ChatGPT message format
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ChatGPTMessage {
    pub id: String,
    pub author: ChatGPTAuthor,
    pub create_time: Option<f64>,
    pub content: ChatGPTContent,
}

/// ChatGPT author info
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ChatGPTAuthor {
    pub role: String,
    pub name: Option<String>,
}

/// ChatGPT content (various types)
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ChatGPTContent {
    pub content_type: Option<String>,
    pub parts: Option<Vec<serde_json::Value>>,
    // Code content
    pub language: Option<String>,
    pub text: Option<String>,
    // Quote content
    pub url: Option<String>,
    pub title: Option<String>,
    // Reasoning recap
    pub content: Option<String>,
}

/// Import ChatGPT conversations from JSON string.
///
/// Creates exchange nodes: each user message paired with its assistant response.
/// Handles tree structure by linearizing to message chain.
///
/// Format: "Human: {question}\n\nAssistant: {response}"
pub fn import_chatgpt_conversations(db: &Database, json_content: &str) -> Result<ImportResult, String> {
    let conversations: Vec<ChatGPTConversation> = serde_json::from_str(json_content)
        .map_err(|e| format!("Failed to parse ChatGPT JSON: {}", e))?;

    let mut result = ImportResult {
        conversations_imported: 0,
        exchanges_imported: 0,
        skipped: 0,
        errors: Vec::new(),
    };

    let n_convos = conversations.len();
    let radius = 300.0;

    for (i, conv) in conversations.into_iter().enumerate() {
        // Get conversation ID
        let conv_id = conv.conversation_id
            .or(conv.id)
            .unwrap_or_else(|| format!("chatgpt-{}", uuid::Uuid::new_v4()));

        // Check if conversation already exists
        if let Ok(Some(_)) = db.get_node(&conv_id) {
            result.skipped += 1;
            continue;
        }

        // Linearize tree structure to message list
        let messages = linearize_chatgpt_tree(&conv.mapping, conv.current_node.as_deref());

        // Pair messages into exchanges
        let exchanges = pair_chatgpt_messages(&messages);

        if exchanges.is_empty() {
            result.skipped += 1;
            continue;
        }

        // Calculate position in circle
        let angle = (2.0 * std::f64::consts::PI * i as f64) / n_convos.max(1) as f64;
        let x = radius * angle.cos();
        let y = radius * angle.sin();

        let created_at = conv.create_time
            .map(|t| (t * 1000.0) as i64)
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
        let updated_at = conv.update_time
            .map(|t| (t * 1000.0) as i64)
            .unwrap_or(created_at);

        let exchange_count = exchanges.len();

        // 1. Create conversation container node
        let container = Node {
            id: conv_id.clone(),
            node_type: NodeType::Context,
            title: conv.title.clone().unwrap_or_else(|| "Untitled".to_string()),
            url: None,
            content: Some(format!("{} exchanges", exchange_count)),
            position: Position { x, y },
            created_at,
            updated_at,
            cluster_id: None,
            cluster_label: None,
            depth: 0,
            is_item: false,
            is_universe: false,
            parent_id: None,
            child_count: exchange_count as i32,
            ai_title: None,
            summary: None,
            tags: None,
            emoji: Some("💬".to_string()),
            is_processed: false,
            conversation_id: None,
            sequence_index: None,
            is_pinned: false,
            last_accessed_at: None,
            latest_child_date: None,
            is_private: None,
            privacy_reason: None,
            source: Some("chatgpt".to_string()),
            pdf_available: None,
            content_type: None,
            associated_idea_id: None,
            privacy: None,
        };

        if let Err(e) = db.insert_node(&container) {
            result.errors.push(format!("Failed to insert conversation {}: {}", conv_id, e));
            continue;
        }

        result.conversations_imported += 1;

        // 2. Create exchange nodes
        let exchange_radius = 80.0;

        for (index, exchange) in exchanges.into_iter().enumerate() {
            let exchange_id = format!("{}-ex-{}", conv_id, index);

            let ex_angle = (2.0 * std::f64::consts::PI * index as f64) / exchange_count.max(1) as f64;
            let ex_x = x + exchange_radius * ex_angle.cos();
            let ex_y = y + exchange_radius * ex_angle.sin();

            let exchange_node = Node {
                id: exchange_id.clone(),
                node_type: NodeType::Thought,
                title: exchange.title,
                url: None,
                content: Some(exchange.content),
                position: Position { x: ex_x, y: ex_y },
                created_at: exchange.created_at,
                updated_at: exchange.created_at,
                cluster_id: None,
                cluster_label: None,
                depth: 0,
                is_item: true,
                is_universe: false,
                parent_id: None,
                child_count: 0,
                ai_title: None,
                summary: None,
                tags: None,
                emoji: Some("💬".to_string()),
                is_processed: false,
                conversation_id: Some(conv_id.clone()),
                sequence_index: Some(index as i32),
                is_pinned: false,
                last_accessed_at: None,
                latest_child_date: None,
                is_private: None,
                privacy_reason: None,
                source: Some("chatgpt".to_string()),
                pdf_available: None,
                content_type: None,
                associated_idea_id: None,
                privacy: None,
            };

            if let Err(e) = db.insert_node(&exchange_node) {
                result.errors.push(format!("Failed to insert exchange {}: {}", exchange_id, e));
                continue;
            }

            result.exchanges_imported += 1;
        }
    }

    Ok(result)
}

/// Linearized message from ChatGPT tree
struct ChatGPTLinearMessage {
    role: String,
    text: String,
    create_time: Option<f64>,
}

/// Linearize ChatGPT tree structure to ordered message list
fn linearize_chatgpt_tree(mapping: &HashMap<String, ChatGPTNode>, current_node: Option<&str>) -> Vec<ChatGPTLinearMessage> {
    // Find root node (parent = None)
    let root_id = mapping.iter()
        .find(|(_, node)| node.parent.is_none())
        .map(|(id, _)| id.clone());

    let Some(root_id) = root_id else {
        return Vec::new();
    };

    let mut messages = Vec::new();
    let mut current = Some(root_id);
    let mut visited = std::collections::HashSet::new();

    while let Some(ref node_id) = current {
        if visited.contains(node_id) {
            break;
        }
        visited.insert(node_id.clone());

        if let Some(node) = mapping.get(node_id) {
            // Extract message if present
            if let Some(ref msg) = node.message {
                if let Some(text) = extract_chatgpt_content(&msg.content) {
                    let role = msg.author.role.clone();

                    // Skip system and tool messages for cleaner output
                    if role == "user" || role == "assistant" {
                        messages.push(ChatGPTLinearMessage {
                            role,
                            text,
                            create_time: msg.create_time,
                        });
                    }
                }
            }

            // Move to next node
            // Prefer current_node path if we're at a branch point
            if node.children.is_empty() {
                current = None;
            } else if node.children.len() == 1 {
                current = Some(node.children[0].clone());
            } else {
                // Multiple children - try to follow current_node hint
                if let Some(target) = current_node {
                    // Check if any child leads to current_node
                    let mut found = false;
                    for child_id in &node.children {
                        if child_id == target || is_ancestor_of(mapping, child_id, target) {
                            current = Some(child_id.clone());
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        current = Some(node.children[0].clone());
                    }
                } else {
                    current = Some(node.children[0].clone());
                }
            }
        } else {
            current = None;
        }
    }

    messages
}

/// Check if node_id is an ancestor of target_id in the tree
fn is_ancestor_of(mapping: &HashMap<String, ChatGPTNode>, node_id: &str, target_id: &str) -> bool {
    let mut current = Some(target_id.to_string());
    let mut depth = 0;

    while let Some(ref id) = current {
        if depth > 1000 {
            return false; // Prevent infinite loops
        }
        if id == node_id {
            return true;
        }
        current = mapping.get(id).and_then(|n| n.parent.clone());
        depth += 1;
    }
    false
}

/// Extract text content from ChatGPT content object
fn extract_chatgpt_content(content: &ChatGPTContent) -> Option<String> {
    let content_type = content.content_type.as_deref().unwrap_or("text");

    match content_type {
        "text" => {
            // Join text parts
            content.parts.as_ref().map(|parts| {
                parts.iter()
                    .filter_map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            }).filter(|s| !s.trim().is_empty())
        }
        "code" => {
            // Format as code block
            content.text.as_ref().map(|text| {
                let lang = content.language.as_deref().unwrap_or("");
                format!("```{}\n{}\n```", lang, text)
            })
        }
        "multimodal_text" => {
            // Extract text parts, note images
            content.parts.as_ref().map(|parts| {
                let mut texts = Vec::new();
                let mut image_count = 0;

                for part in parts {
                    if let Some(text) = part.as_str() {
                        if !text.trim().is_empty() {
                            texts.push(text.to_string());
                        }
                    } else if part.is_object() {
                        // Image or other attachment
                        image_count += 1;
                    }
                }

                if image_count > 0 {
                    texts.push(format!("[{} image(s) attached]", image_count));
                }

                texts.join("\n")
            }).filter(|s| !s.trim().is_empty())
        }
        "tether_quote" => {
            // Web quote - format as blockquote
            content.text.as_ref().map(|text| {
                let url = content.url.as_deref().unwrap_or("source");
                format!("> {}\n> — {}", text, url)
            })
        }
        "execution_output" => {
            // Code execution output
            content.text.as_ref().map(|text| {
                format!("Output:\n```\n{}\n```", text)
            })
        }
        "reasoning_recap" => {
            // o1 reasoning recap
            content.content.clone()
        }
        _ => None, // Skip thoughts, system_error, etc.
    }
}

/// Pair ChatGPT messages into exchanges (user + assistant)
fn pair_chatgpt_messages(messages: &[ChatGPTLinearMessage]) -> Vec<Exchange> {
    let mut exchanges = Vec::new();
    let mut i = 0;

    while i < messages.len() {
        let msg = &messages[i];

        if msg.role == "user" {
            let human_text = msg.text.clone();
            let human_time = msg.create_time
                .map(|t| (t * 1000.0) as i64)
                .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

            // Collect all following assistant messages (may have multiple parts)
            let mut assistant_parts = Vec::new();
            while i + 1 < messages.len() && messages[i + 1].role == "assistant" {
                i += 1;
                assistant_parts.push(messages[i].text.clone());
            }

            if !assistant_parts.is_empty() {
                let assistant_text = assistant_parts.join("\n\n");
                exchanges.push(Exchange {
                    title: create_exchange_title(&human_text),
                    content: format!("Human: {}\n\nAssistant: {}", human_text, assistant_text),
                    created_at: human_time,
                });
            }
        } else if msg.role == "assistant" {
            // Orphan assistant message
            let time = msg.create_time
                .map(|t| (t * 1000.0) as i64)
                .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

            exchanges.push(Exchange {
                title: create_exchange_title(&msg.text),
                content: format!("Assistant: {}", msg.text),
                created_at: time,
            });
        }

        i += 1;
    }

    exchanges
}

// ==================== OpenAIRE Paper Import ====================

/// OpenAIRE import result summary
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAireImportResult {
    pub papers_imported: usize,
    pub pdfs_downloaded: usize,
    pub pdfs_skipped: usize,
    pub duplicates_skipped: usize,
    pub doi_duplicates_skipped: usize,
    pub garbage_skipped: usize,
    pub hash_duplicates_skipped: usize,
    pub errors: Vec<String>,
}

/// Compute content hash for deduplication (SHA-256 of normalized title + abstract)
/// Uses SHA-256 for stability across Rust versions
fn compute_content_hash(title: &str, abstract_text: Option<&str>) -> String {
    use sha2::{Sha256, Digest};

    // Normalize: lowercase, collapse whitespace, trim
    let normalized_title = title.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let normalized_abstract = abstract_text
        .map(|a| a.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();

    let mut hasher = Sha256::new();
    hasher.update(normalized_title.as_bytes());
    hasher.update(b"|"); // Separator to prevent collisions
    hasher.update(normalized_abstract.as_bytes());
    let result = hasher.finalize();
    // Use first 16 bytes (32 hex chars) for reasonable uniqueness
    format!("{:032x}", u128::from_be_bytes(result[..16].try_into().unwrap()))
}

/// Check if content is garbage (purely numeric, empty, or too short)
fn is_garbage_content(title: &str, abstract_text: Option<&str>) -> bool {
    // Empty or very short title is suspicious
    let title_clean = title.trim();
    if title_clean.is_empty() || title_clean.len() < 5 {
        return true;
    }

    // Purely numeric title (e.g., "12345")
    if title_clean.chars().all(|c| c.is_ascii_digit() || c.is_whitespace() || c == '.' || c == '-') {
        return true;
    }

    // Abstract check: if present, must be meaningful
    if let Some(abs) = abstract_text {
        let abs_clean = abs.trim();
        // Too short to be a real abstract
        if !abs_clean.is_empty() && abs_clean.len() < 50 {
            return true;
        }
        // Purely numeric abstract
        if abs_clean.chars().all(|c| c.is_ascii_digit() || c.is_whitespace() || c == '.' || c == '-') {
            return true;
        }
    }

    false
}

/// Import scientific papers from OpenAIRE
///
/// Fetches papers matching the query, creates nodes with paper metadata,
/// and optionally downloads PDFs using multi-source resolver.
pub async fn import_openaire_papers(
    db: &Database,
    query: String,
    country: Option<String>,
    fos: Option<String>,
    from_year: Option<String>,
    to_year: Option<String>,
    max_papers: u32,
    download_pdfs: bool,
    max_pdf_size_mb: u32,
    api_key: Option<String>,
    unpaywall_email: Option<String>,
    core_api_key: Option<String>,
    on_progress: impl Fn(usize, usize),
) -> Result<OpenAireImportResult, String> {
    let client = OpenAireClient::new_with_key(api_key);

    let mut result = OpenAireImportResult {
        papers_imported: 0,
        pdfs_downloaded: 0,
        pdfs_skipped: 0,
        duplicates_skipped: 0,
        doi_duplicates_skipped: 0,
        garbage_skipped: 0,
        hash_duplicates_skipped: 0,
        errors: Vec::new(),
    };

    // Pre-load all existing IDs for O(1) duplicate checking
    let existing_ids = db.get_all_openaire_ids().unwrap_or_default();
    let existing_dois = db.get_all_paper_dois().unwrap_or_default();
    let existing_hashes = db.get_all_content_hashes().unwrap_or_default();
    println!("[OpenAIRE] Pre-loaded {} IDs, {} DOIs, {} hashes for duplicate check",
        existing_ids.len(), existing_dois.len(), existing_hashes.len());

    let page_size = 100u32.min(max_papers);
    let mut current_page = 1u32;
    let mut total_available: Option<u32> = None;

    // Fetch papers in pages until we have enough NEW papers
    loop {
        if result.papers_imported >= max_papers as usize {
            break;
        }

        let query_obj = OpenAireQuery {
            search: query.clone(),
            country: country.clone(),
            fos: fos.clone(),
            from_year: from_year.clone(),
            to_year: to_year.clone(),
            access_right: Some("OPEN".to_string()),
            page_size,
            page: current_page,
            sort_by: None,
        };

        let (papers, total_count) = client.fetch_papers(&query_obj).await?;

        // Store total count on first fetch
        if total_available.is_none() {
            total_available = Some(total_count);
        }

        println!("[OpenAIRE] Page {}: fetched {} papers (total available: {})",
            current_page, papers.len(), total_count);

        if papers.is_empty() {
            println!("[OpenAIRE] No more papers on page {}, stopping", current_page);
            break;
        }

        for paper in papers {
            // Stop when we've imported enough NEW papers
            if result.papers_imported >= max_papers as usize {
                break;
            }

            // 1. OpenAIRE ID dedup (existing check)
            if existing_ids.contains(&paper.id) {
                result.duplicates_skipped += 1;
                continue;
            }

            // 2. DOI dedup - papers with same DOI are definitely duplicates
            if let Some(ref doi) = paper.doi {
                let doi_lower = doi.to_lowercase();
                if existing_dois.contains(&doi_lower) {
                    result.doi_duplicates_skipped += 1;
                    continue;
                }
            }

            // 3. Content sanity check - skip garbage papers
            if is_garbage_content(&paper.title, paper.description.as_deref()) {
                result.garbage_skipped += 1;
                continue;
            }

            // 4. Content hash dedup - papers with same title+abstract are duplicates
            let content_hash = compute_content_hash(&paper.title, paper.description.as_deref());
            if existing_hashes.contains(&content_hash) {
                result.hash_duplicates_skipped += 1;
                continue;
            }

            // Import the paper
            match import_single_paper(
                db,
                &paper,
                download_pdfs,
                max_pdf_size_mb,
                &client,
                &content_hash,
                unpaywall_email.as_deref(),
                core_api_key.as_deref(),
            ).await {
                Ok(pdf_downloaded) => {
                    result.papers_imported += 1;
                    if pdf_downloaded {
                        result.pdfs_downloaded += 1;
                    } else if download_pdfs && !paper.pdf_urls.is_empty() {
                        result.pdfs_skipped += 1;
                    }
                    on_progress(result.papers_imported, max_papers as usize);
                }
                Err(e) => {
                    result.errors.push(format!("Failed to import '{}': {}", paper.title, e));
                }
            }

            // Rate limiting between papers
            client.rate_limit_delay().await;
        }

        current_page += 1;

        // Stop if we've exhausted all available papers
        let papers_fetched_so_far = (current_page - 1) * page_size;
        if papers_fetched_so_far >= total_available.unwrap_or(0) {
            println!("[OpenAIRE] Reached end of results (page {} * {} >= {}). Imported: {}, Duplicates: {}",
                current_page - 1, page_size, total_available.unwrap_or(0),
                result.papers_imported, result.duplicates_skipped);
            break;
        }
    }

    println!(
        "[OpenAIRE] Import complete: {} papers, {} PDFs, {} pdf_skipped, {} id_dups, {} doi_dups, {} garbage, {} hash_dups, {} errors",
        result.papers_imported,
        result.pdfs_downloaded,
        result.pdfs_skipped,
        result.duplicates_skipped,
        result.doi_duplicates_skipped,
        result.garbage_skipped,
        result.hash_duplicates_skipped,
        result.errors.len()
    );

    Ok(result)
}

/// Import a single paper into the database
async fn import_single_paper(
    db: &Database,
    paper: &OpenAirePaper,
    download_pdfs: bool,
    max_pdf_size_mb: u32,
    client: &OpenAireClient,
    content_hash: &str,
    unpaywall_email: Option<&str>,
    core_api_key: Option<&str>,
) -> Result<bool, String> {
    let node_id = format!("paper-{}", uuid::Uuid::new_v4());

    // Use abstract as content (for embeddings)
    let content = paper.description.clone().unwrap_or_default();

    // Serialize authors to JSON
    let authors_json = serde_json::to_string(&paper.authors).ok();

    // Serialize subjects to JSON
    let subjects_json = serde_json::to_string(&paper.subjects).ok();

    // Get first PDF URL
    let pdf_url = paper.pdf_urls.first().cloned();

    // Parse publication date to timestamp (use 0 for unknown dates, not import time)
    let created_at = paper.publication_date
        .as_ref()
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| dt.and_utc().timestamp_millis())
        .unwrap_or(0);  // 0 = unknown date (epoch)

    let now = chrono::Utc::now().timestamp_millis();

    // Create the node
    let node = Node {
        id: node_id.clone(),
        node_type: NodeType::Paper,
        title: paper.title.clone(),
        url: paper.doi.as_ref().map(|doi| format!("https://doi.org/{}", doi)),
        content: Some(content),
        position: Position { x: 0.0, y: 0.0 },
        created_at,
        updated_at: now,
        cluster_id: None,
        cluster_label: None,
        depth: 0,
        is_item: true,
        is_universe: false,
        parent_id: None,
        child_count: 0,
        ai_title: Some(paper.title.clone()),
        summary: paper.description.clone(),
        tags: None,
        emoji: Some("📄".to_string()),
        is_processed: false,
        conversation_id: None,
        sequence_index: None,
        is_pinned: false,
        last_accessed_at: None,
        latest_child_date: None,
        is_private: None,
        privacy_reason: None,
        source: Some("openaire".to_string()),  // Updated to "openaire-pdf" after successful download
        pdf_available: None,  // Updated after PDF download attempt
        content_type: Some("paper".to_string()),
        associated_idea_id: None,
        privacy: None,
    };

    db.insert_node(&node).map_err(|e| e.to_string())?;

    // Strip HTML from abstract and format with section detection
    let raw_abstract = paper.description.as_deref().unwrap_or("");
    let clean_abstract = strip_html_tags(raw_abstract);
    let formatted = format_abstract(&clean_abstract);
    let abstract_sections_json = if formatted.sections.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&formatted.sections).unwrap_or_default())
    };
    let abstract_formatted = if formatted.had_structure {
        Some(formatted.markdown.as_str())
    } else {
        None
    };

    // Insert paper metadata (store clean abstract without HTML tags)
    db.insert_paper(
        &node_id,
        Some(&paper.id),
        paper.doi.as_deref(),
        authors_json.as_deref(),
        paper.publication_date.as_deref(),
        paper.journal.as_deref(),
        paper.publisher.as_deref(),
        Some(&clean_abstract),
        abstract_formatted,
        abstract_sections_json.as_deref(),
        pdf_url.as_deref(),
        subjects_json.as_deref(),
        Some(&paper.access_right),
        Some(content_hash),
    ).map_err(|e| e.to_string())?;

    // Download document (PDF/DOCX/DOC) if requested - using multi-source resolver
    let mut pdf_downloaded = false;
    let mut pdf_source = None;

    if download_pdfs {
        // Use the PDF resolver with priority-based fallback chain
        use crate::papers::resolver::PdfResolver;

        let mut resolver = PdfResolver::new(
            unpaywall_email.map(|s| s.to_string()),
            core_api_key.map(|s| s.to_string()),
        );

        // Extract identifiers for resolver (arXiv ID, PMCID from paper metadata)
        let mut identifiers = Vec::new();
        if let Some(ref doi) = paper.doi {
            identifiers.push(doi.clone());
        }
        // Add PDF URLs so resolver can extract arXiv IDs and PMCIDs from them
        for url in &paper.pdf_urls {
            identifiers.push(url.clone());
        }

        match resolver.resolve(
            paper.doi.as_deref(),
            &identifiers,
            &paper.pdf_urls,
        ).await {
            Ok(resolved_pdf) => {
                // Validate size
                if resolved_pdf.bytes.len() <= (max_pdf_size_mb as usize * 1024 * 1024) {
                    if let Err(e) = db.update_paper_document(&node_id, &resolved_pdf.bytes, "pdf") {
                        eprintln!("[PDF Resolver] Failed to store PDF: {}", e);
                    } else {
                        pdf_downloaded = true;
                        pdf_source = Some(resolved_pdf.source.clone());

                        // Update node source for graph badge display
                        let source = format!("{}-pdf", resolved_pdf.source);
                        if let Err(e) = db.update_node_source(&node_id, &source) {
                            eprintln!("[PDF Resolver] Failed to update node source: {}", e);
                        }

                        println!("[PDF Resolver] ✓ Downloaded from {}: {}", resolved_pdf.source, paper.title);
                    }
                } else {
                    eprintln!("[PDF Resolver] PDF from {} too large: {} MB (max: {} MB)",
                        resolved_pdf.source,
                        resolved_pdf.bytes.len() / 1024 / 1024,
                        max_pdf_size_mb);
                }
            }
            Err(e) => {
                // Resolver failed - try fallback to OpenAIRE URL
                if let Some(url) = &pdf_url {
                    match client.download_document(url, max_pdf_size_mb).await {
                        Ok(Some((doc_bytes, format))) => {
                            if let Err(e) = db.update_paper_document(&node_id, &doc_bytes, &format) {
                                eprintln!("[OpenAIRE] Failed to store document: {}", e);
                            } else {
                                pdf_downloaded = true;
                                pdf_source = Some("openaire".to_string());
                                let source = format!("openaire-{}", format);
                                if let Err(e) = db.update_node_source(&node_id, &source) {
                                    eprintln!("[OpenAIRE] Failed to update node source: {}", e);
                                }
                            }
                        }
                        Ok(None) => {
                            eprintln!("[PDF Resolver] All sources failed for: {}", paper.title);
                        }
                        Err(e) => {
                            eprintln!("[PDF Resolver] All sources failed (including OpenAIRE): {}", e);
                        }
                    }
                }
            }
        }

        // Store pdf_source in database if we got a PDF
        if let Some(source) = pdf_source {
            if let Err(e) = db.update_paper_pdf_source(&node_id, &source) {
                eprintln!("[PDF Resolver] Failed to update pdf_source: {}", e);
            }
        }
    }

    Ok(pdf_downloaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_exchange_title() {
        assert_eq!(
            create_exchange_title("Hello world"),
            "Hello world"
        );

        // 70 chars - gets truncated to 60 + "..."
        let long_msg = "This is a very long message that should be truncated after sixty chars";
        let result = create_exchange_title(long_msg);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 63); // 60 chars + "..."

        // Multi-line: uses first line
        assert_eq!(
            create_exchange_title("First line\nSecond line"),
            "First line"
        );
    }

    #[test]
    fn test_parse_timestamp() {
        let ts = "2024-01-01T00:00:00.000Z";
        let result = parse_timestamp(ts);
        assert!(result > 0);
    }

    #[test]
    fn test_pair_messages() {
        let messages = vec![
            ClaudeMessage {
                uuid: "1".to_string(),
                sender: "human".to_string(),
                text: "Hello".to_string(),
                created_at: "2024-01-01T00:00:00.000Z".to_string(),
            },
            ClaudeMessage {
                uuid: "2".to_string(),
                sender: "assistant".to_string(),
                text: "Hi there!".to_string(),
                created_at: "2024-01-01T00:00:01.000Z".to_string(),
            },
            ClaudeMessage {
                uuid: "3".to_string(),
                sender: "human".to_string(),
                text: "How are you?".to_string(),
                created_at: "2024-01-01T00:00:02.000Z".to_string(),
            },
            ClaudeMessage {
                uuid: "4".to_string(),
                sender: "assistant".to_string(),
                text: "I'm doing well!".to_string(),
                created_at: "2024-01-01T00:00:03.000Z".to_string(),
            },
        ];

        let exchanges = pair_messages(&messages);
        assert_eq!(exchanges.len(), 2);
        assert_eq!(exchanges[0].title, "Hello");
        assert!(exchanges[0].content.contains("Human: Hello"));
        assert!(exchanges[0].content.contains("Assistant: Hi there!"));
        assert_eq!(exchanges[1].title, "How are you?");
    }
}
