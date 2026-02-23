use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Page,
    Thought,
    Context,
    Cluster,
    Paper,     // Scientific paper from OpenAIRE
    Bookmark,  // Web capture from browser extension
}

impl NodeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeType::Page => "page",
            NodeType::Thought => "thought",
            NodeType::Context => "context",
            NodeType::Cluster => "cluster",
            NodeType::Paper => "paper",
            NodeType::Bookmark => "bookmark",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "page" => Some(NodeType::Page),
            "thought" => Some(NodeType::Thought),
            "context" => Some(NodeType::Context),
            "cluster" => Some(NodeType::Cluster),
            "paper" => Some(NodeType::Paper),
            "bookmark" => Some(NodeType::Bookmark),
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
    #[serde(rename = "belongs_to")]
    BelongsTo,  // Item belongs to category (multi-path associations)
    // Code relationships
    Calls,      // function -> function it calls
    #[serde(rename = "uses_type")]
    UsesType,   // function -> struct/enum it references
    Implements, // impl -> trait it implements
    #[serde(rename = "defined_in")]
    DefinedIn,  // code item -> module/file it's defined in
    Imports,    // module -> module it imports
    Tests,      // test function -> function it tests
    Documents,  // doc -> code item it documents (via backtick references)
    // Holerabbit browsing relationships
    Clicked,     // web page -> web page (followed link)
    Backtracked, // web page -> web page (returned via back button)
    #[serde(rename = "session_item")]
    SessionItem, // session -> web page (belongs to session)
    // Category relationships
    Sibling,     // category -> category (sibling relationship based on paper cross-edges)
    // Team mode epistemic edges
    Prerequisite,  // A must be understood before B
    Contradicts,   // A contradicts B (tension edge)
    Supports,      // A provides evidence for B
    #[serde(rename = "evolved_from")]
    EvolvedFrom,   // B is a refined version of A
    Questions,     // A raises a question about B
    // Spore meta edges
    Summarizes,    // summary covers these nodes
    Tracks,        // status node tracks workstream
    Flags,         // contradiction flag on nodes
    Resolves,      // decision resolving a contradiction
    #[serde(rename = "derives_from")]
    DerivesFrom,   // content derived from source
    Supersedes,    // new node/edge replaces old one
    // Signal messaging relationships
    #[serde(rename = "replies_to")]
    RepliesTo,       // message -> message it quotes
    #[serde(rename = "shares_link")]
    SharesLink,      // message -> link node (URL shared in message)
    #[serde(rename = "temporal_thread")]
    TemporalThread,  // consecutive messages within temporal window
}

