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
            let assistant_text = if i + 1 < messages.len() && messages[i + 1].sender == "assistant" {
                i += 1; // Consume the assistant message
                messages[i].text.clone()
            } else {
                // Human message without response (rare)
                String::from("*No response*")
            };

            exchanges.push(Exchange {
                title: create_exchange_title(&human_text),
                content: format!("Human: {}\n\nAssistant: {}", human_text, assistant_text),
                created_at: human_time,
            });
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
