//! Remote client for team server HTTP API.
//!
//! All methods are async — the CLI uses #[tokio::main] so .await works.
//! Uses reqwest::Client (async, NOT blocking — blocking panics in tokio runtime).

use crate::db::{Node, Edge};
use serde::{Deserialize, Serialize};

pub struct RemoteClient {
    base_url: String,
    client: reqwest::Client,
}

// ============================================================================
// Request / Response types (match server's types)
// ============================================================================

#[derive(Serialize, Deserialize)]
pub struct CreateNodeRequest {
    pub title: String,
    pub content: Option<String>,
    pub url: Option<String>,
    pub content_type: Option<String>,
    pub tags: Option<String>,
    pub author: Option<String>,
    pub connects_to: Option<Vec<String>>,
    pub is_item: Option<bool>,
}

#[derive(Deserialize, Serialize)]
pub struct CreateNodeResponse {
    pub node: Node,
    pub edges_created: Vec<EdgeSummary>,
    pub ambiguous: Vec<AmbiguousResult>,
}

#[derive(Deserialize, Serialize)]
pub struct EdgeSummary {
    pub edge_id: String,
    pub target_id: String,
    pub target_title: String,
}

#[derive(Deserialize, Serialize)]
pub struct AmbiguousResult {
    pub term: String,
    pub candidates: Vec<CandidateNode>,
}

#[derive(Deserialize, Serialize)]
pub struct CandidateNode {
    pub id: String,
    pub title: String,
}

#[derive(Serialize, Deserialize)]
pub struct CreateEdgeRequest {
    pub source: String,   // UUID, ID prefix, or title text
    pub target: String,   // UUID, ID prefix, or title text
    pub edge_type: Option<String>,
    pub reason: Option<String>,
    pub author: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct CreateEdgeResponse {
    pub edge: Edge,
    pub source_resolved: CandidateNode,
    pub target_resolved: CandidateNode,
}

#[derive(Serialize, Deserialize)]
pub struct PatchNodeRequest {
    pub title: Option<String>,
    pub content: Option<String>,
    pub tags: Option<String>,
    pub content_type: Option<String>,
    pub parent_id: Option<String>,
    pub author: Option<String>,
}

#[derive(Serialize)]
pub struct PatchEdgeRequest {
    pub reason: Option<String>,
    pub edge_type: Option<String>,
    pub author: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct NodeWithEdges {
    pub node: Node,
    pub edges: Vec<Edge>,
}

#[derive(Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub nodes: usize,
    pub edges: usize,
    pub uptime_secs: u64,
}

// ============================================================================
// Client implementation
// ============================================================================

impl RemoteClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    async fn check_response(&self, resp: reqwest::Response) -> Result<reqwest::Response, String> {
        if resp.status().is_success() {
            Ok(resp)
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(format!("Server error {}: {}", status, body))
        }
    }

    // --- Health ---

    pub async fn health(&self) -> Result<HealthResponse, String> {
        let resp = self.client.get(self.url("/health"))
            .send().await.map_err(|e| format!("HTTP error: {}", e))?;
        let resp = self.check_response(resp).await?;
        resp.json().await.map_err(|e| format!("Parse error: {}", e))
    }

    // --- Nodes ---

    pub async fn create_node(&self, req: &CreateNodeRequest) -> Result<CreateNodeResponse, String> {
        let resp = self.client.post(self.url("/nodes"))
            .json(req).send().await.map_err(|e| format!("HTTP error: {}", e))?;
        let resp = self.check_response(resp).await?;
        resp.json().await.map_err(|e| format!("Parse error: {}", e))
    }

    pub async fn get_node(&self, id: &str) -> Result<NodeWithEdges, String> {
        let resp = self.client.get(self.url(&format!("/nodes/{}", id)))
            .send().await.map_err(|e| format!("HTTP error: {}", e))?;
        let resp = self.check_response(resp).await?;
        resp.json().await.map_err(|e| format!("Parse error: {}", e))
    }

    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<Node>, String> {
        let resp = self.client.get(self.url("/nodes"))
            .query(&[("search", query), ("limit", &limit.to_string())])
            .send().await.map_err(|e| format!("HTTP error: {}", e))?;
        let resp = self.check_response(resp).await?;
        resp.json().await.map_err(|e| format!("Parse error: {}", e))
    }