impl EdgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeType::Reference => "reference",
            EdgeType::Because => "because",
            EdgeType::Related => "related",
            EdgeType::Contains => "contains",
            EdgeType::BelongsTo => "belongs_to",
            EdgeType::Calls => "calls",
            EdgeType::UsesType => "uses_type",
            EdgeType::Implements => "implements",
            EdgeType::DefinedIn => "defined_in",
            EdgeType::Imports => "imports",
            EdgeType::Tests => "tests",
            EdgeType::Documents => "documents",
            EdgeType::Clicked => "clicked",
            EdgeType::Backtracked => "backtracked",
            EdgeType::SessionItem => "session_item",
            EdgeType::Sibling => "sibling",
            EdgeType::Prerequisite => "prerequisite",
            EdgeType::Contradicts => "contradicts",
            EdgeType::Supports => "supports",
            EdgeType::EvolvedFrom => "evolved_from",
            EdgeType::Questions => "questions",
            EdgeType::Summarizes => "summarizes",
            EdgeType::Tracks => "tracks",
            EdgeType::Flags => "flags",
            EdgeType::Resolves => "resolves",
            EdgeType::DerivesFrom => "derives_from",
            EdgeType::Supersedes => "supersedes",
            EdgeType::RepliesTo => "replies_to",
            EdgeType::SharesLink => "shares_link",
            EdgeType::TemporalThread => "temporal_thread",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "reference" => Some(EdgeType::Reference),
            "because" => Some(EdgeType::Because),
            "related" => Some(EdgeType::Related),
            "contains" => Some(EdgeType::Contains),
            "belongs_to" => Some(EdgeType::BelongsTo),
            "calls" => Some(EdgeType::Calls),
            "uses_type" => Some(EdgeType::UsesType),
            "implements" => Some(EdgeType::Implements),
            "defined_in" => Some(EdgeType::DefinedIn),
            "imports" => Some(EdgeType::Imports),
            "tests" => Some(EdgeType::Tests),
            "documents" => Some(EdgeType::Documents),
            "clicked" => Some(EdgeType::Clicked),
            "backtracked" => Some(EdgeType::Backtracked),
            "session_item" => Some(EdgeType::SessionItem),
            "sibling" => Some(EdgeType::Sibling),
            "prerequisite" => Some(EdgeType::Prerequisite),
            "contradicts" => Some(EdgeType::Contradicts),
            "supports" => Some(EdgeType::Supports),
            "evolved_from" => Some(EdgeType::EvolvedFrom),
            "questions" => Some(EdgeType::Questions),
            "summarizes" => Some(EdgeType::Summarizes),
            "tracks" => Some(EdgeType::Tracks),
            "flags" => Some(EdgeType::Flags),
            "resolves" => Some(EdgeType::Resolves),
            "derives_from" => Some(EdgeType::DerivesFrom),
            "supersedes" => Some(EdgeType::Supersedes),
            "replies_to" => Some(EdgeType::RepliesTo),
            "shares_link" => Some(EdgeType::SharesLink),
            "temporal_thread" => Some(EdgeType::TemporalThread),
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
    #[serde(rename = "pdfAvailable")]
    pub pdf_available: Option<bool>,      // For papers: whether PDF is stored in database

    // Content classification (for mini-clustering)
    #[serde(rename = "contentType")]
    pub content_type: Option<String>,     // "idea" | "code" | "debug" | "paste" | NULL
    #[serde(rename = "associatedIdeaId")]
    pub associated_idea_id: Option<String>, // Links supporting item to specific idea node

    // Privacy scoring (continuous scale)
    pub privacy: Option<f64>,             // 0.0 = private, 1.0 = public, NULL = unscored

    // Sovereignty tracking (team mode)
    /// JSON array of field names manually edited, e.g. '["title","parent_id"]'. NULL = never edited.
    #[serde(rename = "humanEdited")]
    pub human_edited: Option<String>,
    #[serde(rename = "humanCreated")]
    pub human_created: bool,              // 1 = manually created by human, never deleted by rebuild
    pub author: Option<String>,           // Who created/last edited this node

    // Spore agent coordination fields
    #[serde(rename = "agentId")]
    pub agent_id: Option<String>,         // 'human', 'spore:summarizer', 'spore:ingestor', etc.
    #[serde(rename = "nodeClass")]
    pub node_class: Option<String>,       // 'knowledge' | 'meta' | 'operational'
    #[serde(rename = "metaType")]
    pub meta_type: Option<String>,        // For meta nodes: 'summary' | 'contradiction' | 'status'
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

    // Team mode fields
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<i64>,
    pub author: Option<String>,
    pub reason: Option<String>,            // Why this edge exists (short provenance)

    // Spore agent coordination fields
    pub content: Option<String>,           // Full reasoning/explanation for this edge
    #[serde(rename = "agentId")]
    pub agent_id: Option<String>,          // 'human', 'spore:synthesizer', etc.
    #[serde(rename = "supersededBy")]
    pub superseded_by: Option<String>,     // Edge ID that replaced this one
    pub metadata: Option<String>,          // JSON blob for extensible properties
}

/// Edge with joined source and target node data (for query-edges results)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeWithNodes {
    #[serde(flatten)]
    pub edge: Edge,
    #[serde(rename = "sourceTitle")]
    pub source_title: Option<String>,
    #[serde(rename = "sourceContent")]
    pub source_content: Option<String>,
    #[serde(rename = "targetTitle")]
    pub target_title: Option<String>,
    #[serde(rename = "targetContent")]
    pub target_content: Option<String>,
}

