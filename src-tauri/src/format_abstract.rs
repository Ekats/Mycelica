//! Abstract formatter for scientific papers
//! Converts plain text abstracts to structured markdown with section headers

use regex::Regex;
use std::sync::LazyLock;

/// Result of formatting an abstract
#[derive(Debug, Clone)]
pub struct FormattedAbstract {
    /// Markdown-formatted abstract with **Section** headers
    pub markdown: String,
    /// List of detected sections (e.g., ["Background", "Methods", "Results"])
    pub sections: Vec<String>,
    /// True if any structure was detected
    pub had_structure: bool,
}

/// Section keywords sorted by length (multi-word first for proper matching)
static SECTION_KEYWORDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut keywords = vec![
        // Multi-word (must match first)
        "Materials and Methods",
        "Patients and Methods",
        "Subjects and Methods",
        "Design and Methods",
        "Main Outcome Measures",
        "Main Outcome Measure",
        "Primary Outcomes",
        "Secondary Outcomes",
        "Primary Outcome",
        "Secondary Outcome",
        "Data Sources",
        "Data Source",
        "Data Extraction",
        "Data Synthesis",
        "Data Analysis",
        "Study Design",
        "Study Selection",
        "Study Population",
        "Statistical Analysis",
        "Statistical Methods",
        "Clinical Implications",
        "Clinical Significance",
        "Key Findings",
        "Key Results",
        "Key Messages",
        "Key Points",
        "Trial Registration",
        "What is Known",
        "What is New",
        "What This Adds",
        "Practice Implications",
        "Policy Implications",
        "Future Directions",
        "Future Research",
        "Strengths and Limitations",
        "Author Summary",
        "Plain Language Summary",
        "Lay Summary",
        "Take Home Message",
        "Evidence Acquisition",
        "Evidence Synthesis",
        // Single-word
        "Background",
        "Introduction",
        "Context",
        "Rationale",
        "Purpose",
        "Objective",
        "Objectives",
        "Aim",
        "Aims",
        "Hypothesis",
        "Methods",
        "Methodology",
        "Design",
        "Setting",
        "Participants",
        "Patients",
        "Subjects",
        "Sample",
        "Population",
        "Interventions",
        "Intervention",
        "Exposure",
        "Exposures",
        "Measurements",
        "Measures",
        "Outcomes",
        "Results",
        "Findings",
        "Observations",
        "Analysis",
        "Discussion",
        "Interpretation",
        "Conclusions",
        "Conclusion",
        "Summary",
        "Implications",
        "Significance",
        "Relevance",
        "Limitations",
        "Funding",
        "Acknowledgements",
        "Acknowledgments",
    ];
    // Sort by length descending so longer phrases match first
    keywords.sort_by(|a, b| b.len().cmp(&a.len()));
    keywords
});

/// Regex pattern for section labels
static SECTION_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    let keywords = SECTION_KEYWORDS.iter()
        .map(|k| regex::escape(k))
        .collect::<Vec<_>>()
        .join("|");
    // Match: "Keyword:" at word boundary (case insensitive)
    // Requires colon to distinguish section headers from regular usage of words like "Background"
    Regex::new(&format!(
        r"(?i)\b({}):\s*",
        keywords
    )).unwrap()
});

/// Regex for ALL CAPS section headers (e.g., "BACKGROUND METHODS RESULTS")
static ALL_CAPS_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    let keywords = SECTION_KEYWORDS.iter()
        .filter(|k| !k.contains(' ')) // Only single words for caps detection
        .map(|k| k.to_uppercase())
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!(r"\b({})\b", keywords)).unwrap()
});

/// HTML entity replacements
static HTML_ENTITIES: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(|| {
    vec![
        // Common entities
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&apos;", "'"),
        ("&nbsp;", " "),
        // Greek letters (common in scientific text)
        ("&alpha;", "α"),
        ("&beta;", "β"),
        ("&gamma;", "γ"),
        ("&delta;", "δ"),
        ("&epsilon;", "ε"),
        ("&mu;", "μ"),
        ("&pi;", "π"),
        ("&sigma;", "σ"),
        ("&tau;", "τ"),
        ("&omega;", "ω"),
        ("&Alpha;", "Α"),
        ("&Beta;", "Β"),
        ("&Gamma;", "Γ"),
        ("&Delta;", "Δ"),
        // Mathematical symbols
        ("&le;", "≤"),
        ("&ge;", "≥"),
        ("&ne;", "≠"),
        ("&plusmn;", "±"),
        ("&times;", "×"),
        ("&divide;", "÷"),
        ("&deg;", "°"),
        ("&infin;", "∞"),
        // Punctuation
        ("&ndash;", "\u{2013}"),  // en-dash
        ("&mdash;", "\u{2014}"),  // em-dash
        ("&lsquo;", "\u{2018}"),  // left single quote
        ("&rsquo;", "\u{2019}"),  // right single quote
        ("&ldquo;", "\u{201C}"),  // left double quote
        ("&rdquo;", "\u{201D}"),  // right double quote
        ("&hellip;", "\u{2026}"), // ellipsis
    ]
});

