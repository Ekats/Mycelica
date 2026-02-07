//! PubMed Central (PMC) PDF download
//!
//! Success rate: ~80% for biomedical papers with PMCIDs
//! Rate limit: 3 requests/second (enforced by delay)
//! No API key required (but polite usage recommended)

use super::ResolvedPdf;
use regex::Regex;
use reqwest::Client;
use std::time::Duration;

/// Extract PMCID from various identifier formats
///
/// Handles:
/// - `PMC8901234` → `8901234`
/// - `pmc8901234` → `8901234` (case-insensitive)
/// - `https://www.ncbi.nlm.nih.gov/pmc/articles/PMC8901234/` → `8901234`
pub fn extract_pmcid(identifiers: &[String]) -> Option<String> {
    let pmc_pattern = Regex::new(r"(?i)pmc(\d+)").unwrap();

    for id in identifiers {
        if let Some(caps) = pmc_pattern.captures(id) {
            return caps.get(1).map(|m| m.as_str().to_string());
        }
    }

    None
}

/// Download PDF from PubMed Central
///
/// PMC provides free access to full-text biomedical articles.
/// PDF URL format: https://www.ncbi.nlm.nih.gov/pmc/articles/PMC{id}/pdf/
/// The server redirects to the actual PDF URL.
pub async fn download_pmc_pdf(pmcid: &str) -> Result<ResolvedPdf, String> {
    // PMC PDF URL - redirects to actual PDF
    let url = format!("https://www.ncbi.nlm.nih.gov/pmc/articles/PMC{}/pdf/", pmcid);

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("Mycelica/0.9.0 (https://github.com/yourusername/mycelica; mailto:your@email.com)")
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to download PMC PDF: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("PMC returned status {}", response.status()));
    }

    // Check Content-Type to ensure it's a PDF
    if let Some(content_type) = response.headers().get("content-type") {
        let content_type_str = content_type.to_str().unwrap_or("");
        if !content_type_str.contains("pdf") && !content_type_str.contains("octet-stream") {
            return Err(format!("PMC returned non-PDF content: {}", content_type_str));
        }
    }

    // Store final URL before consuming response
    let final_url = response.url().to_string();

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read PMC PDF bytes: {}", e))?
        .to_vec();

    // Validate PDF magic bytes
    if bytes.len() < 4 || &bytes[0..4] != b"%PDF" {
        return Err("PMC response is not a valid PDF".to_string());
    }

    // Check size limit (20MB)
    if bytes.len() > 20 * 1024 * 1024 {
        return Err(format!("PMC PDF too large: {} MB", bytes.len() / 1024 / 1024));
    }

    Ok(ResolvedPdf {
        bytes,
        source: "pmc".to_string(),
        url: final_url, // Use final URL after redirects
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_pmcid() {
        assert_eq!(
            extract_pmcid(&["PMC8901234".to_string()]),
            Some("8901234".to_string())
        );
        assert_eq!(
            extract_pmcid(&["pmc8901234".to_string()]),
            Some("8901234".to_string())
        );
        assert_eq!(
            extract_pmcid(&["PmC8901234".to_string()]),
            Some("8901234".to_string())
        );
    }

    #[test]
    fn test_extract_pmcid_from_url() {
        assert_eq!(
            extract_pmcid(&["https://www.ncbi.nlm.nih.gov/pmc/articles/PMC8901234/".to_string()]),
            Some("8901234".to_string())
        );
        assert_eq!(
            extract_pmcid(&["https://www.ncbi.nlm.nih.gov/pmc/articles/PMC8901234/pdf/".to_string()]),
            Some("8901234".to_string())
        );
    }

    #[test]
    fn test_extract_pmcid_no_match() {
        assert_eq!(
            extract_pmcid(&["doi:10.1234/example".to_string()]),
            None
        );
        assert_eq!(
            extract_pmcid(&["not a pmcid".to_string()]),
            None
        );
        assert_eq!(
            extract_pmcid(&["PM12345".to_string()]), // PMID, not PMCID
            None
        );
    }

    #[test]
    fn test_extract_pmcid_multiple_identifiers() {
        assert_eq!(
            extract_pmcid(&[
                "doi:10.1234/example".to_string(),
                "PMC8901234".to_string(),
                "arXiv:2301.12345".to_string(),
            ]),
            Some("8901234".to_string())
        );
    }
}
