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

    // Truncate content for API efficiency
    let content_preview = if content.len() > 3000 {
        &content[..3000]
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
                    format!("{}...", &item.content[..800])
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
                    format!("{}...", &item.content[..800])
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
    ];
    common.iter().any(|c| c.eq_ignore_ascii_case(word))
}

/// Topic info for grouping (label + item count)
#[derive(Debug, Clone)]
pub struct TopicInfo {
    #[allow(dead_code)]
    pub id: String,
    pub label: String,
    pub item_count: i32,
}

/// Result of topic grouping - a parent category with its children
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryGrouping {
    pub name: String,
    pub description: Option<String>,
    #[serde(alias = "topics")]  // Accept both "children" and "topics" from AI
    pub children: Vec<String>,  // Topic labels that belong to this category
}

/// Group topics into 5-12 parent categories using AI
/// Used for recursive hierarchy building when a level has too many children
/// Preserves project/product names as umbrella categories
pub async fn group_topics_into_categories(
    topics: &[TopicInfo],
    context: &GroupingContext,
) -> Result<Vec<CategoryGrouping>, String> {
    let api_key = get_api_key().ok_or("ANTHROPIC_API_KEY not set")?;

    if topics.is_empty() {
        return Ok(vec![]);
    }

    // Detect project names from topic labels
    let detected_projects = detect_project_names(topics);
    let projects_hint = if detected_projects.is_empty() {
        String::new()
    } else {
        format!(
            "\nDETECTED PROJECT/PRODUCT NAMES (use as categories): {}",
            detected_projects.join(", ")
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
{projects_hint}

YOUR TASK:
Create 5-12 SUB-CATEGORIES for the {count} topics listed below.

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
   - Target 5-8 categories, not 15

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
        count = topics.len(),
        topics_section = topics_section
    );

    // Use Sonnet for hierarchy grouping - this is a one-time operation that
    // defines the entire UX structure and requires nuanced semantic understanding
    let request = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 4000,  // More tokens for potentially large topic lists
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

    let text = api_response
        .content
        .first()
        .map(|c| c.text.clone())
        .unwrap_or_default();

    parse_category_groupings(&text, topics)
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
            format!("{}...", &first_child[..22])
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
        // Capitalize first letter
        let capitalized = format!("{}{}", word[..1].to_uppercase(), &word[1..]);
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
        &text[..30000]
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