/// Strip all HTML/XML tags from text
pub fn strip_html_tags(text: &str) -> String {
    let tag_re = Regex::new(r"<[^>]+>").unwrap();
    tag_re.replace_all(text, "").to_string()
}

/// Clean HTML entities and normalize whitespace
fn clean_input(text: &str) -> String {
    // Strip HTML/XML tags FIRST (before entity decoding)
    let mut result = strip_html_tags(text);

    // Decode HTML entities
    for (entity, replacement) in HTML_ENTITIES.iter() {
        result = result.replace(entity, replacement);
    }

    // Handle numeric entities (&#123; or &#x7B;)
    let numeric_entity = Regex::new(r"&#(\d+);").unwrap();
    result = numeric_entity.replace_all(&result, |caps: &regex::Captures| {
        caps.get(1)
            .and_then(|m| m.as_str().parse::<u32>().ok())
            .and_then(char::from_u32)
            .map(|c| c.to_string())
            .unwrap_or_else(|| caps[0].to_string())
    }).to_string();

    let hex_entity = Regex::new(r"&#x([0-9A-Fa-f]+);").unwrap();
    result = hex_entity.replace_all(&result, |caps: &regex::Captures| {
        caps.get(1)
            .and_then(|m| u32::from_str_radix(m.as_str(), 16).ok())
            .and_then(char::from_u32)
            .map(|c| c.to_string())
            .unwrap_or_else(|| caps[0].to_string())
    }).to_string();

    // Normalize whitespace
    let whitespace = Regex::new(r"\s+").unwrap();
    result = whitespace.replace_all(&result, " ").to_string();

    result.trim().to_string()
}

/// Detect if text uses ALL CAPS section headers
fn has_all_caps_sections(text: &str) -> bool {
    let matches: Vec<_> = ALL_CAPS_REGEX.find_iter(text).collect();
    // Need at least 2 different section keywords to consider it structured
    matches.len() >= 2
}

/// Format abstract with detected sections
pub fn format_abstract(raw: &str) -> FormattedAbstract {
    // Handle empty/missing abstracts
    if raw.trim().is_empty() {
        return FormattedAbstract {
            markdown: "[Abstract not available]".to_string(),
            sections: vec![],
            had_structure: false,
        };
    }

    let cleaned = clean_input(raw);

    // Check for ALL CAPS sections first (common in some journals)
    if has_all_caps_sections(&cleaned) {
        return format_all_caps_sections(&cleaned);
    }

    // Try standard section detection
    let matches: Vec<_> = SECTION_REGEX.find_iter(&cleaned).collect();

    if matches.is_empty() {
        // No structure detected - return clean paragraphs
        return FormattedAbstract {
            markdown: cleaned,
            sections: vec![],
            had_structure: false,
        };
    }

    // Extract sections
    let mut sections = Vec::new();
    let mut parts = Vec::new();
    let mut last_end = 0;

    for m in matches.iter() {
        // Get text before this section (if any)
        if m.start() > last_end {
            let pre_text = cleaned[last_end..m.start()].trim();
            if !pre_text.is_empty() && parts.is_empty() {
                // Text before first section - treat as introduction
                parts.push(pre_text.to_string());
            }
        }

        // Extract section name from match
        let matched_text = m.as_str().trim();
        // Find the keyword within the match
        for keyword in SECTION_KEYWORDS.iter() {
            if matched_text.to_lowercase().contains(&keyword.to_lowercase()) {
                sections.push(keyword.to_string());
                break;
            }
        }

        last_end = m.end();
    }

    // Build markdown with section headers
    let mut markdown = String::new();
    let mut section_idx = 0;
    last_end = 0;

    for m in SECTION_REGEX.find_iter(&cleaned) {
        // Add any text before this section
        if m.start() > last_end {
            let pre_text = cleaned[last_end..m.start()].trim();
            if !pre_text.is_empty() {
                if !markdown.is_empty() {
                    markdown.push_str("\n\n");
                }
                markdown.push_str(pre_text);
            }
        }

        // Add section header
        if section_idx < sections.len() {
            if !markdown.is_empty() {
                markdown.push_str("\n\n");
            }
            markdown.push_str(&format!("**{}**\n", sections[section_idx]));
            section_idx += 1;
        }

        last_end = m.end();
    }

    // Add remaining text after last section
    if last_end < cleaned.len() {
        let remaining = cleaned[last_end..].trim();
        if !remaining.is_empty() {
            markdown.push_str(remaining);
        }
    }

    FormattedAbstract {
        markdown: markdown.trim().to_string(),
        sections,
        had_structure: true,
    }
}

