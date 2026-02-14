//! Signal Desktop import module.
//!
//! Reads encrypted Signal Desktop database (SQLCipher 4) and imports
//! messages from a specified conversation as Mycelica nodes with edges.
//! Implements three-tier message filtering, link nodes, threading,
//! decision detection, and incremental re-import.
//!
//! Reference: docs/implementation/signal-import-mapping-spec.md

use crate::db::{Database, Node, Edge, NodeType, EdgeType, Position};
use regex::Regex;
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::collections::HashMap;

// =============================================================================
// Tuneable constants (spec §7.3)
// =============================================================================

const MIN_BODY_LENGTH_FOR_NODE: usize = 15;
const MIN_BODY_LENGTH_FOR_LINK_COMMENTARY: usize = 15;
const TEMPORAL_THREAD_MAX_GAP_SECS: f64 = 300.0;
const TEMPORAL_THREAD_MIN_WEIGHT: f64 = 0.3;
const BURST_MAX_GAP_SECS: f64 = 30.0;
const DECISION_LOOKBACK_MESSAGES: usize = 10;
const DECISION_LOOKBACK_SECS: i64 = 900;
const DECISION_MIN_AGREEMENTS: usize = 2;
const DECISION_MIN_CONFIDENCE: f64 = 0.7;
const TITLE_MAX_LENGTH: usize = 80;
const LINK_NODE_ID_HASH_PREFIX_LEN: usize = 12;

// =============================================================================
// Signal data structures
// =============================================================================

