//! Anthropic Claude API client for AI-powered node processing
//!
//! Provides title, summary, and tag generation for conversation nodes.

use serde::{Deserialize, Serialize};
use crate::settings;
use crate::local_embeddings;
use crate::classification::{classify_content, ContentType};

/// Result of AI analysis for a node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiAnalysisResult {
    pub title: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub content_type: String,  // idea, code, debug, paste, trivial
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
///
/// Uses tiered processing:
/// - HIDDEN items (debug, code, paste, trivial): Skip API, extract simple title
/// - VISIBLE/SUPPORTING items: Full API analysis
pub async fn analyze_node(raw_title: &str, content: &str) -> Result<AiAnalysisResult, String> {
    // === Tiered processing: classify first to determine processing depth ===
    let content_type = classify_content(content);

    // HIDDEN tier: skip expensive API call, just extract title
    if content_type.is_hidden() {
        return Ok(extract_basic_metadata(raw_title, content, content_type));
    }

    // VISIBLE/SUPPORTING: proceed with full API analysis
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
        r#"Analyze this conversation and provide JSON:

CONTENT:
{}

1. "content_type": Classify by PRIMARY purpose:

VISIBLE (original thought):
- "insight": Realization, conclusion, crystallized understanding
  Language: "I realized", "the answer is", "so basically", "the key is"
- "exploration": Researching, thinking out loud, no firm conclusion
  Language: "what if", "let me try", "I wonder", "maybe"
- "synthesis": Summarizing, connecting previous understanding
  Language: "to summarize", "overall", "the pattern is"
- "question": Inquiry that frames investigation
- "planning": Roadmaps, TODOs, intentions

SUPPORTING (lower weight):
- "investigation": Problem-solving focused on fixing
  Language: "the issue was", "turns out", "fixed by"
- "discussion": Back-and-forth Q&A without synthesis
- "reference": Factual lookup, definitions, external info
- "creative": Fiction, poetry, roleplay

HIDDEN:
- "debug": Error messages, stack traces, build failures
- "code": Code blocks, implementations
- "paste": Logs, terminal output, data dumps
- "trivial": Greetings, acknowledgments, fragments

2. "title": 3-6 words capturing the insight/topic
3. "summary": 50-100 words
4. "tags": 3-5 specific tags

JSON only: {{"content_type":"...","title":"...","summary":"...","tags":[...]}}"#,
        content_preview
    );

    let request = AnthropicRequest {
        model: "claude-haiku-4-5-20251001".to_string(),
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

            let content_type = json
                .get("content_type")
                .and_then(|v| v.as_str())
                .unwrap_or("exploration")  // Default matches pattern matcher
                .to_string();

            Ok(AiAnalysisResult { title, summary, tags, content_type })
        }
        Err(_) => {
            // Fallback: use raw title and extract what we can
            Ok(AiAnalysisResult {
                title: fallback_title.to_string(),
                summary: String::new(),
                tags: vec![],
                content_type: "exploration".to_string(),  // Default matches pattern matcher
            })
        }
    }
}

// ==================== Cheap AI Classification ====================

