//! Shared team mode helpers used by both CLI and server binaries.
//!
//! Extracted from cli.rs to avoid duplication. All functions accept
//! explicit author/source parameters rather than pulling from settings.

use crate::db::{Database, Node, NodeType, Edge, EdgeType, Position};
use crate::local_embeddings;
use serde::Serialize;

/// Summary of a node for resolve results and API responses
#[derive(Debug, Clone, Serialize)]
pub struct NodeSummary {
    pub id: String,
    pub title: String,
}

/// Result of attempting to resolve a node reference
pub enum ResolveResult {
    Found(Node),
    Ambiguous(Vec<NodeSummary>),
    NotFound(String),
}

/// Result of a single connects_to term
#[derive(Debug, Serialize)]
pub enum ConnectResult {
    Linked { edge_id: String, target: NodeSummary },
    Ambiguous { term: String, candidates: Vec<NodeSummary> },
    NotFound { term: String },
}

/// Create a human-authored node with sovereignty fields and generate embedding.
///
/// Returns the node ID on success.
pub fn create_human_node(
    db: &Database,
    title: &str,
    content: Option<&str>,
    url: Option<&str>,
    content_type: &str,
    tags_json: Option<&str>,
    author: &str,
    source: &str,
    is_item: Option<bool>,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();

    let node = Node {
        id: id.clone(),
        node_type: NodeType::Thought,
        title: title.to_string(),
        url: url.map(|s| s.to_string()),
        content: content.map(|s| s.to_string()),
        position: Position { x: 0.0, y: 0.0 },
        created_at: now,
        updated_at: now,
        cluster_id: None,
        cluster_label: None,
        depth: 0,
        is_item: is_item.unwrap_or(true),
        is_universe: false,
        parent_id: None,
        child_count: 0,
        ai_title: None,
        summary: None,
        tags: tags_json.map(|s| s.to_string()),
        emoji: None,
        is_processed: false,
        conversation_id: None,
        sequence_index: None,
        is_pinned: false,
        last_accessed_at: Some(now),
        latest_child_date: None,
        is_private: None,
        privacy_reason: None,
        privacy: None,
        source: Some(source.to_string()),
        pdf_available: None,
        content_type: Some(content_type.to_string()),
        associated_idea_id: None,
        human_edited: None,
        human_created: true,
        author: Some(author.to_string()),
    };

    db.insert_node(&node).map_err(|e| e.to_string())?;

    // Generate embedding so node is immediately searchable
    let embed_text = format!("{}\n{}", title, content.unwrap_or(""));
    let embed_text = &embed_text[..embed_text.len().min(2000)];
    match local_embeddings::generate(embed_text) {
        Ok(embedding) => {
            db.update_node_embedding(&id, &embedding).ok();
        }
        Err(e) => {
            eprintln!("Warning: failed to generate embedding: {}", e);
        }
    }

    Ok(id)
}

/// Resolve a node reference (UUID, ID prefix, or title text).
///
/// Returns structured result instead of printing — callers format for their context.
pub fn resolve_node(db: &Database, reference: &str) -> ResolveResult {
    // Try exact ID match
    if let Ok(Some(node)) = db.get_node(reference) {
        return ResolveResult::Found(node);
    }

    // Try ID prefix match (hex/uuid chars, 6+ chars)
    if reference.len() >= 6 && reference.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        if let Ok(matches) = db.search_nodes_by_id_prefix(reference, 10) {
            match matches.len() {
                1 => return ResolveResult::Found(matches.into_iter().next().unwrap()),
                n if n > 1 => {
                    let candidates = matches.iter().map(|n| NodeSummary {
                        id: n.id.clone(),
                        title: n.ai_title.clone().unwrap_or_else(|| n.title.clone()),
                    }).collect();
                    return ResolveResult::Ambiguous(candidates);
                }
                _ => {}
            }
        }
    }

    // Try FTS search
    if let Ok(fts_results) = db.search_nodes(reference) {
        let items: Vec<_> = fts_results.into_iter().filter(|n| n.is_item).collect();
        match items.len() {
            1 => return ResolveResult::Found(items.into_iter().next().unwrap()),
            n if n > 1 => {
                let candidates = items.iter().take(10).map(|n| NodeSummary {
                    id: n.id.clone(),
                    title: n.ai_title.clone().unwrap_or_else(|| n.title.clone()),
                }).collect();
                return ResolveResult::Ambiguous(candidates);
            }
            _ => {}
        }
    }

    // Fallback: title substring LIKE
    if let Ok(substr_results) = db.search_nodes_by_title_substring(reference, 10) {
        match substr_results.len() {
            1 => return ResolveResult::Found(substr_results.into_iter().next().unwrap()),
            n if n > 1 => {
                let candidates = substr_results.iter().map(|n| NodeSummary {
                    id: n.id.clone(),
                    title: n.ai_title.clone().unwrap_or_else(|| n.title.clone()),
                }).collect();
                return ResolveResult::Ambiguous(candidates);
            }
            _ => {}
        }
    }

    ResolveResult::NotFound(format!("No node found matching '{}'", reference))
}

/// Process --connects-to terms: resolve each, create Related edges.
///
/// Returns a result per term so callers can format for their context.
pub fn create_connects_to_edges(
    db: &Database,
    source_node_id: &str,
    terms: &[String],
    author: &str,
) -> Vec<ConnectResult> {
    let now = chrono::Utc::now().timestamp_millis();
    let mut results = Vec::new();

    for term in terms {
        match resolve_node(db, term) {
            ResolveResult::Found(target) => {
                let edge_id = uuid::Uuid::new_v4().to_string();
                let edge = Edge {
                    id: edge_id.clone(),
                    source: source_node_id.to_string(),
                    target: target.id.clone(),
                    edge_type: EdgeType::Related,
                    label: None,
                    weight: Some(1.0),
                    edge_source: Some("user".to_string()),
                    evidence_id: None,
                    confidence: Some(1.0),
                    created_at: now,
                    updated_at: Some(now),
                    author: Some(author.to_string()),
                    reason: Some(format!("Connected via --connects-to '{}'", term)),
                };
                if db.insert_edge(&edge).is_ok() {
                    results.push(ConnectResult::Linked {
                        edge_id,
                        target: NodeSummary {
                            id: target.id.clone(),
                            title: target.ai_title.unwrap_or(target.title),
                        },
                    });
                }
            }
            ResolveResult::Ambiguous(candidates) => {
                results.push(ConnectResult::Ambiguous {
                    term: term.clone(),
                    candidates,
                });
            }
            ResolveResult::NotFound(_) => {
                results.push(ConnectResult::NotFound { term: term.clone() });
            }
        }
    }

    results
}
