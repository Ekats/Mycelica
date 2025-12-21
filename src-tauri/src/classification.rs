// =============================================================================
// Content Classification Module
// =============================================================================
//
// Classifies content into types for mini-clustering:
// - idea: Conversational, questions, explanations, decisions (default)
// - code: Code snippets with syntax patterns
// - debug: Error logs, stack traces, debugging content
// - paste: Long non-conversational text dumps
//
// Classification uses pattern matching, not AI, for speed and consistency.

use crate::db::Database;

/// Content types for mini-clustering
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    Idea,          // Substantial discussion - renders in graph
    Investigation, // Incremental discoveries - renders in graph
    Code,          // Code snippets - hidden in supporting panel
    Debug,         // Error logs and debugging - hidden in supporting panel
    Paste,         // Long pastes/dumps - hidden in supporting panel
    Trivial,       // Acknowledgment, noise - hidden
}

impl ContentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentType::Idea => "idea",
            ContentType::Investigation => "investigation",
            ContentType::Code => "code",
            ContentType::Debug => "debug",
            ContentType::Paste => "paste",
            ContentType::Trivial => "trivial",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "idea" => Some(ContentType::Idea),
            "investigation" => Some(ContentType::Investigation),
            "code" => Some(ContentType::Code),
            "debug" => Some(ContentType::Debug),
            "paste" => Some(ContentType::Paste),
            "trivial" => Some(ContentType::Trivial),
            _ => None,
        }
    }
}

/// Classify content by analyzing patterns
pub fn classify_content(content: &str) -> ContentType {
    // Very short content is likely trivial
    if content.len() < 100 {
        return ContentType::Trivial;
    }

    // 1. Check for trivial patterns (acknowledgments, noise)
    if is_trivial(content) {
        return ContentType::Trivial;
    }

    // 2. Check for code patterns (most distinctive)
    if has_code_patterns(content) {
        return ContentType::Code;
    }

    // 3. Check for debug/error patterns
    if has_debug_patterns(content) {
        return ContentType::Debug;
    }

    // 4. Check for paste patterns (long non-conversational)
    if is_paste(content) {
        return ContentType::Paste;
    }

    // 5. Check for investigation patterns (discovery, working-out)
    if is_investigation(content) {
        return ContentType::Investigation;
    }

    // 6. Default to idea (substantial content)
    ContentType::Idea
}

/// Detect trivial content (acknowledgments, noise)
fn is_trivial(content: &str) -> bool {
    let lower = content.to_lowercase();
    let len = content.len();

    // Short exchanges with no substance
    if len < 300 {
        // Count trivial markers
        let trivial_phrases = [
            "okay", "ok", "thanks", "thank you", "got it", "sure", "yes", "no",
            "alright", "sounds good", "perfect", "great", "cool", "nice",
            "understood", "will do", "done", "fixed", "noted",
        ];
        let has_trivial = trivial_phrases.iter().any(|p| lower.contains(p));

        // No questions, no explanation, just acknowledgment
        let question_count = content.matches('?').count();
        let sentence_count = content.matches('.').count() + content.matches('!').count();

        if has_trivial && question_count == 0 && sentence_count < 3 {
            return true;
        }
    }

    false
}