/// Classify content type using minimal AI prompt - CHEAP (~$0.00001 per item)
///
/// Only returns content_type, no title/summary/tags.
/// Uses ~50 tokens per item with Haiku = ~$0.04 for 4000 items.
pub async fn classify_content_ai(content: &str) -> Result<String, String> {
    let api_key = get_api_key().ok_or("ANTHROPIC_API_KEY not set")?;

    // Truncate content for API efficiency
    let content_preview = if content.len() > 1500 {
        let mut end = 1500;
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        &content[..end]
    } else {
        content
    };

    let prompt = format!(
        r#"Classify this content. Return ONLY one word from this list:

VISIBLE: insight, exploration, synthesis, question, planning
SUPPORTING: investigation, discussion, reference, creative
HIDDEN: debug, code, paste, trivial

Content:
{}

Classification:"#,
        content_preview
    );

    let request = AnthropicRequest {
        model: "claude-haiku-4-5-20251001".to_string(),
        max_tokens: 20,  // Minimal - just one word
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
        .map(|c| c.text.trim().to_lowercase())
        .unwrap_or_else(|| "exploration".to_string());

    // Validate and normalize the response
    let valid_types = [
        "insight", "exploration", "synthesis", "question", "planning",
        "investigation", "discussion", "reference", "creative",
        "debug", "code", "paste", "trivial"
    ];

    if valid_types.contains(&text.as_str()) {
        Ok(text)
    } else {
        // Default to exploration for unrecognized responses
        Ok("exploration".to_string())
    }
}

/// Batch classify multiple items using AI - CHEAP
/// Returns a map of item_id -> content_type
pub async fn classify_batch_ai(items: &[(String, String)]) -> Result<Vec<(String, String)>, String> {
    let mut results = Vec::new();

    for (id, content) in items {
        match classify_content_ai(content).await {
            Ok(content_type) => results.push((id.clone(), content_type)),
            Err(e) => {
                eprintln!("[AI Classify] Failed for {}: {}", id, e);
                // Use pattern matcher fallback
                let fallback = classify_content(content);
                results.push((id.clone(), fallback.as_str().to_string()));
            }
        }
    }

    Ok(results)
}

/// Extract basic metadata for HIDDEN items without API call
/// Returns a simple title and the pre-classified content type
fn extract_basic_metadata(raw_title: &str, content: &str, content_type: ContentType) -> AiAnalysisResult {
    // Clean up raw title or generate from content
    let title = if raw_title.trim().is_empty() || raw_title.starts_with("claude_") {
        // Generate title from first meaningful line of content
        extract_title_from_content(content, content_type)
    } else {
        // Clean up existing title
        clean_title(raw_title)
    };

    AiAnalysisResult {
        title,
        summary: String::new(),  // No summary for HIDDEN
        tags: vec![],            // No tags for HIDDEN
        content_type: content_type.as_str().to_string(),
    }
}

/// Extract a reasonable title from content for HIDDEN items
fn extract_title_from_content(content: &str, content_type: ContentType) -> String {
    // Type-specific prefixes for context
    let prefix = match content_type {
        ContentType::Debug => "Debug:",
        ContentType::Code => "Code:",
        ContentType::Paste => "Paste:",
        ContentType::Trivial => "",
        _ => "",  // Shouldn't reach here for HIDDEN
    };

    // Find first meaningful line
    let first_line = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() &&
            !trimmed.starts_with("```") &&
            !trimmed.starts_with('#') &&
            !trimmed.starts_with("//") &&
            trimmed.len() > 3
        })
        .next()
        .unwrap_or("Untitled");

    // Truncate to reasonable length
    let truncated = if first_line.len() > 60 {
        let mut end = 57;
        while end > 0 && !first_line.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &first_line[..end])
    } else {
        first_line.to_string()
    };

    if prefix.is_empty() {
        truncated
    } else {
        format!("{} {}", prefix, truncated)
    }
}

/// Clean up a raw title (remove file extensions, clean special chars)
fn clean_title(raw: &str) -> String {
    let mut title = raw.to_string();

    // Remove common file extensions from import filenames
    for ext in &[".json", ".txt", ".md", ".html"] {
        if title.ends_with(ext) {
            title = title[..title.len() - ext.len()].to_string();
        }
    }

    // Replace underscores with spaces
    title = title.replace('_', " ");

    // Truncate if too long
    if title.len() > 80 {
        let mut end = 77;
        while end > 0 && !title.is_char_boundary(end) {
            end -= 1;
        }
        title = format!("{}...", &title[..end]);
    }

    title
}

// ==================== Multi-Path Clustering Data Structures ====================

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

// NOTE: Old AI clustering code has been removed.
// Clustering now uses embedding similarity (see clustering.rs::cluster_with_embeddings).
// AI is only used for naming clusters after they form (see name_clusters below).

// ==================== Cluster Naming ====================

