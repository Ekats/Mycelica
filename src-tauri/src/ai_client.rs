//! Anthropic Claude API client for AI-powered node processing
//!
//! Provides title, summary, and tag generation for conversation nodes.

use serde::{Deserialize, Serialize};
use crate::settings;

/// Result of AI analysis for a node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiAnalysisResult {
    pub title: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub emoji: String,
}

/// Anthropic API message format
#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

/// Anthropic API request format
#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<Message>,
}

/// Anthropic API response format
#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    text: String,
}

/// Check if AI features are available (API key is set)
pub fn is_available() -> bool {
    settings::has_api_key()
}

/// Get the API key from settings (checks env var first, then stored setting)
fn get_api_key() -> Option<String> {
    settings::get_api_key()
}

/// Analyze a node's content to generate title, summary, and tags
pub async fn analyze_node(raw_title: &str, content: &str) -> Result<AiAnalysisResult, String> {
    let api_key = get_api_key().ok_or("ANTHROPIC_API_KEY not set")?;

    // Truncate content for API efficiency (find safe UTF-8 boundary)
    let content_preview = if content.len() > 3000 {
        // Find a safe char boundary at or before 3000
        let mut end = 3000;
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        &content[..end]
    } else {
        content
    };

    let prompt = format!(
        r#"Analyze this conversation content and provide a structured analysis.

ORIGINAL TITLE: {}

CONTENT:
{}

Provide the following in JSON format:
1. "title": A concise, descriptive title (5-10 words) - if the original title is good, keep it; otherwise create a better one
2. "summary": A brief summary (50-100 words) of the main topic and key points
3. "tags": 3-5 specific tags (technologies, concepts, task types)
4. "emoji": A single emoji that best represents the topic (e.g., 🐍 for Python, 🦀 for Rust, 🤖 for AI, 🔧 for debugging)

Be specific with tags - use actual technology names (Python, React), specific concepts (state management), or task types (debugging).
Choose an emoji that captures the primary topic or technology being discussed.

Respond ONLY with valid JSON, no markdown:
{{"title": "...", "summary": "...", "tags": ["tag1", "tag2", "tag3"], "emoji": "🔮"}}"#,
        raw_title, content_preview
    );

    let request = AnthropicRequest {
        model: "claude-3-5-haiku-20241022".to_string(),
        max_tokens: 400,
        messages: vec![Message {
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

    let text = api_response
        .content
        .first()
        .map(|c| c.text.clone())
        .unwrap_or_default();

    // Parse JSON response
    parse_ai_response(&text, raw_title)
}

/// Parse the AI response JSON into structured data
fn parse_ai_response(text: &str, fallback_title: &str) -> Result<AiAnalysisResult, String> {
    // Try to extract JSON from the response (handle potential markdown wrapping)
    let json_text = if text.starts_with("```") {
        text.lines()
            .skip(1)
            .take_while(|l| !l.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.to_string()
    };

    // Try to parse as JSON
    match serde_json::from_str::<serde_json::Value>(&json_text) {
        Ok(json) => {
            let title = json
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or(fallback_title)
                .to_string();

            let summary = json
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let tags: Vec<String> = json
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let emoji = json
                .get("emoji")
                .and_then(|v| v.as_str())
                .unwrap_or("💭")
                .to_string();

            Ok(AiAnalysisResult { title, summary, tags, emoji })
        }
        Err(_) => {
            // Fallback: use raw title and extract what we can
            Ok(AiAnalysisResult {
                title: fallback_title.to_string(),
                summary: String::new(),
                tags: vec![],
                emoji: "💭".to_string(),
            })
        }
    }
}

// ==================== AI Clustering ====================

/// Result of AI clustering for a single item (single-cluster assignment)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterAssignment {
    pub item_id: String,
    pub cluster_id: i32,
    pub cluster_label: String,
    #[serde(default)]
    pub is_new_cluster: bool,
}

// ==================== Multi-Path Clustering ====================

/// Single cluster assignment with strength (for multi-path associations)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterWithStrength {
    pub id: i32,
    pub label: String,
    pub strength: f64,  // 0.0 to 1.0
    #[serde(default)]
    pub is_new: bool,
}

/// Multi-cluster assignment for one item (brain-like associations)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiClusterAssignment {
    pub item_id: String,
    pub clusters: Vec<ClusterWithStrength>,
}

/// Existing cluster info for AI context
#[derive(Debug, Clone)]
pub struct ExistingCluster {
    pub id: i32,
    pub label: String,
    pub count: i32,
}

/// Item to be clustered
#[derive(Debug, Clone)]
pub struct ClusterItem {
    pub id: String,
    pub title: String,
    pub content: String,
    pub ai_title: Option<String>,
    pub summary: Option<String>,
    pub tags: Option<String>,
}

/// Cluster items using Claude AI
/// Returns cluster assignments for each item
#[allow(dead_code)]
pub async fn cluster_items_with_ai(
    items: &[ClusterItem],
    existing_clusters: &[ExistingCluster],
    next_cluster_id: i32,
) -> Result<Vec<ClusterAssignment>, String> {
    let api_key = get_api_key().ok_or("ANTHROPIC_API_KEY not set")?;

    if items.is_empty() {
        return Ok(vec![]);
    }

    // Build existing clusters section
    let clusters_section = if existing_clusters.is_empty() {
        "None yet - create new categories as needed.".to_string()
    } else {
        existing_clusters
            .iter()
            .map(|c| format!("- [id: {}] \"{}\" ({} items)", c.id, c.label, c.count))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Build items section - prefer AI-processed summary when available
    let items_section = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let display_text = if let Some(summary) = &item.summary {
                format!("{} | {} | Tags: {}",
                    item.ai_title.as_deref().unwrap_or(&item.title),
                    summary,
                    item.tags.as_deref().unwrap_or("none"))
            } else {
                let content_preview = if item.content.len() > 800 {
                    let mut end = 800;
                    while end > 0 && !item.content.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}...", &item.content[..end])
                } else {
                    item.content.clone()
                };
                format!("{} | {}", item.title, content_preview.replace('\n', " ").trim())
            };
            format!("{}. [{}] {}", i + 1, item.id, display_text)
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let prompt = format!(
        r#"You are organizing a knowledge base. Given these items and existing categories, assign each item to the best category or suggest a new one.

EXISTING CATEGORIES:
{clusters_section}

ITEMS TO CATEGORIZE:
{items_section}

RULES:
1. Prefer assigning to existing categories when they fit well
2. Create new categories only when items don't fit existing ones
3. New cluster IDs should start from {next_cluster_id}
4. Category names should be 2-4 words, specific and descriptive (e.g., "Rust Development", "AI Integration", "Browser Extensions")
5. Avoid generic names like "General" or "Other" - find a meaningful grouping
6. If an item truly doesn't fit anywhere, use cluster_id: -1 with label: "Miscellaneous"

Return ONLY valid JSON array, no markdown:
[
  {{"item_id": "uuid-1", "cluster_id": 0, "cluster_label": "Existing Category"}},
  {{"item_id": "uuid-2", "cluster_id": {next_cluster_id}, "cluster_label": "New Category Name", "is_new_cluster": true}}
]"#,
        clusters_section = clusters_section,
        items_section = items_section,
        next_cluster_id = next_cluster_id
    );

    let request = AnthropicRequest {
        model: "claude-3-5-haiku-20241022".to_string(),
        max_tokens: 2000,
        messages: vec![Message {
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

    let text = api_response
        .content
        .first()
        .map(|c| c.text.clone())
        .unwrap_or_default();

    // Parse the clustering response
    parse_clustering_response(&text, items)
}

/// Parse the AI clustering response
#[allow(dead_code)]
fn parse_clustering_response(text: &str, items: &[ClusterItem]) -> Result<Vec<ClusterAssignment>, String> {
    // Remove markdown wrapping if present
    let json_text = if text.starts_with("```") {
        text.lines()
            .skip(1)
            .take_while(|l| !l.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.to_string()
    };

    // Parse JSON array
    match serde_json::from_str::<Vec<ClusterAssignment>>(&json_text) {
        Ok(assignments) => Ok(assignments),
        Err(e) => {
            eprintln!("Failed to parse clustering response: {}\nResponse: {}", e, text);
            // Fallback: assign all items to Miscellaneous
            Ok(items
                .iter()
                .map(|item| ClusterAssignment {
                    item_id: item.id.clone(),
                    cluster_id: -1,
                    cluster_label: "Miscellaneous".to_string(),
                    is_new_cluster: false,
                })
                .collect())
        }
    }
}

// ==================== Multi-Path AI Clustering ====================

/// Cluster items with AI using multi-path associations
/// Returns 1-4 category assignments per item with confidence scores
pub async fn cluster_items_with_ai_multipath(
    items: &[ClusterItem],
    existing_clusters: &[ExistingCluster],
    next_cluster_id: i32,
) -> Result<Vec<MultiClusterAssignment>, String> {
    let api_key = get_api_key().ok_or("ANTHROPIC_API_KEY not set")?;

    if items.is_empty() {
        return Ok(vec![]);
    }

    // Build existing clusters section
    let clusters_section = if existing_clusters.is_empty() {
        "None yet - create new categories as needed.".to_string()
    } else {
        existing_clusters
            .iter()
            .map(|c| format!("- [id: {}] \"{}\" ({} items)", c.id, c.label, c.count))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Build items section - prefer AI-processed summary when available
    let items_section = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            // Use AI-processed content if available, otherwise fall back to raw content
            let display_text = if let Some(summary) = &item.summary {
                // AI has processed this item - use clean summary
                format!("{} | {} | Tags: {}",
                    item.ai_title.as_deref().unwrap_or(&item.title),
                    summary,
                    item.tags.as_deref().unwrap_or("none"))
            } else {
                // Not processed - use truncated raw content
                let content_preview = if item.content.len() > 800 {
                    let mut end = 800;
                    while end > 0 && !item.content.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}...", &item.content[..end])
                } else {
                    item.content.clone()
                };
                format!("{} | {}", item.title, content_preview.replace('\n', " ").trim())
            };
            format!("{}. [{}] {}", i + 1, item.id, display_text)
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let prompt = format!(
        r#"You are organizing a knowledge base with brain-like multi-category associations.
Each item can belong to MULTIPLE categories with varying strengths.

EXISTING CATEGORIES:
{clusters_section}

ITEMS TO CATEGORIZE:
{items_section}

RULES:
1. Assign 1-4 relevant categories per item with confidence scores (0.0 to 1.0)
2. Primary category should have highest strength (0.7-1.0)
3. Secondary categories can have lower strengths (0.3-0.7)
4. Only include weak associations (0.1-0.3) if genuinely relevant
5. Prefer existing categories when they fit; create new ones when needed
6. New cluster IDs should start from {next_cluster_id}
7. Category names: 2-4 words, specific (e.g., "Rust Development", "AI Integration")
8. If an item truly doesn't fit anywhere, use id: -1 with label: "Miscellaneous" and strength: 1.0

Return ONLY valid JSON array, no markdown:
[
  {{
    "item_id": "uuid-1",
    "clusters": [
      {{"id": 0, "label": "Rust Development", "strength": 0.9}},
      {{"id": 2, "label": "AI Integration", "strength": 0.6, "is_new": true}}
    ]
  }},
  {{
    "item_id": "uuid-2",
    "clusters": [
      {{"id": {next_cluster_id}, "label": "New Category", "strength": 0.85, "is_new": true}}
    ]
  }}
]"#,
        clusters_section = clusters_section,
        items_section = items_section,
        next_cluster_id = next_cluster_id
    );

    let request = AnthropicRequest {
        model: "claude-3-5-haiku-20241022".to_string(),
        max_tokens: 3000,  // More tokens for multi-cluster output
        messages: vec![Message {
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

    let text = api_response
        .content
        .first()
        .map(|c| c.text.clone())
        .unwrap_or_default();

    // Parse the multi-path clustering response
    parse_multipath_response(&text, items)
}

/// Parse the multi-path AI clustering response
fn parse_multipath_response(text: &str, items: &[ClusterItem]) -> Result<Vec<MultiClusterAssignment>, String> {
    // Remove markdown wrapping if present
    let json_text = if text.starts_with("```") {
        text.lines()
            .skip(1)
            .take_while(|l| !l.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.to_string()
    };

    // Parse JSON array
    match serde_json::from_str::<Vec<MultiClusterAssignment>>(&json_text) {
        Ok(assignments) => {
            // Validate and normalize strengths
            let normalized: Vec<MultiClusterAssignment> = assignments
                .into_iter()
                .map(|mut a| {
                    // Ensure at least one cluster per item
                    if a.clusters.is_empty() {
                        a.clusters.push(ClusterWithStrength {
                            id: -1,
                            label: "Miscellaneous".to_string(),
                            strength: 1.0,
                            is_new: false,
                        });
                    }
                    // Sort by strength (highest first)
                    a.clusters.sort_by(|x, y| y.strength.partial_cmp(&x.strength).unwrap_or(std::cmp::Ordering::Equal));
                    // Clamp strengths to 0.0-1.0
                    for c in &mut a.clusters {
                        c.strength = c.strength.clamp(0.0, 1.0);
                    }
                    a
                })
                .collect();
            Ok(normalized)
        }
        Err(e) => {
            eprintln!("Failed to parse multi-path clustering response: {}\nResponse: {}", e, text);
            // Fallback: assign all items to Miscellaneous with single association
            Ok(items
                .iter()
                .map(|item| MultiClusterAssignment {
                    item_id: item.id.clone(),
                    clusters: vec![ClusterWithStrength {
                        id: -1,
                        label: "Miscellaneous".to_string(),
                        strength: 1.0,
                        is_new: false,
                    }],
                })
                .collect())
        }
    }
}

// ==================== Recursive Hierarchy Grouping ====================

/// Context for AI grouping - provides hierarchy awareness
#[derive(Debug, Clone)]
pub struct GroupingContext {
    /// Name of the parent node being subdivided
    pub parent_name: String,
    /// Description/summary of parent (if available)
    pub parent_description: Option<String>,
    /// Full path from Universe to parent: ["Universe", "Programming", ...]
    pub hierarchy_path: Vec<String>,
    /// Current depth in hierarchy (0 = Universe)
    pub current_depth: i32,
    /// Sibling category names at the same level (only these are forbidden for duplicates)
    pub sibling_names: Vec<String>,
    /// All names already used in hierarchy (informational only - same name allowed in different branches)
    #[allow(dead_code)]
    pub forbidden_names: Vec<String>,
    /// Embedding-detected clusters that MUST be kept together as umbrella categories
    pub mandatory_clusters: Vec<DetectedProjectCluster>,
}

/// Detect likely project/product names from topic labels
/// Looks for recurring prefixes, capitalized proper nouns, etc.
fn detect_project_names(topics: &[TopicInfo]) -> Vec<String> {
    use std::collections::HashMap;

    let mut word_counts: HashMap<String, i32> = HashMap::new();

    for topic in topics {
        // Split on common delimiters and take first word
        let label = &topic.label;

        // Check for patterns like "Mycelica: Architecture" or "Mycelica - Frontend"
        for delimiter in &[":", "-", "/", "—", "–"] {
            if let Some(prefix) = label.split(delimiter).next() {
                let prefix = prefix.trim();
                // Must be 2+ chars, start with uppercase, not be a common word
                if prefix.len() >= 2
                    && prefix.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                    && !is_common_word(prefix)
                {
                    *word_counts.entry(prefix.to_string()).or_insert(0) += 1;
                }
            }
        }

        // Also check first word if it's capitalized and repeated
        if let Some(first_word) = label.split_whitespace().next() {
            if first_word.len() >= 3
                && first_word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                && !is_common_word(first_word)
            {
                *word_counts.entry(first_word.to_string()).or_insert(0) += 1;
            }
        }
    }

    // Return words that appear 3+ times (likely project names)
    let mut projects: Vec<_> = word_counts
        .into_iter()
        .filter(|(_, count)| *count >= 3)
        .map(|(name, _)| name)
        .collect();

    projects.sort();
    projects
}

/// Check if a word is too common to be a project name
fn is_common_word(word: &str) -> bool {
    let common = [
        "The", "This", "That", "What", "How", "Why", "When", "Where",
        "New", "Old", "First", "Last", "Some", "All", "Any", "More",
        "Using", "Building", "Creating", "Making", "Adding", "Fixing",
        "About", "With", "From", "Into", "Over", "Under", "Between",
        "Discussion", "Conversation", "Chat", "Talk", "Help", "Question",
        "Code", "Debug", "Error", "Issue", "Bug", "Feature", "Update",
        "Project", "Development", "Implementation", "Design", "Architecture",
        "System", "Application", "Service", "Module", "Component",
        "Claude", "Assistant", "User", "Human", "Model", "API",
    ];
    common.iter().any(|c| c.eq_ignore_ascii_case(word))
}

/// A major project detected from global item title frequency
#[derive(Debug, Clone)]
pub struct MajorProject {
    /// Project name (proper noun)
    pub name: String,
    /// Number of items containing this name
    pub item_count: usize,
    /// Percentage of total items
    pub percentage: f32,
    /// Item IDs containing this project name
    pub item_ids: Vec<String>,
}

/// Detect major projects from global proper noun frequency in item titles
///
/// Scans ALL items' ai_title fields for proper nouns (capitalized, 3+ chars).
/// Returns nouns appearing in 2%+ of items (min 20 occurrences).
/// Filters out "title-starter" words that appear >70% as first word (e.g., "Strategy", "Analysis").
/// These represent major projects that should become umbrella categories.
pub fn detect_major_projects_globally(db: &crate::db::Database) -> Vec<MajorProject> {
    use std::collections::HashMap;

    // Track noun statistics including position
    #[derive(Default)]
    struct NounStats {
        item_ids: Vec<String>,
        first_word_count: usize,
        total_count: usize,
    }

    // Get all items
    let items = match db.get_items() {
        Ok(items) => items,
        Err(e) => {
            eprintln!("[MajorProjects] Failed to get items: {}", e);
            return vec![];
        }
    };

    if items.is_empty() {
        return vec![];
    }

    let total_items = items.len();

    // Count proper nouns across all item titles, tracking position
    let mut noun_stats: HashMap<String, NounStats> = HashMap::new();

    for item in &items {
        // Use ai_title if available, fallback to title
        let title = item.ai_title.as_ref().unwrap_or(&item.title);

        // Also check cluster_label for additional signal
        let texts = if let Some(label) = &item.cluster_label {
            vec![title.as_str(), label.as_str()]
        } else {
            vec![title.as_str()]
        };

        // Track which nouns we've seen in THIS item (avoid double counting)
        let mut seen_in_item: std::collections::HashSet<String> = std::collections::HashSet::new();

        for text in texts {
            let words: Vec<&str> = text.split_whitespace().collect();
            for (pos, word) in words.iter().enumerate() {
                // Clean punctuation
                let word = word.trim_matches(|c: char| !c.is_alphanumeric());

                // Must be proper noun: capitalized, 3+ chars, not common
                if word.len() >= 3
                    && word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                    && !is_common_word(word)
                    && !seen_in_item.contains(word)
                {
                    seen_in_item.insert(word.to_string());
                    let stats = noun_stats.entry(word.to_string()).or_default();
                    stats.item_ids.push(item.id.clone());
                    stats.total_count += 1;
                    if pos == 0 {
                        stats.first_word_count += 1;
                    }
                }
            }
        }
    }

    // Filter to nouns appearing in 2%+ of items (min 20 occurrences)
    // Also filter out title-starters (>70% appear as first word)
    let min_percentage = 0.02;
    let min_occurrences = 20;
    let threshold = ((total_items as f64 * min_percentage).ceil() as usize).max(min_occurrences);

    let mut projects: Vec<MajorProject> = Vec::new();

    // Check if word has generic noun suffix (not a project name)
    fn is_generic_noun_suffix(word: &str) -> bool {
        let lower = word.to_lowercase();
        lower.ends_with("tion") ||  // Configuration, Communication
        lower.ends_with("ment") ||  // Management, Development
        lower.ends_with("ness") ||  // Awareness
        lower.ends_with("ity") ||   // Complexity, Activity
        lower.ends_with("ence") ||  // Experience, Reference
        lower.ends_with("ance") ||  // Performance, Guidance
        lower.ends_with("ing") ||   // Processing, Debugging, Mapping
        lower.ends_with("ics") ||   // Techniques, Graphics
        lower.ends_with("sis") ||   // Analysis
        lower.ends_with("egy")      // Strategy
    }

    for (name, stats) in noun_stats {
        if stats.item_ids.len() < threshold {
            continue;
        }

        // Filter out generic noun suffixes (not project names)
        if is_generic_noun_suffix(&name) {
            eprintln!("[MajorProjects] Filtered '{}' (generic noun suffix, {} items)",
                name, stats.item_ids.len());
            continue;
        }

        // Filter out title-starters: words that appear >75% as first word
        let first_word_ratio = stats.first_word_count as f32 / stats.total_count as f32;
        if first_word_ratio > 0.75 {
            eprintln!("[MajorProjects] Filtered '{}' ({:.0}% first-word, {} items)",
                name, first_word_ratio * 100.0, stats.item_ids.len());
            continue;
        }

        let item_count = stats.item_ids.len();
        let percentage = (item_count as f32 / total_items as f32) * 100.0;
        projects.push(MajorProject {
            name,
            item_count,
            percentage,
            item_ids: stats.item_ids,
        });
    }

    // Sort by item count descending
    projects.sort_by(|a, b| b.item_count.cmp(&a.item_count));

    projects
}

/// Detect project clusters from topic embeddings using pairwise cosine similarity
///
/// Finds tight clusters of semantically related topics that should be grouped together.
/// Uses greedy expansion: start with highly-connected seeds, expand to all topics
/// that have high similarity to ALL current cluster members.
///
/// For naming, extracts proper nouns from BOTH topic labels AND item titles under
/// those topics (where project names like "Mycelica" actually appear).
pub fn detect_project_clusters_from_embeddings(
    db: &crate::db::Database,
    topics: &[TopicInfo],
    embeddings: &[(String, Vec<f32>)],
    min_cluster_size: usize,
    min_avg_similarity: f32,
) -> Vec<DetectedProjectCluster> {
    use crate::similarity::cosine_similarity;
    use std::collections::{HashMap, HashSet};

    if topics.len() < min_cluster_size {
        return vec![];
    }

    // Build embedding lookup
    let embedding_map: HashMap<&str, &Vec<f32>> = embeddings
        .iter()
        .map(|(id, emb)| (id.as_str(), emb))
        .collect();

    // Filter to topics that have embeddings
    let topics_with_emb: Vec<&TopicInfo> = topics
        .iter()
        .filter(|t| embedding_map.contains_key(t.id.as_str()))
        .collect();

    if topics_with_emb.len() < min_cluster_size {
        return vec![];
    }

    // Compute pairwise similarities
    let n = topics_with_emb.len();
    let mut similarities: Vec<Vec<f32>> = vec![vec![0.0; n]; n];

    for i in 0..n {
        similarities[i][i] = 1.0;
        for j in (i + 1)..n {
            let emb_i = embedding_map.get(topics_with_emb[i].id.as_str()).unwrap();
            let emb_j = embedding_map.get(topics_with_emb[j].id.as_str()).unwrap();
            let sim = cosine_similarity(emb_i, emb_j);
            similarities[i][j] = sim;
            similarities[j][i] = sim;
        }
    }

    // Find tight clusters using greedy expansion
    let mut used: HashSet<usize> = HashSet::new();
    let mut clusters: Vec<DetectedProjectCluster> = vec![];

    // Sort topics by how many high-similarity neighbors they have (connectivity)
    let mut connectivity: Vec<(usize, usize)> = (0..n)
        .map(|i| {
            let count = (0..n)
                .filter(|&j| i != j && similarities[i][j] >= min_avg_similarity)
                .count();
            (i, count)
        })
        .collect();
    connectivity.sort_by(|a, b| b.1.cmp(&a.1));

    // Expansion tolerance: allow slightly lower similarity when expanding
    let expansion_threshold = min_avg_similarity - 0.10;

    for (seed_idx, _) in connectivity {
        if used.contains(&seed_idx) {
            continue;
        }

        // Find all topics with high similarity to seed
        let mut cluster_indices: Vec<usize> = vec![seed_idx];

        for j in 0..n {
            if j != seed_idx && !used.contains(&j) {
                // Check similarity to ALL current cluster members
                let all_similar = cluster_indices
                    .iter()
                    .all(|&k| similarities[j][k] >= expansion_threshold);

                if all_similar {
                    cluster_indices.push(j);
                }
            }
        }

        // Only keep if cluster is large enough
        if cluster_indices.len() >= min_cluster_size {
            // Calculate actual average similarity
            let mut total_sim = 0.0;
            let mut pairs = 0;
            for i in 0..cluster_indices.len() {
                for j in (i + 1)..cluster_indices.len() {
                    total_sim += similarities[cluster_indices[i]][cluster_indices[j]];
                    pairs += 1;
                }
            }
            let avg_sim = if pairs > 0 {
                total_sim / pairs as f32
            } else {
                0.0
            };

            if avg_sim >= min_avg_similarity {
                // Mark as used
                for &idx in &cluster_indices {
                    used.insert(idx);
                }

                // Extract topic info
                let topic_ids: Vec<String> = cluster_indices
                    .iter()
                    .map(|&i| topics_with_emb[i].id.clone())
                    .collect();
                let topic_labels: Vec<String> = cluster_indices
                    .iter()
                    .map(|&i| topics_with_emb[i].label.clone())
                    .collect();

                // Get item titles from topics in this cluster (where project names actually appear)
                let item_titles = collect_item_titles_from_topics(db, &topic_ids);

                // Extract proper nouns from BOTH topic labels AND item titles
                let proper_nouns = extract_proper_nouns_from_texts(&topic_labels, &item_titles);
                let name = generate_cluster_name(&topic_labels, &item_titles, &proper_nouns);

                clusters.push(DetectedProjectCluster {
                    name,
                    topic_ids,
                    topic_labels,
                    avg_similarity: avg_sim,
                });
            }
        }
    }

    clusters
}

/// Collect item titles from topics (items are children of topics where is_item=true)
fn collect_item_titles_from_topics(db: &crate::db::Database, topic_ids: &[String]) -> Vec<String> {
    let mut titles = Vec::new();

    for topic_id in topic_ids {
        if let Ok(children) = db.get_children(topic_id) {
            for child in children {
                if child.is_item {
                    // Prefer ai_title, fallback to title
                    let title = child.ai_title.unwrap_or(child.title);
                    titles.push(title);
                }
            }
        }
    }

    titles
}

/// Extract proper nouns appearing in 50%+ of labels (legacy, for backward compat)
#[allow(dead_code)]
fn extract_proper_nouns_from_labels(labels: &[String]) -> Vec<String> {
    use std::collections::HashMap;

    let mut noun_counts: HashMap<String, usize> = HashMap::new();

    for label in labels {
        for word in label.split_whitespace() {
            // Must start with uppercase, be 3+ chars, not be a common word
            if word.len() >= 3
                && word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                && !is_common_word(word)
            {
                *noun_counts.entry(word.to_string()).or_insert(0) += 1;
            }
        }
    }

    // Return nouns appearing in 50%+ of labels
    let threshold = (labels.len() as f64 * 0.5).ceil() as usize;
    let mut nouns: Vec<String> = noun_counts
        .into_iter()
        .filter(|(_, count)| *count >= threshold.max(2)) // At least 2 occurrences
        .map(|(noun, _)| noun)
        .collect();

    nouns.sort();
    nouns
}

/// Extract proper nouns from both topic labels AND item titles
/// Uses lower threshold for items (15%, min 3) since project names may only appear in some items
fn extract_proper_nouns_from_texts(topic_labels: &[String], item_titles: &[String]) -> Vec<String> {
    use std::collections::HashMap;

    let mut noun_counts: HashMap<String, usize> = HashMap::new();
    let total_texts = topic_labels.len() + item_titles.len();

    // Count proper nouns from topic labels (weight higher since more relevant)
    for label in topic_labels {
        for word in label.split_whitespace() {
            let word = word.trim_matches(|c: char| !c.is_alphanumeric());
            if word.len() >= 3
                && word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                && !is_common_word(word)
            {
                // Topic labels get 3x weight
                *noun_counts.entry(word.to_string()).or_insert(0) += 3;
            }
        }
    }

    // Count proper nouns from item titles
    for title in item_titles {
        for word in title.split_whitespace() {
            let word = word.trim_matches(|c: char| !c.is_alphanumeric());
            if word.len() >= 3
                && word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                && !is_common_word(word)
            {
                *noun_counts.entry(word.to_string()).or_insert(0) += 1;
            }
        }
    }

    // Lower threshold: 15% of total texts, minimum 3 occurrences
    // For 50 items, threshold = max(7.5, 3) = 7.5 → 8 occurrences needed
    let threshold = ((total_texts as f64 * 0.15).ceil() as usize).max(3);

    let mut nouns: Vec<(String, usize)> = noun_counts
        .into_iter()
        .filter(|(_, count)| *count >= threshold)
        .collect();

    // Sort by frequency descending
    nouns.sort_by(|a, b| b.1.cmp(&a.1));
    nouns.into_iter().map(|(noun, _)| noun).collect()
}

/// Generate cluster name from labels and proper nouns (legacy, for backward compat)
#[allow(dead_code)]
fn generate_cluster_name_from_labels(labels: &[String], proper_nouns: &[String]) -> String {
    // If we found a dominant proper noun, use it
    if let Some(noun) = proper_nouns.first() {
        return noun.clone();
    }

    // Otherwise, find common theme words
    let common_words = find_common_words_in_labels(labels);
    if !common_words.is_empty() {
        return common_words.into_iter().take(2).collect::<Vec<_>>().join(" ");
    }

    "Related Topics".to_string()
}

/// Generate cluster name from topic labels, item titles, and extracted proper nouns
/// Prioritizes: proper nouns > common keywords > first significant word from items
fn generate_cluster_name(
    topic_labels: &[String],
    item_titles: &[String],
    proper_nouns: &[String],
) -> String {
    // Priority 1: Use dominant proper noun if found
    if let Some(noun) = proper_nouns.first() {
        return noun.clone();
    }

    // Priority 2: Find common theme words from topic labels
    let common_words = find_common_words_in_labels(topic_labels);
    if !common_words.is_empty() {
        let name = common_words.into_iter().take(2).collect::<Vec<_>>().join(" ");
        if !name.is_empty() {
            return name;
        }
    }

    // Priority 3: Find common keywords from item titles
    let item_keywords = find_common_words_in_labels(item_titles);
    if !item_keywords.is_empty() {
        let name = item_keywords.into_iter().take(2).collect::<Vec<_>>().join(" ");
        if !name.is_empty() {
            return name;
        }
    }

    // Priority 4: Use first significant capitalized word from any item title
    for title in item_titles.iter().chain(topic_labels.iter()) {
        for word in title.split_whitespace() {
            let word = word.trim_matches(|c: char| !c.is_alphanumeric());
            if word.len() >= 4
                && word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                && !is_common_word(word)
            {
                return format!("{} Group", word);
            }
        }
    }

    // Final fallback: Use topic count as identifier (avoids generic "Related Topics")
    format!("Cluster ({})", topic_labels.len())
}

/// Find common words across labels (excluding stopwords)
fn find_common_words_in_labels(labels: &[String]) -> Vec<String> {
    use std::collections::HashMap;

    let stopwords = [
        "the", "a", "an", "and", "or", "in", "of", "to", "for", "with", "on", "at", "by", "from",
        "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do", "does",
        "did", "will", "would", "could", "should", "may", "might", "must", "shall", "can",
        "this", "that", "these", "those", "it", "its", "as", "if", "then", "than", "so",
    ];
    let mut word_counts: HashMap<String, usize> = HashMap::new();

    for label in labels {
        for word in label.to_lowercase().split_whitespace() {
            // Clean punctuation
            let word = word.trim_matches(|c: char| !c.is_alphanumeric());
            if word.len() > 3 && !stopwords.contains(&word) {
                *word_counts.entry(word.to_string()).or_insert(0) += 1;
            }
        }
    }

    let threshold = (labels.len() as f64 * 0.4).ceil() as usize;
    let mut words: Vec<(String, usize)> = word_counts
        .into_iter()
        .filter(|(_, count)| *count >= threshold.max(2))
        .collect();

    words.sort_by(|a, b| b.1.cmp(&a.1));
    words
        .into_iter()
        .map(|(w, _)| capitalize_first_char(&w))
        .collect()
}

/// Capitalize first character of a string
fn capitalize_first_char(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().chain(chars).collect(),
    }
}

/// Topic info for grouping (label + item count)
#[derive(Debug, Clone)]
pub struct TopicInfo {
    #[allow(dead_code)]
    pub id: String,
    pub label: String,
    pub item_count: i32,
}

/// A project cluster detected from embedding similarity
#[derive(Debug, Clone)]
pub struct DetectedProjectCluster {
    /// Cluster name (from proper noun extraction or keywords)
    pub name: String,
    /// Topic IDs belonging to this cluster
    pub topic_ids: Vec<String>,
    /// Topic labels (for prompt inclusion)
    pub topic_labels: Vec<String>,
    /// Average pairwise similarity within cluster
    pub avg_similarity: f32,
}

/// Result of topic grouping - a parent category with its children
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryGrouping {
    pub name: String,
    pub description: Option<String>,
    #[serde(alias = "topics")]  // Accept both "children" and "topics" from AI
    pub children: Vec<String>,  // Topic labels that belong to this category
}

/// OpenAI fallback for grouping when Claude is overloaded
async fn call_openai_grouping(client: &reqwest::Client, prompt: &str) -> Result<String, String> {
    let openai_key = settings::get_openai_api_key()
        .ok_or("OpenAI API key not set - cannot fallback")?;

    #[derive(Serialize)]
    struct OpenAIRequest {
        model: String,
        messages: Vec<OpenAIMessage>,
        max_tokens: u32,
    }

    #[derive(Serialize)]
    struct OpenAIMessage {
        role: String,
        content: String,
    }

    #[derive(Deserialize)]
    struct OpenAIResponse {
        choices: Vec<OpenAIChoice>,
        usage: Option<OpenAIUsage>,
    }

    #[derive(Deserialize)]
    struct OpenAIChoice {
        message: OpenAIMessageContent,
    }

    #[derive(Deserialize)]
    struct OpenAIMessageContent {
        content: String,
    }

    #[derive(Deserialize)]
    struct OpenAIUsage {
        total_tokens: u64,
    }

    let request = OpenAIRequest {
        model: "gpt-4o".to_string(),
        messages: vec![OpenAIMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }],
        max_tokens: 4000,
    };

    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", openai_key))
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("OpenAI request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("OpenAI API error {}: {}", status, body));
    }

    let api_response: OpenAIResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse OpenAI response: {}", e))?;

    // Track token usage
    if let Some(usage) = &api_response.usage {
        let _ = settings::add_openai_tokens(usage.total_tokens);
    }

    api_response
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .ok_or_else(|| "No response from OpenAI".to_string())
}

/// Group topics into parent categories using AI
/// Used for recursive hierarchy building when a level has too many children
/// Preserves project/product names as umbrella categories
/// max_groups: maximum number of categories to create (default 8 if None)
pub async fn group_topics_into_categories(
    topics: &[TopicInfo],
    context: &GroupingContext,
    max_groups: Option<usize>,
) -> Result<Vec<CategoryGrouping>, String> {
    let max_groups = max_groups.unwrap_or(8);
    let api_key = get_api_key().ok_or("ANTHROPIC_API_KEY not set")?;

    if topics.is_empty() {
        return Ok(vec![]);
    }

    // Detect project names from topic labels (heuristic-based)
    let detected_projects = detect_project_names(topics);
    let projects_hint = if detected_projects.is_empty() {
        String::new()
    } else {
        format!(
            "\nDETECTED PROJECT/PRODUCT NAMES (use as categories): {}",
            detected_projects.join(", ")
        )
    };

    // Build mandatory clusters section (embedding-based detection)
    let mandatory_section = if context.mandatory_clusters.is_empty() {
        String::new()
    } else {
        let clusters_text: Vec<String> = context
            .mandatory_clusters
            .iter()
            .map(|c| {
                let topics_preview: String = c
                    .topic_labels
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                let more = if c.topic_labels.len() > 5 {
                    format!(", ... (+{} more)", c.topic_labels.len() - 5)
                } else {
                    String::new()
                };
                format!(
                    "- \"{}\" ({} topics, {:.0}% cohesion): {}{}",
                    c.name,
                    c.topic_ids.len(),
                    c.avg_similarity * 100.0,
                    topics_preview,
                    more
                )
            })
            .collect();

        format!(
            r#"

MANDATORY PROJECT CLUSTERS (CRITICAL - these topics MUST stay together):
{}

RULES FOR MANDATORY CLUSTERS:
- Create a category for each mandatory cluster using its suggested name (or similar)
- ALL topics listed in a mandatory cluster MUST be assigned to that single category
- Do NOT split mandatory cluster topics across different categories
- Do NOT merge multiple mandatory clusters into one category
- These clusters were detected via semantic similarity - trust them
"#,
            clusters_text.join("\n")
        )
    };

    // Build topics list for prompt
    let topics_section = topics
        .iter()
        .map(|t| format!("- \"{}\" ({} items)", t.label, t.item_count))
        .collect::<Vec<_>>()
        .join("\n");

    // Build context sections for prompt
    let hierarchy_path_str = if context.hierarchy_path.is_empty() {
        "Root level".to_string()
    } else {
        context.hierarchy_path.join(" → ")
    };

    let parent_desc_section = context.parent_description
        .as_ref()
        .map(|d| format!("- Parent description: {}", d))
        .unwrap_or_default();

    // Only siblings are truly forbidden (same name OK in different branches)
    let siblings_str = if context.sibling_names.is_empty() {
        "None (this is top level)".to_string()
    } else {
        context.sibling_names.join(", ")
    };

    let prompt = format!(
        r#"You are organizing a knowledge graph hierarchy.

CURRENT POSITION IN HIERARCHY:
- Parent node: "{parent_name}"
- Full path: {hierarchy_path}
- Depth: {depth} (0=Universe, 1=Projects/Domains, 2=Sub-areas, etc.)
{parent_desc_section}

EXISTING SIBLINGS (do not duplicate these names): {siblings}
{projects_hint}{mandatory_section}

YOUR TASK:
Create {max_groups} or fewer SUB-CATEGORIES for the {count} topics listed below.

CRITICAL RULES:

1. PRESERVE PROJECT/PRODUCT NAMES AS TOP-LEVEL CATEGORIES
   - If topics mention "Mycelica", "Ascension", or any named project/product,
     that name becomes a CATEGORY containing all related topics
   - Project names are NAMESPACES, not topics to dissolve into generic buckets
   - "Mycelica Architecture" → goes under "Mycelica" category, not "Architecture"

2. NAME OVERLAP IS ALLOWED ACROSS DIFFERENT BRANCHES
   - "Mycelica/Architecture" and "Ascension/Architecture" can BOTH exist
   - Same conceptual name in different project contexts = separate categories
   - Only avoid duplicating sibling names listed above

3. PREFER BROADER CATEGORIES AT TOP LEVELS
   - Level 1 (under Universe): Projects, Domains, Life Areas
   - Level 2+: Get more specific WITHIN those umbrellas
   - Target 3-{max_groups} categories maximum

4. NO GENERIC CATCH-ALL NAMES:
   - FORBIDDEN: "Other", "Miscellaneous", "General", "Various", "Mixed", "Uncategorized"
   - For orphan topics: find a meaningful connector or use "{parent_name} Tangents"

5. MEANINGFUL SPECIFIC NAMES (2-4 words):
   - BAD: "Other Topics", "Misc Programming"
   - GOOD: "Side Projects", "Exploratory Research", "Legacy Systems"

TOPICS TO ORGANIZE ({count} total):
{topics_section}

Return ONLY valid JSON, no markdown:
{{
  "categories": [
    {{
      "name": "Specific Meaningful Name",
      "description": "What these topics have in common",
      "topics": ["Topic Label 1", "Topic Label 2"]
    }}
  ]
}}"#,
        parent_name = context.parent_name,
        hierarchy_path = hierarchy_path_str,
        depth = context.current_depth,
        parent_desc_section = parent_desc_section,
        siblings = siblings_str,
        projects_hint = projects_hint,
        mandatory_section = mandatory_section,
        count = topics.len(),
        topics_section = topics_section,
        max_groups = max_groups
    );

    // Try Claude first, fall back to OpenAI if overloaded (529)
    let client = reqwest::Client::new();

    // Try Anthropic first
    let request = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 4000,
        messages: vec![Message {
            role: "user".to_string(),
            content: prompt.clone(),
        }],
    };

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&request)
        .send()
        .await;

    let text = match response {
        Ok(resp) if resp.status().is_success() => {
            let api_response: AnthropicResponse = resp
                .json()
                .await
                .map_err(|e| format!("Failed to parse response: {}", e))?;
            if let Some(usage) = &api_response.usage {
                let _ = settings::add_anthropic_tokens(usage.input_tokens, usage.output_tokens);
            }
            api_response.content.first().map(|c| c.text.clone()).unwrap_or_default()
        }
        Ok(resp) if resp.status().as_u16() == 529 => {
            // Claude overloaded - try OpenAI fallback
            println!("[AI] Claude overloaded (529), trying OpenAI fallback...");
            call_openai_grouping(&client, &prompt).await?
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            // Try OpenAI fallback for other errors too
            println!("[AI] Claude error {}, trying OpenAI fallback...", status);
            match call_openai_grouping(&client, &prompt).await {
                Ok(text) => text,
                Err(_) => return Err(format!("API error {}: {}", status, body)),
            }
        }
        Err(e) => {
            // Network error - try OpenAI
            println!("[AI] Claude request failed: {}, trying OpenAI fallback...", e);
            call_openai_grouping(&client, &prompt).await?
        }
    };

    parse_category_groupings(&text, topics)
}

/// Group categories into 4-8 uber-categories, preserving project names
/// Used for consolidating Universe's direct children into navigable top-level domains
/// Projects (Mycelica, etc.) stay as their own categories; generic topics get grouped by theme
pub async fn group_into_uber_categories(
    categories: &[TopicInfo],
) -> Result<Vec<CategoryGrouping>, String> {
    let api_key = get_api_key().ok_or("ANTHROPIC_API_KEY not set")?;

    if categories.is_empty() {
        return Ok(vec![]);
    }

    // Build categories list for prompt
    let categories_section = categories
        .iter()
        .map(|c| format!("- \"{}\" ({} items)", c.label, c.item_count))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        r#"You are consolidating a knowledge graph's top-level categories into 4-8 uber-categories.

CURRENT CATEGORIES ({count} total):
{categories_section}

YOUR TASK:
Group these into 4-8 top-level categories that make the graph navigable.

CRITICAL RULES:

1. PRESERVE PROJECT/PRODUCT NAMES AS THEIR OWN CATEGORIES
   - If you see "Mycelica", "Ascension", or ANY named project/product/app,
     that name MUST become its own top-level category
   - Project names are NAMESPACES - never dissolve them into generic buckets
   - "Mycelica UI", "Mycelica Backend", "Mycelica Architecture" → ALL go under "Mycelica"
   - Even single-topic projects stay as their own category

2. GROUP NON-PROJECT CATEGORIES BY THEME
   - Categories that aren't project-specific CAN be grouped into broader themes
   - Use meaningful 1-3 word names (not forced single-word ALL-CAPS)
   - Examples: "Philosophy & Mind", "Technical Learning", "Personal Growth"

3. NAMING RULES:
   - Project names: Keep exact name (e.g., "Mycelica", "Ascension")
   - Theme groups: 1-3 descriptive words (e.g., "AI & Machine Learning", "Life & Health")
   - FORBIDDEN: "Other", "Miscellaneous", "General", "Various", "Mixed"

4. TARGET 4-8 CATEGORIES:
   - Each project gets its own category (even if small)
   - Non-project categories can be merged into themes
   - Result should have 4-8 total uber-categories

5. WHAT COUNTS AS A PROJECT:
   - Named software/apps (Mycelica, VSCode extensions, specific tools)
   - Named creative works (book titles, game names)
   - Named initiatives or ventures
   - NOT generic topics like "Programming" or "AI Research"

Return ONLY valid JSON, no markdown:
{{
  "categories": [
    {{
      "name": "Mycelica",
      "description": "Knowledge graph application development",
      "children": ["Mycelica UI", "Mycelica Backend", "Mycelica Architecture"]
    }},
    {{
      "name": "AI & Machine Learning",
      "description": "Artificial intelligence concepts and research",
      "children": ["Neural Networks", "LLM Research", "ML Infrastructure"]
    }}
  ]
}}
"#,
        count = categories.len(),
        categories_section = categories_section
    );

    // Try Claude first, fall back to OpenAI if overloaded
    let client = reqwest::Client::new();

    let request = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 2000,
        messages: vec![Message {
            role: "user".to_string(),
            content: prompt.clone(),
        }],
    };

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&request)
        .send()
        .await;

    let text = match response {
        Ok(resp) if resp.status().is_success() => {
            let api_response: AnthropicResponse = resp
                .json()
                .await
                .map_err(|e| format!("Failed to parse response: {}", e))?;
            if let Some(usage) = &api_response.usage {
                let _ = settings::add_anthropic_tokens(usage.input_tokens, usage.output_tokens);
            }
            api_response.content.first().map(|c| c.text.clone()).unwrap_or_default()
        }
        Ok(resp) if resp.status().as_u16() == 529 => {
            println!("[AI] Claude overloaded (529), trying OpenAI fallback...");
            call_openai_grouping(&client, &prompt).await?
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            println!("[AI] Claude error {}, trying OpenAI fallback...", status);
            match call_openai_grouping(&client, &prompt).await {
                Ok(text) => text,
                Err(_) => return Err(format!("API error {}: {}", status, body)),
            }
        }
        Err(e) => {
            println!("[AI] Claude request failed: {}, trying OpenAI fallback...", e);
            call_openai_grouping(&client, &prompt).await?
        }
    };

    parse_category_groupings(&text, categories)
}

/// Check if a category name is a generic catch-all that should be renamed
fn is_generic_category_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    let generic_patterns = [
        "other", "misc", "diverse", "various", "uncategorized",
        "unclassified", "general", "remaining", "leftover"
    ];
    generic_patterns.iter().any(|p| lower.contains(p))
}

/// Generate a meaningful name from child topics
/// Uses multiple strategies to create a better name than generic "Other"
fn generate_meaningful_name(children: &[String], index: usize) -> String {
    if children.is_empty() {
        return format!("Exploratory Set {}", index + 1);
    }

    // Strategy 1: Find common word prefix among children
    if children.len() >= 2 {
        if let Some(common) = find_common_theme(children) {
            return common;
        }
    }

    // Strategy 2: Use first 2-3 child names for context
    let sample: Vec<_> = children.iter().take(3).collect();
    if sample.len() >= 2 {
        // Extract key words from first two children
        let words1: Vec<_> = sample[0].split_whitespace().take(2).collect();
        let words2: Vec<_> = sample[1].split_whitespace().take(2).collect();

        // Look for overlapping concepts
        for w1 in &words1 {
            for w2 in &words2 {
                if w1.to_lowercase() == w2.to_lowercase() && w1.len() > 3 {
                    return format!("{} Related Topics", w1);
                }
            }
        }
    }

    // Strategy 3: First child name truncated (last resort)
    if let Some(first_child) = children.first() {
        let short_name = if first_child.len() > 25 {
            let mut end = 22;
            while end > 0 && !first_child.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &first_child[..end])
        } else {
            first_child.clone()
        };
        format!("{} and Related", short_name)
    } else {
        format!("Exploratory Set {}", index + 1)
    }
}

