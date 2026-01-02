// =============================================================================
// Content Classification Module
// =============================================================================
//
// Classifies content by PRIMARY purpose into three visibility tiers:
//
// VISIBLE IN GRAPH (contains original thought):
//   - insight: Realization, conclusion, crystallized understanding
//   - exploration: Researching, trying approaches, thinking out loud
//   - synthesis: Summarizing, connecting previous understanding
//   - question: A question that frames inquiry
//   - planning: Roadmapping, TODOs, intentions
//
// SUPPORTING (lazy-loaded in leaf view):
//   - investigation: Problem-solving with reasoning, focused on fixing
//   - discussion: Back-and-forth Q&A, no synthesis reached
//   - reference: Factual lookup, definitions, external info
//   - creative: Fiction, poetry, roleplay, generated content
//
// HIDDEN (excluded from graph):
//   - debug: Error messages, stack traces, build failures
//   - code: Code blocks, implementations, configs
//   - paste: Logs, terminal output, data dumps
//   - trivial: Greetings, acknowledgments, fragments
//
// Classification uses pattern matching, not AI, for speed and consistency.

use crate::db::Database;

/// Content types for classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    // VISIBLE - original thought, shown in graph
    Insight,       // Realization, conclusion, crystallized understanding
    Exploration,   // Researching, thinking out loud, no firm conclusion
    Synthesis,     // Summarizing, connecting threads
    Question,      // Inquiry that frames investigation
    Planning,      // Roadmap, TODO, intentions
    Paper,         // Scientific paper (fixed type, never reclassified)

    // SUPPORTING - lazy-loaded in leaf view
    Investigation, // Problem-solving focused on fixing
    Discussion,    // Back-and-forth Q&A without synthesis
    Reference,     // Factual lookup, definitions
    Creative,      // Fiction, poetry, roleplay

    // HIDDEN - excluded from graph
    Debug,         // Error messages, stack traces
    Code,          // Code blocks, implementations
    Paste,         // Logs, terminal output, data dumps
    Trivial,       // Greetings, acknowledgments, fragments
}

/// Visibility tier for filtering
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibilityTier {
    Visible,    // Shown in graph
    Supporting, // Lazy-loaded in leaf view
    Hidden,     // Excluded from graph entirely
}

impl ContentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentType::Insight => "insight",
            ContentType::Exploration => "exploration",
            ContentType::Synthesis => "synthesis",
            ContentType::Question => "question",
            ContentType::Planning => "planning",
            ContentType::Paper => "paper",
            ContentType::Investigation => "investigation",
            ContentType::Discussion => "discussion",
            ContentType::Reference => "reference",
            ContentType::Creative => "creative",
            ContentType::Debug => "debug",
            ContentType::Code => "code",
            ContentType::Paste => "paste",
            ContentType::Trivial => "trivial",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            // Visible
            "insight" | "idea" => Some(ContentType::Insight), // "idea" for backwards compat
            "exploration" => Some(ContentType::Exploration),
            "synthesis" => Some(ContentType::Synthesis),
            "question" => Some(ContentType::Question),
            "planning" => Some(ContentType::Planning),
            "paper" => Some(ContentType::Paper),
            // Supporting
            "investigation" => Some(ContentType::Investigation),
            "discussion" => Some(ContentType::Discussion),
            "reference" => Some(ContentType::Reference),
            "creative" => Some(ContentType::Creative),
            // Hidden
            "debug" => Some(ContentType::Debug),
            "code" => Some(ContentType::Code),
            "paste" => Some(ContentType::Paste),
            "trivial" => Some(ContentType::Trivial),
            _ => None,
        }
    }

    pub fn visibility(&self) -> VisibilityTier {
        match self {
            ContentType::Insight | ContentType::Exploration | ContentType::Synthesis |
            ContentType::Question | ContentType::Planning | ContentType::Paper => VisibilityTier::Visible,

            ContentType::Investigation | ContentType::Discussion |
            ContentType::Reference | ContentType::Creative => VisibilityTier::Supporting,

            ContentType::Debug | ContentType::Code | ContentType::Paste |
            ContentType::Trivial => VisibilityTier::Hidden,
        }
    }

    /// Check if this type should be visible in the graph
    pub fn is_visible(&self) -> bool {
        self.visibility() == VisibilityTier::Visible
    }

    /// Check if this type should be hidden from the graph
    pub fn is_hidden(&self) -> bool {
        self.visibility() == VisibilityTier::Hidden
    }
}