/// Signal message constructed from SQL columns + JSON blob.
/// NOT a serde target — fields are populated manually from query results.
#[derive(Debug)]
struct SignalMessage {
    id: String,
    conversation_id: String,
    msg_type: String,
    body: Option<String>,
    sent_at: Option<i64>,
    source: Option<String>,
    source_service_id: Option<String>,
    quote: Option<SignalQuote>,
    preview: Option<Vec<SignalPreview>>,
    has_attachments: Option<bool>,
    reactions: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct SignalQuote {
    id: Option<i64>,
    text: Option<String>,
    author: Option<String>,
    author_aci: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SignalPreview {
    url: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct SignalConversation {
    id: String,
    name: Option<String>,
    profile_name: Option<String>,
    #[serde(rename = "type")]
    conv_type: Option<String>,
}

// =============================================================================
// Import result
// =============================================================================

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalImportResult {
    pub messages_processed: usize,
    pub nodes_created: usize,
    pub nodes_skipped_dedup: usize,
    pub nodes_skipped_filter: usize,
    pub metadata_attached: usize,
    pub edits_detected: usize,
    pub edges_created: usize,
    pub replies_found: usize,
    pub links_found: usize,
    pub link_nodes_created: usize,
    pub temporal_threads: usize,
    pub decisions_detected: usize,
    pub errors: Vec<String>,
}

// =============================================================================
// Message classification (spec §1.1)
// =============================================================================

#[derive(Debug, PartialEq)]
enum MessageTier {
    Skip,
    Metadata,
    FullNode,
}

/// Protocol message types that are noise (spec §1.1 Tier A)
const SKIP_MSG_TYPES: &[&str] = &[
    "keychange",
    "verified-change",
    "group-v2-change",
    "profile-change",
    "timer-notification",
    "change-number-notification",
    "contact-removed-notification",
];

fn build_ack_regex() -> Regex {
    Regex::new(
        r"(?i)^(ok|okay|k|yes|yeah|yep|yup|no|nah|nope|sure|agreed|sounds good|perfect|nice|lol|lmao|haha|thanks|thx|ty|cool|bet|word|right|true|fair|same|exactly|indeed|correct|ack|roger|noted|got it|makes sense|will do)$"
    ).unwrap()
}

fn build_pure_emoji_regex() -> Regex {
    // Matches strings that are entirely emoji/whitespace, up to 8 chars
    Regex::new(r"^[\p{Emoji}\p{Emoji_Component}\s]{0,8}$").unwrap()
}

fn build_url_regex() -> Regex {
    Regex::new(r#"https?://[^\s<>"'\)\]\}]+"#).unwrap()
}

fn build_agreement_regex() -> Regex {
    Regex::new(
        r"(?i)^(agreed|let'?s do (it|that|this)|sounds good|sounds right|let'?s go with|perfect|i'?m (in|down|on board)|ship it|lgtm|approved|go for it|works for me|makes sense to me|i agree|deal|done|settled|confirmed|yes,? let'?s|good plan|love it)"
    ).unwrap()
}

fn build_proposal_regex() -> Regex {
    Regex::new(
        r"(?i)(should we|how about|what if|i (think|suggest|propose|recommend)|we could|let'?s|option [a-z0-9]|plan:|proposal:|idea:)"
    ).unwrap()
}

fn has_url(body: &str, url_re: &Regex) -> bool {
    url_re.is_match(body)
}

fn classify_message(
    msg: &SignalMessage,
    ack_re: &Regex,
    emoji_re: &Regex,
    url_re: &Regex,
) -> MessageTier {
    // Tier A: protocol noise types
    if SKIP_MSG_TYPES.contains(&msg.msg_type.as_str()) {
        return MessageTier::Skip;
    }

    let body = match &msg.body {
        Some(b) => b.trim(),
        None => return MessageTier::Skip,
    };

    // Tier A: empty body
    if body.is_empty() {
        return MessageTier::Skip;
    }

    let has_quote = msg.quote.is_some();
    let body_has_url = has_url(body, url_re);

    // Tier A: pure emoji, no quote
    if !has_quote && body.len() <= 8 && emoji_re.is_match(body) {
        return MessageTier::Skip;
    }

    // Tier A: single short word, no quote, no URL
    if !has_quote && !body_has_url {
        let word_count = body.split_whitespace().count();
        if word_count == 1 && body.len() < 6 {
            return MessageTier::Skip;
        }
    }

    // Tier B: short acknowledgment with quote → metadata on parent
    if has_quote && body.len() < MIN_BODY_LENGTH_FOR_NODE && ack_re.is_match(body) {
        return MessageTier::Metadata;
    }

    // Tier B: pure emoji with quote → reaction metadata
    if has_quote && emoji_re.is_match(body) {
        return MessageTier::Metadata;
    }

    // Tier C: everything else that has content
    // Explicit inclusions: URL (any length), substantive reply, >=15 chars
    if body_has_url || body.len() >= MIN_BODY_LENGTH_FOR_NODE || (has_quote && body.len() >= MIN_BODY_LENGTH_FOR_NODE) {
        return MessageTier::FullNode;
    }

    // Short messages without quote/URL that aren't ack patterns: skip
    // (they're too short to be meaningful standalone nodes)
    if body.len() < MIN_BODY_LENGTH_FOR_NODE {
        return MessageTier::Skip;
    }

    MessageTier::FullNode
}

// =============================================================================
// Signal DB access
// =============================================================================

/// Read Signal Desktop decryption key from config.json
pub fn read_signal_key(config_path: &str) -> Result<String, String> {
    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("Signal config not found at {}: {}. Is Signal Desktop installed?", config_path, e))?;

    let config: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse Signal config.json: {}", e))?;

    config.get("key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "No 'key' field in Signal config.json".to_string())
}

/// Open Signal Desktop database with SQLCipher decryption (read-only)
fn open_signal_db(db_path: &str, key: &str) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("Failed to open Signal database at {}: {}. Is Signal Desktop installed?", db_path, e))?;

    // Decrypt with SQLCipher PRAGMA
    conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", key))
        .map_err(|e| format!("Failed to set decryption key: {}", e))?;

    // Verify decryption worked
    conn.execute_batch("SELECT count(*) FROM messages;")
        .map_err(|_| "Failed to decrypt Signal database. Wrong key or Signal may be running (lock conflict).".to_string())?;

    Ok(conn)
}

/// Query messages for a conversation from Signal DB.
///
/// Signal Desktop stores primary fields (id, body, type, sent_at, source, etc.)
/// as SQL table columns. Supplementary data (quote, preview, reactions) lives
/// in the `json` column blob. We read both and merge them into SignalMessage.
fn query_messages(conn: &Connection, conversation_id: &str, after_sent_at: Option<i64>) -> Result<Vec<SignalMessage>, String> {
    let mut messages = Vec::new();

    let (sql, sent_at_param) = match after_sent_at {
        Some(ts) => (
            "SELECT id, body, type, sent_at, source, sourceServiceId, hasAttachments, conversationId, json
             FROM messages WHERE conversationId = ?1 AND sent_at > ?2 ORDER BY sent_at ASC",
            Some(ts),
        ),
        None => (
            "SELECT id, body, type, sent_at, source, sourceServiceId, hasAttachments, conversationId, json
             FROM messages WHERE conversationId = ?1 ORDER BY sent_at ASC",
            None,
        ),
    };

    let mut stmt = conn.prepare(sql)
        .map_err(|e| format!("Failed to prepare messages query: {}", e))?;

    let map_row = |row: &rusqlite::Row| -> rusqlite::Result<SignalMessage> {
        let id: Option<String> = row.get(0)?;
        let body: Option<String> = row.get(1)?;
        let msg_type: Option<String> = row.get(2)?;
        let sent_at: Option<i64> = row.get(3)?;
        let source: Option<String> = row.get(4)?;
        let source_service_id: Option<String> = row.get(5)?;
        let has_attachments_int: Option<i32> = row.get(6)?;
        let conversation_id: Option<String> = row.get(7)?;
        let json_str: Option<String> = row.get(8)?;

        // Parse supplementary fields from JSON blob
        let json_val: Option<serde_json::Value> = json_str
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok());

        let quote = json_val.as_ref()
            .and_then(|v| v.get("quote"))
            .and_then(|q| serde_json::from_value::<SignalQuote>(q.clone()).ok());

        let preview = json_val.as_ref()
            .and_then(|v| v.get("preview"))
            .and_then(|p| serde_json::from_value::<Vec<SignalPreview>>(p.clone()).ok());

        let reactions = json_val.as_ref()
            .and_then(|v| v.get("reactions"))
            .and_then(|r| serde_json::from_value::<Vec<serde_json::Value>>(r.clone()).ok());

        Ok(SignalMessage {
            id: id.unwrap_or_default(),
            conversation_id: conversation_id.unwrap_or_default(),
            msg_type: msg_type.unwrap_or_default(),
            body,
            sent_at,
            source,
            source_service_id,
            quote,
            preview,
            has_attachments: has_attachments_int.map(|v| v > 0),
            reactions,
        })
    };

    let rows_result = if let Some(ts) = sent_at_param {
        stmt.query_map(params![conversation_id, ts], map_row)
    } else {
        stmt.query_map(params![conversation_id], map_row)
    };

    match rows_result {
        Ok(rows) => {
            for row in rows {
                match row {
                    Ok(msg) => messages.push(msg),
                    Err(e) => eprintln!("[Signal] Warning: failed to read message row: {}", e),
                }
            }
        }
        Err(e) => return Err(format!("Failed to query messages: {}", e)),
    }

    Ok(messages)
}

/// Get conversation metadata
fn get_conversation_metadata(conn: &Connection, conversation_id: &str) -> Result<SignalConversation, String> {
    let json_str: String = conn.query_row(
        "SELECT json FROM conversations WHERE id = ?1",
        params![conversation_id],
        |row| row.get(0),
    ).map_err(|e| format!("Conversation '{}' not found: {}", conversation_id, e))?;

    serde_json::from_str(&json_str)
        .map_err(|e| format!("Failed to parse conversation JSON: {}", e))
}

/// List available conversations (for --list mode)
pub fn list_signal_conversations(
    signal_db_path: &str,
    signal_key: &str,
) -> Result<Vec<(String, String, usize)>, String> {
    let conn = open_signal_db(signal_db_path, signal_key)?;

    let mut stmt = conn.prepare(
        "SELECT c.id, c.json, COUNT(m.rowid) as msg_count
         FROM conversations c
         LEFT JOIN messages m ON m.conversationId = c.id
         GROUP BY c.id
         ORDER BY msg_count DESC"
    ).map_err(|e| format!("Failed to query conversations: {}", e))?;

    let results: Vec<(String, String, usize)> = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let json_str: String = row.get(1)?;
        let count: usize = row.get(2)?;
        Ok((id, json_str, count))
    })
    .map_err(|e| format!("Failed to iterate conversations: {}", e))?
    .filter_map(|r| r.ok())
    .map(|(id, json_str, count)| {
        let name = serde_json::from_str::<serde_json::Value>(&json_str)
            .ok()
            .and_then(|v| {
                v.get("name").and_then(|n| n.as_str().map(String::from))
                    .or_else(|| v.get("profileName").and_then(|n| n.as_str().map(String::from)))
            })
            .unwrap_or_else(|| "Unknown".to_string());
        (id, name, count)
    })
    .filter(|(_, _, count)| *count > 0)
    .collect();

    Ok(results)
}

// =============================================================================
// Author mapping
// =============================================================================

pub type AuthorMap = HashMap<String, String>;

/// Parse "--author-map" flag: "+372xxx=E,+1xxx=F"
pub fn parse_author_map(raw: &str) -> AuthorMap {
    let mut map = HashMap::new();
    for pair in raw.split(',') {
        let pair = pair.trim();
        if let Some(eq_pos) = pair.find('=') {
            let key = pair[..eq_pos].trim().to_string();
            let value = pair[eq_pos + 1..].trim().to_string();
            if !key.is_empty() && !value.is_empty() {
                map.insert(key, value);
            }
        }
    }
    map
}