/// Try to find a common theme among child topic names
fn find_common_theme(children: &[String]) -> Option<String> {
    use std::collections::HashMap;

    // Count word frequency across all children (excluding stopwords)
    let stopwords = ["the", "a", "an", "and", "or", "in", "of", "to", "for", "with", "on", "at", "by", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do", "does", "did", "will", "would", "could", "should", "may", "might", "must", "shall", "topics", "items", "related"];

    let mut word_counts: HashMap<String, usize> = HashMap::new();

    for child in children {
        for word in child.split_whitespace() {
            let lower = word.to_lowercase();
            // Skip short words and stopwords
            if lower.len() > 3 && !stopwords.contains(&lower.as_str()) {
                *word_counts.entry(lower).or_insert(0) += 1;
            }
        }
    }

    // Find most common word that appears in at least 40% of children
    let threshold = (children.len() as f64 * 0.4).ceil() as usize;
    let mut candidates: Vec<_> = word_counts.into_iter()
        .filter(|(_, count)| *count >= threshold)
        .collect();

    candidates.sort_by(|a, b| b.1.cmp(&a.1));

    if let Some((word, _)) = candidates.first() {
        // Capitalize first letter (safe for UTF-8)
        let mut chars = word.chars();
        let capitalized = match chars.next() {
            Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
            None => word.clone(),
        };
        return Some(format!("{} Discussions", capitalized));
    }

    None
}

/// Deduplicate category names within a batch
/// If "Mental Health Support" appears 3 times, rename to:
/// "Mental Health Support", "Mental Health Support (2)", "Mental Health Support (3)"
fn deduplicate_category_names(categories: &mut [CategoryGrouping]) {
    use std::collections::HashMap;

    // Count occurrences of each name
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    for cat in categories.iter() {
        *name_counts.entry(cat.name.clone()).or_insert(0) += 1;
    }

    // Track how many times we've seen each duplicate name
    let mut seen_counts: HashMap<String, usize> = HashMap::new();

    for cat in categories.iter_mut() {
        let count = name_counts.get(&cat.name).copied().unwrap_or(1);
        if count > 1 {
            let seen = seen_counts.entry(cat.name.clone()).or_insert(0);
            *seen += 1;
            if *seen > 1 {
                let old_name = cat.name.clone();
                cat.name = format!("{} ({})", old_name, seen);
                println!("Deduplicating category name: '{}' -> '{}'", old_name, cat.name);
            }
        }
    }
}

/// Parse the AI category grouping response
fn parse_category_groupings(text: &str, topics: &[TopicInfo]) -> Result<Vec<CategoryGrouping>, String> {
    // Remove markdown wrapping if present
    let json_text = if text.starts_with("```") {
        text.lines()
            .skip(1)
            .take_while(|l| !l.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.to_string()
    };

    // Parse JSON
    #[derive(Deserialize)]
    struct GroupingResponse {
        categories: Vec<CategoryGrouping>,
    }

    match serde_json::from_str::<GroupingResponse>(&json_text) {
        Ok(response) => {
            // Post-process: rename any generic category names
            let mut categories: Vec<CategoryGrouping> = response.categories
                .into_iter()
                .enumerate()
                .map(|(i, mut cat)| {
                    if is_generic_category_name(&cat.name) {
                        let new_name = generate_meaningful_name(&cat.children, i);
                        println!("Renaming generic category '{}' -> '{}'", cat.name, new_name);
                        cat.name = new_name;
                    }
                    cat
                })
                .collect();

            // Deduplicate names within this batch (e.g., AI returned "Mental Health" 3x)
            deduplicate_category_names(&mut categories);

            // Validate that all topics are assigned
            let assigned: std::collections::HashSet<String> = categories
                .iter()
                .flat_map(|c| c.children.iter().cloned())
                .collect();

            let all_topics: std::collections::HashSet<String> = topics
                .iter()
                .map(|t| t.label.clone())
                .collect();

            // Check for unassigned topics
            let unassigned: Vec<String> = all_topics.difference(&assigned).cloned().collect();

            if !unassigned.is_empty() {
                // Generate a meaningful name for unassigned topics
                let catchall_name = generate_meaningful_name(&unassigned, categories.len());
                categories.push(CategoryGrouping {
                    name: catchall_name,
                    description: Some("Topics awaiting better categorization".to_string()),
                    children: unassigned,
                });
            }

            Ok(categories)
        }
        Err(e) => {
            eprintln!("Failed to parse category groupings: {}\nResponse: {}", e, text);
            // Fallback: put all topics in one category with meaningful name
            let fallback_name = generate_meaningful_name(
                &topics.iter().map(|t| t.label.clone()).collect::<Vec<_>>(),
                0
            );
            Ok(vec![CategoryGrouping {
                name: fallback_name,
                description: Some("Fallback grouping - parsing failed".to_string()),
                children: topics.iter().map(|t| t.label.clone()).collect(),
            }])
        }
    }
}

// ==================== OpenAI Embeddings ====================

/// OpenAI embeddings API request format
#[derive(Debug, Serialize)]
struct OpenAiEmbeddingRequest {
    model: String,
    input: String,
}

/// OpenAI embeddings API response format
#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<EmbeddingData>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    total_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

/// Check if embeddings are available (OpenAI API key is set)
pub fn embeddings_available() -> bool {
    settings::has_openai_api_key()
}

/// Generate an embedding for text using OpenAI's text-embedding-3-small model
/// Returns a 1536-dimensional vector
pub async fn generate_embedding(text: &str) -> Result<Vec<f32>, String> {
    let api_key = settings::get_openai_api_key()
        .ok_or("OPENAI_API_KEY not set")?;

    // Truncate text if too long (roughly 8000 tokens ≈ 32000 chars)
    let text_to_embed = if text.len() > 30000 {
        let mut end = 30000;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        &text[..end]
    } else {
        text
    };

    let request = OpenAiEmbeddingRequest {
        model: "text-embedding-3-small".to_string(),
        input: text_to_embed.to_string(),
    };

    let client = reqwest::Client::new();
    let response = client
        .post("https://api.openai.com/v1/embeddings")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("Embedding HTTP request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("OpenAI embedding API error {}: {}", status, body));
    }

    let api_response: OpenAiEmbeddingResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse embedding response: {}", e))?;

    // Track token usage
    if let Some(usage) = &api_response.usage {
        let _ = settings::add_openai_tokens(usage.total_tokens);
    }

    api_response.data
        .into_iter()
        .next()
        .map(|d| d.embedding)
        .ok_or_else(|| "No embedding in response".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ai_response() {
        let json = r#"{"title": "Test Title", "summary": "A test summary", "tags": ["rust", "testing"]}"#;
        let result = parse_ai_response(json, "Fallback").unwrap();
        assert_eq!(result.title, "Test Title");
        assert_eq!(result.summary, "A test summary");
        assert_eq!(result.tags, vec!["rust", "testing"]);
    }

    #[test]
    fn test_parse_ai_response_with_markdown() {
        let json = "```json\n{\"title\": \"Test\", \"summary\": \"Sum\", \"tags\": [\"a\"]}\n```";
        let result = parse_ai_response(json, "Fallback").unwrap();
        assert_eq!(result.title, "Test");
    }

    #[test]
    fn test_parse_clustering_response() {
        let json = r#"[{"item_id": "abc", "cluster_id": 0, "cluster_label": "Test"}]"#;
        let items = vec![ClusterItem {
            id: "abc".to_string(),
            title: "Test".to_string(),
            content: "Content".to_string(),
            ai_title: None,
            summary: None,
            tags: None,
        }];
        let result = parse_clustering_response(json, &items).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].cluster_id, 0);
        assert_eq!(result[0].cluster_label, "Test");
    }

    #[test]
    fn test_parse_category_groupings() {
        let json = r#"{"categories": [{"name": "Programming", "children": ["Rust", "Python"]}]}"#;
        let topics = vec![
            TopicInfo { id: "1".to_string(), label: "Rust".to_string(), item_count: 5 },
            TopicInfo { id: "2".to_string(), label: "Python".to_string(), item_count: 3 },
        ];
        let result = parse_category_groupings(json, &topics).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "Programming");
        assert_eq!(result[0].children.len(), 2);
    }
}
