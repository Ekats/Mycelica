//! Remote client for team server (Phase 3 stub).
//!
//! All methods return Err until the remote server is implemented in Phase 3.
//! Both CLI (--remote) and team client import from here.

use crate::db::{Node, Edge};

pub struct RemoteClient {
    pub base_url: String,
}

impl RemoteClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
        }
    }

    pub fn create_node(&self, _node: &Node) -> Result<String, String> {
        Err(self.not_impl())
    }

    pub fn create_edge(&self, _edge: &Edge) -> Result<String, String> {
        Err(self.not_impl())
    }

    pub fn search(&self, _query: &str, _limit: u32) -> Result<Vec<Node>, String> {
        Err(self.not_impl())
    }

    pub fn get_recent(&self, _limit: u32) -> Result<Vec<Node>, String> {
        Err(self.not_impl())
    }

    pub fn get_orphans(&self, _limit: u32) -> Result<Vec<Node>, String> {
        Err(self.not_impl())
    }

    pub fn health(&self) -> Result<bool, String> {
        Err(self.not_impl())
    }

    fn not_impl(&self) -> String {
        format!(
            "Remote mode not yet implemented (Phase 3). Server URL: {}",
            self.base_url
        )
    }
}