/// Name clusters using AI (single call for all clusters)
/// Takes cluster IDs with sample member titles, returns cluster ID -> name mappings
pub async fn name_clusters(
    clusters: &[(i32, Vec<String>)],  // (cluster_id, member_titles)
) -> Result<Vec<(i32, String)>, String> {
    let api_key = get_api_key().ok_or("ANTHROPIC_API_KEY not set")?;

    if clusters.is_empty() {
        return Ok(vec![]);
    }

    // Build prompt with cluster samples
    let clusters_section: String = clusters
        .iter()
        .map(|(id, titles)| {
            let titles_preview = titles
                .iter()
                .take(10)
                .map(|t| format!("\"{}\"", t.chars().take(60).collect::<String>()))
                .collect::<Vec<_>>()
                .join(", ");
            format!("Cluster {}: [{}]", id, titles_preview)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        r#"Name these topic clusters based on their member items. For each cluster, provide a specific, descriptive name (2-4 words).

CLUSTERS TO NAME:
{clusters_section}

RULES:
1. Names should be specific and descriptive (e.g., "Rust Development", "AI Integration", "Browser Extensions")
2. Avoid generic names like "General", "Other", "Miscellaneous", "Various"
3. Focus on the common theme across the items
4. Use proper nouns when items share a project/product name (e.g., "Mycelica Development")

Return ONLY valid JSON, no markdown:
{{"clusters": [{{"id": 1, "name": "Specific Name"}}, {{"id": 2, "name": "Another Name"}}]}}"#,
        clusters_section = clusters_section
    );

    let request = AnthropicRequest {
        model: "claude-haiku-4-5-20251001".to_string(),
        max_tokens: 2000,  // 30 clusters * ~50 chars each = 1500+ chars needed
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

    // Parse response
    parse_cluster_names_response(&text, clusters)
}

/// Parse the cluster naming response
fn parse_cluster_names_response(
    text: &str,
    clusters: &[(i32, Vec<String>)],
) -> Result<Vec<(i32, String)>, String> {
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

    // Try to parse JSON
    match serde_json::from_str::<serde_json::Value>(&json_text) {
        Ok(json) => {
            let mut results = Vec::new();
            if let Some(clusters_arr) = json.get("clusters").and_then(|v| v.as_array()) {
                for item in clusters_arr {
                    if let (Some(id), Some(name)) = (
                        item.get("id").and_then(|v| v.as_i64()),
                        item.get("name").and_then(|v| v.as_str()),
                    ) {
                        results.push((id as i32, name.to_string()));
                    }
                }
            }
            Ok(results)
        }
        Err(e) => {
            eprintln!("Failed to parse cluster names response: {}\nResponse: {}", e, text);
            // Fallback: return generic names
            Ok(clusters
                .iter()
                .map(|(id, _)| (*id, format!("Cluster {}", id)))
                .collect())
        }
    }
}

/// Ask AI to name a cluster based on sample item titles (for embedding clustering)
pub async fn name_cluster_from_samples(
    titles: &[String],
    forbidden_names: &[String],
) -> Result<String, String> {
    let api_key = get_api_key().ok_or("ANTHROPIC_API_KEY not set")?;

    if titles.is_empty() {
        return Err("No titles provided".into());
    }

    let forbidden_str = if forbidden_names.is_empty() {
        "None".to_string()
    } else {
        forbidden_names.iter().take(20).cloned().collect::<Vec<_>>().join(", ")
    };

    let titles_list = titles.iter()
        .take(12)
        .map(|t| format!("- {}", t.chars().take(80).collect::<String>()))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        r#"These items belong to the same semantic cluster. Generate a 2-4 word category name.

Items:
{titles_list}

Forbidden names (already used): {forbidden_str}

RULES:
1. Be specific and descriptive (e.g., "Rust Development", "Game Physics", "API Design")
2. Avoid generic names like "General", "Various", "Miscellaneous", "Items"
3. Focus on the common theme across items
4. Respond with ONLY the category name, nothing else"#,
        titles_list = titles_list,
        forbidden_str = forbidden_str
    );

    let request = AnthropicRequest {
        model: "claude-haiku-4-5-20251001".to_string(),
        max_tokens: 50,
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

    let name = api_response
        .content
        .first()
        .map(|c| c.text.trim().to_string())
        .unwrap_or_default();

    // Validate
    if name.len() < 3 || name.len() > 50 {
        return Err(format!("Invalid name length: {} chars", name.len()));
    }

    Ok(name)
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
        // Articles, pronouns, question words
        "The", "This", "That", "What", "How", "Why", "When", "Where",
        "New", "Old", "First", "Last", "Some", "All", "Any", "More",
        // Action verbs (often start titles)
        "Using", "Building", "Creating", "Making", "Adding", "Fixing",
        "Working", "Testing", "Running", "Setting", "Getting", "Trying",
        "Looking", "Finding", "Checking", "Moving", "Starting", "Finishing",
        // Prepositions
        "About", "With", "From", "Into", "Over", "Under", "Between",
        // Generic tech terms
        "Discussion", "Conversation", "Chat", "Talk", "Help", "Question",
        "Code", "Debug", "Error", "Issue", "Bug", "Feature", "Update",
        "Project", "Development", "Implementation", "Design", "Architecture",
        "System", "Application", "Service", "Module", "Component",
        "Claude", "Assistant", "User", "Human", "Model", "API",
        // Category descriptors (not project names)
        "Technical", "Scientific", "Audio", "Digital", "Visual",
        "Mixed", "Diverse", "Various", "General", "Miscellaneous",
        "Personal", "Professional", "Creative", "Academic", "Experimental",
        "Interdisciplinary", "Language", "Estonian", "Regional",
        "Theoretical", "Practical",
        // STATUS/PROGRESS WORDS - common false positives for project detection
        // These often appear capitalized at title start: "Progress on X", "Update about Y"
        "Progress", "Status", "Update", "Updates", "Work", "Changes", "Change",
        "Fix", "Fixes", "Problem", "Problems", "Solution", "Solutions",
        "Notes", "Note", "Ideas", "Idea", "Thoughts", "Thought",
        "Summary", "Overview", "Review", "Analysis", "Research",
        "Plan", "Plans", "Planning", "Task", "Tasks", "Todo", "Todos",
        "Draft", "Drafts", "Version", "Versions", "Revision", "Revisions",
        "Session", "Sessions", "Meeting", "Meetings", "Discussion",
        "Exploration", "Exploring", "Investigation", "Investigating",
        "Debugging", "Troubleshooting", "Refactoring", "Optimization",
        // Common English words that slip through
        "Today", "Yesterday", "Tomorrow", "Week", "Month", "Year",
        "Morning", "Evening", "Night", "Day", "Time", "Date",
        "Part", "Parts", "Section", "Sections", "Chapter", "Chapters",
        "Step", "Steps", "Phase", "Phases", "Stage", "Stages",
        "Test", "Tests", "Example", "Examples", "Sample", "Samples",
        "Data", "Info", "Information", "Details", "Context", "Background",
        "Quick", "Simple", "Basic", "Advanced", "Final", "Initial",
        "Main", "Core", "Key", "Important", "Critical", "Essential",
    ];
    common.iter().any(|c| c.eq_ignore_ascii_case(word))
}

/// A major project detected from item titles
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

/// Candidate word collected from titles (before AI filtering)
#[derive(Debug, Clone)]
pub struct CandidateWord {
    pub word: String,
    pub count: usize,
    pub item_ids: Vec<String>,
}

/// Collect ALL capitalized words from item titles
///
/// Very loose collection - just capitalized words with 5+ occurrences.
/// No heuristic filtering - let AI decide what's a project.
pub fn collect_capitalized_words(db: &crate::db::Database) -> Vec<CandidateWord> {
    use std::collections::HashMap;

    // Get all items (not just visible - projects might span content types)
    let items = match db.get_items() {
        Ok(items) => items,
        Err(e) => {
            eprintln!("[ProjectDetection] Failed to get items: {}", e);
            return vec![];
        }
    };

    if items.is_empty() {
        return vec![];
    }

    // Count all capitalized words
    let mut word_stats: HashMap<String, Vec<String>> = HashMap::new();

    for item in &items {
        // Use ai_title if available, fallback to title
        let title = item.ai_title.as_ref().unwrap_or(&item.title);

        // Track which words we've seen in THIS item (avoid double counting)
        let mut seen_in_item: std::collections::HashSet<String> = std::collections::HashSet::new();

        for word in title.split_whitespace() {
            // Clean punctuation
            let word = word.trim_matches(|c: char| !c.is_alphanumeric());

            // Must be: capitalized, 2+ chars
            if word.len() >= 2
                && word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                && !seen_in_item.contains(word)
            {
                seen_in_item.insert(word.to_string());
                word_stats.entry(word.to_string())
                    .or_default()
                    .push(item.id.clone());
            }
        }
    }

    // Return words with 3+ occurrences, sorted by count descending
    // Lowered from 5 to catch smaller projects like crDroid customizations
    let mut candidates: Vec<CandidateWord> = word_stats
        .into_iter()
        .filter(|(_, ids)| ids.len() >= 3)
        .map(|(word, item_ids)| CandidateWord {
            word,
            count: item_ids.len(),
            item_ids,
        })
        .collect();

    candidates.sort_by(|a, b| b.count.cmp(&a.count));

    eprintln!("[ProjectDetection] Collected {} capitalized words with 3+ occurrences", candidates.len());
    candidates
}

/// Detect user's software projects using AI
///
/// Sends all capitalized words to Haiku, which identifies actual project names.
/// Returns MajorProject structs for projects the user built.
pub async fn detect_projects_with_ai(
    db: &crate::db::Database,
    candidates: Vec<CandidateWord>,
) -> Vec<MajorProject> {
    if candidates.is_empty() {
        return vec![];
    }

    // Get API key
    let api_key = match get_api_key() {
        Some(key) => key,
        None => {
            eprintln!("[ProjectDetection] No API key, skipping project detection");
            return vec![];
        }
    };

    // Get total item count for percentage calculation
    let total_items = db.get_items().map(|i| i.len()).unwrap_or(1);

    // Build the word list for AI (top 100 by count)
    let mut word_list = String::new();
    for candidate in candidates.iter().take(100) {
        word_list.push_str(&format!("{}: {} occurrences\n", candidate.word, candidate.count));
    }

    let prompt = format!(r#"Capitalized words from a developer's knowledge base:

{}
Which are SOFTWARE PROJECTS the user BUILT or CUSTOMIZED (not libraries, not generic words)?

Include:
- Apps, tools, or systems the user created
- Custom ROMs or Android forks (crDroid, LineageOS builds)
- Firmware customizations (Klipper configs, custom firmware)
- Game mods or projects they actively develop

Exclude:
- Generic platform names (Android, Linux, Windows)
- Libraries they just USE (React, Rust, Python)
- Single generic words (Code, App, System)

Return ONLY a raw JSON array. No markdown. No code fences. No explanation.
Correct: ["Mycelica", "crDroid", "Klipper"]
If none: []

WRONG (will break parsing):
- ```json ["..."] ```
- Any text before or after the array"#, word_list);

    let request = AnthropicRequest {
        model: "claude-haiku-4-5-20251001".to_string(),
        max_tokens: 200,
        messages: vec![Message {
            role: "user".to_string(),
            content: prompt,
        }],
    };

    let client = reqwest::Client::new();
    let response = match client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&request)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[ProjectDetection] HTTP error: {}", e);
            return vec![];
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        eprintln!("[ProjectDetection] API error {}: {}", status, body);
        return vec![];
    }

    let api_response: AnthropicResponse = match response.json().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[ProjectDetection] Parse error: {}", e);
            return vec![];
        }
    };

    // Track token usage
    if let Some(usage) = &api_response.usage {
        let _ = crate::settings::add_anthropic_tokens(usage.input_tokens, usage.output_tokens);
    }

    // Parse the JSON array response
    let raw_text = api_response
        .content
        .first()
        .map(|c| c.text.trim())
        .unwrap_or("[]");

    // Strip markdown fences if present (AI sometimes wraps in ```json ... ```)
    let text = if raw_text.starts_with("```") {
        raw_text.lines()
            .skip(1)
            .take_while(|l| !l.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        raw_text.to_string()
    };

    // Find the JSON array brackets - extract only the [...] part
    let text = text.trim();
    let json_array = match (text.find('['), text.rfind(']')) {
        (Some(start), Some(end)) if start < end => &text[start..=end],
        _ => {
            eprintln!("[ProjectDetection] No valid JSON array found in response: {}",
                      &raw_text[..raw_text.len().min(100)]);
            return vec![];
        }
    };

    // Parse JSON - NO FALLBACK (the comma-split fallback caused garbage)
    let project_names: Vec<String> = match serde_json::from_str::<Vec<String>>(json_array) {
        Ok(names) => names,
        Err(e) => {
            eprintln!("[ProjectDetection] JSON parse failed: {} - response: {}",
                      e, &raw_text[..raw_text.len().min(200)]);
            return vec![]; // Return empty, don't try to salvage garbage
        }
    };

    eprintln!("[ProjectDetection] AI detected {} projects: {:?}", project_names.len(), project_names);

    // Build MajorProject structs from AI-detected names
    let mut projects: Vec<MajorProject> = Vec::new();

    for name in project_names {
        // Find the candidate with this name (case-insensitive)
        if let Some(candidate) = candidates.iter().find(|c| c.word.eq_ignore_ascii_case(&name)) {
            let percentage = (candidate.count as f32 / total_items as f32) * 100.0;
            projects.push(MajorProject {
                name: candidate.word.clone(), // Use original casing
                item_count: candidate.count,
                percentage,
                item_ids: candidate.item_ids.clone(),
            });
        }
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

5. NAMING PRINCIPLES (quality over length):

   a) MAXIMIZE INFORMATION DENSITY
      - Every word must add meaning. No filler words.
      - BAD: "Technical Implementation Details" → GOOD: "React Hooks"

   b) AVOID ANCESTOR WORDS
      - Never repeat words already in the path: {hierarchy_path}
      - Find synonyms or go more specific

   c) DIFFERENTIATE FROM SIBLINGS
      - Your names must be distinct from: {siblings}
      - Each category occupies unique semantic space

   d) PREFER CONCRETE NOUNS
      - Concrete: "API Endpoints", "Database Migrations", "Auth Tokens"
      - NOT abstract scaffolding: "Core Concepts", "Key Areas", "Main Topics"
      - Name tells WHAT's inside, not that something IS inside

   e) DEPTH-APPROPRIATE SPECIFICITY
      - Depth 1: Broad domains — "AI & Data", "Career", "Creative"
      - Depth 2-3: Sub-areas — "Machine Learning", "Interview Prep"
      - Depth 4+: Specific — "BERT Fine-tuning", "System Design"

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

