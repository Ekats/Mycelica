//! CORE API v3 client
//!
//! Success rate: ~9% (hosts PDFs directly for ~30M papers)
//! Rate limit: 10,000 requests/month free tier
//! API key: Required (get from https://core.ac.uk/services/api)
//! API docs: https://core.ac.uk/docs/

use super::ResolvedPdf;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Deserialize, Serialize)]
pub struct CoreSearchResponse {
    pub results: Vec<CoreWork>,
    #[serde(rename = "totalHits")]
    pub total_hits: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CoreWork {
    pub id: String,
    pub title: Option<String>,
    pub doi: Option<String>,
    #[serde(rename = "downloadUrl")]
    pub download_url: Option<String>,
    #[serde(rename = "sourceFulltextUrls")]
    pub source_fulltext_urls: Option<Vec<String>>,
}

/// Search CORE API by DOI
///
/// API endpoint: https://api.core.ac.uk/v3/search/works?q=doi:{doi}
pub async fn search_core_by_doi(doi: &str, api_key: &str) -> Result<CoreSearchResponse, String> {
    if doi.is_empty() {
        return Err("DOI is required for CORE search".to_string());
    }
    if api_key.is_empty() {
        return Err("API key is required for CORE API".to_string());
    }

    // Escape the DOI for URL query
    let escaped_doi = urlencoding::encode(doi);
    let url = format!("https://api.core.ac.uk/v3/search/works?q=doi:{}", escaped_doi);

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Mycelica/0.9.0 (https://github.com/yourusername/mycelica)")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Failed to query CORE API: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("CORE API returned status {}", response.status()));
    }

    let search_results: CoreSearchResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse CORE response: {}", e))?;

    Ok(search_results)
}

/// Download PDF from CORE
///
/// Prefers CORE's hosted downloadUrl over source URLs (higher success rate)
pub async fn download_core_pdf(core_work: &CoreWork) -> Result<ResolvedPdf, String> {
    // Try CORE's hosted download URL first (most reliable)
    if let Some(download_url) = &core_work.download_url {
        if let Ok(pdf) = download_pdf_from_url(download_url).await {
            return Ok(pdf);
        }
    }

    // Try source fulltext URLs as fallback
    if let Some(source_urls) = &core_work.source_fulltext_urls {
        for url in source_urls {
            if url.ends_with(".pdf") || url.contains("/pdf") {
                if let Ok(pdf) = download_pdf_from_url(url).await {
                    return Ok(pdf);
                }
            }
        }
    }

    Err("No accessible PDF found via CORE".to_string())
}

/// Download PDF from a given URL
async fn download_pdf_from_url(url: &str) -> Result<ResolvedPdf, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("Mycelica/0.9.0 (https://github.com/yourusername/mycelica)")
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to download PDF from {}: {}", url, e))?;

    if !response.status().is_success() {
        return Err(format!("URL {} returned status {}", url, response.status()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read PDF bytes: {}", e))?
        .to_vec();

    // Validate PDF magic bytes
    if bytes.len() < 4 || &bytes[0..4] != b"%PDF" {
        return Err(format!("URL {} did not return a valid PDF", url));
    }

    // Check size limit (20MB)
    if bytes.len() > 20 * 1024 * 1024 {
        return Err(format!("PDF from {} too large: {} MB", url, bytes.len() / 1024 / 1024));
    }

    Ok(ResolvedPdf {
        bytes,
        source: "core".to_string(),
        url: url.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_work_deserialization() {
        let json = r#"{
            "id": "12345",
            "title": "Example Paper",
            "doi": "10.1234/example",
            "downloadUrl": "https://core.ac.uk/download/12345.pdf",
            "sourceFulltextUrls": ["https://arxiv.org/pdf/2301.12345.pdf"]
        }"#;

        let work: CoreWork = serde_json::from_str(json).unwrap();
        assert_eq!(work.id, "12345");
        assert_eq!(work.download_url, Some("https://core.ac.uk/download/12345.pdf".to_string()));
        assert_eq!(work.source_fulltext_urls.as_ref().unwrap().len(), 1);
    }
}
