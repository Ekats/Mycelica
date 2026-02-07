//! PDF resolution and text extraction for scientific papers
//!
//! This module provides multi-source PDF downloading with priority-based fallback:
//! 1. arXiv (95% success for physics/CS)
//! 2. PMC (80% success for biomedical)
//! 3. Unpaywall (26% success via repository lookups)
//! 4. CORE (9% success via direct hosting)
//! 5. OpenAIRE URLs (5% baseline)

pub mod arxiv;
pub mod pmc;
pub mod unpaywall;
pub mod core_api;
pub mod pdf_extractor;
pub mod section_parser;
pub mod resolver;

/// Result of a successful PDF resolution
#[derive(Debug, Clone)]
pub struct ResolvedPdf {
    /// PDF bytes
    pub bytes: Vec<u8>,
    /// Source that provided the PDF ("arxiv", "pmc", "unpaywall", "core", "openaire")
    pub source: String,
    /// Original URL where PDF was downloaded from
    pub url: String,
}

/// Statistics for PDF resolution across all sources
#[derive(Debug, Default, Clone)]
pub struct ResolutionStats {
    pub arxiv_success: u32,
    pub arxiv_attempts: u32,
    pub pmc_success: u32,
    pub pmc_attempts: u32,
    pub unpaywall_success: u32,
    pub unpaywall_attempts: u32,
    pub core_success: u32,
    pub core_attempts: u32,
    pub openaire_success: u32,
    pub openaire_attempts: u32,
}

impl ResolutionStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn print_summary(&self) {
        println!("\nPDF Resolution Results:");
        self.print_source("arXiv", self.arxiv_success, self.arxiv_attempts);
        self.print_source("PMC", self.pmc_success, self.pmc_attempts);
        self.print_source("Unpaywall", self.unpaywall_success, self.unpaywall_attempts);
        self.print_source("CORE", self.core_success, self.core_attempts);
        self.print_source("OpenAIRE", self.openaire_success, self.openaire_attempts);

        let total_success = self.arxiv_success + self.pmc_success +
                           self.unpaywall_success + self.core_success + self.openaire_success;
        let total_attempts = self.arxiv_attempts + self.pmc_attempts +
                            self.unpaywall_attempts + self.core_attempts + self.openaire_attempts;

        if total_attempts > 0 {
            println!("\nTotal: {}/{} PDFs ({:.0}% success rate)",
                     total_success, total_attempts,
                     (total_success as f64 / total_attempts as f64) * 100.0);
        }
    }

    fn print_source(&self, name: &str, success: u32, attempts: u32) {
        if attempts > 0 {
            let rate = (success as f64 / attempts as f64) * 100.0;
            println!("  {:12} {:3}/{:3}  ({:.0}%)",
                     format!("{}:", name), success, attempts, rate);
        }
    }
}

/// Extracted sections from a PDF
#[derive(Debug, Clone)]
pub struct ExtractedSections {
    pub abstract_text: Option<String>,
    pub conclusion: Option<String>,
    pub full_text: String,
}
