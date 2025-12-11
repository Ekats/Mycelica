//! Import pipeline for external data sources.
//!
//! Creates individual message nodes with conversation context tracking.

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
    pub messages_imported: usize,
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

/// Truncate message to create a title
fn truncate_message_title(text: &str, sender: &str) -> String {
    let prefix = if sender == "human" { "Human" } else { "Claude" };
    let clean_text = text.trim();
    let preview: String = clean_text.chars().take(50).collect();
    let suffix = if clean_text.len() > 50 { "..." } else { "" };
    format!("{}: {}{}", prefix, preview, suffix)
}

/// Import Claude conversations from JSON string.
///
/// Creates:
/// 1. One container node per conversation (is_item = false)
/// 2. Individual message nodes for each message (is_item = true)
///
/// Messages get conversation_id and sequence_index for context reconstruction.
pub fn import_claude_conversations(db: &Database, json_content: &str) -> Result<ImportResult, String> {
    let conversations: Vec<ClaudeConversation> = serde_json::from_str(json_content)
        .map_err(|e| format!("Failed to parse conversations JSON: {}", e))?;

    let _now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let mut result = ImportResult {
        conversations_imported: 0,
        messages_imported: 0,
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

        let message_count = conv.chat_messages.len();

        // 1. Create conversation container node (is_item = false)
        let container = Node {
            id: conv_id.clone(),
            node_type: NodeType::Context,
            title: conv.name.clone().unwrap_or_else(|| "Untitled".to_string()),
            url: None,
            content: Some(format!("{} messages", message_count)),
            position: Position { x, y },
            created_at,
            updated_at,
            cluster_id: None,
            cluster_label: None,
            depth: 0, // Will be set by hierarchy builder
            is_item: false, // Container, not a leaf - won't be clustered
            is_universe: false,
            parent_id: None, // Will be set by hierarchy builder
            child_count: message_count as i32,
            ai_title: None,
            summary: None,
            tags: None,
            emoji: Some("💬".to_string()),
            is_processed: false,
            conversation_id: None, // Container doesn't belong to a conversation
            sequence_index: None,
            is_pinned: false,
            last_accessed_at: None,
        };

        if let Err(e) = db.insert_node(&container) {
            result.errors.push(format!("Failed to insert conversation {}: {}", conv_id, e));
            continue;
        }

        result.conversations_imported += 1;

        // 2. Create individual message nodes
        let msg_radius = 80.0;

        for (index, msg) in conv.chat_messages.into_iter().enumerate() {
            let msg_id = format!("{}-msg-{}", conv_id, index);

            // Position messages around their conversation container
            let msg_angle = (2.0 * std::f64::consts::PI * index as f64) / message_count.max(1) as f64;
            let msg_x = x + msg_radius * msg_angle.cos();
            let msg_y = y + msg_radius * msg_angle.sin();

            let msg_created = parse_timestamp(&msg.created_at);

            let msg_node = Node {
                id: msg_id.clone(),
                node_type: NodeType::Thought,
                title: truncate_message_title(&msg.text, &msg.sender),
                url: None,
                content: Some(msg.text),
                position: Position { x: msg_x, y: msg_y },
                created_at: msg_created,
                updated_at: msg_created,
                cluster_id: None,
                cluster_label: None,
                depth: 0, // Will be set by hierarchy builder
                is_item: true, // This IS a leaf - will be clustered
                is_universe: false,
                parent_id: Some(conv_id.clone()), // Structural parent = conversation container
                child_count: 0,
                ai_title: None,
                summary: None,
                tags: None,
                emoji: if msg.sender == "human" { Some("👤".to_string()) } else { Some("🤖".to_string()) },
                is_processed: false,
                conversation_id: Some(conv_id.clone()), // Links to parent conversation
                sequence_index: Some(index as i32), // Order in conversation
                is_pinned: false,
                last_accessed_at: None,
            };

            if let Err(e) = db.insert_node(&msg_node) {
                result.errors.push(format!("Failed to insert message {}: {}", msg_id, e));
                continue;
            }

            result.messages_imported += 1;
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_message_title() {
        assert_eq!(
            truncate_message_title("Hello world", "human"),
            "Human: Hello world"
        );

        assert_eq!(
            truncate_message_title("This is a very long message that should be truncated after fifty characters", "assistant"),
            "Claude: This is a very long message that should be truncat..."
        );
    }

    #[test]
    fn test_parse_timestamp() {
        let ts = "2024-01-01T00:00:00.000Z";
        let result = parse_timestamp(ts);
        assert!(result > 0);
    }
}