/// Maximum categories per uber-grouping batch to prevent response truncation
/// Smaller batches = more coherent groupings (merge logic handles cross-batch duplicates)
const UBER_GROUPING_BATCH_SIZE: usize = 40;

/// Sort topics by embedding similarity using greedy nearest-neighbor
/// Returns sorted TopicInfo vector so adjacent topics are semantically related
///
/// embeddings_map: Pre-fetched embeddings keyed by topic ID (avoids async lock issues)
fn sort_by_embedding_similarity(
    categories: &[TopicInfo],
    embeddings_map: &std::collections::HashMap<String, Vec<f32>>,
) -> Vec<TopicInfo> {
    use crate::similarity::cosine_similarity;

    // Get embeddings for all categories from the pre-fetched map
    let embeddings: Vec<Option<&Vec<f32>>> = categories
        .iter()
        .map(|c| embeddings_map.get(&c.id))
        .collect();

    // Count how many have embeddings
    let with_embeddings = embeddings.iter().filter(|e| e.is_some()).count();
    if with_embeddings < categories.len() / 2 {
        println!("[AI] Only {}/{} topics have embeddings, using original order",
                 with_embeddings, categories.len());
        return categories.to_vec();
    }

    // Greedy nearest-neighbor ordering
    let mut remaining: std::collections::HashSet<usize> = (0..categories.len()).collect();
    let mut sorted_indices: Vec<usize> = Vec::with_capacity(categories.len());

    // Start with first topic that has an embedding
    let start = (0..categories.len())
        .find(|&i| embeddings[i].is_some())
        .unwrap_or(0);
    sorted_indices.push(start);
    remaining.remove(&start);

    // Greedily add nearest neighbor
    while !remaining.is_empty() {
        let last_idx = *sorted_indices.last().unwrap();
        let last_emb = embeddings[last_idx];

        // Find nearest remaining topic
        let nearest = remaining
            .iter()
            .max_by(|&&a, &&b| {
                let sim_a = match (last_emb, embeddings[a]) {
                    (Some(e1), Some(e2)) => cosine_similarity(e1, e2),
                    _ => -1.0,
                };
                let sim_b = match (last_emb, embeddings[b]) {
                    (Some(e1), Some(e2)) => cosine_similarity(e1, e2),
                    _ => -1.0,
                };
                sim_a.partial_cmp(&sim_b).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
            .unwrap();

        sorted_indices.push(nearest);
        remaining.remove(&nearest);
    }

    println!("[AI] Sorted {} topics by embedding similarity for coherent batching", categories.len());

    // Return sorted categories
    sorted_indices.iter().map(|&i| categories[i].clone()).collect()
}

/// Group categories into 4-8 uber-categories, preserving project names
/// Used for consolidating Universe's direct children into navigable top-level domains
/// Projects (Mycelica, etc.) stay as their own categories; generic topics get grouped by theme
///
/// embeddings_map: Pre-fetched embeddings for sorting (pass empty HashMap to skip sorting)
/// tag_anchors: Optional list of persistent tag titles to use as preferred category names
pub async fn group_into_uber_categories(
    categories: &[TopicInfo],
    embeddings_map: &std::collections::HashMap<String, Vec<f32>>,
    tag_anchors: Option<&[String]>,
) -> Result<Vec<CategoryGrouping>, String> {
    if categories.is_empty() {
        return Ok(vec![]);
    }

    let anchors = tag_anchors.unwrap_or(&[]);

    // If small enough, do single call
    if categories.len() <= UBER_GROUPING_BATCH_SIZE {
        return group_into_uber_categories_single(categories, &[], anchors).await;
    }

    // Sort categories by embedding similarity BEFORE batching
    // This ensures each batch contains semantically related topics
    let sorted_categories = if embeddings_map.is_empty() {
        println!("[AI] No embeddings provided, using original order");
        categories.to_vec()
    } else {
        sort_by_embedding_similarity(categories, embeddings_map)
    };

    // Batch processing for large category sets
    let num_batches = (sorted_categories.len() + UBER_GROUPING_BATCH_SIZE - 1) / UBER_GROUPING_BATCH_SIZE;
    println!("[AI] Grouping {} categories in {} similarity-sorted batches of ~{}",
             sorted_categories.len(), num_batches, UBER_GROUPING_BATCH_SIZE);

    let mut all_groupings: Vec<CategoryGrouping> = Vec::new();

    for (batch_idx, batch) in sorted_categories.chunks(UBER_GROUPING_BATCH_SIZE).enumerate() {
        println!("[AI] Processing uber-category batch {}/{} ({} categories, {} existing uber-categories)",
                 batch_idx + 1, num_batches, batch.len(), all_groupings.len());

        // Pass existing uber-categories so AI prefers assigning to them
        match group_into_uber_categories_single(batch, &all_groupings, anchors).await {
            Ok(batch_groupings) => {
                // Merge with existing groupings
                for new_grouping in batch_groupings {
                    if let Some(existing) = find_similar_uber_category(&mut all_groupings, &new_grouping.name) {
                        // Merge children into existing category
                        existing.children.extend(new_grouping.children);
                        println!("[AI] Merged '{}' into existing '{}'", new_grouping.name, existing.name);
                    } else {
                        all_groupings.push(new_grouping);
                    }
                }
            }
            Err(e) => {
                println!("[AI] Batch {} failed: {}", batch_idx + 1, e);
                // Continue with other batches
            }
        }
    }

    println!("[AI] Uber-grouping complete: {} total categories", all_groupings.len());
    Ok(all_groupings)
}

/// Find an uber-category with a similar name for merging
fn find_similar_uber_category<'a>(
    categories: &'a mut [CategoryGrouping],
    name: &str,
) -> Option<&'a mut CategoryGrouping> {
    let name_lower = name.to_lowercase();

    for cat in categories.iter_mut() {
        let cat_lower = cat.name.to_lowercase();

        // Exact match (case-insensitive)
        if cat_lower == name_lower {
            return Some(cat);
        }

        // One contains the other
        if cat_lower.contains(&name_lower) || name_lower.contains(&cat_lower) {
            return Some(cat);
        }
    }

    None
}