/// Resolve author from message fields using author map
fn resolve_author(msg: &SignalMessage, author_map: &AuthorMap) -> String {
    // Try source (phone number) first
    if let Some(source) = &msg.source {
        if let Some(name) = author_map.get(source) {
            return name.clone();
        }
    }
    // Try serviceId
    if let Some(service_id) = &msg.source_service_id {
        if let Some(name) = author_map.get(service_id) {
            return name.clone();
        }
        // Fallback: first 8 chars of serviceId (never store raw phone numbers)
        return service_id.chars().take(8).collect();
    }
    // For outgoing messages without source
    if msg.msg_type == "outgoing" {
        return "self".to_string();
    }
    "unknown".to_string()
}

// =============================================================================
// Title generation
// =============================================================================

/// Generate title from message body: first sentence up to TITLE_MAX_LENGTH chars.
/// All slicing uses char_indices to avoid panics on multi-byte UTF-8.
fn generate_title(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "Empty message".to_string();
    }

    // Find first sentence boundary within limit (count by chars)
    let mut char_count = 0;
    for (i, c) in trimmed.char_indices() {
        char_count += 1;
        if (c == '.' || c == '!' || c == '?') && i > 0 && char_count <= TITLE_MAX_LENGTH {
            let end = i + c.len_utf8();
            return trimmed[..end].to_string();
        }
        if char_count > TITLE_MAX_LENGTH + 20 {
            break;
        }
    }

    // No sentence boundary found — truncate at word boundary
    if trimmed.chars().count() <= TITLE_MAX_LENGTH {
        return trimmed.to_string();
    }

    // Find byte offset of the TITLE_MAX_LENGTH-th char
    let byte_limit = trimmed.char_indices()
        .nth(TITLE_MAX_LENGTH)
        .map(|(i, _)| i)
        .unwrap_or(trimmed.len());
    let truncated = &trimmed[..byte_limit];

    if let Some(last_space) = truncated.rfind(' ') {
        if last_space > byte_limit / 2 {
            return format!("{}...", &trimmed[..last_space]);
        }
    }

    format!("{}...", truncated)
}

// =============================================================================
// URL extraction & link nodes (spec §3)
// =============================================================================

/// Normalize URL for dedup: strip trailing punctuation, trailing slash, fragments
fn normalize_url(url: &str) -> String {
    let mut normalized = url.to_string();
    // Strip trailing punctuation commonly captured by regex
    while normalized.ends_with('.') || normalized.ends_with(',')
        || normalized.ends_with(')') || normalized.ends_with(']')
    {
        normalized.pop();
    }
    // Strip fragment
    if let Some(hash_pos) = normalized.find('#') {
        normalized = normalized[..hash_pos].to_string();
    }
    // Strip trailing slash
    if normalized.ends_with('/') {
        normalized.pop();
    }
    normalized
}

/// Generate deterministic link node ID from normalized URL
fn link_node_id(normalized_url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalized_url.as_bytes());
    let hash = hex::encode(hasher.finalize());
    format!("signal-link-{}", &hash[..LINK_NODE_ID_HASH_PREFIX_LEN])
}

/// Extract domain + path prefix for link node title
fn link_title(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        let domain = parsed.host_str().unwrap_or("unknown");
        let path = parsed.path();
        let display = if path.len() > 1 {
            let truncated_path = if path.len() > 40 {
                format!("{}...", &path[..40])
            } else {
                path.to_string()
            };
            format!("{}{}", domain, truncated_path)
        } else {
            domain.to_string()
        };
        if display.len() > TITLE_MAX_LENGTH {
            format!("{}...", &display[..TITLE_MAX_LENGTH - 3])
        } else {
            display
        }
    } else {
        let truncated = if url.len() > TITLE_MAX_LENGTH {
            format!("{}...", &url[..TITLE_MAX_LENGTH - 3])
        } else {
            url.to_string()
        };
        truncated
    }
}

/// Extract commentary (non-URL text) from a message body
fn extract_commentary(body: &str, url_re: &Regex) -> String {
    url_re.replace_all(body, "").trim().to_string()
}

// =============================================================================
// Threading (spec §2)
// =============================================================================

struct ThreadTracker {
    counter: u32,
    thread_map: HashMap<i64, String>,  // sent_at -> thread_id
    conv_prefix: String,
}

impl ThreadTracker {
    fn new(conv_id_prefix: &str) -> Self {
        Self {
            counter: 0,
            thread_map: HashMap::new(),
            conv_prefix: conv_id_prefix.to_string(),
        }
    }

    fn assign_thread(
        &mut self,
        sent_at: i64,
        quote_sent_at: Option<i64>,
        prev_sent_at: Option<i64>,
        same_author_or_rapid: bool,
    ) -> String {
        // Priority 1: explicit quote → inherit quoted message's thread
        if let Some(quoted_ts) = quote_sent_at {
            if let Some(thread_id) = self.thread_map.get(&quoted_ts) {
                let tid = thread_id.clone();
                self.thread_map.insert(sent_at, tid.clone());
                return tid;
            }
        }

        // Priority 2: temporal continuity
        if let Some(prev_ts) = prev_sent_at {
            let gap_secs = (sent_at - prev_ts) as f64 / 1000.0;
            if gap_secs <= TEMPORAL_THREAD_MAX_GAP_SECS && same_author_or_rapid {
                if let Some(thread_id) = self.thread_map.get(&prev_ts) {
                    let tid = thread_id.clone();
                    self.thread_map.insert(sent_at, tid.clone());
                    return tid;
                }
            }
        }

        // Priority 3: new thread
        self.counter += 1;
        let tid = format!("signal-thread-{}-{:04}", self.conv_prefix, self.counter);
        self.thread_map.insert(sent_at, tid.clone());
        tid
    }
}

/// Calculate temporal thread weight (spec §2.3)
fn temporal_thread_weight(gap_ms: i64) -> f64 {
    let gap_secs = gap_ms as f64 / 1000.0;
    (1.0 - gap_secs / TEMPORAL_THREAD_MAX_GAP_SECS).max(0.0)
}

// =============================================================================
// Decision detection (spec §4)
// =============================================================================

struct ProcessedMessage {
    node_id: String,
    sent_at: i64,
    author: String,
    body: String,
    is_proposal: bool,
}