/// Detect code patterns
fn has_code_patterns(content: &str) -> bool {
    let lower = content.to_lowercase();

    // Strong indicators: code blocks
    if content.contains("```") {
        return true;
    }

    // Count code-like patterns
    let mut code_signals = 0;

    // File extensions in context
    let extension_patterns = [
        ".rs", ".tsx", ".ts", ".js", ".jsx", ".py", ".go", ".java",
        ".cpp", ".c", ".h", ".hpp", ".cs", ".rb", ".php", ".swift",
        ".kt", ".scala", ".vue", ".svelte", ".json", ".yaml", ".yml",
        ".toml", ".sql", ".sh", ".bash", ".zsh", ".ps1", ".dockerfile"
    ];
    for ext in extension_patterns {
        if lower.contains(ext) {
            code_signals += 1;
        }
    }

    // Syntax keywords (weighted more heavily)
    let syntax_patterns = [
        "fn ", "pub fn", "async fn", "impl ", "struct ", "enum ", "trait ",  // Rust
        "const ", "let ", "mut ", "-> ", "=> ", ":: ", "&mut", "&self",
        "function ", "export ", "import ", "require(", "module.",           // JS/TS
        "def ", "class ", "self.", "__init__", "elif ", "lambda ",          // Python
        "func ", "package ", "go func", "interface{",                       // Go
        "public ", "private ", "protected ", "static ", "void ",            // Java/C#
        "#include", "#define", "#ifdef", "int main", "std::",               // C/C++
        "SELECT ", "INSERT ", "UPDATE ", "DELETE ", "FROM ", "WHERE ",      // SQL
        "CREATE TABLE", "ALTER TABLE", "DROP TABLE",
    ];
    for pattern in syntax_patterns {
        if content.contains(pattern) {
            code_signals += 2;
        }
    }

    // High indentation ratio (4+ spaces at line start)
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() > 3 {
        let indented_lines = lines.iter()
            .filter(|line| line.starts_with("    ") || line.starts_with("\t"))
            .count();
        let ratio = indented_lines as f32 / lines.len() as f32;
        if ratio > 0.3 {
            code_signals += 3;
        }
    }

    // Threshold: need multiple signals
    code_signals >= 3
}

/// Detect debug/error patterns
fn has_debug_patterns(content: &str) -> bool {
    let lower = content.to_lowercase();

    // Strong error indicators
    let error_patterns = [
        "error:", "error[", "error -", "error!", "error at",
        "failed:", "failed to", "failure:", "failure at",
        "exception:", "exception at", "exception in",
        "panic:", "panic at", "panicked at",
        "stack trace:", "stacktrace:", "backtrace:",
        "traceback (most recent",
        "at line ", "on line ",
        "segmentation fault", "core dumped",
        "unhandled exception", "uncaught exception",
        "null pointer", "nullpointerexception",
        "undefined is not", "cannot read property",
        "typeerror:", "referenceerror:", "syntaxerror:",
        "assertion failed", "assert_eq!", "assert!",
        "warning:", "warn:", "deprecated:",
    ];

    let mut debug_signals = 0;
    for pattern in error_patterns {
        if lower.contains(pattern) {
            debug_signals += 2;
        }
    }

    // Log level indicators
    let log_patterns = [
        "[error]", "[warn]", "[warning]", "[info]", "[debug]", "[trace]",
        "error  |", "warn  |", "info  |",
        "level=error", "level=warn",
    ];
    for pattern in log_patterns {
        if lower.contains(pattern) {
            debug_signals += 1;
        }
    }

    // Exit codes
    if lower.contains("exit code") || lower.contains("status code") || lower.contains("returned 1") {
        debug_signals += 1;
    }

    // Threshold
    debug_signals >= 2
}

/// Detect paste patterns (long non-conversational dumps)
fn is_paste(content: &str) -> bool {
    // Must be fairly long
    if content.len() < 1500 {
        return false;
    }

    // Count conversation markers
    let lower = content.to_lowercase();
    let question_marks = content.matches('?').count();
    let first_person = lower.matches(" i ").count() +
                       lower.matches("i'm").count() +
                       lower.matches("i've").count() +
                       lower.matches("i'll").count();
    let second_person = lower.matches(" you ").count() +
                        lower.matches("you're").count() +
                        lower.matches("you've").count() +
                        lower.matches("you'll").count();

    // Conversation density (markers per 1000 chars)
    let length_factor = content.len() as f32 / 1000.0;
    let conversation_density = (question_marks + first_person + second_person) as f32 / length_factor;

    // Low conversation density = paste
    conversation_density < 2.0
}

