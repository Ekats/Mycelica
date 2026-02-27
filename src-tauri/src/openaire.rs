//! OpenAIRE API client for fetching scientific papers
//!
//! API Documentation: https://graph.openaire.eu/develop/api.html
//! Rate limit: ~15 requests/second

use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Validate URL for document download
fn is_valid_download_url(url_str: &str) -> bool {
    let parsed = match url::Url::parse(url_str) {
        Ok(u) => u,
        Err(_) => return false,
    };

    // Must be http/https
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }

    // Check hostname
    if let Some(host) = parsed.host_str() {
        if host == "localhost" || host == "127.0.0.1" || !host.contains('.') {
            return false;
        }
    } else {
        return false;
    }
    true
}

/// Check if URL points to a Word document
fn is_word_document_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.ends_with(".docx") || lower.ends_with(".doc")
}

/// Detect document format from magic bytes
fn detect_document_format(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"%PDF") {
        return Some("pdf");
    }
    // DOCX is a ZIP file starting with PK\x03\x04
    if bytes.starts_with(&[0x50, 0x4B, 0x03, 0x04]) {
        // Validate it's actually a DOCX by checking for word/document.xml
        if let Ok(mut archive) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) {
            if archive.by_name("word/document.xml").is_ok() {
                return Some("docx");
            }
        }
        // Could be a ZIP but not DOCX
        return None;
    }
    // Old DOC format (OLE Compound Document)
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return Some("doc");
    }
    None
}

/// OpenAIRE API client with rate limiting
pub struct OpenAireClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

/// Query parameters for paper search
#[derive(Debug, Clone, Default)]
pub struct OpenAireQuery {
    pub search: String,
    pub country: Option<String>,       // e.g., "EE" for Estonia
    pub fos: Option<String>,           // Field of science
    pub access_right: Option<String>,  // e.g., "OPEN"
    pub from_year: Option<String>,     // e.g., "2020" - filters from Jan 1
    pub to_year: Option<String>,       // e.g., "2025" - filters to Dec 31
    pub page_size: u32,                // Max 100
    pub page: u32,                     // Page number (1-indexed)
    pub sort_by: Option<String>,       // e.g., "publicationDate", "popularity", "influence"
}

/// Parsed paper from OpenAIRE response
#[derive(Debug, Clone, Serialize)]
pub struct OpenAirePaper {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub authors: Vec<Author>,
    pub publication_date: Option<String>,
    pub doi: Option<String>,
    pub journal: Option<String>,
    pub publisher: Option<String>,
    pub subjects: Vec<Subject>,
    pub pdf_urls: Vec<String>,
    pub access_right: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    #[serde(rename = "fullName")]
    pub full_name: String,
    pub orcid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subject {
    pub scheme: String,
    pub value: String,
}

/// Response from OpenAIRE API
#[derive(Debug, Deserialize)]
struct OpenAireResponse {
    header: Option<OpenAireHeader>,
    results: Option<Vec<RawPaper>>,
}

/// Header from OpenAIRE API response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OpenAireHeader {
    #[serde(rename = "numFound")]
    num_found: Option<u32>,
    page: Option<u32>,
    #[serde(rename = "pageSize")]
    page_size: Option<u32>,
}

/// Raw paper structure from API (nested JSON)
#[derive(Debug, Deserialize)]
struct RawPaper {
    #[serde(rename = "mainTitle")]
    main_title: Option<String>,
    #[serde(alias = "description")]
    descriptions: Option<Vec<String>>,
    id: Option<String>,
    authors: Option<Vec<RawAuthor>>,
    #[serde(rename = "publicationDate")]
    publication_date: Option<String>,
    pids: Option<Vec<RawPid>>,
    subjects: Option<Vec<RawSubjectWrapper>>,
    source: Option<Vec<RawSource>>,
    publisher: Option<String>,
    #[serde(rename = "bestAccessRight")]
    best_access_right: Option<RawAccessRight>,
    instances: Option<Vec<RawInstance>>,
}

#[derive(Debug, Deserialize)]
struct RawAuthor {
    #[serde(rename = "fullName")]
    full_name: Option<String>,
    pid: Option<RawAuthorPid>,
}

#[derive(Debug, Deserialize)]
struct RawAuthorPid {
    id: Option<RawPidValue>,
}