/// Detect decisions by scanning backward from agreement signals (spec §4.2)
///
/// For each agreement signal, scans backward to find the proposal it agrees with:
/// 1. Prioritize messages matching proposal_re ("should we", "how about", etc.)
/// 2. Fall back to following existing agreement chains
/// This prevents Supports edges pointing at non-proposals (factual statements).
fn detect_decisions(
    processed: &[ProcessedMessage],
    agreement_re: &Regex,
    proposal_re: &Regex,
    edges: &mut Vec<Edge>,
    db: &Database,
    participant_count: usize,
) -> usize {
    let mut decisions_detected = 0;
    // Track agreements per proposal: proposal_node_id -> set of agreeing authors
    let mut agreements: HashMap<String, Vec<String>> = HashMap::new();
    let now = chrono::Utc::now().timestamp_millis();

    for (i, msg) in processed.iter().enumerate() {
        if !agreement_re.is_match(&msg.body) {
            continue;
        }

        // Look backward for the proposal this agrees with
        let mut target_node_id: Option<String> = None;

        // Scan backward up to DECISION_LOOKBACK_MESSAGES / DECISION_LOOKBACK_SECS
        let lookback_start = if i > DECISION_LOOKBACK_MESSAGES { i - DECISION_LOOKBACK_MESSAGES } else { 0 };

        // Pass 1: find a proposal-pattern match (highest priority per spec §4.2)
        for j in (lookback_start..i).rev() {
            let prior = &processed[j];
            if (msg.sent_at - prior.sent_at) > DECISION_LOOKBACK_SECS * 1000 {
                break;
            }
            if prior.is_proposal || proposal_re.is_match(&prior.body) {
                target_node_id = Some(prior.node_id.clone());
                break;
            }
        }

        // Pass 2: if no proposal found, follow existing agreement chains
        if target_node_id.is_none() {
            for j in (lookback_start..i).rev() {
                let prior = &processed[j];
                if (msg.sent_at - prior.sent_at) > DECISION_LOOKBACK_SECS * 1000 {
                    break;
                }
                if agreement_re.is_match(&prior.body) {
                    if let Some(agreeing_to) = agreements.keys()
                        .find(|k| agreements[*k].iter().any(|a| a == &prior.author))
                    {
                        target_node_id = Some(agreeing_to.clone());
                        break;
                    }
                }
            }
        }

        if let Some(ref target_id) = target_node_id {
            // Create Supports edge
            edges.push(Edge {
                id: format!("signal-supports-{}-{}", &msg.node_id, &target_id),
                source: msg.node_id.clone(),
                target: target_id.clone(),
                edge_type: EdgeType::Supports,
                label: None,
                weight: Some(0.9),
                edge_source: Some("import".to_string()),
                evidence_id: None,
                confidence: Some(0.9),
                created_at: now,
                updated_at: None,
                author: Some(msg.author.clone()),
                reason: Some("Agreement signal detected".to_string()),
                content: None,
                agent_id: Some("human".to_string()),
                superseded_by: None,
                metadata: None,
            });

            // Track agreements
            agreements.entry(target_id.clone())
                .or_default()
                .push(msg.author.clone());

            // Check if enough agreements to flag as decision
            let unique_authors: Vec<&String> = {
                let all = &agreements[target_id];
                let mut seen = std::collections::HashSet::new();
                all.iter().filter(|a| seen.insert(a.as_str())).collect()
            };

            let min_needed = DECISION_MIN_AGREEMENTS.min(participant_count / 2).max(2);
            if unique_authors.len() >= min_needed {
                // Determine confidence
                let has_quote_agreement = processed[..=i].iter().any(|m| {
                    agreement_re.is_match(&m.body) && m.sent_at != msg.sent_at
                    // Simplified: we'd check if quote points to target
                });
                let confidence = if has_quote_agreement { 0.95 } else { 0.75 };

                if confidence >= DECISION_MIN_CONFIDENCE {
                    // Update the proposal node's tags with decision metadata
                    if let Ok(Some(node)) = db.get_node(target_id) {
                        let mut tags_val: serde_json::Value = node.tags
                            .as_deref()
                            .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
                            .unwrap_or(serde_json::json!({}));

                        tags_val["decision_detected"] = serde_json::json!(true);
                        tags_val["decision_confidence"] = serde_json::json!(confidence);
                        tags_val["agreement_count"] = serde_json::json!(unique_authors.len());
                        tags_val["agreeing_authors"] = serde_json::json!(
                            unique_authors.iter().map(|a| a.as_str()).collect::<Vec<_>>()
                        );

                        if let Ok(tags_str) = serde_json::to_string(&tags_val) {
                            db.update_node_tags(target_id, &tags_str).ok();
                        }
                    }
                    decisions_detected += 1;
                }
            }
        }
    }

    decisions_detected
}

// =============================================================================
// Core import logic
// =============================================================================