/// Single-batch uber-category grouping (internal)
async fn group_into_uber_categories_single(
    categories: &[TopicInfo],
    existing_uber_categories: &[CategoryGrouping],
    tag_anchors: &[String],
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

    // Build existing uber-categories section if we have any from previous batches
    let existing_section = if existing_uber_categories.is_empty() {
        String::new()
    } else {
        let existing_list = existing_uber_categories
            .iter()
            .map(|c| format!("- \"{}\" ({})", c.name, c.description.as_deref().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            r#"EXISTING UBER-CATEGORIES FROM PREVIOUS BATCHES:
{existing_list}
→ Prefer assigning to these when a good match exists
→ Only create new uber-categories if nothing fits

"#,
            existing_list = existing_list
        )
    };

    // Build tag anchors section if we have persistent tags
    let tag_anchors_section = if tag_anchors.is_empty() {
        String::new()
    } else {
        let tags_list = tag_anchors.join(", ");
        format!(
            r#"PREFERRED CATEGORY NAMES (from user's established tags):
{tags_list}
→ Use these exact names when they fit the content well
→ These represent stable categories the user has built up over time

"#,
            tags_list = tags_list
        )
    };

    let task_description = if existing_uber_categories.is_empty() {
        "Group these into 8-10 top-level categories that make the graph navigable."
    } else {
        "Group these into the existing uber-categories above, OR create new ones (target 8-10 new max)."
    };

    let prompt = format!(
        r#"You are consolidating a knowledge graph's top-level categories into uber-categories.

{existing_section}{tag_anchors_section}CURRENT CATEGORIES ({count} total):
{categories_section}

YOUR TASK:
{task_description}

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

4. TARGET 8-10 CATEGORIES:
   - Each project gets its own category (even if small)
   - Non-project categories can be merged into themes
   - Result should have 8-10 total uber-categories

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
        existing_section = existing_section,
        tag_anchors_section = tag_anchors_section,
        count = categories.len(),
        categories_section = categories_section,
        task_description = task_description
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

/// Check if embeddings are available (local embeddings enabled OR OpenAI API key is set)
pub fn embeddings_available() -> bool {
    settings::use_local_embeddings() || settings::has_openai_api_key()
}

/// Generate an embedding for text.
/// Uses local embeddings (384-dim) if enabled, otherwise OpenAI (1536-dim).
pub async fn generate_embedding(text: &str) -> Result<Vec<f32>, String> {
    // Route to local embeddings if enabled
    if settings::use_local_embeddings() {
        return local_embeddings::generate(text);
    }

    // Otherwise use OpenAI
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

    // NOTE: test_parse_clustering_response removed - old AI clustering code deleted

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