    pub async fn get_nodes_since(&self, since_ms: i64) -> Result<Vec<Node>, String> {
        let resp = self.client.get(self.url("/nodes"))
            .query(&[("since", &since_ms.to_string())])
            .send().await.map_err(|e| format!("HTTP error: {}", e))?;
        let resp = self.check_response(resp).await?;
        resp.json().await.map_err(|e| format!("Parse error: {}", e))
    }

    pub async fn patch_node(&self, id: &str, req: &PatchNodeRequest) -> Result<Node, String> {
        let resp = self.client.patch(self.url(&format!("/nodes/{}", id)))
            .json(req).send().await.map_err(|e| format!("HTTP error: {}", e))?;
        let resp = self.check_response(resp).await?;
        resp.json().await.map_err(|e| format!("Parse error: {}", e))
    }

    pub async fn delete_node(&self, id: &str) -> Result<(), String> {
        let resp = self.client.delete(self.url(&format!("/nodes/{}", id)))
            .send().await.map_err(|e| format!("HTTP error: {}", e))?;
        self.check_response(resp).await?;
        Ok(())
    }

    // --- Edges ---

    pub async fn create_edge(&self, req: &CreateEdgeRequest) -> Result<CreateEdgeResponse, String> {
        let resp = self.client.post(self.url("/edges"))
            .json(req).send().await.map_err(|e| format!("HTTP error: {}", e))?;
        let resp = self.check_response(resp).await?;
        resp.json().await.map_err(|e| format!("Parse error: {}", e))
    }

    pub async fn get_edges_since(&self, since_ms: i64) -> Result<Vec<Edge>, String> {
        let resp = self.client.get(self.url("/edges"))
            .query(&[("since", &since_ms.to_string())])
            .send().await.map_err(|e| format!("HTTP error: {}", e))?;
        let resp = self.check_response(resp).await?;
        resp.json().await.map_err(|e| format!("Parse error: {}", e))
    }

    pub async fn patch_edge(&self, id: &str, req: &PatchEdgeRequest) -> Result<Edge, String> {
        let resp = self.client.patch(self.url(&format!("/edges/{}", id)))
            .json(req).send().await.map_err(|e| format!("HTTP error: {}", e))?;
        let resp = self.check_response(resp).await?;
        resp.json().await.map_err(|e| format!("Parse error: {}", e))
    }

    pub async fn delete_edge(&self, id: &str) -> Result<(), String> {
        let resp = self.client.delete(self.url(&format!("/edges/{}", id)))
            .send().await.map_err(|e| format!("HTTP error: {}", e))?;
        self.check_response(resp).await?;
        Ok(())
    }

    // --- Other endpoints ---

    pub async fn get_recent(&self, limit: u32) -> Result<Vec<Node>, String> {
        let resp = self.client.get(self.url("/recent"))
            .query(&[("n", &limit.to_string())])
            .send().await.map_err(|e| format!("HTTP error: {}", e))?;
        let resp = self.check_response(resp).await?;
        resp.json().await.map_err(|e| format!("Parse error: {}", e))
    }

    pub async fn get_orphans(&self, limit: u32) -> Result<Vec<Node>, String> {
        let resp = self.client.get(self.url("/orphans"))
            .query(&[("limit", &limit.to_string())])
            .send().await.map_err(|e| format!("HTTP error: {}", e))?;
        let resp = self.check_response(resp).await?;
        resp.json().await.map_err(|e| format!("Parse error: {}", e))
    }

    pub async fn snapshot(&self, output_path: &str) -> Result<(), String> {
        let resp = self.client.get(self.url("/snapshot"))
            .send().await.map_err(|e| format!("HTTP error: {}", e))?;
        let resp = self.check_response(resp).await?;
        let bytes = resp.bytes().await.map_err(|e| format!("Download error: {}", e))?;
        std::fs::write(output_path, bytes).map_err(|e| format!("Write error: {}", e))?;
        Ok(())
    }
}