pub fn import_signal_conversation(
    db: &Database,
    signal_db_path: &str,
    signal_key: &str,
    conversation_id: &str,
    author_map: &AuthorMap,
) -> Result<SignalImportResult, String> {
    let mut result = SignalImportResult {
        messages_processed: 0,
        nodes_created: 0,
        nodes_skipped_dedup: 0,
        nodes_skipped_filter: 0,
        metadata_attached: 0,
        edits_detected: 0,
        edges_created: 0,
        replies_found: 0,
        links_found: 0,
        link_nodes_created: 0,
        temporal_threads: 0,
        decisions_detected: 0,
        errors: Vec::new(),
    };

    // Open Signal DB (read-only)
    let signal_conn = open_signal_db(signal_db_path, signal_key)?;

    // Get conversation metadata
    let conv_meta = get_conversation_metadata(&signal_conn, conversation_id)?;

    let conv_id_prefix = &conversation_id[..conversation_id.len().min(8)];
    let container_id = format!("signal-conv-{}", conv_id_prefix);

    // Check for incremental import: find latest imported sent_at
    let last_imported_sent_at = get_last_imported_sent_at(db, &container_id);

    // Load existing sent_at -> node_id map for edge resolution across imports
    let mut sent_at_to_node_id: HashMap<i64, String> = load_existing_sent_at_map(db, &container_id);

    // Query messages (incremental if re-importing)
    let messages = query_messages(&signal_conn, conversation_id, last_imported_sent_at)?;

    if messages.is_empty() && last_imported_sent_at.is_some() {
        eprintln!("[Signal] No new messages since last import.");
        return Ok(result);
    }

    // Build compiled regexes
    let ack_re = build_ack_regex();
    let emoji_re = build_pure_emoji_regex();
    let url_re = build_url_regex();
    let agreement_re = build_agreement_regex();
    let proposal_re = build_proposal_regex();

    // Create or verify container node
    if db.get_node(&container_id).ok().flatten().is_none() {
        let conv_name = conv_meta.name
            .or(conv_meta.profile_name)
            .unwrap_or_else(|| format!("Signal: {}", conv_id_prefix));

        let container = Node {
            id: container_id.clone(),
            node_type: NodeType::Context,
            title: conv_name,
            url: None,
            content: Some(format!("{} messages", messages.len())),
            position: Position { x: 0.0, y: 0.0 },
            created_at: messages.first()
                .and_then(|m| m.sent_at)
                .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
            updated_at: chrono::Utc::now().timestamp_millis(),
            cluster_id: None,
            cluster_label: None,
            depth: 0,
            is_item: false,
            is_universe: false,
            parent_id: None,
            child_count: 0,
            ai_title: None,
            summary: None,
            tags: Some(serde_json::json!({
                "signal_conversation_id": conversation_id,
                "import_runs": []
            }).to_string()),
            emoji: Some("📱".to_string()),
            is_processed: false,
            conversation_id: None,
            sequence_index: None,
            is_pinned: false,
            last_accessed_at: None,
            latest_child_date: None,
            is_private: None,
            privacy_reason: None,
            source: Some("signal".to_string()),
            pdf_available: None,
            content_type: None,
            associated_idea_id: None,
            privacy: None,
            human_edited: None,
            human_created: false,
            author: None,
            agent_id: Some("human".to_string()),
            node_class: Some("knowledge".to_string()),
            meta_type: None,
        };

        if let Err(e) = db.insert_node(&container) {
            return Err(format!("Failed to create container node: {}", e));
        }
    }

    // Track link nodes already created (normalized_url -> node_id)
    let mut link_nodes: HashMap<String, String> = HashMap::new();

    // Collect edges for batch insert
    let mut edges: Vec<Edge> = Vec::new();

    // Track processed messages for decision detection
    let mut processed_messages: Vec<ProcessedMessage> = Vec::new();

    // Previous message tracking for temporal threading
    let mut prev_node_id: Option<String> = None;
    let mut prev_sent_at: Option<i64> = None;
    let mut prev_author: Option<String> = None;

    // Threading
    let mut thread_tracker = ThreadTracker::new(conv_id_prefix);

    let now = chrono::Utc::now().timestamp_millis();

    // Count participants for decision detection
    let mut participants: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Calculate base sequence_index for incremental imports
    let base_sequence_index = if last_imported_sent_at.is_some() {
        // Get the max sequence_index of existing nodes
        get_max_sequence_index(db, &container_id).unwrap_or(0) + 1
    } else {
        0
    };

    // =========================================================================
    // Main processing loop (spec §7.2)
    // =========================================================================

    for (msg_idx, msg) in messages.iter().enumerate() {
        result.messages_processed += 1;

        let sent_at = match msg.sent_at {
            Some(ts) => ts,
            None => {
                result.nodes_skipped_filter += 1;
                continue;
            }
        };

        let author = resolve_author(msg, author_map);
        participants.insert(author.clone());

        // sequence_index counts ALL messages including filtered (spec §1.2)
        let sequence_index = base_sequence_index + msg_idx as i32;

        // Classify message tier
        let tier = classify_message(msg, &ack_re, &emoji_re, &url_re);

        match tier {
            MessageTier::Skip => {
                result.nodes_skipped_filter += 1;
                continue;
            }
            MessageTier::Metadata => {
                // Attach as reaction to quoted message's node
                if let Some(ref quote) = msg.quote {
                    if let Some(quoted_ts) = quote.id {
                        if let Some(parent_node_id) = sent_at_to_node_id.get(&quoted_ts) {
                            attach_reaction_metadata(
                                db,
                                parent_node_id,
                                &author,
                                msg.body.as_deref().unwrap_or(""),
                                sent_at,
                            );
                            result.metadata_attached += 1;
                        }
                    }
                }
                continue;
            }
            MessageTier::FullNode => {
                // Continue to node creation below
            }
        }

        let body = msg.body.as_deref().unwrap_or("");
        let node_id = format!("signal-{}-{}", conv_id_prefix, sent_at);

        // Dedup / edit detection (spec §5.2)
        match db.get_node(&node_id) {
            Ok(Some(existing)) => {
                if existing.content.as_deref() == Some(body) {
                    // Unchanged — skip
                    result.nodes_skipped_dedup += 1;
                    sent_at_to_node_id.insert(sent_at, node_id.clone());
                    // Still need to track for threading
                    prev_node_id = Some(node_id);
                    prev_sent_at = Some(sent_at);
                    prev_author = Some(author);
                    continue;
                } else {
                    // Edit detected — update content and title
                    let old_content = existing.content.as_deref().unwrap_or("");
                    let new_title = generate_title(body);

                    db.update_node_content(&node_id, body).ok();
                    db.update_node_title(&node_id, &new_title).ok();

                    // Append to edit_history in tags
                    let mut tags_val: serde_json::Value = existing.tags
                        .as_deref()
                        .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
                        .unwrap_or(serde_json::json!({}));

                    let edit_entry = serde_json::json!({
                        "previous_content": old_content,
                        "edited_at": now
                    });
                    if let Some(arr) = tags_val.get_mut("edit_history").and_then(|v| v.as_array_mut()) {
                        arr.push(edit_entry);
                    } else {
                        tags_val["edit_history"] = serde_json::json!([edit_entry]);
                    }
                    if let Ok(tags_str) = serde_json::to_string(&tags_val) {
                        db.update_node_tags(&node_id, &tags_str).ok();
                    }

                    result.edits_detected += 1;
                    sent_at_to_node_id.insert(sent_at, node_id.clone());
                    prev_node_id = Some(node_id);
                    prev_sent_at = Some(sent_at);
                    prev_author = Some(author);
                    continue;
                }
            }
            _ => {
                // New message — create node
            }
        }

        // =====================================================================
        // Create message node (Tier C)
        // =====================================================================

        let title = generate_title(body);

        // Extract URLs
        let urls: Vec<String> = url_re.find_iter(body)
            .map(|m| m.as_str().to_string())
            .collect();

        // Build thread ID
        let quote_sent_at = msg.quote.as_ref().and_then(|q| q.id);
        let same_author_or_rapid = if let Some(ref pa) = prev_author {
            pa == &author || prev_sent_at.map(|pts| (sent_at - pts) < (BURST_MAX_GAP_SECS * 1000.0) as i64).unwrap_or(false)
        } else {
            false
        };
        let thread_id = thread_tracker.assign_thread(sent_at, quote_sent_at, prev_sent_at, same_author_or_rapid);

        let is_proposal = proposal_re.is_match(body);

        // Build tags/metadata JSON
        let tags_json = serde_json::json!({
            "signal_message_id": msg.id,
            "signal_type": msg.msg_type,
            "has_attachments": msg.has_attachments.unwrap_or(false),
            "reactions": [],
            "edit_history": [],
            "thread_id": thread_id,
            "urls_found": urls.iter().map(|u| normalize_url(u)).collect::<Vec<_>>(),
            "raw_sent_at": sent_at
        });

        let node = Node {
            id: node_id.clone(),
            node_type: NodeType::Thought,
            title,
            url: None,
            content: Some(body.to_string()),
            position: Position { x: 0.0, y: 0.0 },
            created_at: sent_at,
            updated_at: sent_at,
            cluster_id: None,
            cluster_label: None,
            depth: 0,
            is_item: true,
            is_universe: false,
            parent_id: None,
            child_count: 0,
            ai_title: None,
            summary: None,
            tags: Some(tags_json.to_string()),
            emoji: Some("📱".to_string()),
            is_processed: false,
            conversation_id: Some(container_id.clone()),
            sequence_index: Some(sequence_index),
            is_pinned: false,
            last_accessed_at: None,
            latest_child_date: None,
            is_private: None,
            privacy_reason: None,
            source: Some("signal".to_string()),
            pdf_available: None,
            content_type: None,
            associated_idea_id: None,
            privacy: None,
            human_edited: None,
            human_created: true,
            author: Some(author.clone()),
            agent_id: Some("human".to_string()),
            node_class: Some("knowledge".to_string()),
            meta_type: None,
        };

        if let Err(e) = db.insert_node(&node) {
            result.errors.push(format!("Failed to insert node {}: {}", node_id, e));
            continue;
        }
        result.nodes_created += 1;
        sent_at_to_node_id.insert(sent_at, node_id.clone());

        // =====================================================================
        // Create edges
        // =====================================================================

        // --- RepliesTo edge (spec §2.1 Signal 1) ---
        if let Some(ref quote) = msg.quote {
            if let Some(quoted_ts) = quote.id {
                if let Some(quoted_node_id) = sent_at_to_node_id.get(&quoted_ts) {
                    edges.push(Edge {
                        id: format!("signal-replies-{}-{}", &node_id, &quoted_node_id),
                        source: node_id.clone(),
                        target: quoted_node_id.clone(),
                        edge_type: EdgeType::RepliesTo,
                        label: quote.text.as_ref().map(|t| {
                            if t.chars().count() > 60 {
                                let truncated: String = t.chars().take(57).collect();
                                format!("{}...", truncated)
                            } else { t.clone() }
                        }),
                        weight: Some(1.0),
                        edge_source: Some("import".to_string()),
                        evidence_id: None,
                        confidence: Some(1.0),
                        created_at: now,
                        updated_at: None,
                        author: Some(author.clone()),
                        reason: Some("Signal quote/reply".to_string()),
                        content: None,
                        agent_id: Some("human".to_string()),
                        superseded_by: None,
                        metadata: None,
                    });
                    result.replies_found += 1;
                }
            }
        }

        // --- TemporalThread edge (spec §2.2, §2.3) ---
        if let (Some(ref p_node_id), Some(p_ts)) = (&prev_node_id, prev_sent_at) {
            let gap_ms = sent_at - p_ts;
            let weight = temporal_thread_weight(gap_ms);
            if weight >= TEMPORAL_THREAD_MIN_WEIGHT {
                // Burst detection: same author, <30s → weight 1.0
                let final_weight = if prev_author.as_ref() == Some(&author)
                    && (gap_ms as f64 / 1000.0) < BURST_MAX_GAP_SECS
                {
                    1.0
                } else {
                    weight
                };

                edges.push(Edge {
                    id: format!("signal-temporal-{}-{}", &node_id, &p_node_id),
                    source: p_node_id.clone(),
                    target: node_id.clone(),
                    edge_type: EdgeType::TemporalThread,
                    label: None,
                    weight: Some(final_weight),
                    edge_source: Some("import".to_string()),
                    evidence_id: None,
                    confidence: Some(final_weight),
                    created_at: now,
                    updated_at: None,
                    author: None,
                    reason: None,
                    content: None,
                    agent_id: Some("human".to_string()),
                    superseded_by: None,
                    metadata: None,
                });
                result.temporal_threads += 1;
            }
        }

        // --- Link nodes + SharesLink edges (spec §3) ---
        for raw_url in &urls {
            let normalized = normalize_url(raw_url);
            let ln_id = link_node_id(&normalized);
            result.links_found += 1;

            // Create or reuse link node
            if !link_nodes.contains_key(&normalized) {
                // Check if link node already exists in DB (from previous import)
                if db.get_node(&ln_id).ok().flatten().is_none() {
                    let ln_title = link_title(&normalized);
                    let link_node = Node {
                        id: ln_id.clone(),
                        node_type: NodeType::Thought,
                        title: ln_title,
                        url: Some(normalized.clone()),
                        content: Some(normalized.clone()),
                        position: Position { x: 0.0, y: 0.0 },
                        created_at: sent_at,
                        updated_at: sent_at,
                        cluster_id: None,
                        cluster_label: None,
                        depth: 0,
                        is_item: true,
                        is_universe: false,
                        parent_id: None,
                        child_count: 0,
                        ai_title: None,
                        summary: None,
                        tags: Some(serde_json::json!({
                            "url": normalized,
                            "domain": url::Url::parse(&normalized).ok()
                                .and_then(|u| u.host_str().map(String::from))
                                .unwrap_or_default(),
                            "first_shared_at": sent_at,
                            "shared_by": [&author],
                            "share_count": 1
                        }).to_string()),
                        emoji: Some("🔗".to_string()),
                        is_processed: false,
                        conversation_id: Some(container_id.clone()),
                        sequence_index: None,
                        is_pinned: false,
                        last_accessed_at: None,
                        latest_child_date: None,
                        is_private: None,
                        privacy_reason: None,
                        source: Some("signal".to_string()),
                        pdf_available: None,
                        content_type: None,
                        associated_idea_id: None,
                        privacy: None,
                        human_edited: None,
                        human_created: false,
                        author: Some(author.clone()),
                        agent_id: Some("human".to_string()),
                        node_class: Some("reference".to_string()),
                        meta_type: None,
                    };

                    if let Err(e) = db.insert_node(&link_node) {
                        result.errors.push(format!("Failed to insert link node {}: {}", ln_id, e));
                    } else {
                        result.link_nodes_created += 1;
                    }
                } else {
                    // Link node exists from previous import — update shared_by
                    update_link_node_shared_by(db, &ln_id, &author, sent_at);
                }
                link_nodes.insert(normalized.clone(), ln_id.clone());
            } else {
                // Link node already created in this import run — update metadata
                update_link_node_shared_by(db, &ln_id, &author, sent_at);
            }

            // Create SharesLink edge
            let commentary = extract_commentary(body, &url_re);
            edges.push(Edge {
                id: format!("signal-shares-{}-{}", &node_id, &ln_id),
                source: node_id.clone(),
                target: ln_id.clone(),
                edge_type: EdgeType::SharesLink,
                label: None,
                weight: Some(0.8),
                edge_source: Some("import".to_string()),
                evidence_id: None,
                confidence: Some(0.8),
                created_at: now,
                updated_at: None,
                author: Some(author.clone()),
                reason: None,
                content: None,
                agent_id: Some("human".to_string()),
                superseded_by: None,
                metadata: if !commentary.is_empty() && commentary.len() >= MIN_BODY_LENGTH_FOR_LINK_COMMENTARY {
                    Some(serde_json::json!({
                        "commentary": commentary,
                        "shared_at": sent_at
                    }).to_string())
                } else {
                    Some(serde_json::json!({"shared_at": sent_at}).to_string())
                },
            });
        }

        // Track for decision detection
        processed_messages.push(ProcessedMessage {
            node_id: node_id.clone(),
            sent_at,
            author: author.clone(),
            body: body.to_string(),
            is_proposal,
        });

        // Update previous message tracking
        prev_node_id = Some(node_id);
        prev_sent_at = Some(sent_at);
        prev_author = Some(author);
    }

    // =========================================================================
    // Decision detection pass (spec §4)
    // =========================================================================

    result.decisions_detected = detect_decisions(
        &processed_messages,
        &agreement_re,
        &proposal_re,
        &mut edges,
        db,
        participants.len(),
    );

    // =========================================================================
    // Batch insert edges
    // =========================================================================

    if !edges.is_empty() {
        match db.insert_edges_batch(&edges) {
            Ok(inserted) => {
                result.edges_created = inserted;
            }
            Err(e) => {
                result.errors.push(format!("Failed to batch insert edges: {}", e));
            }
        }
    }

    // =========================================================================
    // Update container node metadata (spec §5.4)
    // =========================================================================

    update_container_import_metadata(db, &container_id, &result);

    Ok(result)
}