/// Full edge explanation with surrounding context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeExplanation {
    pub edge: Edge,
    #[serde(rename = "sourceNode")]
    pub source_node: Node,
    #[serde(rename = "targetNode")]
    pub target_node: Node,
    #[serde(rename = "adjacentEdges")]
    pub adjacent_edges: Vec<Edge>,
    #[serde(rename = "supersessionChain")]
    pub supersession_chain: Vec<Edge>,
}

/// Single hop in a path between two nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathHop {
    pub edge: Edge,
    #[serde(rename = "nodeId")]
    pub node_id: String,
    #[serde(rename = "nodeTitle")]
    pub node_title: String,
}

/// A node reached by Dijkstra traversal from a source, with distance metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextNode {
    pub rank: usize,
    #[serde(rename = "nodeId")]
    pub node_id: String,
    #[serde(rename = "nodeTitle")]
    pub node_title: String,
    /// Weighted distance from source (lower = more relevant)
    pub distance: f64,
    /// Relevance score: 1.0 / (1.0 + distance), normalized to 0.0-1.0
    pub relevance: f64,
    /// Number of hops from source
    pub hops: usize,
    /// The path from source to this node
    pub path: Vec<PathHop>,
    #[serde(rename = "nodeClass")]
    pub node_class: Option<String>,
    #[serde(rename = "isItem")]
    pub is_item: bool,
}

/// Summary of an orchestrator run (edges grouped by run_id)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    #[serde(rename = "runId")]
    pub run_id: String,
    #[serde(rename = "startedAt")]
    pub started_at: i64,
    #[serde(rename = "endedAt")]
    pub ended_at: i64,
    #[serde(rename = "edgeCount")]
    pub edge_count: i64,
    pub agents: String,
}

/// Persistent tag for guiding clustering across rebuilds
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: String,
    pub title: String,
    #[serde(rename = "parentTagId")]
    pub parent_tag_id: Option<String>,
    pub depth: i32,
    // Note: centroid stored in DB as BLOB, not loaded into struct (too large)
    #[serde(rename = "itemCount")]
    pub item_count: i32,
    pub pinned: bool,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
}

/// Item-to-tag assignment with confidence score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemTag {
    #[serde(rename = "itemId")]
    pub item_id: String,
    #[serde(rename = "tagId")]
    pub tag_id: String,
    pub confidence: f64,
    pub source: String,  // "ai" or "user"
}

/// Scientific paper metadata from OpenAIRE
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paper {
    pub id: i64,
    #[serde(rename = "nodeId")]
    pub node_id: String,
    #[serde(rename = "openAireId")]
    pub openaire_id: Option<String>,
    pub doi: Option<String>,
    pub authors: Option<String>,        // JSON array
    #[serde(rename = "publicationDate")]
    pub publication_date: Option<String>,
    pub journal: Option<String>,
    pub publisher: Option<String>,
    #[serde(rename = "abstract")]
    pub abstract_text: Option<String>,
    #[serde(rename = "abstractFormatted")]
    pub abstract_formatted: Option<String>,  // Markdown with **Section** headers
    #[serde(rename = "abstractSections")]
    pub abstract_sections: Option<String>,   // JSON array of detected sections
    #[serde(rename = "pdfUrl")]
    pub pdf_url: Option<String>,
    #[serde(rename = "pdfAvailable")]
    pub pdf_available: bool,
    #[serde(rename = "docFormat")]
    pub doc_format: Option<String>,     // "pdf", "docx", "doc", or NULL
    pub subjects: Option<String>,       // JSON array (FOS, keywords)
    #[serde(rename = "accessRight")]
    pub access_right: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
}

/// Author information for papers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperAuthor {
    #[serde(rename = "fullName")]
    pub full_name: String,
    pub orcid: Option<String>,
}

/// Subject/keyword for papers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperSubject {
    pub scheme: String,  // "FOS", "keyword", etc.
    pub value: String,
}

// ============================================================================
// API Keys (team server authentication)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: String,
    pub key_hash: String,
    pub user_name: String,
    pub role: String,  // "admin" or "editor"
    pub created_at: i64,
}