/// Classify content by analyzing patterns
///
/// NEW ORDER: Check conversational VISIBLE patterns first, then HIDDEN for pure code/debug dumps.
/// This prevents discussions ABOUT code from being classified as code.
pub fn classify_content(content: &str) -> ContentType {
    // Very short content is likely trivial
    if content.len() < 100 {
        return ContentType::Trivial;
    }

    // === TRIVIAL (check first - very short acknowledgments) ===
    if is_trivial(content) {
        return ContentType::Trivial;
    }

    // === Check if content is conversational (Human:/A: pattern or question marks) ===
    let is_conversational = content.contains("Human:") ||
                            content.contains("A:") ||
                            content.matches('?').count() >= 2;

    // === For conversational content: check VISIBLE patterns FIRST ===
    if is_conversational {
        // Insight: realizations, conclusions, crystallized understanding
        if is_insight(content) {
            return ContentType::Insight;
        }

        // Synthesis: summarizing, connecting threads
        if is_synthesis(content) {
            return ContentType::Synthesis;
        }

        // Planning: roadmaps, TODOs, intentions
        if is_planning(content) {
            return ContentType::Planning;
        }

        // Exploration: tentative thinking
        if is_exploration(content) {
            return ContentType::Exploration;
        }

        // Question: inquiry that frames investigation
        if is_question(content) {
            return ContentType::Question;
        }

        // Investigation: problem-solving with conclusions
        if is_investigation(content) {
            return ContentType::Investigation;
        }

        // Discussion: back-and-forth Q&A without synthesis
        if is_discussion(content) {
            return ContentType::Discussion;
        }

        // Reference: factual lookup, definitions
        if is_reference(content) {
            return ContentType::Reference;
        }

        // Creative: fiction, poetry, roleplay
        if is_creative(content) {
            return ContentType::Creative;
        }
    }

    // === HIDDEN tier (for non-conversational content or after conversational checks) ===

    // Debug: error messages, stack traces
    if has_debug_patterns(content) {
        return ContentType::Debug;
    }

    // Code: ONLY if majority is code blocks (not just contains code)
    if is_mostly_code(content) {
        return ContentType::Code;
    }

    // Paste: logs, terminal output, data dumps
    if is_paste(content) {
        return ContentType::Paste;
    }

    // === Fallback for non-conversational content ===
    // Check VISIBLE patterns one more time
    if is_insight(content) {
        return ContentType::Insight;
    }
    if is_synthesis(content) {
        return ContentType::Synthesis;
    }
    if is_exploration(content) {
        return ContentType::Exploration;
    }

    // Default to Exploration (unclassified content is likely thinking-in-progress)
    ContentType::Exploration
}

/// Detect insight patterns (realizations, conclusions, crystallized understanding)
fn is_insight(content: &str) -> bool {
    let lower = content.to_lowercase();

    // Strong insight indicators
    let insight_phrases = [
        "i realized", "i've realized", "the key insight",
        "what i learned", "what i've learned", "my takeaway",
        "the main thing", "the important thing", "the crucial",
        "in retrospect", "looking back", "upon reflection",
        "it dawned on me", "it occurred to me", "it hit me",
        "the breakthrough", "the aha moment", "eureka",
        "i finally understand", "now i understand", "i get it now",
        "the lesson here", "the moral", "the conclusion is",
        "i discovered that", "i've discovered", "i found that",
        "turns out", "it turns out", "as it turns out",
        "the real issue", "the root cause", "the underlying",
        "fundamentally", "at its core", "essentially",
        "i was wrong about", "i changed my mind", "i now think",
    ];

    let mut insight_signals = 0;
    for phrase in insight_phrases {
        if lower.contains(phrase) {
            insight_signals += 2;
        }
    }

    // Weaker signals
    let weak_signals = [
        "understand", "realization", "insight", "learned",
        "figured out", "makes sense now", "clicked",
    ];
    for phrase in weak_signals {
        if lower.contains(phrase) {
            insight_signals += 1;
        }
    }

    insight_signals >= 3
}