// =============================================================================
// Helper functions
// =============================================================================

/// Get the latest imported sent_at for incremental import (spec §5.1)
/// Uses json_extract on tags.raw_sent_at (integer millis) rather than created_at,
/// because the returned value is compared against Signal's integer sent_at column.
/// Single SQL query — no Rust-side JSON parsing.
fn get_last_imported_sent_at(db: &Database, container_id: &str) -> Option<i64> {
    let conn = db.raw_conn().lock().ok()?;
    let result: Option<i64> = conn.query_row(
        "SELECT MAX(json_extract(tags, '$.raw_sent_at')) FROM nodes WHERE source = 'signal' AND conversation_id = ?1 AND is_item = 1 AND tags IS NOT NULL",
        params![container_id],
        |row| row.get(0),
    ).ok()?;
    result
}

/// Load existing sent_at -> node_id map for cross-import edge resolution (spec §5.1)
fn load_existing_sent_at_map(db: &Database, container_id: &str) -> HashMap<i64, String> {
    let mut map = HashMap::new();
    let conn = match db.raw_conn().lock() {
        Ok(c) => c,
        Err(_) => return map,
    };

    let mut stmt = match conn.prepare(
        "SELECT id, tags FROM nodes WHERE source = 'signal' AND conversation_id = ?1 AND is_item = 1"
    ) {
        Ok(s) => s,
        Err(_) => return map,
    };

    let rows_result = stmt.query_map(params![container_id], |row| {
        let id: String = row.get(0)?;
        let tags: Option<String> = row.get(1)?;
        Ok((id, tags))
    });

    let rows: Vec<(String, Option<String>)> = match rows_result {
        Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
        Err(_) => return map,
    };

    for (node_id, tags) in rows {
        if let Some(ref tags_str) = tags {
            if let Ok(tags_val) = serde_json::from_str::<serde_json::Value>(tags_str) {
                if let Some(raw_sent_at) = tags_val.get("raw_sent_at").and_then(|v| v.as_i64()) {
                    map.insert(raw_sent_at, node_id);
                }
            }
        }
    }

    map
}