/// Format text with ALL CAPS section headers
fn format_all_caps_sections(text: &str) -> FormattedAbstract {
    let mut sections = Vec::new();
    let mut markdown = String::new();
    let mut last_end = 0;

    for m in ALL_CAPS_REGEX.find_iter(text) {
        // Add text before this section
        if m.start() > last_end {
            let pre_text = text[last_end..m.start()].trim();
            if !pre_text.is_empty() {
                markdown.push_str(pre_text);
                markdown.push(' ');
            }
        }

        // Convert CAPS to Title Case and add as section header
        let section_name = m.as_str();
        let title_case = section_name.chars().next().unwrap().to_uppercase().to_string()
            + &section_name[1..].to_lowercase();

        sections.push(title_case.clone());
        markdown.push_str(&format!("\n\n**{}**\n", title_case));

        last_end = m.end();
    }

    // Add remaining text
    if last_end < text.len() {
        let remaining = text[last_end..].trim();
        if !remaining.is_empty() {
            markdown.push_str(remaining);
        }
    }

    FormattedAbstract {
        markdown: markdown.trim().to_string(),
        sections,
        had_structure: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_medical() {
        let abstract_text = "Background: Heart disease is prevalent. Methods: We studied 100 patients. Results: 80% showed improvement. Conclusion: Treatment is effective.";
        let result = format_abstract(abstract_text);

        assert!(result.had_structure);
        assert_eq!(result.sections.len(), 4);
        assert!(result.markdown.contains("**Background**"));
        assert!(result.markdown.contains("**Methods**"));
        assert!(result.markdown.contains("**Results**"));
        assert!(result.markdown.contains("**Conclusion**"));
    }

    #[test]
    fn test_all_caps() {
        let abstract_text = "BACKGROUND Heart disease is common. METHODS We analyzed data. RESULTS Significant findings. CONCLUSION Treatment works.";
        let result = format_abstract(abstract_text);

        assert!(result.had_structure);
        assert!(result.sections.contains(&"Background".to_string()));
        assert!(result.markdown.contains("**Background**"));
    }

    #[test]
    fn test_no_structure() {
        let abstract_text = "This is a simple abstract about chemistry without any section headers. It discusses molecular interactions.";
        let result = format_abstract(abstract_text);

        assert!(!result.had_structure);
        assert!(result.sections.is_empty());
        assert_eq!(result.markdown, abstract_text);
    }

    #[test]
    fn test_html_entities() {
        let abstract_text = "Results: p &lt; 0.05, CI [1.2&ndash;3.4], &alpha;=0.05";
        let result = format_abstract(abstract_text);

        assert!(result.markdown.contains("p < 0.05"));
        assert!(result.markdown.contains("α=0.05"));
        assert!(result.markdown.contains("1.2\u{2013}3.4")); // en-dash
    }

    #[test]
    fn test_multiword_sections() {
        let abstract_text = "Materials and Methods: We used advanced techniques. Data Analysis: Statistical tests were performed.";
        let result = format_abstract(abstract_text);

        assert!(result.had_structure);
        assert!(result.sections.contains(&"Materials and Methods".to_string()));
        assert!(result.markdown.contains("**Materials and Methods**"));
    }

    #[test]
    fn test_empty_abstract() {
        let result = format_abstract("");
        assert!(!result.had_structure);
        assert_eq!(result.markdown, "[Abstract not available]");

        let result2 = format_abstract("   ");
        assert!(!result2.had_structure);
        assert_eq!(result2.markdown, "[Abstract not available]");
    }

    #[test]
    fn test_jats_tags_stripped() {
        let abstract_text = "<jats:p>Background: <jats:italic>This is italic</jats:italic> text.</jats:p>";
        let result = format_abstract(abstract_text);

        assert!(!result.markdown.contains("<jats:"));
        assert!(result.markdown.contains("This is italic"));
    }
}
