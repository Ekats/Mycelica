//! Section parser for academic papers
//!
//! Extracts abstract and conclusion from full text using:
//! - Pattern matching for section headers
//! - Text cleaning (hyphenation, whitespace)
//! - Multiple fallback patterns

use super::ExtractedSections;

/// Parse sections from full text
///
/// Extracts:
/// - Abstract (between "Abstract" and "Introduction")
/// - Conclusion (between "Conclusion" and "References")
pub fn parse_sections(full_text: &str) -> ExtractedSections {
    // TODO: Implement in Phase 5
    // This is a stub that will be implemented later
    ExtractedSections {
        abstract_text: None,
        conclusion: None,
        full_text: full_text.to_string(),
    }
}

#[cfg(test)]
/// Clean text by removing hyphenation and normalizing whitespace
fn clean_text(text: &str) -> String {
    text.replace("-\n", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_text_removes_hyphenation() {
        assert_eq!(clean_text("exam-\nple"), "example");
    }

    #[test]
    fn test_clean_text_normalizes_whitespace() {
        assert_eq!(clean_text("hello  \n  world"), "hello world");
    }
}