/// Get max sequence_index for incremental import
fn get_max_sequence_index(db: &Database, container_id: &str) -> Option<i32> {
    let conn = db.raw_conn().lock().ok()?;
    let result: Option<i32> = conn.query_row(
        "SELECT MAX(sequence_index) FROM nodes WHERE conversation_id = ?1",
        params![container_id],
        |row| row.get(0),
    ).ok()?;
    result
}

/// Attach reaction metadata to a parent node (Tier B handling, spec §1.1)
fn attach_reaction_metadata(
    db: &Database,
    parent_node_id: &str,
    author: &str,
    content: &str,
    sent_at: i64,
) {
    if let Ok(Some(node)) = db.get_node(parent_node_id) {
        let mut tags_val: serde_json::Value = node.tags
            .as_deref()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
            .unwrap_or(serde_json::json!({}));

        let reaction = serde_json::json!({
            "author": author,
            "content": content,
            "sent_at": sent_at
        });

        let reactions = tags_val.get_mut("reactions")
            .and_then(|v| v.as_array_mut());
        if let Some(arr) = reactions {
            arr.push(reaction);
        } else {
            tags_val["reactions"] = serde_json::json!([reaction]);
        }

        if let Ok(tags_str) = serde_json::to_string(&tags_val) {
            db.update_node_tags(parent_node_id, &tags_str).ok();
        }
    }
}

