use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Page,
    Thought,
    Context,
    Cluster,
}

impl NodeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeType::Page => "page",
            NodeType::Thought => "thought",
            NodeType::Context => "context",
            NodeType::Cluster => "cluster",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "page" => Some(NodeType::Page),
            "thought" => Some(NodeType::Thought),
            "context" => Some(NodeType::Context),
            "cluster" => Some(NodeType::Cluster),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EdgeType {
    Reference,
    Because,
    Related,
    Contains,
    BelongsTo,  // Item belongs to category (multi-path associations)
}

impl EdgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeType::Reference => "reference",
            EdgeType::Because => "because",
            EdgeType::Related => "related",
            EdgeType::Contains => "contains",
            EdgeType::BelongsTo => "belongs_to",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "reference" => Some(EdgeType::Reference),
            "because" => Some(EdgeType::Because),
            "related" => Some(EdgeType::Related),
            "contains" => Some(EdgeType::Contains),
            "belongs_to" => Some(EdgeType::BelongsTo),
            _ => None,
        }
    }
}

// Dynamic hierarchy - no fixed level constants
// depth: 0 = Universe (root), increases toward items
// is_item: true = openable content (conversations, notes, etc.)
// is_universe: true = single root node

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub title: String,  // Raw title (original from import)
    pub url: Option<String>,
    pub content: Option<String>,  // Raw content (original from import)
    pub position: Position,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
    #[serde(rename = "clusterId")]
    pub cluster_id: Option<i32>,  // Temporary: used during clustering, cleared after hierarchy build
    #[serde(rename = "clusterLabel")]
    pub cluster_label: Option<String>,

    // Dynamic hierarchy fields
    pub depth: i32,                      // 0 = Universe, increases toward items
    #[serde(rename = "isItem")]
    pub is_item: bool,                   // true = openable in Leaf reader
    #[serde(rename = "isUniverse")]
    pub is_universe: bool,               // true = root node (exactly one)
    #[serde(rename = "parentId")]
    pub parent_id: Option<String>,       // Parent node ID (null for Universe)
    #[serde(rename = "childCount")]
    pub child_count: i32,                // Number of direct children

    // AI-processed fields
    #[serde(rename = "aiTitle")]
    pub ai_title: Option<String>,  // AI-generated clean title
    pub summary: Option<String>,   // AI-generated summary
    pub tags: Option<String>,      // JSON array of tags
    pub emoji: Option<String>,     // Topic emoji (AI-suggested or matched)
    #[serde(rename = "isProcessed")]
    pub is_processed: bool,        // Whether AI has processed this node

    // Note: Embeddings stored in DB but not loaded into Node struct (too large)
    // Use get_node_embedding() / get_nodes_with_embeddings() for similarity search

    // Conversation context fields (for message Leafs)
    #[serde(rename = "conversationId")]
    pub conversation_id: Option<String>,  // ID of parent conversation this message belongs to
    #[serde(rename = "sequenceIndex")]
    pub sequence_index: Option<i32>,      // Position in original conversation (0, 1, 2...)

    // Quick access fields (for Sidebar)
    #[serde(rename = "isPinned")]
    pub is_pinned: bool,                  // User-pinned favorite
    #[serde(rename = "lastAccessedAt")]
    pub last_accessed_at: Option<i64>,    // For recency tracking in sidebar

    // Hierarchy date propagation
    #[serde(rename = "latestChildDate")]
    pub latest_child_date: Option<i64>,   // MAX(children's created_at), bubbled up from leaves

    // Privacy filtering
    #[serde(rename = "isPrivate")]
    pub is_private: Option<bool>,         // None = not scanned, Some(true) = private, Some(false) = safe
    #[serde(rename = "privacyReason")]
    pub privacy_reason: Option<String>,   // Why it was marked private (for review)

    // Import source tracking
    pub source: Option<String>,           // "claude", "googlekeep", "markdown", etc.

    // Content classification (for mini-clustering)
    #[serde(rename = "contentType")]
    pub content_type: Option<String>,     // "idea" | "code" | "debug" | "paste" | NULL
    #[serde(rename = "associatedIdeaId")]
    pub associated_idea_id: Option<String>, // Links supporting item to specific idea node
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(rename = "type")]
    pub edge_type: EdgeType,
    pub label: Option<String>,
    pub weight: Option<f64>,  // Association strength (0.0 to 1.0) for multi-path edges
    #[serde(rename = "edgeSource")]
    pub edge_source: Option<String>,  // 'ai', 'user', or NULL for legacy - tracks origin for re-clustering
    #[serde(rename = "evidenceId")]
    pub evidence_id: Option<String>,  // Node ID that explains WHY this edge exists
    pub confidence: Option<f64>,      // How certain we are (0.0-1.0), distinct from weight
    #[serde(rename = "createdAt")]
    pub created_at: i64,
}
