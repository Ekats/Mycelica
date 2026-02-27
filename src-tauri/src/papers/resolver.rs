//! PDF Resolver - Priority-based fallback chain
//!
//! Tries sources in order of success rate:
//! 1. arXiv (95%)
//! 2. PMC (80%)
//! 3. Unpaywall (26%)
//! 4. CORE (9%)
//! 5. OpenAIRE URLs (5%)

use super::{ResolvedPdf, ResolutionStats};
use super::arxiv::{extract_arxiv_id, download_arxiv_pdf};
use super::pmc::{extract_pmcid, download_pmc_pdf};
use super::unpaywall::{lookup_unpaywall, download_unpaywall_pdf};
use super::core_api::{search_core_by_doi, download_core_pdf};

/// PDF Resolver that tries multiple sources in priority order
pub struct PdfResolver {
    pub stats: ResolutionStats,
    unpaywall_email: Option<String>,
    core_api_key: Option<String>,
}

impl PdfResolver {
    pub fn new(unpaywall_email: Option<String>, core_api_key: Option<String>) -> Self {
        Self {
            stats: ResolutionStats::new(),
            unpaywall_email,
            core_api_key,
        }
    }

    /// Resolve PDF using priority-based fallback chain
    ///
    /// Priority order:
    /// 1. arXiv (if arXiv ID found in identifiers)
    /// 2. PMC (if PMCID found in identifiers)
    /// 3. Unpaywall (if DOI available and email configured)
    /// 4. CORE (if DOI available and API key configured)
    /// 5. OpenAIRE URLs (fallback)
    pub async fn resolve(
        &mut self,
        doi: Option<&str>,
        identifiers: &[String],
        openaire_urls: &[String],
    ) -> Result<ResolvedPdf, String> {
        // 1. Try arXiv (95% success)
        if let Some(arxiv_id) = extract_arxiv_id(identifiers) {
            self.stats.arxiv_attempts += 1;
            eprintln!("[PDF Resolver] Trying arXiv ID: {:?} (from identifiers: {:?})", arxiv_id, identifiers);
            match download_arxiv_pdf(&arxiv_id).await {
                Ok(pdf) => {
                    self.stats.arxiv_success += 1;
                    return Ok(pdf);
                }
                Err(e) => {
                    eprintln!("[PDF Resolver] arXiv failed: {}", e);
                }
            }
        }

        // 2. Try PMC (80% success)
        if let Some(pmcid) = extract_pmcid(identifiers) {
            self.stats.pmc_attempts += 1;
            eprintln!("[PDF Resolver] Trying PMC ID: {:?} (from identifiers: {:?})", pmcid, identifiers);
            match download_pmc_pdf(&pmcid).await {
                Ok(pdf) => {
                    self.stats.pmc_success += 1;
                    return Ok(pdf);
                }
                Err(e) => {
                    eprintln!("[PDF Resolver] PMC failed: {}", e);
                }
            }
        }

        // 3. Try Unpaywall (26% success)
        if let (Some(doi_str), Some(email)) = (doi, &self.unpaywall_email) {
            self.stats.unpaywall_attempts += 1;
            match lookup_unpaywall(doi_str, email).await {
                Ok(unpaywall_data) => {
                    match download_unpaywall_pdf(&unpaywall_data).await {
                        Ok(pdf) => {
                            self.stats.unpaywall_success += 1;
                            return Ok(pdf);
                        }
                        Err(e) => {
                            eprintln!("[PDF Resolver] Unpaywall download failed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[PDF Resolver] Unpaywall lookup failed: {}", e);
                }
            }
        }

        // 4. Try CORE (9% success)
        if let (Some(doi_str), Some(api_key)) = (doi, &self.core_api_key) {
            self.stats.core_attempts += 1;
            match search_core_by_doi(doi_str, api_key).await {
                Ok(search_results) => {
                    if let Some(first_work) = search_results.results.first() {
                        match download_core_pdf(first_work).await {
                            Ok(pdf) => {
                                self.stats.core_success += 1;
                                return Ok(pdf);
                            }
                            Err(e) => {
                                eprintln!("[PDF Resolver] CORE download failed: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[PDF Resolver] CORE search failed: {}", e);
                }
            }
        }

        // 5. Try OpenAIRE URLs (5% baseline success) - fallback
        for _url in openaire_urls {
            self.stats.openaire_attempts += 1;
            // We'll need to call the existing OpenAIRE download method here
            // For now, just track the attempt
            // TODO: Integrate with existing OpenAireClient::download_document
        }

        Err("No PDF source succeeded".to_string())
    }

    /// Get statistics summary
    pub fn get_stats(&self) -> &ResolutionStats {
        &self.stats
    }
}