/// Update link node shared_by metadata when URL is re-shared
fn update_link_node_shared_by(db: &Database, link_node_id: &str, author: &str, _sent_at: i64) {
    if let Ok(Some(node)) = db.get_node(link_node_id) {
        let mut tags_val: serde_json::Value = node.tags
            .as_deref()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
            .unwrap_or(serde_json::json!({}));

        // Update shared_by array
        let shared_by = tags_val.get_mut("shared_by")
            .and_then(|v| v.as_array_mut());
        if let Some(arr) = shared_by {
            let author_val = serde_json::Value::String(author.to_string());
            if !arr.contains(&author_val) {
                arr.push(author_val);
            }
        }

        // Increment share_count
        let count = tags_val.get("share_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) + 1;
        tags_val["share_count"] = serde_json::json!(count);

        if let Ok(tags_str) = serde_json::to_string(&tags_val) {
            db.update_node_tags(link_node_id, &tags_str).ok();
        }
    }
}

/// Update container node with import run metadata (spec §5.4)
fn update_container_import_metadata(db: &Database, container_id: &str, result: &SignalImportResult) {
    if let Ok(Some(node)) = db.get_node(container_id) {
        let mut tags_val: serde_json::Value = node.tags
            .as_deref()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
            .unwrap_or(serde_json::json!({}));

        let run = serde_json::json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "messages_processed": result.messages_processed,
            "nodes_created": result.nodes_created,
            "nodes_skipped": result.nodes_skipped_dedup + result.nodes_skipped_filter,
            "nodes_updated": result.edits_detected,
            "edges_created": result.edges_created,
        });

        let import_runs = tags_val.get_mut("import_runs")
            .and_then(|v| v.as_array_mut());
        if let Some(arr) = import_runs {
            arr.push(run);
        } else {
            tags_val["import_runs"] = serde_json::json!([run]);
        }

        if let Ok(tags_str) = serde_json::to_string(&tags_val) {
            db.update_node_tags(container_id, &tags_str).ok();
        }

        // Update container content to reflect total
        let total = get_max_sequence_index(db, container_id)
            .map(|max| max + 1)
            .unwrap_or(0);
        let content = format!("{} messages imported", total);
        db.update_node_content(container_id, &content).ok();
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_author_map() {
        let map = parse_author_map("+372xxx=E,+1xxx=F,+372yyy=M");
        assert_eq!(map.len(), 3);
        assert_eq!(map.get("+372xxx").unwrap(), "E");
        assert_eq!(map.get("+1xxx").unwrap(), "F");
        assert_eq!(map.get("+372yyy").unwrap(), "M");
    }

    #[test]
    fn test_parse_author_map_with_spaces() {
        let map = parse_author_map(" +372xxx = E , +1xxx = F ");
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("+372xxx").unwrap(), "E");
    }

    #[test]
    fn test_generate_title_short() {
        assert_eq!(generate_title("Hello world"), "Hello world");
    }

    #[test]
    fn test_generate_title_sentence() {
        let body = "Should we use NATS or Redis for the Spore message bus? I think NATS would be better.";
        let title = generate_title(body);
        assert_eq!(title, "Should we use NATS or Redis for the Spore message bus?");
    }

    #[test]
    fn test_generate_title_long_no_period() {
        let body = "a ".repeat(50); // 100 chars, no period
        let title = generate_title(&body);
        assert!(title.len() <= TITLE_MAX_LENGTH + 3); // +3 for "..."
        assert!(title.ends_with("..."));
    }

    #[test]
    fn test_normalize_url() {
        assert_eq!(normalize_url("https://example.com/path/"), "https://example.com/path");
        assert_eq!(normalize_url("https://example.com/path."), "https://example.com/path");
        assert_eq!(normalize_url("https://example.com/path#section"), "https://example.com/path");
        assert_eq!(normalize_url("https://example.com/path),"), "https://example.com/path");
    }

    #[test]
    fn test_link_node_id_deterministic() {
        let id1 = link_node_id("https://docs.nats.io/nats-concepts/jetstream");
        let id2 = link_node_id("https://docs.nats.io/nats-concepts/jetstream");
        assert_eq!(id1, id2);
        assert!(id1.starts_with("signal-link-"));
        assert_eq!(id1.len(), "signal-link-".len() + LINK_NODE_ID_HASH_PREFIX_LEN);
    }

    #[test]
    fn test_link_title() {
        assert_eq!(link_title("https://docs.nats.io/nats-concepts/jetstream"), "docs.nats.io/nats-concepts/jetstream");
        assert_eq!(link_title("https://example.com"), "example.com");
    }

    #[test]
    fn test_url_regex() {
        let re = build_url_regex();
        assert!(re.is_match("check https://docs.nats.io/path out"));
        assert!(re.is_match("http://example.com"));
        assert!(!re.is_match("no urls here"));
    }

    #[test]
    fn test_classify_skip_empty() {
        let ack_re = build_ack_regex();
        let emoji_re = build_pure_emoji_regex();
        let url_re = build_url_regex();

        let msg = SignalMessage {
            id: "1".to_string(),
            conversation_id: "c1".to_string(),
            msg_type: "incoming".to_string(),
            body: None,
            sent_at: Some(1000),
            source: None,
            source_service_id: None,
            quote: None,
            preview: None,
            has_attachments: None,
            reactions: None,
        };
        assert_eq!(classify_message(&msg, &ack_re, &emoji_re, &url_re), MessageTier::Skip);
    }

    #[test]
    fn test_classify_skip_protocol() {
        let ack_re = build_ack_regex();
        let emoji_re = build_pure_emoji_regex();
        let url_re = build_url_regex();

        let msg = SignalMessage {
            id: "1".to_string(),
            conversation_id: "c1".to_string(),
            msg_type: "keychange".to_string(),
            body: Some("Safety number changed".to_string()),
            sent_at: Some(1000),
            source: None,
            source_service_id: None,
            quote: None,
            preview: None,
            has_attachments: None,
            reactions: None,
        };
        assert_eq!(classify_message(&msg, &ack_re, &emoji_re, &url_re), MessageTier::Skip);
    }

    #[test]
    fn test_classify_skip_short_word() {
        let ack_re = build_ack_regex();
        let emoji_re = build_pure_emoji_regex();
        let url_re = build_url_regex();

        let msg = SignalMessage {
            id: "1".to_string(),
            conversation_id: "c1".to_string(),
            msg_type: "incoming".to_string(),
            body: Some("lol".to_string()),
            sent_at: Some(1000),
            source: None,
            source_service_id: None,
            quote: None,
            preview: None,
            has_attachments: None,
            reactions: None,
        };
        assert_eq!(classify_message(&msg, &ack_re, &emoji_re, &url_re), MessageTier::Skip);
    }

    #[test]
    fn test_classify_metadata_ack_with_quote() {
        let ack_re = build_ack_regex();
        let emoji_re = build_pure_emoji_regex();
        let url_re = build_url_regex();

        let msg = SignalMessage {
            id: "1".to_string(),
            conversation_id: "c1".to_string(),
            msg_type: "incoming".to_string(),
            body: Some("agreed".to_string()),
            sent_at: Some(1000),
            source: None,
            source_service_id: None,
            quote: Some(SignalQuote {
                id: Some(999),
                text: Some("Should we use NATS?".to_string()),
                author: None,
                author_aci: None,
            }),
            preview: None,
            has_attachments: None,
            reactions: None,
        };
        assert_eq!(classify_message(&msg, &ack_re, &emoji_re, &url_re), MessageTier::Metadata);
    }

    #[test]
    fn test_classify_full_node() {
        let ack_re = build_ack_regex();
        let emoji_re = build_pure_emoji_regex();
        let url_re = build_url_regex();

        let msg = SignalMessage {
            id: "1".to_string(),
            conversation_id: "c1".to_string(),
            msg_type: "incoming".to_string(),
            body: Some("NATS has better clustering and is built for this exact use case".to_string()),
            sent_at: Some(1000),
            source: None,
            source_service_id: None,
            quote: None,
            preview: None,
            has_attachments: None,
            reactions: None,
        };
        assert_eq!(classify_message(&msg, &ack_re, &emoji_re, &url_re), MessageTier::FullNode);
    }

    #[test]
    fn test_classify_url_message_short() {
        let ack_re = build_ack_regex();
        let emoji_re = build_pure_emoji_regex();
        let url_re = build_url_regex();

        let msg = SignalMessage {
            id: "1".to_string(),
            conversation_id: "c1".to_string(),
            msg_type: "incoming".to_string(),
            body: Some("https://docs.nats.io".to_string()),
            sent_at: Some(1000),
            source: None,
            source_service_id: None,
            quote: None,
            preview: None,
            has_attachments: None,
            reactions: None,
        };
        assert_eq!(classify_message(&msg, &ack_re, &emoji_re, &url_re), MessageTier::FullNode);
    }

    #[test]
    fn test_temporal_thread_weight() {
        assert!((temporal_thread_weight(0) - 1.0).abs() < 0.001);
        assert!((temporal_thread_weight(150_000) - 0.5).abs() < 0.001); // 150s
        assert!((temporal_thread_weight(300_000) - 0.0).abs() < 0.001); // 300s = max
        assert!((temporal_thread_weight(600_000) - 0.0).abs() < 0.001); // clamped
    }

    #[test]
    fn test_extract_commentary() {
        let url_re = build_url_regex();
        assert_eq!(
            extract_commentary("check https://docs.nats.io out", &url_re),
            "check  out"
        );
        assert_eq!(
            extract_commentary("https://example.com", &url_re),
            ""
        );
    }

    #[test]
    fn test_agreement_regex() {
        let re = build_agreement_regex();
        assert!(re.is_match("agreed"));
        assert!(re.is_match("Sounds good"));
        assert!(re.is_match("let's do it"));
        assert!(re.is_match("LGTM"));
        assert!(re.is_match("ship it"));
        assert!(!re.is_match("I'm not sure about this"));
    }

    #[test]
    fn test_proposal_regex() {
        let re = build_proposal_regex();
        assert!(re.is_match("Should we use NATS or Redis?"));
        assert!(re.is_match("How about we use NATS?"));
        assert!(re.is_match("I think we should go with NATS"));
        assert!(re.is_match("let's try NATS"));
        assert!(!re.is_match("NATS has better clustering"));
    }
}
