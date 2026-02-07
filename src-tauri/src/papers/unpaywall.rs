//! Unpaywall API client
//!
//! Success rate: ~26% (lookup service, points to repository URLs that may 403)
//! Rate limit: None specified, but polite usage recommended
//! API key: Requires email (not a key, just for identification)
//! API docs: https://unpaywall.org/products/api

use super::ResolvedPdf;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Deserialize, Serialize)]
pub struct UnpaywallResponse {
    pub doi: String,
    pub is_oa: bool,
    pub best_oa_location: Option<OaLocation>,
    pub oa_locations: Option<Vec<OaLocation>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OaLocation {
    pub url: Option<String>,
    pub url_for_pdf: Option<String>,
    pub url_for_landing_page: Option<String>,
    pub version: Option<String>,
    pub license: Option<String>,
    pub host_type: Option<String>, // "publisher" or "repository"
}

/// Look up open access locations for a DOI
///
/// API endpoint: https://api.unpaywall.org/v2/{doi}?email={email}
pub async fn lookup_unpaywall(doi: &str, email: &str) -> Result<UnpaywallResponse, String> {
    if doi.is_empty() {
        return Err("DOI is required for Unpaywall lookup".to_string());
    }
    if email.is_empty() {
        return Err("Email is required for Unpaywall API".to_string());
    }

    let url = format!("https://api.unpaywall.org/v2/{}?email={}", doi, email);

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Mycelica/0.9.0 (mailto:{})")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to query Unpaywall API: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Unpaywall API returned status {}", response.status()));
    }

    let unpaywall_data: UnpaywallResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Unpaywall response: {}", e))?;

    Ok(unpaywall_data)
}

/// Download PDF from Unpaywall OA location
///
/// Prefers repository URLs over publisher URLs (higher success rate)
pub async fn download_unpaywall_pdf(
    unpaywall_data: &UnpaywallResponse,
) -> Result<ResolvedPdf, String> {
    if !unpaywall_data.is_oa {
        return Err("Paper is not open access according to Unpaywall".to_string());
    }

    // Try best location first
    if let Some(best_location) = &unpaywall_data.best_oa_location {
        if let Some(pdf_url) = get_pdf_url(best_location) {
            if let Ok(pdf) = download_pdf_from_url(&pdf_url).await {
                return Ok(pdf);
            }
        }
    }

    // Try other repository locations (prefer repositories over publishers)
    if let Some(locations) = &unpaywall_data.oa_locations {
        // First try all repository locations
        for location in locations {
            if let Some(host_type) = &location.host_type {
                if host_type == "repository" {
                    if let Some(pdf_url) = get_pdf_url(location) {
                        if let Ok(pdf) = download_pdf_from_url(&pdf_url).await {
                            return Ok(pdf);
                        }
                    }
                }
            }
        }

        // Then try publisher locations
        for location in locations {
            if let Some(host_type) = &location.host_type {
                if host_type == "publisher" {
                    if let Some(pdf_url) = get_pdf_url(location) {
                        if let Ok(pdf) = download_pdf_from_url(&pdf_url).await {
                            return Ok(pdf);
                        }
                    }
                }
            }
        }
    }

    Err("No accessible PDF found via Unpaywall".to_string())
}

/// Extract PDF URL from OA location (prefers direct PDF URL)
fn get_pdf_url(location: &OaLocation) -> Option<String> {
    location.url_for_pdf.clone().or_else(|| location.url.clone())
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
        source: "unpaywall".to_string(),
        url: url.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_pdf_url_prefers_pdf() {
        let location = OaLocation {
            url: Some("https://example.com/landing".to_string()),
            url_for_pdf: Some("https://example.com/file.pdf".to_string()),
            url_for_landing_page: Some("https://example.com/page".to_string()),
            version: Some("publishedVersion".to_string()),
            license: Some("cc-by".to_string()),
            host_type: Some("repository".to_string()),
        };

        assert_eq!(
            get_pdf_url(&location),
            Some("https://example.com/file.pdf".to_string())
        );
    }

    #[test]
    fn test_get_pdf_url_fallback_to_url() {
        let location = OaLocation {
            url: Some("https://example.com/paper.pdf".to_string()),
            url_for_pdf: None,
            url_for_landing_page: Some("https://example.com/page".to_string()),
            version: Some("publishedVersion".to_string()),
            license: Some("cc-by".to_string()),
            host_type: Some("repository".to_string()),
        };

        assert_eq!(
            get_pdf_url(&location),
            Some("https://example.com/paper.pdf".to_string())
        );
    }

    #[test]
    fn test_get_pdf_url_none() {
        let location = OaLocation {
            url: None,
            url_for_pdf: None,
            url_for_landing_page: Some("https://example.com/page".to_string()),
            version: Some("publishedVersion".to_string()),
            license: Some("cc-by".to_string()),
            host_type: Some("repository".to_string()),
        };

        assert_eq!(get_pdf_url(&location), None);
    }
}