/// Detect investigation patterns (incremental discoveries, working-out steps)
fn is_investigation(content: &str) -> bool {
    let lower = content.to_lowercase();

    let mut investigation_signals = 0;

    // Discovery phrases
    let discovery_patterns = [
        "figured out", "found out", "discovered", "realized",
        "turns out", "it seems", "apparently", "now i understand",
        "aha", "eureka", "the issue was", "the problem was",
        "root cause", "the fix is", "solution is", "works now",
        "finally got", "managed to", "succeeded in",
        "after trying", "after testing", "after debugging",
        "narrowed down", "isolated", "identified",
    ];
    for pattern in discovery_patterns {
        if lower.contains(pattern) {
            investigation_signals += 2;
        }
    }

    // Working-out phrases
    let working_patterns = [
        "let me try", "trying", "testing", "checking",
        "investigating", "looking into", "digging into",
        "step 1", "step 2", "next step", "first,", "then,",
        "hypothesis", "theory", "suspect", "might be",
        "could be", "possibly", "maybe the", "what if",
    ];
    for pattern in working_patterns {
        if lower.contains(pattern) {
            investigation_signals += 1;
        }
    }

    // Progress indicators
    if lower.contains("progress") || lower.contains("update:") || lower.contains("status:") {
        investigation_signals += 1;
    }

    // Threshold: need multiple signals to be investigation
    investigation_signals >= 3
}

/// Classify all unclassified items in the database
pub fn classify_all_items(db: &Database) -> Result<usize, String> {
    let items = db.get_items().map_err(|e| e.to_string())?;
    let total_items = items.len();
    let mut classified = 0;
    let mut already_classified = 0;
    let mut empty_content = 0;

    for item in items {
        // Skip already classified items (likely from AI processing)
        if item.content_type.is_some() {
            already_classified += 1;
            continue;
        }

        // Get content to classify
        let content = item.content.as_deref().unwrap_or("");
        if content.is_empty() {
            empty_content += 1;
            continue;
        }

        let content_type = classify_content(content);
        db.set_content_type(&item.id, content_type.as_str())
            .map_err(|e| e.to_string())?;
        classified += 1;
    }

    // Log diagnostic info if nothing was classified
    if classified == 0 && (already_classified > 0 || empty_content > 0) {
        eprintln!(
            "Classification: {} items total, {} already classified by AI, {} with empty content",
            total_items, already_classified, empty_content
        );
    }

    Ok(classified)
}

/// Classify items under a specific parent
pub fn classify_children(db: &Database, parent_id: &str) -> Result<usize, String> {
    let children = db.get_children(parent_id).map_err(|e| e.to_string())?;
    let mut classified = 0;

    for child in children {
        // Only classify items (leaves)
        if !child.is_item {
            continue;
        }

        // Skip already classified items
        if child.content_type.is_some() {
            continue;
        }

        // Get content to classify
        let content = child.content.as_deref().unwrap_or("");
        if content.is_empty() {
            continue;
        }

        let content_type = classify_content(content);
        db.set_content_type(&child.id, content_type.as_str())
            .map_err(|e| e.to_string())?;
        classified += 1;
    }

    Ok(classified)
}

// =============================================================================
// Association Algorithm
// =============================================================================
//
// Associates supporting items (code/debug/paste) with specific idea nodes
// using embedding similarity and time proximity.

use crate::db::Node;

/// Compute associations for all items under a parent
/// Returns the number of items that were associated with ideas
pub fn compute_associations(db: &Database, parent_id: &str) -> Result<usize, String> {
    let children = db.get_children(parent_id).map_err(|e| e.to_string())?;

    // Separate ideas from supporting items
    let ideas: Vec<&Node> = children.iter()
        .filter(|n| n.is_item && (n.content_type.as_deref() == Some("idea") || n.content_type.is_none()))
        .collect();

    let supporting: Vec<&Node> = children.iter()
        .filter(|n| n.is_item && matches!(n.content_type.as_deref(), Some("code" | "debug" | "paste")))
        .collect();

    if ideas.is_empty() || supporting.is_empty() {
        return Ok(0);
    }

    let mut associated = 0;

    for item in supporting {
        // Skip already associated items
        if item.associated_idea_id.is_some() {
            continue;
        }

        if let Some(best_idea_id) = find_best_match(db, item, &ideas)? {
            db.set_associated_idea(&item.id, &best_idea_id)
                .map_err(|e| e.to_string())?;
            associated += 1;
        }
    }

    Ok(associated)
}