#[derive(Debug, Deserialize)]
struct RawPidValue {
    scheme: Option<String>,
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawPid {
    scheme: Option<String>,
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawSubjectWrapper {
    subject: Option<RawSubject>,
}

#[derive(Debug, Deserialize)]
struct RawSubject {
    scheme: Option<String>,
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawSource {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAccessRight {
    code: Option<String>,
    label: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct RawInstance {
    urls: Option<Vec<String>>,
    #[serde(rename = "accessRight")]
    access_right: Option<RawAccessRight>,
    #[serde(rename = "type")]
    instance_type: Option<String>,
}

impl OpenAireClient {
    /// Create a new OpenAIRE client (public API, lower rate limits)
    pub fn new() -> Self {
        Self::new_with_key(None)
    }

    /// Create a new OpenAIRE client with optional API key
    /// With key: higher rate limits, authenticated access
    pub fn new_with_key(api_key: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            base_url: "https://api.openaire.eu/graph/v2/researchProducts".to_string(),
            api_key,
        }
    }

    /// Fetch papers from OpenAIRE API
    /// Returns (papers, total_count) for pagination
    pub async fn fetch_papers(&self, query: &OpenAireQuery) -> Result<(Vec<OpenAirePaper>, u32), String> {
        let mut url = format!("{}?pageSize={}", self.base_url, query.page_size.min(100));

        // Add page number (1-indexed)
        if query.page > 0 {
            url.push_str(&format!("&page={}", query.page));
        }

        // Add search query
        if !query.search.is_empty() {
            url.push_str(&format!("&search={}", urlencoding::encode(&query.search)));
        }

        // Add country filter
        if let Some(country) = &query.country {
            url.push_str(&format!("&countryCode={}", country));
        }

        // Add field of science filter
        if let Some(fos) = &query.fos {
            url.push_str(&format!("&fos={}", urlencoding::encode(fos)));
        }

        // Add date range filters
        if let Some(from) = &query.from_year {
            url.push_str(&format!("&fromPublicationDate={}-01-01", from));
        }
        if let Some(to) = &query.to_year {
            url.push_str(&format!("&toPublicationDate={}-12-31", to));
        }

        // Add access rights filter
        if let Some(access) = &query.access_right {
            url.push_str(&format!("&bestOpenAccessRightLabel={}", access));
        }

        // Add sort order
        if let Some(sort) = &query.sort_by {
            url.push_str(&format!("&sortBy={}", urlencoding::encode(sort)));
        }

        // Log query details
        println!("[OpenAIRE] Searching: \"{}\"", query.search);
        if query.country.is_some() || query.fos.is_some() || query.from_year.is_some() || query.to_year.is_some() {
            let mut filters = Vec::new();
            if let Some(c) = &query.country { filters.push(format!("country: {}", c)); }
            if let Some(f) = &query.fos { filters.push(format!("field: {}", f)); }
            if let Some(from) = &query.from_year { filters.push(format!("from: {}", from)); }
            if let Some(to) = &query.to_year { filters.push(format!("to: {}", to)); }
            println!("[OpenAIRE]   Filters: {}", filters.join(", "));
        }

        let mut request = self.client
            .get(&url)
            .header("Accept", "application/json");

        // Add auth header if API key is present
        if let Some(key) = &self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("API error {}: {}", status, body));
        }

        let api_response: OpenAireResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        let papers: Vec<OpenAirePaper> = api_response
            .results
            .unwrap_or_default()
            .into_iter()
            .filter_map(|raw| self.parse_paper(raw))
            .collect();

        // Extract total count from header
        let total_count = api_response.header
            .and_then(|h| h.num_found)
            .unwrap_or(0);

        println!("[OpenAIRE]   Fetched {}/{} papers", papers.len(), total_count);

        Ok((papers, total_count))
    }

    /// Parse raw API paper into our structure
    fn parse_paper(&self, raw: RawPaper) -> Option<OpenAirePaper> {
        let title = raw.main_title?;
        let id = raw.id.unwrap_or_default();

        // Extract abstract (first description)
        let description = raw.descriptions.and_then(|d| d.into_iter().next());

        // Extract authors
        let authors: Vec<Author> = raw.authors
            .unwrap_or_default()
            .into_iter()
            .filter_map(|a| {
                let full_name = a.full_name?;
                let orcid = a.pid
                    .and_then(|p| p.id)
                    .filter(|id| id.scheme.as_deref() == Some("orcid"))
                    .and_then(|id| id.value);
                Some(Author { full_name, orcid })
            })
            .collect();

        // Extract DOI
        let doi = raw.pids
            .unwrap_or_default()
            .into_iter()
            .find(|p| p.scheme.as_deref() == Some("doi"))
            .and_then(|p| p.value);

        // Extract journal name
        let journal = raw.source
            .and_then(|s| s.into_iter().next())
            .and_then(|s| s.name);

        // Extract subjects
        let subjects: Vec<Subject> = raw.subjects
            .unwrap_or_default()
            .into_iter()
            .filter_map(|sw| {
                let s = sw.subject?;
                Some(Subject {
                    scheme: s.scheme.unwrap_or_default(),
                    value: s.value.unwrap_or_default(),
                })
            })
            .filter(|s| !s.value.is_empty())
            .collect();

        // Extract PDF URLs from instances
        let all_urls: Vec<String> = raw.instances
            .unwrap_or_default()
            .into_iter()
            .filter(|inst| {
                // Only include OPEN access instances (label="OPEN" or code="c_abf2")
                inst.access_right
                    .as_ref()
                    .map(|ar| {
                        ar.label.as_deref() == Some("OPEN") || ar.code.as_deref() == Some("c_abf2")
                    })
                    .unwrap_or(false)
            })
            .flat_map(|inst| inst.urls.unwrap_or_default())
            .collect::<Vec<_>>();

        // Prefer actual PDF URLs over DOI/landing page links
        let pdf_urls: Vec<String> = {
            let pdfs: Vec<_> = all_urls.iter()
                .filter(|u| u.to_lowercase().ends_with(".pdf"))
                .cloned()
                .collect();
            if pdfs.is_empty() { all_urls } else { pdfs }
        };

        let access_right = raw.best_access_right
            .and_then(|ar| ar.code)
            .unwrap_or_else(|| "UNKNOWN".to_string());

        Some(OpenAirePaper {
            id,
            title,
            description,
            authors,
            publication_date: raw.publication_date,
            doi,
            journal,
            publisher: raw.publisher,
            subjects,
            pdf_urls,
            access_right,
        })
    }

    /// Count papers matching query without fetching all data
    /// Useful for showing preview count before import
    pub async fn count_papers(&self, query: &OpenAireQuery) -> Result<u32, String> {
        // Use fetch_papers with page_size=1 to get the count from header
        let count_query = OpenAireQuery {
            page_size: 1,
            ..query.clone()
        };

        println!("[OpenAIRE] Counting: \"{}\"", query.search);

        let (_, total_count) = self.fetch_papers(&count_query).await?;

        println!("[OpenAIRE]   Found {} papers", total_count);
        Ok(total_count)
    }

    /// Download PDF from URL with size limit
    /// Returns None if download fails or exceeds size limit
    /// Falls back to scraping landing page for citation_pdf_url if direct download fails
    pub async fn download_pdf(&self, url: &str, max_size_mb: u32) -> Result<Option<Vec<u8>>, String> {
        // Validate URL before attempting download
        if !is_valid_download_url(url) {
            println!("[OpenAIRE] Invalid URL, skipping: {}", url);
            return Ok(None);
        }

        let max_size = (max_size_mb as usize) * 1024 * 1024;

        // First try direct download
        let result = self.try_download_pdf(url, max_size).await;

        if result.is_ok() && result.as_ref().unwrap().is_some() {
            return result;
        }

        // If direct download failed, try to scrape PDF URL from landing page
        println!("[OpenAIRE] Direct download failed, scraping landing page: {}", url);
        if let Some(pdf_url) = self.extract_pdf_url_from_page(url).await {
            if pdf_url != url {
                println!("[OpenAIRE] Found PDF URL in page: {}", pdf_url);
                return self.try_download_pdf(&pdf_url, max_size).await;
            }
        }

        result
    }

    /// Try to download PDF directly from URL
    async fn try_download_pdf(&self, url: &str, max_size: usize) -> Result<Option<Vec<u8>>, String> {
        println!("[OpenAIRE] Downloading PDF: {}", url);

        let response = self.client
            .get(url)
            .header("Accept", "application/pdf")
            .header("User-Agent", "Mozilla/5.0 (compatible; Mycelica/1.0)")
            .send()
            .await
            .map_err(|e| format!("PDF download failed: {}", e))?;

        if !response.status().is_success() {
            println!("[OpenAIRE] PDF download failed: {}", response.status());
            return Ok(None);
        }

        // Check content-length header first
        if let Some(content_length) = response.content_length() {
            if content_length as usize > max_size {
                println!("[OpenAIRE] PDF too large: {} MB (limit: {} MB)",
                    content_length / 1024 / 1024, max_size / 1024 / 1024);
                return Ok(None);
            }
        }

        let raw_bytes = response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read PDF bytes: {}", e))?;

        // Handle gzip-compressed responses
        let bytes: Vec<u8> = if raw_bytes.starts_with(&[0x1f, 0x8b]) {
            println!("[OpenAIRE] Decompressing gzip response...");
            use std::io::Read;
            let mut decoder = flate2::read::GzDecoder::new(&raw_bytes[..]);
            let mut decompressed = Vec::new();
            match decoder.read_to_end(&mut decompressed) {
                Ok(_) => decompressed,
                Err(e) => {
                    println!("[OpenAIRE] Gzip decompression failed: {}", e);
                    return Ok(None);
                }
            }
        } else {
            raw_bytes.to_vec()
        };

        if bytes.len() > max_size {
            println!("[OpenAIRE] PDF too large: {} MB (limit: {} MB)",
                bytes.len() / 1024 / 1024, max_size / 1024 / 1024);
            return Ok(None);
        }

        // Verify it's actually a PDF
        if !bytes.starts_with(b"%PDF") {
            println!("[OpenAIRE] Not a valid PDF file (got {} bytes starting with {:?})",
                bytes.len(), &bytes[..std::cmp::min(20, bytes.len())]);
            return Ok(None);
        }

        println!("[OpenAIRE] PDF downloaded: {} KB", bytes.len() / 1024);
        Ok(Some(bytes))
    }

    /// Extract PDF URL from landing page by scraping citation_pdf_url meta tag
    async fn extract_pdf_url_from_page(&self, url: &str) -> Option<String> {
        // Fetch the landing page HTML
        let response = self.client
            .get(url)
            .header("User-Agent", "Mozilla/5.0 (compatible; Mycelica/1.0)")
            .send()
            .await
            .ok()?;

        if !response.status().is_success() {
            return None;
        }

        let html = response.text().await.ok()?;

        // Try citation_pdf_url meta tag (academic standard)
        // Format: <meta name="citation_pdf_url" content="...">
        let pdf_meta_re = Regex::new(r#"<meta\s+name="citation_pdf_url"\s+content="([^"]+)""#).ok()?;
        if let Some(cap) = pdf_meta_re.captures(&html) {
            let extracted_url = cap[1].to_string();
            if is_valid_download_url(&extracted_url) {
                return Some(extracted_url);
            }
        }

        // Also try alternate format (content before name)
        // Format: <meta content="..." name="citation_pdf_url">
        let pdf_meta_re2 = Regex::new(r#"<meta\s+content="([^"]+)"\s+name="citation_pdf_url""#).ok()?;
        if let Some(cap) = pdf_meta_re2.captures(&html) {
            let extracted_url = cap[1].to_string();
            if is_valid_download_url(&extracted_url) {
                return Some(extracted_url);
            }
        }

        // Try DC.identifier with PDF extension
        let dc_re = Regex::new(r#"<meta\s+name="DC\.identifier"\s+content="([^"]+\.pdf[^"]*)""#).ok()?;
        if let Some(cap) = dc_re.captures(&html) {
            let extracted_url = cap[1].to_string();
            if is_valid_download_url(&extracted_url) {
                return Some(extracted_url);
            }
        }

        // Try og:url or direct PDF links as last resort
        let og_pdf_re = Regex::new(r#"<meta\s+property="og:url"\s+content="([^"]+\.pdf[^"]*)""#).ok()?;
        if let Some(cap) = og_pdf_re.captures(&html) {
            let extracted_url = cap[1].to_string();
            if is_valid_download_url(&extracted_url) {
                return Some(extracted_url);
            }
        }

        None
    }

    /// Download document (PDF, DOCX, DOC) from URL with size limit
    /// Returns (bytes, format) where format is "pdf", "docx", or "doc"
    /// Falls back to scraping landing page for PDF URLs if direct download fails
    pub async fn download_document(&self, url: &str, max_size_mb: u32) -> Result<Option<(Vec<u8>, String)>, String> {
        // Validate URL before attempting download
        if !is_valid_download_url(url) {
            println!("[OpenAIRE] Invalid URL, skipping: {}", url);
            return Ok(None);
        }

        let max_size = (max_size_mb as usize) * 1024 * 1024;

        // First try direct download
        let result = self.try_download_document(url, max_size).await;

        if result.is_ok() && result.as_ref().unwrap().is_some() {
            return result;
        }

        // If direct download failed and URL looks like it could be a landing page (not .pdf/.docx/.doc),
        // try to scrape PDF URL from the page
        let lower_url = url.to_lowercase();
        if !lower_url.ends_with(".pdf") && !is_word_document_url(url) {
            println!("[OpenAIRE] Direct download failed, scraping landing page: {}", url);
            if let Some(pdf_url) = self.extract_pdf_url_from_page(url).await {
                if pdf_url != url {
                    println!("[OpenAIRE] Found PDF URL in page: {}", pdf_url);
                    return self.try_download_document(&pdf_url, max_size).await;
                }
            }
        }

        result
    }

    /// Try to download document directly from URL
    async fn try_download_document(&self, url: &str, max_size: usize) -> Result<Option<(Vec<u8>, String)>, String> {
        println!("[OpenAIRE] Downloading document: {}", url);

        let response = self.client
            .get(url)
            .header("Accept", "application/pdf, application/msword, application/vnd.openxmlformats-officedocument.wordprocessingml.document, */*")
            .header("User-Agent", "Mozilla/5.0 (compatible; Mycelica/1.0)")
            .send()
            .await
            .map_err(|e| format!("Document download failed: {}", e))?;

        if !response.status().is_success() {
            println!("[OpenAIRE] Document download failed: {}", response.status());
            return Ok(None);
        }

        // Check content-length header first
        if let Some(content_length) = response.content_length() {
            if content_length as usize > max_size {
                println!("[OpenAIRE] Document too large: {} MB (limit: {} MB)",
                    content_length / 1024 / 1024, max_size / 1024 / 1024);
                return Ok(None);
            }
        }

        let raw_bytes = response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read document bytes: {}", e))?;

        // Handle gzip-compressed responses
        let bytes: Vec<u8> = if raw_bytes.starts_with(&[0x1f, 0x8b]) {
            println!("[OpenAIRE] Decompressing gzip response...");
            use std::io::Read;
            let mut decoder = flate2::read::GzDecoder::new(&raw_bytes[..]);
            let mut decompressed = Vec::new();
            match decoder.read_to_end(&mut decompressed) {
                Ok(_) => {
                    println!("[OpenAIRE] Decompressed gzip: {} -> {} bytes", raw_bytes.len(), decompressed.len());
                    decompressed
                }
                Err(e) => {
                    println!("[OpenAIRE] Gzip decompression failed: {}", e);
                    return Ok(None);
                }
            }
        } else {
            raw_bytes.to_vec()
        };

        if bytes.len() > max_size {
            println!("[OpenAIRE] Document too large: {} MB (limit: {} MB)",
                bytes.len() / 1024 / 1024, max_size / 1024 / 1024);
            return Ok(None);
        }

        // Detect format from magic bytes
        if let Some(format) = detect_document_format(&bytes) {
            println!("[OpenAIRE] Document downloaded: {} KB, format: {}", bytes.len() / 1024, format);
            return Ok(Some((bytes, format.to_string())));
        }

        println!("[OpenAIRE] Unknown document format (got {} bytes starting with {:?})",
            bytes.len(), &bytes[..std::cmp::min(20, bytes.len())]);
        Ok(None)
    }

    /// Sleep for rate limiting (call between requests)
    /// Authenticated requests can use shorter delays (~15 req/s)
    /// Public API should use longer delays (~7 req/s)
    pub async fn rate_limit_delay(&self) {
        let delay_ms = if self.api_key.is_some() { 70 } else { 150 };
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
}

impl Default for OpenAireClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_building() {
        let query = OpenAireQuery {
            search: "semiotics".to_string(),
            country: Some("EE".to_string()),
            fos: Some("humanities".to_string()),
            access_right: Some("OPEN".to_string()),
            from_year: None,
            to_year: None,
            page_size: 50,
            page: 1,
            sort_by: None,
        };
        assert_eq!(query.page_size, 50);
        assert_eq!(query.page, 1);
    }
}