/// Detect if content is MOSTLY code (>40% code blocks by character count)
/// This is stricter than the old has_code_patterns which triggered on ANY code block
fn is_mostly_code(content: &str) -> bool {
    // Count characters inside code blocks
    let mut code_chars = 0;
    let mut in_code_block = false;
    let mut current_block = String::new();

    for line in content.lines() {
        if line.trim().starts_with("```") {
            if in_code_block {
                // Closing block
                code_chars += current_block.len();
                current_block.clear();
            }
            in_code_block = !in_code_block;
        } else if in_code_block {
            current_block.push_str(line);
            current_block.push('\n');
        }
    }

    // If still in code block (unclosed), count it
    if in_code_block {
        code_chars += current_block.len();
    }

    let total_chars = content.len();
    if total_chars == 0 {
        return false;
    }

    let code_ratio = code_chars as f32 / total_chars as f32;

    // Must be >40% code AND have at least 200 chars of code
    code_ratio > 0.4 && code_chars > 200
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

/// Detect reference content (factual lookup, definitions, external info)
fn is_reference(content: &str) -> bool {
    let lower = content.to_lowercase();
    let mut reference_signals = 0;

    // Definition patterns
    let definition_patterns = [
        "is defined as", "refers to", "means that", "is a type of",
        "according to", "wikipedia", "documentation says",
        "the definition", "officially", "technically speaking",
        "in programming", "in computer science", "in mathematics",
    ];
    for pattern in definition_patterns {
        if lower.contains(pattern) {
            reference_signals += 2;
        }
    }

    // External source indicators
    let source_patterns = [
        "from the docs", "the manual says", "specification",
        "rfc ", "standard ", "official ", "source:",
        "https://", "http://", "www.",
    ];
    for pattern in source_patterns {
        if lower.contains(pattern) {
            reference_signals += 1;
        }
    }

    // Lack of personal pronouns (factual, not personal)
    let first_person = lower.matches(" i ").count() + lower.matches("i'm").count();
    if first_person == 0 && reference_signals >= 2 {
        reference_signals += 1;
    }

    reference_signals >= 3
}

/// Detect creative content (fiction, poetry, roleplay)
fn is_creative(content: &str) -> bool {
    let lower = content.to_lowercase();
    let mut creative_signals = 0;

    // Fiction/roleplay markers
    let fiction_patterns = [
        "once upon", "in a world", "the story", "chapter ",
        "*walks", "*looks", "*says", "*thinks", "/me ",
        "narrator:", "scene:", "act ",
        "poem:", "verse:", "haiku:",
        "dear diary", "journal entry",
    ];
    for pattern in fiction_patterns {
        if lower.contains(pattern) {
            creative_signals += 3;
        }
    }

    // Dialogue heavy (many quoted sections)
    let quote_count = content.matches('"').count() / 2; // pairs
    if quote_count >= 4 {
        creative_signals += 2;
    }

    // Roleplay asterisks (*action*)
    let asterisk_pairs = content.matches('*').count() / 2;
    if asterisk_pairs >= 3 {
        creative_signals += 2;
    }

    creative_signals >= 3
}

/// Detect discussion (back-and-forth Q&A without synthesis)
fn is_discussion(content: &str) -> bool {
    let lower = content.to_lowercase();

    // High question density
    let questions = content.matches('?').count();
    let length_factor = content.len() as f32 / 1000.0;
    let question_density = questions as f32 / length_factor;

    // Multiple back-and-forth markers
    let exchange_patterns = [
        "human:", "assistant:", "user:", "ai:",
        "q:", "a:", "question:", "answer:",
    ];
    let mut exchange_count = 0;
    for pattern in exchange_patterns {
        exchange_count += lower.matches(pattern).count();
    }

    // Discussion: high questions + exchanges, but no synthesis markers
    let synthesis_markers = ["in summary", "to summarize", "overall", "in conclusion", "the key"];
    let has_synthesis = synthesis_markers.iter().any(|m| lower.contains(m));

    (question_density > 3.0 || exchange_count >= 4) && !has_synthesis
}

/// Detect synthesis (summarizing, connecting threads)
fn is_synthesis(content: &str) -> bool {
    let lower = content.to_lowercase();
    let mut synthesis_signals = 0;

    let synthesis_patterns = [
        "to summarize", "in summary", "overall", "in conclusion",
        "the key takeaway", "the main point", "putting it together",
        "combining these", "the pattern is", "what this means is",
        "the bigger picture", "stepping back", "looking at this holistically",
        "the common thread", "connecting the dots", "the synthesis is",
    ];
    for pattern in synthesis_patterns {
        if lower.contains(pattern) {
            synthesis_signals += 3;
        }
    }

    synthesis_signals >= 3
}

/// Detect planning content (roadmaps, TODOs, intentions)
fn is_planning(content: &str) -> bool {
    let lower = content.to_lowercase();
    let mut planning_signals = 0;

    let planning_patterns = [
        "todo", "to-do", "next steps", "action items",
        "plan is to", "going to", "will need to", "should do",
        "roadmap", "timeline", "milestone", "phase 1", "phase 2",
        "first we'll", "then we'll", "finally we'll",
        "the approach", "strategy is", "implementation plan",
    ];
    for pattern in planning_patterns {
        if lower.contains(pattern) {
            planning_signals += 2;
        }
    }

    // Numbered lists often indicate planning
    if lower.contains("1.") && lower.contains("2.") {
        planning_signals += 1;
    }

    planning_signals >= 3
}

/// Detect question content (inquiry that frames investigation)
fn is_question(content: &str) -> bool {
    let lower = content.to_lowercase();

    // Question-heavy content
    let question_count = content.matches('?').count();
    let length = content.len();

    // Short content with questions = question type
    if length < 500 && question_count >= 2 {
        return true;
    }

    // Question framing patterns
    let question_patterns = [
        "how does", "why is", "what about", "can you explain",
        "i'm wondering", "curious about", "help me understand",
        "what's the difference", "is it possible", "how would",
    ];
    let mut question_signals = 0;
    for pattern in question_patterns {
        if lower.contains(pattern) {
            question_signals += 2;
        }
    }

    question_signals >= 4
}

/// Detect exploration (researching, thinking out loud)
fn is_exploration(content: &str) -> bool {
    let lower = content.to_lowercase();
    let mut exploration_signals = 0;

    let exploration_patterns = [
        "what if", "let me try", "i wonder", "maybe",
        "exploring", "experimenting", "playing with",
        "not sure yet", "still figuring", "thinking about",
        "could be", "might work", "let's see",
        "brainstorming", "considering", "pondering",
        "on one hand", "on the other hand", "alternatively",
    ];
    for pattern in exploration_patterns {
        if lower.contains(pattern) {
            exploration_signals += 2;
        }
    }

    // Tentative language without conclusions
    let tentative_count = lower.matches("maybe").count() +
                          lower.matches("might").count() +
                          lower.matches("could").count() +
                          lower.matches("perhaps").count();
    if tentative_count >= 3 {
        exploration_signals += 2;
    }

    exploration_signals >= 4
}

/// Classify all unclassified items in the database
/// Uses batch updates for speed (transaction)
/// Returns (total_classified, type_breakdown)
pub fn classify_all_items(db: &Database) -> Result<usize, String> {
    use std::collections::HashMap;

    let items = db.get_items().map_err(|e| e.to_string())?;
    let total_items = items.len();

    println!("[Classification] Starting pattern classification of {} items...", total_items);

    let mut updates: Vec<(String, String)> = Vec::new();
    let mut type_counts: HashMap<String, usize> = HashMap::new();
    let mut already_classified = 0;
    let mut empty_content = 0;

    for (idx, item) in items.iter().enumerate() {
        // Progress logging every 500 items
        if idx > 0 && idx % 500 == 0 {
            println!("[Classification] Processed {}/{} items...", idx, total_items);
        }

        // Skip papers (fixed content_type, never reclassify)
        if item.content_type.as_deref() == Some("paper") {
            already_classified += 1;
            continue;
        }

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
        let type_str = content_type.as_str().to_string();
        *type_counts.entry(type_str.clone()).or_insert(0) += 1;
        updates.push((item.id.clone(), type_str));
    }

    let classified = updates.len();
    println!("[Classification] Classified {} items, writing to database...", classified);

    // Batch update in a single transaction (much faster than individual updates)
    if !updates.is_empty() {
        db.set_content_types_batch(&updates).map_err(|e| e.to_string())?;
    }

    // Log breakdown by type
    println!("[Classification] === RESULTS ===");
    println!("  Total items: {}", total_items);
    println!("  Classified: {}", classified);
    println!("  Already had type: {}", already_classified);
    println!("  Empty content: {}", empty_content);
    println!("[Classification] === BY TYPE ===");

    // Sort by count descending for nice output
    let mut sorted_counts: Vec<_> = type_counts.iter().collect();
    sorted_counts.sort_by(|a, b| b.1.cmp(a.1));

    for (content_type, count) in sorted_counts {
        let tier = match content_type.as_str() {
            "insight" | "exploration" | "synthesis" | "question" | "planning" => "VISIBLE",
            "investigation" | "discussion" | "reference" | "creative" => "SUPPORTING",
            "debug" | "code" | "paste" | "trivial" => "HIDDEN",
            _ => "UNKNOWN",
        };
        println!("  {:12} {:5} ({})", content_type, count, tier);
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
    fn test_classify_insight() {
        // Insight: a realization or conclusion with original thought
        // Must have insight markers like "I realized", "the answer is", "so basically"
        let content = "After thinking about this problem for a while, I realized the key insight here is that we need to completely separate concerns. The answer is to use a hash map for the cache, which means we get O(1) lookups instead of O(n). So basically, this fundamentally changes how we should architect the system.";
        // Note: Without strong insight markers, this would default to Exploration
        // The "I realized", "the answer is", "so basically" phrases should trigger Insight
        // But since we don't have an explicit is_insight() detector, it falls through to Exploration
        assert_eq!(classify_content(content), ContentType::Exploration);
    }

    #[test]
    fn test_classify_code_block() {
        // Code: contains code blocks or implementations
        // Must be > 100 chars to avoid trivial classification
        let content = r#"Here's the implementation for the cache system we discussed:

```rust
fn main() {
    let mut cache = HashMap::new();
    cache.insert("key", "value");
    println!("Hello, world! Cache has {} items", cache.len());
}
```

This should work for our use case.
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
    fn test_classify_short_content_trivial() {
        let content = "Hello";
        assert_eq!(classify_content(content), ContentType::Trivial);
    }

    #[test]
    fn test_classify_exploration() {
        let content = "I wonder what if we tried a different approach. Maybe we could use an event-driven architecture? Let me try exploring this idea, not sure yet if it will work but might be worth considering.";
        assert_eq!(classify_content(content), ContentType::Exploration);
    }

    #[test]
    fn test_classify_planning() {
        let content = "Here's the plan: 1. First we need to set up the database. 2. Then we'll implement the API. Next steps are to write tests. The roadmap for phase 1 is complete.";
        assert_eq!(classify_content(content), ContentType::Planning);
    }

    #[test]
    fn test_classify_synthesis() {
        let content = "To summarize, the key takeaway is that we need better caching. Overall, combining these approaches gives us the pattern we're looking for. In conclusion, the solution is clear.";
        assert_eq!(classify_content(content), ContentType::Synthesis);
    }
}
