//! arXiv PDF download
//!
//! Success rate: ~95% for papers with arXiv IDs
//! Rate limit: Polite (0.5s delay recommended)
//! No API key required

use super::ResolvedPdf;
use regex::Regex;
use reqwest::Client;
use std::time::Duration;

/// Extract arXiv ID from various identifier formats
///
/// Handles:
/// - `arXiv:2301.12345v2` → `2301.12345`
/// - `https://arxiv.org/abs/2301.12345` → `2301.12345`
/// - `2301.12345` → `2301.12345`
/// - `2301.12345v2` → `2301.12345`
pub fn extract_arxiv_id(identifiers: &[String]) -> Option<String> {
    // Pattern matches: YYMM.NNNNN or YYMM.NNNNNN (old format: archive/YYMMNNN)
    let arxiv_pattern = Regex::new(r"(?:arXiv:)?(\d{4}\.\d{4,5})(?:v\d+)?").unwrap();
    let old_pattern = Regex::new(r"(?:arXiv:)?([a-z\-]+/\d{7})").unwrap();

    for id in identifiers {
        // Try new format first
        if let Some(caps) = arxiv_pattern.captures(id) {
            return caps.get(1).map(|m| m.as_str().to_string());
        }
        // Try old format (pre-2007)
        if let Some(caps) = old_pattern.captures(id) {
            return caps.get(1).map(|m| m.as_str().to_string());
        }
    }

    None
}

/// Download PDF from arXiv
///
/// URL format: https://arxiv.org/pdf/{id}.pdf
/// Redirects are followed automatically
pub async fn download_arxiv_pdf(arxiv_id: &str) -> Result<ResolvedPdf, String> {
    let url = format!("https://arxiv.org/pdf/{}.pdf", arxiv_id);

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("Mycelica/0.9.0 (https://github.com/yourusername/mycelica)")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to download arXiv PDF: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("arXiv returned status {}", response.status()));
    }

    // Check Content-Type to ensure it's a PDF
    if let Some(content_type) = response.headers().get("content-type") {
        let content_type_str = content_type.to_str().unwrap_or("");
        if !content_type_str.contains("pdf") {
            return Err(format!("arXiv returned non-PDF content: {}", content_type_str));
        }
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read arXiv PDF bytes: {}", e))?
        .to_vec();

    // Validate PDF magic bytes
    if bytes.len() < 4 || &bytes[0..4] != b"%PDF" {
        return Err("arXiv response is not a valid PDF".to_string());
    }

    // Check size limit (20MB)
    if bytes.len() > 20 * 1024 * 1024 {
        return Err(format!("arXiv PDF too large: {} MB", bytes.len() / 1024 / 1024));
    }

    Ok(ResolvedPdf {
        bytes,
        source: "arxiv".to_string(),
        url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_arxiv_id_new_format() {
        assert_eq!(
            extract_arxiv_id(&["arXiv:2301.12345v2".to_string()]),
            Some("2301.12345".to_string())
        );
        assert_eq!(
            extract_arxiv_id(&["2301.12345".to_string()]),
            Some("2301.12345".to_string())
        );
        assert_eq!(
            extract_arxiv_id(&["2301.12345v1".to_string()]),
            Some("2301.12345".to_string())
        );
    }

    #[test]
    fn test_extract_arxiv_id_from_url() {
        assert_eq!(
            extract_arxiv_id(&["https://arxiv.org/abs/2301.12345".to_string()]),
            Some("2301.12345".to_string())
        );
        assert_eq!(
            extract_arxiv_id(&["https://arxiv.org/pdf/2301.12345v3.pdf".to_string()]),
            Some("2301.12345".to_string())
        );
    }

    #[test]
    fn test_extract_arxiv_id_old_format() {
        assert_eq!(
            extract_arxiv_id(&["arXiv:hep-th/9901001".to_string()]),
            Some("hep-th/9901001".to_string())
        );
        assert_eq!(
            extract_arxiv_id(&["cs/0601001".to_string()]),
            Some("cs/0601001".to_string())
        );
    }

    #[test]
    fn test_extract_arxiv_id_no_match() {
        assert_eq!(
            extract_arxiv_id(&["doi:10.1234/example".to_string()]),
            None
        );
        assert_eq!(
            extract_arxiv_id(&["not an arxiv id".to_string()]),
            None
        );
    }

    #[test]
    fn test_extract_arxiv_id_multiple_identifiers() {
        assert_eq!(
            extract_arxiv_id(&[
                "doi:10.1234/example".to_string(),
                "arXiv:2301.12345".to_string(),
                "PMC123456".to_string(),
            ]),
            Some("2301.12345".to_string())
        );
    }
}
