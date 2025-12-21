//! Import pipeline for external data sources.
//!
//! Creates exchange nodes (human + assistant paired) for better clustering.

use crate::db::{Database, Node, NodeType, Position};
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
            content_type: None,
            associated_idea_id: None,
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
                content_type: None,
                associated_idea_id: None,
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
            content_type: None,
            associated_idea_id: None,
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
            content_type: None,
            associated_idea_id: None,
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
            content_type: None,
            associated_idea_id: None,
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