/// Find the best matching idea for a supporting item
fn find_best_match(db: &Database, item: &Node, ideas: &[&Node]) -> Result<Option<String>, String> {
    // Get item's embedding
    let item_embedding = match db.get_node_embedding(&item.id).map_err(|e| e.to_string())? {
        Some(emb) => emb,
        None => return Ok(None), // Can't match without embedding
    };

    let mut best_score = 0.0f32;
    let mut best_id: Option<String> = None;

    for idea in ideas {
        // Get idea's embedding
        let idea_embedding = match db.get_node_embedding(&idea.id).map_err(|e| e.to_string())? {
            Some(emb) => emb,
            None => continue, // Skip ideas without embeddings
        };

        // Calculate similarity score (0.0 to 1.0)
        let similarity = cosine_similarity(&item_embedding, &idea_embedding);

        // Calculate time proximity score (0.0 to 1.0)
        // Items created closer in time get higher scores
        let time_diff_hours = (item.created_at - idea.created_at).abs() as f32 / 3600000.0; // ms to hours
        let time_score = 1.0 / (1.0 + time_diff_hours);

        // Combined score: 70% similarity, 30% time proximity
        let combined = similarity * 0.7 + time_score * 0.3;

        // Track best match above threshold
        if combined > best_score && combined > 0.3 {
            best_score = combined;
            best_id = Some(idea.id.clone());
        }
    }

    Ok(best_id)
}

/// Calculate cosine similarity between two embedding vectors
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let magnitude_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let magnitude_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if magnitude_a == 0.0 || magnitude_b == 0.0 {
        return 0.0;
    }

    dot_product / (magnitude_a * magnitude_b)
}

/// Compute associations for all topics in the database
pub fn compute_all_associations(db: &Database) -> Result<usize, String> {
    // Get all non-item nodes (topics/clusters) that have children
    let topics = db.get_all_nodes().map_err(|e| e.to_string())?;
    let mut total_associated = 0;

    for topic in topics {
        if topic.is_item || topic.child_count == 0 {
            continue;
        }

        match compute_associations(db, &topic.id) {
            Ok(count) => total_associated += count,
            Err(e) => eprintln!("Failed to compute associations for {}: {}", topic.id, e),
        }
    }

    Ok(total_associated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_idea() {
        let content = "I think we should implement the feature this way. What do you think about using a hash map for the cache?";
        assert_eq!(classify_content(content), ContentType::Idea);
    }

    #[test]
    fn test_classify_code_block() {
        let content = r#"Here's the code:
```rust
fn main() {
    println!("Hello, world!");
}
```
"#;
        assert_eq!(classify_content(content), ContentType::Code);
    }

    #[test]
    fn test_classify_code_patterns() {
        let content = r#"
pub fn process_data(input: &str) -> Result<(), Error> {
    let parsed = serde_json::from_str(input)?;
    let mut state = self.state.lock().unwrap();
    state.update(parsed);
    Ok(())
}
"#;
        assert_eq!(classify_content(content), ContentType::Code);
    }

    #[test]
    fn test_classify_debug() {
        let content = r#"
error[E0308]: mismatched types
  --> src/main.rs:5:14
   |
5  |     let x: i32 = "hello";
   |            ---   ^^^^^^^ expected `i32`, found `&str`
   |            |
   |            expected due to this
"#;
        assert_eq!(classify_content(content), ContentType::Debug);
    }

    #[test]
    fn test_classify_debug_stack_trace() {
        let content = r#"
Exception in thread "main" java.lang.NullPointerException
    at com.example.MyClass.method(MyClass.java:123)
    at com.example.Main.main(Main.java:45)
Stack trace:
    at line 123 in file.java
"#;
        assert_eq!(classify_content(content), ContentType::Debug);
    }

    #[test]
    fn test_classify_short_content() {
        let content = "Hello";
        assert_eq!(classify_content(content), ContentType::Idea);
    }
}
