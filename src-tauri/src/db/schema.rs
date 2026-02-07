use rusqlite::{Connection, Result, params, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use super::models::{Node, Edge, NodeType, EdgeType, Position, Tag, Paper};

pub struct Database {
    conn: Mutex<Connection>,
    path: String,
}

impl Database {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        let conn = Connection::open(&path)?;
        let db = Database { conn: Mutex::new(conn), path: path_str };
        db.init()?;
        Ok(db)
    }

    pub fn get_path(&self) -> String {
        self.path.clone()
    }

    #[allow(dead_code)]
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Database { conn: Mutex::new(conn), path: ":memory:".to_string() };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS nodes (
                id TEXT PRIMARY KEY,
                type TEXT NOT NULL,
                title TEXT NOT NULL,
                url TEXT,
                content TEXT,
                position_x REAL NOT NULL DEFAULT 0,
                position_y REAL NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                cluster_id INTEGER,
                cluster_label TEXT,
                ai_title TEXT,
                summary TEXT,
                tags TEXT,
                emoji TEXT,
                is_processed INTEGER NOT NULL DEFAULT 0,
                -- Dynamic hierarchy fields
                depth INTEGER NOT NULL DEFAULT 0,
                is_item INTEGER NOT NULL DEFAULT 0,
                is_universe INTEGER NOT NULL DEFAULT 0,
                parent_id TEXT,
                child_count INTEGER NOT NULL DEFAULT 0,
                -- AI clustering flag: 1 = needs clustering, 0 = already clustered
                needs_clustering INTEGER NOT NULL DEFAULT 1,
                -- Conversation context fields (for message Leafs)
                conversation_id TEXT,    -- Links message to parent conversation
                sequence_index INTEGER,  -- Order within conversation (0-based)
                -- Quick access fields (for Sidebar)
                is_pinned INTEGER NOT NULL DEFAULT 0,
                last_accessed_at INTEGER,
                -- Hierarchy date propagation
                latest_child_date INTEGER,  -- MAX(children's created_at), bubbled up
                -- Import source tracking
                source TEXT  -- e.g. claude, googlekeep, markdown
            );

            -- Learned emoji mappings from AI
            CREATE TABLE IF NOT EXISTS learned_emojis (
                keyword TEXT PRIMARY KEY,
                emoji TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS edges (
                id TEXT PRIMARY KEY,
                source_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
                target_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
                type TEXT NOT NULL,
                label TEXT,
                weight REAL,  -- Association strength (0.0 to 1.0) for multi-path edges
                edge_source TEXT,  -- 'ai', 'user', or NULL for legacy - tracks origin for re-clustering
                evidence_id TEXT,  -- References nodes(id), explains edge reasoning
                confidence REAL,   -- Certainty about this edge (0.0-1.0)
                created_at INTEGER NOT NULL
            );

            -- Persistent tags for guiding clustering across rebuilds
            CREATE TABLE IF NOT EXISTS tags (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                parent_tag_id TEXT REFERENCES tags(id),
                depth INTEGER NOT NULL DEFAULT 0,
                centroid BLOB,  -- Embedding for matching items to this tag
                item_count INTEGER NOT NULL DEFAULT 0,
                pinned INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            -- Item-to-tag assignments (persists across rebuilds)
            CREATE TABLE IF NOT EXISTS item_tags (
                item_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
                tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
                confidence REAL NOT NULL DEFAULT 1.0,
                source TEXT NOT NULL DEFAULT 'ai',
                PRIMARY KEY (item_id, tag_id)
            );

            -- Database metadata for tracking pipeline state
            -- States: fresh, imported, processed, clustered, hierarchized, complete
            CREATE TABLE IF NOT EXISTS db_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            -- Scientific papers from OpenAIRE
            CREATE TABLE IF NOT EXISTS papers (
                id INTEGER PRIMARY KEY,
                node_id TEXT NOT NULL,
                openaire_id TEXT UNIQUE,
                doi TEXT,
                authors TEXT,              -- JSON array of {fullName, orcid}
                publication_date TEXT,
                journal TEXT,
                publisher TEXT,
                abstract TEXT,
                abstract_formatted TEXT,   -- Markdown with **Section** headers
                abstract_sections TEXT,    -- JSON array of detected sections
                pdf_blob BLOB,             -- Stored PDF binary (up to 20MB)
                pdf_url TEXT,              -- Fallback external URL
                pdf_available INTEGER NOT NULL DEFAULT 0,
                subjects TEXT,             -- JSON array of {scheme, value}
                access_right TEXT,         -- OPEN, CLOSED, etc.
                created_at INTEGER NOT NULL,
                FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_papers_node_id ON papers(node_id);
            CREATE INDEX IF NOT EXISTS idx_papers_doi ON papers(doi);
            CREATE INDEX IF NOT EXISTS idx_papers_openaire_id ON papers(openaire_id);

            CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(source_id);
            CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(target_id);
            CREATE INDEX IF NOT EXISTS idx_edges_type ON edges(type);
            CREATE INDEX IF NOT EXISTS idx_nodes_type ON nodes(type);
            CREATE INDEX IF NOT EXISTS idx_tags_parent ON tags(parent_tag_id);
            CREATE INDEX IF NOT EXISTS idx_tags_depth ON tags(depth);
            CREATE INDEX IF NOT EXISTS idx_item_tags_item ON item_tags(item_id);
            CREATE INDEX IF NOT EXISTS idx_item_tags_tag ON item_tags(tag_id);

            -- Hierarchy indexes for fast traversal with 4k+ nodes
            CREATE INDEX IF NOT EXISTS idx_nodes_parent_id ON nodes(parent_id);
            CREATE INDEX IF NOT EXISTS idx_nodes_depth ON nodes(depth);
            CREATE INDEX IF NOT EXISTS idx_nodes_is_item ON nodes(is_item);
            CREATE INDEX IF NOT EXISTS idx_nodes_cluster_id ON nodes(cluster_id);

            -- FOS edge cache for fast view loading
            CREATE TABLE IF NOT EXISTS fos_edges (
                fos_id TEXT NOT NULL,
                edge_id TEXT NOT NULL,
                PRIMARY KEY (fos_id, edge_id),
                FOREIGN KEY (fos_id) REFERENCES nodes(id),
                FOREIGN KEY (edge_id) REFERENCES edges(id)
            );
            CREATE INDEX IF NOT EXISTS idx_fos_edges_fos ON fos_edges(fos_id);

            -- Full-text search
            CREATE VIRTUAL TABLE IF NOT EXISTS nodes_fts USING fts5(
                title,
                content,
                content='nodes',
                content_rowid='rowid'
            );

            -- Triggers to keep FTS in sync
            CREATE TRIGGER IF NOT EXISTS nodes_ai AFTER INSERT ON nodes BEGIN
                INSERT INTO nodes_fts(rowid, title, content) VALUES (NEW.rowid, NEW.title, NEW.content);
            END;

            CREATE TRIGGER IF NOT EXISTS nodes_ad AFTER DELETE ON nodes BEGIN
                INSERT INTO nodes_fts(nodes_fts, rowid, title, content) VALUES('delete', OLD.rowid, OLD.title, OLD.content);
            END;

            CREATE TRIGGER IF NOT EXISTS nodes_au AFTER UPDATE ON nodes BEGIN
                INSERT INTO nodes_fts(nodes_fts, rowid, title, content) VALUES('delete', OLD.rowid, OLD.title, OLD.content);
                INSERT INTO nodes_fts(rowid, title, content) VALUES (NEW.rowid, NEW.title, NEW.content);
            END;

            PRAGMA foreign_keys = ON;
            "
        )?;

        // Migration: Add cluster columns if they don't exist
        let has_cluster_id: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'cluster_id'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_cluster_id {
            conn.execute("ALTER TABLE nodes ADD COLUMN cluster_id INTEGER", [])?;
            conn.execute("ALTER TABLE nodes ADD COLUMN cluster_label TEXT", [])?;
        }

        // Migration: Add AI processing columns if they don't exist
        let has_ai_title: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'ai_title'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_ai_title {
            conn.execute("ALTER TABLE nodes ADD COLUMN ai_title TEXT", [])?;
            conn.execute("ALTER TABLE nodes ADD COLUMN summary TEXT", [])?;
            conn.execute("ALTER TABLE nodes ADD COLUMN tags TEXT", [])?;
            conn.execute("ALTER TABLE nodes ADD COLUMN is_processed INTEGER NOT NULL DEFAULT 0", [])?;
        }

        // Migration: Add emoji column if it doesn't exist
        let has_emoji: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'emoji'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_emoji {
            conn.execute("ALTER TABLE nodes ADD COLUMN emoji TEXT", [])?;
        }

        // Migration: Add dynamic hierarchy columns (depth, is_item, is_universe)
        let has_depth: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'depth'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_depth {
            // Add new dynamic hierarchy columns
            conn.execute("ALTER TABLE nodes ADD COLUMN depth INTEGER NOT NULL DEFAULT 0", [])?;
            conn.execute("ALTER TABLE nodes ADD COLUMN is_item INTEGER NOT NULL DEFAULT 0", [])?;
            conn.execute("ALTER TABLE nodes ADD COLUMN is_universe INTEGER NOT NULL DEFAULT 0", [])?;

            // Check if we have the old 'level' column to migrate from
            let has_level: bool = conn.query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'level'",
                [],
                |row| row.get(0),
            ).unwrap_or(false);

            if has_level {
                // Clean slate migration:
                // 1. Mark items (old level 3/4 nodes with actual content)
                conn.execute(
                    "UPDATE nodes SET is_item = 1, depth = 1 WHERE level IN (3, 4) AND content IS NOT NULL",
                    []
                )?;

                // 2. Delete generated hierarchy scaffolding (old L0/L1/L2 nodes)
                conn.execute(
                    "DELETE FROM nodes WHERE level IN (0, 1, 2)",
                    []
                )?;

                eprintln!("Migration: Converted old level-based hierarchy to dynamic depth system");

                // Try to drop the old level column (SQLite 3.35.0+ supports DROP COLUMN)
                // If this fails, it's not critical - the column is just unused dead weight
                if let Err(_) = conn.execute("ALTER TABLE nodes DROP COLUMN level", []) {
                    eprintln!("Note: Could not drop old 'level' column (SQLite version may not support DROP COLUMN)");
                }
            }
        }

        // Legacy migration: Add parent_id and child_count if missing (from older schemas)
        let has_parent_id: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'parent_id'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_parent_id {
            conn.execute("ALTER TABLE nodes ADD COLUMN parent_id TEXT", [])?;
            conn.execute("ALTER TABLE nodes ADD COLUMN child_count INTEGER NOT NULL DEFAULT 0", [])?;
        }

        // Migration: Add needs_clustering column if it doesn't exist
        let has_needs_clustering: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'needs_clustering'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_needs_clustering {
            conn.execute("ALTER TABLE nodes ADD COLUMN needs_clustering INTEGER NOT NULL DEFAULT 1", [])?;
            // Mark all items as needing clustering
            conn.execute("UPDATE nodes SET needs_clustering = 1 WHERE is_item = 1", [])?;
            eprintln!("Migration: Added needs_clustering column");
        }

        // Migration: Add weight column to edges if it doesn't exist
        let has_edge_weight: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('edges') WHERE name = 'weight'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_edge_weight {
            conn.execute("ALTER TABLE edges ADD COLUMN weight REAL", [])?;
            eprintln!("Migration: Added weight column to edges for multi-path associations");
        }

        // Migration: Add edge_source column to edges if it doesn't exist
        let has_edge_source: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('edges') WHERE name = 'edge_source'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_edge_source {
            conn.execute("ALTER TABLE edges ADD COLUMN edge_source TEXT", [])?;
            eprintln!("Migration: Added edge_source column to edges for user-edit tracking");
        }

        // Migration: Add evidence_id and confidence columns to edges if they don't exist
        let has_evidence_id: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('edges') WHERE name = 'evidence_id'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_evidence_id {
            conn.execute("ALTER TABLE edges ADD COLUMN evidence_id TEXT", [])?;
            conn.execute("ALTER TABLE edges ADD COLUMN confidence REAL", [])?;
            eprintln!("Migration: Added evidence_id and confidence columns to edges for epistemic tracking");
        }

        // Migration: Add conversation context columns to nodes if they don't exist
        let has_conversation_id: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'conversation_id'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_conversation_id {
            conn.execute("ALTER TABLE nodes ADD COLUMN conversation_id TEXT", [])?;
            conn.execute("ALTER TABLE nodes ADD COLUMN sequence_index INTEGER", [])?;
            eprintln!("Migration: Added conversation_id and sequence_index columns to nodes for conversation context");
        }

        // Migration: Add quick access columns (is_pinned, last_accessed_at) if they don't exist
        let has_is_pinned: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'is_pinned'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_is_pinned {
            conn.execute("ALTER TABLE nodes ADD COLUMN is_pinned INTEGER NOT NULL DEFAULT 0", [])?;
            conn.execute("ALTER TABLE nodes ADD COLUMN last_accessed_at INTEGER", [])?;
            eprintln!("Migration: Added is_pinned and last_accessed_at columns to nodes for quick access");
        }

        // Migration: Add embedding column for semantic similarity
        let has_embedding: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'embedding'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_embedding {
            conn.execute("ALTER TABLE nodes ADD COLUMN embedding BLOB", [])?;
            eprintln!("Migration: Added embedding column to nodes for semantic similarity");
        }

        // Migration: Add latest_child_date column for hierarchy date propagation
        let has_latest_child_date: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'latest_child_date'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_latest_child_date {
            conn.execute("ALTER TABLE nodes ADD COLUMN latest_child_date INTEGER", [])?;
            eprintln!("Migration: Added latest_child_date column to nodes for hierarchy date propagation");
        }

        // Migration: Add privacy filtering columns if they don't exist
        let has_is_private: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'is_private'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_is_private {
            conn.execute("ALTER TABLE nodes ADD COLUMN is_private INTEGER", [])?;
            conn.execute("ALTER TABLE nodes ADD COLUMN privacy_reason TEXT", [])?;
            eprintln!("Migration: Added is_private and privacy_reason columns for privacy filtering");
        }

        // Migration: Add source column for import tracking if it doesn't exist
        let has_source: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'source'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_source {
            conn.execute("ALTER TABLE nodes ADD COLUMN source TEXT", [])?;
            eprintln!("Migration: Added source column for import tracking");
        }

        // Migration: Add content classification columns if they don't exist
        let has_content_type: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'content_type'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_content_type {
            conn.execute("ALTER TABLE nodes ADD COLUMN content_type TEXT", [])?;
            conn.execute("ALTER TABLE nodes ADD COLUMN associated_idea_id TEXT", [])?;
            eprintln!("Migration: Added content_type and associated_idea_id columns for mini-clustering");
        }

        // Migration: Add privacy score column (continuous 0.0-1.0 scale)
        let has_privacy_score: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'privacy'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_privacy_score {
            conn.execute("ALTER TABLE nodes ADD COLUMN privacy REAL", [])?;
            eprintln!("Migration: Added privacy score column (0.0=private, 1.0=public)");
        }

        // Migration: Add pdf_available column to nodes (denormalized from papers table)
        let has_pdf_available: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('nodes') WHERE name = 'pdf_available'",
            [],
            |row| row.get(0),
        )?;
        if !has_pdf_available {
            conn.execute("ALTER TABLE nodes ADD COLUMN pdf_available INTEGER DEFAULT 0", [])?;
            eprintln!("Migration: Added pdf_available column to nodes");
        }

        // Sync pdf_available from papers table to nodes (only set to 1 where papers have PDFs)
        let synced = conn.execute(
            "UPDATE nodes SET pdf_available = 1
             WHERE content_type = 'paper'
               AND id IN (SELECT node_id FROM papers WHERE pdf_available = 1)",
            [],
        )?;
        if synced > 0 {
            eprintln!("Migration: Synced pdf_available=1 for {} paper nodes with PDFs", synced);
        }

        // Migration: Add abstract_formatted and abstract_sections columns to papers
        let has_abstract_formatted: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('papers') WHERE name = 'abstract_formatted'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_abstract_formatted {
            conn.execute("ALTER TABLE papers ADD COLUMN abstract_formatted TEXT", [])?;
            conn.execute("ALTER TABLE papers ADD COLUMN abstract_sections TEXT", [])?;
            eprintln!("Migration: Added abstract_formatted and abstract_sections columns to papers");
        }

        // Migration: Add doc_format column to papers for DOCX/DOC support
        let has_doc_format: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('papers') WHERE name = 'doc_format'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_doc_format {
            conn.execute("ALTER TABLE papers ADD COLUMN doc_format TEXT", [])?;
            // Set existing PDFs to have doc_format = 'pdf'
            conn.execute("UPDATE papers SET doc_format = 'pdf' WHERE pdf_available = 1", [])?;
            eprintln!("Migration: Added doc_format column to papers");
        }

        // Migration: Add content_hash column to papers for deduplication
        let has_content_hash: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('papers') WHERE name = 'content_hash'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_content_hash {
            conn.execute("ALTER TABLE papers ADD COLUMN content_hash TEXT", [])?;
            eprintln!("Migration: Added content_hash column to papers for deduplication");
        }

        // Migration: Add parent columns to edges for fast per-view lookups
        let has_source_parent: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('edges') WHERE name = 'source_parent_id'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_source_parent {
            conn.execute("ALTER TABLE edges ADD COLUMN source_parent_id TEXT", [])?;
            conn.execute("ALTER TABLE edges ADD COLUMN target_parent_id TEXT", [])?;
            eprintln!("Migration: Added source_parent_id and target_parent_id columns to edges for fast view lookups");
        }

        // Migration: Add PDF extraction columns to papers
        let has_extracted_abstract: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('papers') WHERE name = 'extracted_abstract'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_extracted_abstract {
            conn.execute("ALTER TABLE papers ADD COLUMN extracted_abstract TEXT", [])?;
            conn.execute("ALTER TABLE papers ADD COLUMN extracted_conclusion TEXT", [])?;
            conn.execute("ALTER TABLE papers ADD COLUMN extraction_status TEXT DEFAULT 'pending'", [])?;
            conn.execute("ALTER TABLE papers ADD COLUMN pdf_source TEXT", [])?;
            eprintln!("Migration: Added PDF extraction columns (extracted_abstract, extracted_conclusion, extraction_status, pdf_source) to papers");
        }

        // Create indexes for dynamic hierarchy columns (after migrations ensure columns exist)
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_depth ON nodes(depth)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_is_item ON nodes(is_item)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_parent ON nodes(parent_id)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_needs_clustering ON nodes(needs_clustering)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_conversation ON nodes(conversation_id)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_pinned ON nodes(is_pinned)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_last_accessed ON nodes(last_accessed_at)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_is_processed ON nodes(is_processed)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_content_type ON nodes(content_type)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_associated_idea ON nodes(associated_idea_id)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_privacy ON nodes(privacy)", [])?;
        // Index for fast per-view edge lookups (edges where both endpoints share the same parent)
        conn.execute("CREATE INDEX IF NOT EXISTS idx_edges_view ON edges(source_parent_id, target_parent_id)", [])?;
        // Index for paper deduplication by content hash
        conn.execute("CREATE INDEX IF NOT EXISTS idx_papers_content_hash ON papers(content_hash)", [])?;

        // Rebuild FTS index to fix any corruption from interrupted writes (e.g., HMR during dev)
        // This is safe to run on every startup - it rebuilds from the content table
        if let Err(e) = conn.execute("INSERT INTO nodes_fts(nodes_fts) VALUES('rebuild')", []) {
            eprintln!("FTS rebuild failed (might be empty): {}", e);
        }

        Ok(())
    }

    // Node operations
    pub fn insert_node(&self, node: &Node) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO nodes (id, type, title, url, content, position_x, position_y, created_at, updated_at, cluster_id, cluster_label, ai_title, summary, tags, emoji, is_processed, depth, is_item, is_universe, parent_id, child_count, conversation_id, sequence_index, is_pinned, last_accessed_at, source, pdf_available, content_type, associated_idea_id, privacy)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30)",
            params![
                node.id,
                node.node_type.as_str(),
                node.title,
                node.url,
                node.content,
                node.position.x,
                node.position.y,
                node.created_at,
                node.updated_at,
                node.cluster_id,
                node.cluster_label,
                node.ai_title,
                node.summary,
                node.tags,
                node.emoji,
                node.is_processed,
                node.depth,
                node.is_item,
                node.is_universe,
                node.parent_id,
                node.child_count,
                node.conversation_id,
                node.sequence_index,
                node.is_pinned,
                node.last_accessed_at,
                node.source,
                node.pdf_available,
                node.content_type,
                node.associated_idea_id,
                node.privacy,
            ],
        )?;
        Ok(())
    }

    pub fn get_node(&self, id: &str) -> Result<Option<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE id = ?1",
            Self::NODE_COLUMNS
        ))?;

        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(Self::row_to_node(row)?))
        } else {
            Ok(None)
        }
    }

    /// Check if a node is protected (Recent Notes container or its descendants)
    /// Returns true only if protection is enabled AND node is in the protected subtree
    pub fn is_node_protected(&self, node_id: &str) -> bool {
        use crate::settings;

        // Check if protection is enabled
        if !settings::is_recent_notes_protected() {
            return false;
        }

        // Check if this IS the Recent Notes container
        if node_id == settings::RECENT_NOTES_CONTAINER_ID {
            return true;
        }

        // Check if this node is a descendant of Recent Notes
        self.is_descendant_of(node_id, settings::RECENT_NOTES_CONTAINER_ID)
    }

    /// Check if a node is a descendant of another node (by traversing parent_id chain)
    pub fn is_descendant_of(&self, node_id: &str, ancestor_id: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        let mut current_id = node_id.to_string();
        let mut depth = 0;
        const MAX_DEPTH: i32 = 50; // Safety limit

        while depth < MAX_DEPTH {
            let parent_id: Option<String> = conn
                .query_row(
                    "SELECT parent_id FROM nodes WHERE id = ?1",
                    params![&current_id],
                    |row| row.get(0),
                )
                .ok()
                .flatten();

            match parent_id {
                Some(pid) if pid == ancestor_id => return true,
                Some(pid) => {
                    current_id = pid;
                    depth += 1;
                }
                None => return false,
            }
        }
        false
    }

    /// Get all protected node IDs (Recent Notes, Holerabbit, Import containers, and their descendants)
    /// Returns empty set if protection is disabled
    /// Uses a single recursive CTE query for O(1) database calls instead of O(n) traversals
    pub fn get_protected_node_ids(&self) -> std::collections::HashSet<String> {
        use crate::settings;

        let mut protected = std::collections::HashSet::new();

        if !settings::is_recent_notes_protected() {
            return protected;
        }

        // Build list of all protected container IDs
        let mut container_ids: Vec<&str> = vec![
            settings::RECENT_NOTES_CONTAINER_ID,
            settings::HOLERABBIT_CONTAINER_ID,
        ];
        container_ids.extend(settings::IMPORT_CONTAINER_IDS.iter().copied());

        // Build WHERE clause with all container IDs
        let placeholders: Vec<String> = (1..=container_ids.len()).map(|i| format!("?{}", i)).collect();
        let where_clause = placeholders.join(" OR id = ");

        let query = format!(
            "WITH RECURSIVE descendants AS (
                SELECT id FROM nodes WHERE id = {}
                UNION ALL
                SELECT n.id FROM nodes n
                JOIN descendants d ON n.parent_id = d.id
            )
            SELECT id FROM descendants",
            where_clause
        );

        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare(&query) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to prepare protected nodes query: {}", e);
                // Fallback: return all container IDs
                for id in &container_ids {
                    protected.insert(id.to_string());
                }
                return protected;
            }
        };

        // Convert to rusqlite params
        let params: Vec<&dyn rusqlite::ToSql> = container_ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();

        let ids: Vec<String> = stmt
            .query_map(params.as_slice(), |row| row.get(0))
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();

        protected.extend(ids);
        protected
    }

    /// Helper to convert a row to Node (reduces duplication)
    fn row_to_node(row: &rusqlite::Row) -> Result<Node> {
        Ok(Node {
            id: row.get(0)?,
            node_type: NodeType::from_str(&row.get::<_, String>(1)?).unwrap_or(NodeType::Thought),
            title: row.get(2)?,
            url: row.get(3)?,
            content: row.get(4)?,
            position: Position { x: row.get(5)?, y: row.get(6)? },
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
            cluster_id: row.get(9)?,
            cluster_label: row.get(10)?,
            ai_title: row.get(11)?,
            summary: row.get(12)?,
            tags: row.get(13)?,
            emoji: row.get(14)?,
            is_processed: row.get::<_, i32>(15).unwrap_or(0) != 0,
            depth: row.get::<_, i32>(16).unwrap_or(0),
            is_item: row.get::<_, i32>(17).unwrap_or(0) != 0,
            is_universe: row.get::<_, i32>(18).unwrap_or(0) != 0,
            parent_id: row.get(19)?,
            child_count: row.get::<_, i32>(20).unwrap_or(0),
            conversation_id: row.get(21)?,
            sequence_index: row.get(22)?,
            is_pinned: row.get::<_, i32>(23).unwrap_or(0) != 0,
            last_accessed_at: row.get(24)?,
            latest_child_date: row.get(25)?,
            is_private: row.get::<_, Option<i32>>(26)?.map(|v| v != 0),
            privacy_reason: row.get(27)?,
            source: row.get(28)?,
            pdf_available: row.get::<_, Option<i32>>(29)?.map(|v| v != 0),
            content_type: row.get(30)?,
            associated_idea_id: row.get(31)?,
            privacy: row.get(32)?,
        })
    }

    /// Standard SELECT columns for nodes (excludes embedding - use dedicated functions)
    const NODE_COLUMNS: &'static str = "id, type, title, url, content, position_x, position_y, created_at, updated_at, cluster_id, cluster_label, ai_title, summary, tags, emoji, is_processed, depth, is_item, is_universe, parent_id, child_count, conversation_id, sequence_index, is_pinned, last_accessed_at, latest_child_date, is_private, privacy_reason, source, pdf_available, content_type, associated_idea_id, privacy";

    /// Get all nodes for graph view - no content_type filtering
    /// All nodes are always shown; filtering is handled by frontend if needed
    /// The include_hidden parameter is kept for API compatibility but ignored
    pub fn get_all_nodes(&self, _include_hidden: bool) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes ORDER BY created_at DESC",
            Self::NODE_COLUMNS
        ))?;

        let nodes = stmt.query_map([], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    pub fn update_node(&self, node: &Node) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET type = ?2, title = ?3, url = ?4, content = ?5,
             position_x = ?6, position_y = ?7, updated_at = ?8, cluster_id = ?9, cluster_label = ?10,
             ai_title = ?11, summary = ?12, tags = ?13, emoji = ?14, is_processed = ?15,
             depth = ?16, is_item = ?17, is_universe = ?18, parent_id = ?19, child_count = ?20,
             conversation_id = ?21, sequence_index = ?22, is_pinned = ?23, last_accessed_at = ?24,
             content_type = ?25, associated_idea_id = ?26, privacy = ?27 WHERE id = ?1",
            params![
                node.id,
                node.node_type.as_str(),
                node.title,
                node.url,
                node.content,
                node.position.x,
                node.position.y,
                node.updated_at,
                node.cluster_id,
                node.cluster_label,
                node.ai_title,
                node.summary,
                node.tags,
                node.emoji,
                node.is_processed,
                node.depth,
                node.is_item,
                node.is_universe,
                node.parent_id,
                node.child_count,
                node.conversation_id,
                node.sequence_index,
                node.is_pinned,
                node.last_accessed_at,
                node.content_type,
                node.associated_idea_id,
                node.privacy,
            ],
        )?;
        Ok(())
    }

    /// Update just the content of a node (simpler API for editing)
    pub fn update_node_content(&self, node_id: &str, content: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET content = ?2, updated_at = ?3 WHERE id = ?1",
            params![node_id, content, chrono::Utc::now().timestamp_millis()],
        )?;
        Ok(())
    }

    /// Update just the source field of a node
    pub fn update_node_source(&self, node_id: &str, source: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET source = ?2 WHERE id = ?1",
            params![node_id, source],
        )?;
        Ok(())
    }

    pub fn delete_node(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM nodes WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Delete all nodes from a specific file path (stored in tags JSON)
    /// Also deletes edges where source OR target is in the deleted set
    pub fn delete_nodes_by_file_path(&self, file_path: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();

        // First get IDs to delete (needed for edge cleanup)
        let mut stmt = conn.prepare(
            "SELECT id FROM nodes WHERE JSON_EXTRACT(tags, '$.file_path') = ?1"
        )?;
        let ids: Vec<String> = stmt.query_map([file_path], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        // Delete edges where source OR target is in deleted set
        for id in &ids {
            conn.execute("DELETE FROM edges WHERE source_id = ?1 OR target_id = ?1", params![id])?;
        }

        // Delete the nodes
        let deleted = conn.execute(
            "DELETE FROM nodes WHERE JSON_EXTRACT(tags, '$.file_path') = ?1",
            params![file_path],
        )?;

        Ok(deleted)
    }

    /// Get node IDs for a specific file path (for targeted edge refresh)
    pub fn get_node_ids_by_file_path(&self, file_path: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id FROM nodes WHERE JSON_EXTRACT(tags, '$.file_path') = ?1"
        )?;
        let ids: Vec<String> = stmt.query_map([file_path], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    /// Delete edges by source and edge type
    pub fn delete_edges_by_source_and_type(&self, source_id: &str, edge_type: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute(
            "DELETE FROM edges WHERE source_id = ?1 AND type = ?2",
            params![source_id, edge_type],
        )?;
        Ok(deleted)
    }

    /// Delete all edges of a specific type
    pub fn delete_edges_by_type(&self, edge_type: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute(
            "DELETE FROM edges WHERE type = ?1",
            params![edge_type],
        )?;
        Ok(deleted)
    }

    // Privacy operations - sets both legacy is_private AND new privacy score
    pub fn update_node_privacy(&self, node_id: &str, is_private: bool, reason: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        // Map boolean to score: is_private=true → 0.0 (private), is_private=false → 1.0 (public)
        let privacy_score = if is_private { 0.0 } else { 1.0 };
        conn.execute(
            "UPDATE nodes SET is_private = ?2, privacy_reason = ?3, privacy = ?4 WHERE id = ?1",
            params![node_id, is_private as i32, reason, privacy_score],
        )?;
        Ok(())
    }

    // Privacy scoring operations (continuous 0.0-1.0 scale)
    /// Update the privacy score for a node (0.0 = private, 1.0 = public)
    pub fn update_privacy_score(&self, node_id: &str, privacy: f64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET privacy = ?2 WHERE id = ?1",
            params![node_id, privacy],
        )?;
        Ok(())
    }

    /// Get items that need privacy scoring (privacy IS NULL)
    pub fn get_items_needing_privacy_scoring(&self) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE is_item = 1 AND privacy IS NULL ORDER BY created_at DESC",
            Self::NODE_COLUMNS
        ))?;
        let nodes = stmt.query_map([], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Count items needing privacy scoring
    pub fn count_items_needing_privacy_scoring(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND privacy IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Get items with privacy score >= threshold (for export filtering)
    pub fn get_shareable_items(&self, min_privacy: f64) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE is_item = 1 AND privacy >= ?1 ORDER BY created_at DESC",
            Self::NODE_COLUMNS
        ))?;
        let nodes = stmt.query_map([min_privacy], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Count shareable items with privacy >= threshold
    pub fn count_shareable_items(&self, min_privacy: f64) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND privacy >= ?1",
            [min_privacy],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Reset all privacy flags to unscanned state (is_private = NULL)
    pub fn reset_all_privacy_flags(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "UPDATE nodes SET is_private = NULL, privacy_reason = NULL WHERE is_private IS NOT NULL",
            [],
        )?;
        Ok(count)
    }

    pub fn get_items_needing_privacy_scan(&self) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE is_item = 1 AND is_private IS NULL ORDER BY created_at DESC",
            Self::NODE_COLUMNS
        ))?;
        let nodes = stmt.query_map([], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    pub fn get_privacy_stats(&self) -> Result<(usize, usize, usize, usize, usize)> {
        let conn = self.conn.lock().unwrap();
        let total: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1",
            [],
            |r| r.get(0),
        ).unwrap_or(0);
        let scanned: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND is_private IS NOT NULL",
            [],
            |r| r.get(0),
        ).unwrap_or(0);
        let private: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND is_private = 1",
            [],
            |r| r.get(0),
        ).unwrap_or(0);
        let unscanned = total - scanned;
        let safe = scanned - private;
        Ok((total as usize, scanned as usize, unscanned as usize, private as usize, safe as usize))
    }

    /// Get category nodes (non-items with children) that need privacy scanning
    /// These are topics/domains/galaxies - scanning them first allows bulk propagation
    pub fn get_category_nodes_needing_privacy_scan(&self) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE is_item = 0 AND child_count > 0 AND is_private IS NULL ORDER BY depth ASC, child_count DESC",
            Self::NODE_COLUMNS
        ))?;
        let nodes = stmt.query_map([], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Propagate privacy status to all descendants of a node
    /// Uses iterative approach to mark all children, grandchildren, etc. as private
    /// Propagate privacy to descendants (only unscanned nodes - for AI scan)
    pub fn propagate_privacy_to_descendants(&self, node_id: &str, reason: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();

        // Use recursive CTE to find all descendants - only update unscanned
        // Set both is_private=1 and privacy=0.0 (private)
        let count = conn.execute(
            "UPDATE nodes SET is_private = 1, privacy_reason = ?2, privacy = 0.0
             WHERE id IN (
                 WITH RECURSIVE descendants AS (
                     SELECT id FROM nodes WHERE parent_id = ?1
                     UNION ALL
                     SELECT n.id FROM nodes n
                     INNER JOIN descendants d ON n.parent_id = d.id
                 )
                 SELECT id FROM descendants
             ) AND is_private IS NULL",
            params![node_id, reason],
        )?;

        Ok(count)
    }

    /// Force propagate privacy to ALL descendants (for manual marking)
    pub fn force_propagate_privacy_to_descendants(&self, node_id: &str, reason: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();

        // First get all descendant IDs
        let mut stmt = conn.prepare(
            "WITH RECURSIVE descendants AS (
                 SELECT id FROM nodes WHERE parent_id = ?1
                 UNION ALL
                 SELECT n.id FROM nodes n
                 INNER JOIN descendants d ON n.parent_id = d.id
             )
             SELECT id FROM descendants"
        )?;

        let ids: Vec<String> = stmt.query_map(params![node_id], |row| row.get(0))?
            .collect::<Result<Vec<_>>>()?;

        // Update all descendants - set both is_private=1 and privacy=0.0
        if !ids.is_empty() {
            conn.execute(
                &format!(
                    "UPDATE nodes SET is_private = 1, privacy_reason = ?1, privacy = 0.0
                     WHERE id IN ({})",
                    ids.iter().map(|_| "?").collect::<Vec<_>>().join(",")
                ),
                rusqlite::params_from_iter(
                    std::iter::once(reason.to_string()).chain(ids.iter().cloned())
                ),
            )?;
        }

        Ok(ids)
    }

    /// Clear privacy from ALL descendants (for manual un-marking)
    pub fn clear_privacy_from_descendants(&self, node_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();

        // First get all descendant IDs
        let mut stmt = conn.prepare(
            "WITH RECURSIVE descendants AS (
                 SELECT id FROM nodes WHERE parent_id = ?1
                 UNION ALL
                 SELECT n.id FROM nodes n
                 INNER JOIN descendants d ON n.parent_id = d.id
             )
             SELECT id FROM descendants"
        )?;

        let ids: Vec<String> = stmt.query_map(params![node_id], |row| row.get(0))?
            .collect::<Result<Vec<_>>>()?;

        // Clear privacy from all descendants - sets both is_private=0 AND privacy=1.0 (public)
        if !ids.is_empty() {
            conn.execute(
                &format!(
                    "UPDATE nodes SET is_private = 0, privacy_reason = NULL, privacy = 1.0
                     WHERE id IN ({})",
                    ids.iter().map(|_| "?").collect::<Vec<_>>().join(",")
                ),
                rusqlite::params_from_iter(ids.iter().cloned()),
            )?;
        }

        Ok(ids)
    }

    /// Get privacy stats including category counts
    pub fn get_privacy_stats_extended(&self) -> Result<(usize, usize, usize, usize, usize, usize, usize)> {
        let conn = self.conn.lock().unwrap();

        // Item stats using new continuous privacy column (0.0 = private, 1.0 = public)
        let total_items: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1",
            [],
            |r| r.get(0),
        ).unwrap_or(0);
        let scored_items: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND privacy IS NOT NULL",
            [],
            |r| r.get(0),
        ).unwrap_or(0);
        let private_items: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND privacy IS NOT NULL AND privacy < 0.3",
            [],
            |r| r.get(0),
        ).unwrap_or(0);
        let public_items: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND privacy IS NOT NULL AND privacy > 0.7",
            [],
            |r| r.get(0),
        ).unwrap_or(0);

        // Category stats (non-items with children)
        let total_categories: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 0 AND child_count > 0",
            [],
            |r| r.get(0),
        ).unwrap_or(0);
        let scored_categories: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 0 AND child_count > 0 AND privacy IS NOT NULL",
            [],
            |r| r.get(0),
        ).unwrap_or(0);

        let unscored_items = total_items - scored_items;

        Ok((
            total_items as usize,
            scored_items as usize,
            unscored_items as usize,
            private_items as usize,
            public_items as usize,
            total_categories as usize,
            scored_categories as usize,
        ))
    }

    /// Get preview of how many items would be included/excluded at a given privacy threshold
    pub fn get_export_preview(&self, min_privacy: f64) -> Result<(usize, usize, usize)> {
        let conn = self.conn.lock().unwrap();

        // Items that would be INCLUDED (privacy >= threshold OR unscored)
        let included: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND (privacy IS NULL OR privacy >= ?1)",
            [min_privacy],
            |r| r.get(0),
        ).unwrap_or(0);

        // Items that would be EXCLUDED (privacy < threshold)
        let excluded: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND privacy IS NOT NULL AND privacy < ?1",
            [min_privacy],
            |r| r.get(0),
        ).unwrap_or(0);

        // Unscored items (will be included by default)
        let unscored: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND privacy IS NULL",
            [],
            |r| r.get(0),
        ).unwrap_or(0);

        Ok((included as usize, excluded as usize, unscored as usize))
    }

    // Edge operations
    pub fn insert_edge(&self, edge: &Edge) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO edges (id, source_id, target_id, type, label, weight, edge_source, evidence_id, confidence, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                edge.id,
                edge.source,
                edge.target,
                edge.edge_type.as_str(),
                edge.label,
                edge.weight,
                edge.edge_source,
                edge.evidence_id,
                edge.confidence,
                edge.created_at,
            ],
        )?;
        Ok(())
    }

    /// Batch insert edges with transaction wrapping and INSERT OR IGNORE
    pub fn insert_edges_batch(&self, edges: &[Edge]) -> Result<usize> {
        if edges.is_empty() {
            return Ok(0);
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO edges (id, source_id, target_id, type, label, weight, edge_source, evidence_id, confidence, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
            )?;

            for edge in edges {
                stmt.execute(params![
                    &edge.id,
                    &edge.source,
                    &edge.target,
                    edge.edge_type.as_str(),
                    &edge.label,
                    &edge.weight,
                    &edge.edge_source,
                    &edge.evidence_id,
                    &edge.confidence,
                    &edge.created_at,
                ])?;
            }
        }

        tx.commit()?;
        Ok(edges.len())
    }

    pub fn get_all_edges(&self) -> Result<Vec<Edge>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, source_id, target_id, type, label, weight, edge_source, evidence_id, confidence, created_at FROM edges"
        )?;

        let edges = stmt.query_map([], |row| {
            Ok(Edge {
                id: row.get(0)?,
                source: row.get(1)?,
                target: row.get(2)?,
                edge_type: EdgeType::from_str(&row.get::<_, String>(3)?).unwrap_or(EdgeType::Related),
                label: row.get(4)?,
                weight: row.get(5)?,
                edge_source: row.get(6)?,
                evidence_id: row.get(7)?,
                confidence: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?.collect::<Result<Vec<_>>>()?;

        Ok(edges)
    }

    /// Get all edges between items (papers) sorted by weight descending.
    /// Returns (source_id, target_id, weight) tuples for dendrogram building.
    /// Only includes edges where both endpoints are items (is_item = true).
    pub fn get_all_item_edges_sorted(&self) -> Result<Vec<(String, String, f64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT e.source_id, e.target_id, e.weight
             FROM edges e
             JOIN nodes n1 ON e.source_id = n1.id
             JOIN nodes n2 ON e.target_id = n2.id
             WHERE n1.is_item = 1 AND n2.is_item = 1
               AND e.weight IS NOT NULL
             ORDER BY e.weight DESC"
        )?;

        let edges = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?.collect::<Result<Vec<_>>>()?;

        Ok(edges)
    }

    pub fn get_edges_for_node(&self, node_id: &str) -> Result<Vec<Edge>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, source_id, target_id, type, label, weight, edge_source, evidence_id, confidence, created_at
             FROM edges WHERE source_id = ?1 OR target_id = ?1"
        )?;

        let edges = stmt.query_map(params![node_id], |row| {
            Ok(Edge {
                id: row.get(0)?,
                source: row.get(1)?,
                target: row.get(2)?,
                edge_type: EdgeType::from_str(&row.get::<_, String>(3)?).unwrap_or(EdgeType::Related),
                label: row.get(4)?,
                weight: row.get(5)?,
                edge_source: row.get(6)?,
                evidence_id: row.get(7)?,
                confidence: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?.collect::<Result<Vec<_>>>()?;

        Ok(edges)
    }

    /// Get edges for multiple nodes in a single bulk query
    /// Returns (source_id, target_id, weight) tuples for edges where either endpoint is in node_ids
    /// Much more efficient than calling get_edges_for_node repeatedly
    pub fn get_edges_for_nodes_bulk(&self, node_ids: &[&str]) -> Result<Vec<(String, String, f64)>> {
        if node_ids.is_empty() {
            return Ok(vec![]);
        }

        let conn = self.conn.lock().unwrap();

        // Build IN clause
        let placeholders: Vec<&str> = node_ids.iter().map(|_| "?").collect();
        let in_clause = placeholders.join(",");

        let query = format!(
            "SELECT source_id, target_id, weight FROM edges
             WHERE source_id IN ({}) OR target_id IN ({})",
            in_clause, in_clause
        );

        let mut stmt = conn.prepare(&query)?;

        // Build params (need to bind twice for both IN clauses)
        let mut params: Vec<&dyn rusqlite::ToSql> = Vec::new();
        for id in node_ids {
            params.push(id);
        }
        for id in node_ids {
            params.push(id);
        }

        let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<f64>>(2)?.unwrap_or(1.0),
            ))
        })?;

        rows.collect()
    }

    /// Count cross-edges between all pairs of topics in one efficient query
    /// Returns HashMap<(topic_a_id, topic_b_id), count> with canonical ordering (a < b)
    ///
    /// A cross-edge is an edge where one endpoint is a child of topic A and the other
    /// is a child of topic B. This is used for grouping topics by connectivity.
    pub fn count_all_cross_edges(&self, topic_ids: &[&str]) -> Result<HashMap<(String, String), usize>> {
        if topic_ids.len() < 2 {
            return Ok(HashMap::new());
        }

        let conn = self.conn.lock().unwrap();

        // Step 1: Get paper IDs for each topic (direct children that are items)
        let mut topic_papers: HashMap<String, Vec<String>> = HashMap::new();
        for &topic_id in topic_ids {
            let mut stmt = conn.prepare(
                "SELECT id FROM nodes WHERE parent_id = ?1 AND is_item = 1"
            )?;
            let papers: Vec<String> = stmt.query_map([topic_id], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            topic_papers.insert(topic_id.to_string(), papers);
        }

        // Step 2: Build paper -> topic lookup
        let mut paper_to_topic: HashMap<String, String> = HashMap::new();
        for (topic_id, papers) in &topic_papers {
            for paper_id in papers {
                paper_to_topic.insert(paper_id.clone(), topic_id.clone());
            }
        }

        if paper_to_topic.is_empty() {
            return Ok(HashMap::new());
        }

        // Step 3: Get all edges involving these papers
        let all_paper_ids: Vec<&str> = paper_to_topic.keys().map(|s| s.as_str()).collect();
        let edges = self.get_edges_for_nodes_bulk(&all_paper_ids)?;

        // Step 4: Count cross-edges between topic pairs
        let mut cross_edge_counts: HashMap<(String, String), usize> = HashMap::new();

        for (source, target, _weight) in edges {
            let source_topic = paper_to_topic.get(&source);
            let target_topic = paper_to_topic.get(&target);

            if let (Some(topic_a), Some(topic_b)) = (source_topic, target_topic) {
                // Only count if papers are in DIFFERENT topics
                if topic_a != topic_b {
                    // Canonical ordering: smaller ID first
                    let key = if topic_a < topic_b {
                        (topic_a.clone(), topic_b.clone())
                    } else {
                        (topic_b.clone(), topic_a.clone())
                    };
                    *cross_edge_counts.entry(key).or_default() += 1;
                }
            }
        }

        // Edges are stored once but we counted from both directions in get_edges_for_nodes_bulk
        // Divide by 2 to get actual count
        for count in cross_edge_counts.values_mut() {
            *count /= 2;
        }

        Ok(cross_edge_counts)
    }

    /// Get cross-edge counts between all child categories of a parent using efficient SQL joins.
    ///
    /// Returns Vec<(topic_a_id, topic_b_id, count)> for all pairs of children (categories or items)
    /// that have edges between them. Pairs with zero edges are not included.
    ///
    /// This is O(E) where E = edges, not O(T²) where T = topics.
    /// Much more efficient than count_all_cross_edges for large datasets.
    ///
    /// The query joins edges with nodes to find which parent each endpoint belongs to,
    /// then groups by parent pairs to count cross-edges.
    pub fn get_cross_edge_counts_for_children(&self, parent_id: &str) -> Result<Vec<(String, String, usize)>> {
        let conn = self.conn.lock().unwrap();

        // This query:
        // 1. Joins edges with nodes to get the parent_id of each endpoint
        // 2. Filters to edges where both endpoints have the specified parent_id as ancestor
        // 3. Groups by the parent_id pairs and counts
        // 4. Only returns pairs where endpoints are in DIFFERENT immediate children
        //
        // Note: This handles direct children. For recursive (papers under sub-categories),
        // we'd need a recursive CTE, but direct children is what we need for uber-category grouping.
        let mut stmt = conn.prepare(
            "SELECT
                n1.parent_id as topic_a,
                n2.parent_id as topic_b,
                COUNT(*) as cross_count
             FROM edges e
             JOIN nodes n1 ON e.source_id = n1.id
             JOIN nodes n2 ON e.target_id = n2.id
             WHERE n1.parent_id IS NOT NULL
               AND n2.parent_id IS NOT NULL
               AND n1.parent_id != n2.parent_id
               AND EXISTS (SELECT 1 FROM nodes p1 WHERE p1.id = n1.parent_id AND p1.parent_id = ?1)
               AND EXISTS (SELECT 1 FROM nodes p2 WHERE p2.id = n2.parent_id AND p2.parent_id = ?1)
             GROUP BY n1.parent_id, n2.parent_id
             HAVING COUNT(*) >= 1"
        )?;

        let results: Vec<(String, String, usize)> = stmt.query_map(params![parent_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, usize>(2)?,
            ))
        })?.filter_map(|r| r.ok()).collect();

        // The query counts each edge once from source->target perspective.
        // Edges are directional in the table, so we get (A,B,count) and (B,A,count) separately.
        // Combine them into canonical (min_id, max_id, total) pairs.
        let mut combined: std::collections::HashMap<(String, String), usize> = std::collections::HashMap::new();
        for (a, b, count) in results {
            let key = if a < b { (a, b) } else { (b, a) };
            *combined.entry(key).or_default() += count;
        }

        Ok(combined.into_iter().map(|((a, b), c)| (a, b, c)).collect())
    }

    /// Get cross-edge counts for sibling categories (categories sharing the same parent).
    /// Uses efficient SQL joins - O(E) not O(T²).
    ///
    /// Returns Vec<(cat_a_id, cat_b_id, count, size_a, size_b)> where:
    /// - cat_a_id, cat_b_id are sibling category IDs (canonical order: a < b)
    /// - count is the number of edges between items in cat_a and items in cat_b
    /// - size_a, size_b are the number of items in each category (for normalization)
    pub fn get_sibling_cross_edge_counts(&self, parent_id: &str) -> Result<Vec<(String, String, usize, usize, usize)>> {
        let conn = self.conn.lock().unwrap();

        // First, get all sibling categories and their item counts
        let mut cat_stmt = conn.prepare(
            "SELECT c.id, COUNT(i.id) as item_count
             FROM nodes c
             LEFT JOIN nodes i ON i.parent_id = c.id AND i.is_item = 1
             WHERE c.parent_id = ?1 AND c.is_item = 0
             GROUP BY c.id"
        )?;

        let category_sizes: std::collections::HashMap<String, usize> = cat_stmt
            .query_map(params![parent_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        if category_sizes.len() < 2 {
            return Ok(vec![]);
        }

        // Get cross-edge counts between items in sibling categories
        // This query finds edges where source and target are items whose parents are different siblings
        let mut edge_stmt = conn.prepare(
            "SELECT
                n1.parent_id as cat_a,
                n2.parent_id as cat_b,
                COUNT(*) as cross_count
             FROM edges e
             JOIN nodes n1 ON e.source_id = n1.id
             JOIN nodes n2 ON e.target_id = n2.id
             WHERE n1.is_item = 1
               AND n2.is_item = 1
               AND n1.parent_id IS NOT NULL
               AND n2.parent_id IS NOT NULL
               AND n1.parent_id != n2.parent_id
               AND EXISTS (SELECT 1 FROM nodes p WHERE p.id = n1.parent_id AND p.parent_id = ?1 AND p.is_item = 0)
               AND EXISTS (SELECT 1 FROM nodes p WHERE p.id = n2.parent_id AND p.parent_id = ?1 AND p.is_item = 0)
             GROUP BY n1.parent_id, n2.parent_id"
        )?;

        let edge_results: Vec<(String, String, usize)> = edge_stmt
            .query_map(params![parent_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, usize>(2)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Combine directional counts into canonical pairs
        let mut combined: std::collections::HashMap<(String, String), usize> = std::collections::HashMap::new();
        for (a, b, count) in edge_results {
            let key = if a < b { (a, b) } else { (b, a) };
            *combined.entry(key).or_default() += count;
        }

        // Build result with sizes
        let results: Vec<(String, String, usize, usize, usize)> = combined
            .into_iter()
            .map(|((a, b), count)| {
                let size_a = category_sizes.get(&a).copied().unwrap_or(0);
                let size_b = category_sizes.get(&b).copied().unwrap_or(0);
                (a, b, count, size_a, size_b)
            })
            .collect();

        Ok(results)
    }

    /// Get all sibling category pairs under a parent (for creating edges even with 0 cross-count).
    /// Returns Vec<(cat_a_id, cat_b_id, size_a, size_b)> in canonical order.
    pub fn get_all_sibling_pairs(&self, parent_id: &str) -> Result<Vec<(String, String, usize, usize)>> {
        let conn = self.conn.lock().unwrap();

        // Get all sibling categories and their item counts
        let mut stmt = conn.prepare(
            "SELECT c.id, COUNT(i.id) as item_count
             FROM nodes c
             LEFT JOIN nodes i ON i.parent_id = c.id AND i.is_item = 1
             WHERE c.parent_id = ?1 AND c.is_item = 0
             GROUP BY c.id
             ORDER BY c.id"
        )?;

        let categories: Vec<(String, usize)> = stmt
            .query_map(params![parent_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Generate all pairs
        let mut pairs = Vec::new();
        for i in 0..categories.len() {
            for j in (i + 1)..categories.len() {
                let (id_a, size_a) = &categories[i];
                let (id_b, size_b) = &categories[j];
                pairs.push((id_a.clone(), id_b.clone(), *size_a, *size_b));
            }
        }

        Ok(pairs)
    }

    pub fn delete_edge(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM edges WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ==================== Holerabbit DB Functions ====================

    /// Get all nodes with a specific content_type
    pub fn get_nodes_by_content_type(&self, content_type: &str) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE content_type = ?1 ORDER BY created_at DESC",
            Self::NODE_COLUMNS
        ))?;
        let nodes = stmt.query_map(params![content_type], Self::row_to_node)?
            .collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Count edges from a source with specific type
    pub fn get_edge_count_by_source_and_type(&self, source_id: &str, edge_type: &str) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE source_id = ?1 AND type = ?2",
            params![source_id, edge_type],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Get edges from a source with specific type
    pub fn get_edges_by_source_and_type(&self, source_id: &str, edge_type: &str) -> Result<Vec<Edge>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, source_id, target_id, type, label, weight, edge_source, evidence_id, confidence, created_at
             FROM edges WHERE source_id = ?1 AND type = ?2"
        )?;
        let edges = stmt.query_map(params![source_id, edge_type], |row| {
            Ok(Edge {
                id: row.get(0)?,
                source: row.get(1)?,
                target: row.get(2)?,
                edge_type: EdgeType::from_str(&row.get::<_, String>(3)?).unwrap_or(EdgeType::Related),
                label: row.get(4)?,
                weight: row.get(5)?,
                edge_source: row.get(6)?,
                evidence_id: row.get(7)?,
                confidence: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?.collect::<Result<Vec<_>>>()?;
        Ok(edges)
    }

    /// Delete edge by source, target, and type
    pub fn delete_edge_by_endpoints(&self, source_id: &str, target_id: &str, edge_type: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM edges WHERE source_id = ?1 AND target_id = ?2 AND type = ?3",
            params![source_id, target_id, edge_type],
        )?;
        Ok(())
    }

    /// Update node title
    pub fn update_node_title(&self, node_id: &str, title: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET title = ?2 WHERE id = ?1",
            params![node_id, title],
        )?;
        Ok(())
    }

    /// Update edge parent columns based on current node hierarchy
    /// This enables O(1) edge lookups per view instead of O(E) filtering
    pub fn update_edge_parents(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();

        // Update source_parent_id and target_parent_id based on node.parent_id
        let updated = conn.execute(
            "UPDATE edges SET
                source_parent_id = (SELECT parent_id FROM nodes WHERE id = edges.source_id),
                target_parent_id = (SELECT parent_id FROM nodes WHERE id = edges.target_id)",
            [],
        )?;

        println!("[Edge Parents] Updated {} edges with parent IDs", updated);
        Ok(updated)
    }

    /// Get edges for a specific view (where both endpoints are children of the given parent)
    /// Uses indexed lookup on (source_parent_id, target_parent_id) for O(1) performance
    pub fn get_edges_for_view(&self, parent_id: &str) -> Result<Vec<Edge>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, source_id, target_id, type, label, weight,
                    edge_source, evidence_id, confidence, created_at
             FROM edges
             WHERE source_parent_id = ?1 AND target_parent_id = ?1"
        )?;

        let edges = stmt.query_map([parent_id], |row| {
            Ok(Edge {
                id: row.get(0)?,
                source: row.get(1)?,
                target: row.get(2)?,
                edge_type: EdgeType::from_str(&row.get::<_, String>(3)?).unwrap_or(EdgeType::Related),
                label: row.get(4)?,
                weight: row.get(5)?,
                edge_source: row.get(6)?,
                evidence_id: row.get(7)?,
                confidence: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?.collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(edges)
    }

    // Search
    pub fn search_nodes(&self, query: &str) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT n.id, n.type, n.title, n.url, n.content, n.position_x, n.position_y, n.created_at, n.updated_at, n.cluster_id, n.cluster_label, n.ai_title, n.summary, n.tags, n.emoji, n.is_processed, n.depth, n.is_item, n.is_universe, n.parent_id, n.child_count, n.conversation_id, n.sequence_index, n.is_pinned, n.last_accessed_at, n.latest_child_date, n.is_private, n.privacy_reason, n.source, n.pdf_available, n.content_type, n.associated_idea_id, n.privacy
             FROM nodes n
             JOIN nodes_fts fts ON n.rowid = fts.rowid
             WHERE nodes_fts MATCH ?1
             ORDER BY rank"
        )?;

        let nodes = stmt.query_map(params![query], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    // Update cluster assignment for a node (legacy - use update_node_clustering instead)
    #[allow(dead_code)]
    pub fn update_node_cluster(&self, node_id: &str, cluster_id: i32, cluster_label: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET cluster_id = ?2, cluster_label = ?3 WHERE id = ?1",
            params![node_id, cluster_id, cluster_label],
        )?;
        Ok(())
    }

    // Update AI processing results for a node
    pub fn update_node_ai(&self, node_id: &str, ai_title: &str, summary: &str, tags: &str, content_type: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET ai_title = ?2, summary = ?3, tags = ?4, content_type = ?5, is_processed = 1 WHERE id = ?1",
            params![node_id, ai_title, summary, tags, content_type],
        )?;
        Ok(())
    }

    // Update only ai_title and summary (preserves tags, content_type) - for code items
    pub fn update_node_ai_summary_only(&self, node_id: &str, ai_title: &str, summary: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET ai_title = ?2, summary = ?3, is_processed = 1 WHERE id = ?1",
            params![node_id, ai_title, summary],
        )?;
        Ok(())
    }

    /// Update only the tags field for a node (used for repairing code node metadata)
    pub fn update_node_tags(&self, node_id: &str, tags: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET tags = ?2 WHERE id = ?1",
            params![node_id, tags],
        )?;
        Ok(())
    }

    // Get items that haven't been processed by AI yet
    pub fn get_unprocessed_nodes(&self) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE is_item = 1 AND (is_processed = 0 OR is_processed IS NULL) ORDER BY created_at DESC",
            Self::NODE_COLUMNS
        ))?;

        let nodes = stmt.query_map([], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    // Learned emoji operations
    pub fn get_learned_emojis(&self) -> Result<std::collections::HashMap<String, String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT keyword, emoji FROM learned_emojis")?;
        let mappings = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?.collect::<Result<std::collections::HashMap<_, _>>>()?;
        Ok(mappings)
    }

    pub fn save_learned_emoji(&self, keyword: &str, emoji: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        conn.execute(
            "INSERT OR REPLACE INTO learned_emojis (keyword, emoji, created_at) VALUES (?1, ?2, ?3)",
            params![keyword.to_lowercase(), emoji, now],
        )?;
        Ok(())
    }

    // ==================== Hierarchy Operations ====================

    /// Get all nodes at a specific depth
    pub fn get_nodes_at_depth(&self, depth: i32) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE depth = ?1 ORDER BY child_count DESC, title",
            Self::NODE_COLUMNS
        ))?;

        let nodes = stmt.query_map(params![depth], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Get children of a specific parent node
    pub fn get_children(&self, parent_id: &str) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE parent_id = ?1 ORDER BY child_count DESC, title",
            Self::NODE_COLUMNS
        ))?;

        let nodes = stmt.query_map(params![parent_id], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Get children with pagination - for large node sets
    pub fn get_children_paginated(&self, parent_id: &str, limit: usize, offset: usize) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE parent_id = ?1 ORDER BY child_count DESC, title LIMIT ?2 OFFSET ?3",
            Self::NODE_COLUMNS
        ))?;

        let nodes = stmt.query_map(params![parent_id, limit as i64, offset as i64], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    // ==================== Content Visibility Query Methods ====================
    //
    // VISIBLE tier: insight, idea, exploration, synthesis, question, planning, paper
    // SUPPORTING tier: investigation, discussion, reference, creative
    // HIDDEN tier: debug, code, paste, trivial
    // NOTE: code_* types (code_function, code_struct, etc.) are VISIBLE for Code Intelligence

    /// SQL fragment for VISIBLE content types (for graph rendering)
    /// Includes: standard visible types + code_* types from code import
    const VISIBLE_CONTENT_TYPES: &'static str =
        "content_type IS NULL OR content_type IN ('insight', 'idea', 'exploration', 'synthesis', 'question', 'planning', 'paper', 'bookmark') OR content_type LIKE 'code_%'";

    /// Get only VISIBLE tier nodes for graph rendering
    /// Categories (is_item = 0) are always included
    /// If include_hidden is true, also includes HIDDEN tier (debug, code, paste, trivial)
    pub fn get_graph_children(&self, parent_id: &str, include_hidden: bool) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let query = if include_hidden {
            // Include all items (VISIBLE + SUPPORTING + HIDDEN)
            format!(
                "SELECT {} FROM nodes WHERE parent_id = ?1 ORDER BY child_count DESC, title",
                Self::NODE_COLUMNS
            )
        } else {
            // Only VISIBLE tier items
            format!(
                "SELECT {} FROM nodes WHERE parent_id = ?1 AND (is_item = 0 OR ({})) ORDER BY child_count DESC, title",
                Self::NODE_COLUMNS, Self::VISIBLE_CONTENT_TYPES
            )
        };
        let mut stmt = conn.prepare(&query)?;

        let nodes = stmt.query_map(params![parent_id], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Get SUPPORTING tier items under a parent (for lazy-loading in leaf view)
    /// Returns only SUPPORTING tier: investigation, discussion, reference, creative
    /// HIDDEN tier (code, debug, paste, trivial) is excluded entirely
    pub fn get_supporting_items(&self, parent_id: &str) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE parent_id = ?1
             AND content_type IN ('investigation', 'discussion', 'reference', 'creative')
             ORDER BY content_type, created_at DESC",
            Self::NODE_COLUMNS
        ))?;

        let nodes = stmt.query_map(params![parent_id], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Get items associated with a specific idea
    pub fn get_associated_items(&self, idea_id: &str) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE associated_idea_id = ?1 ORDER BY content_type, created_at DESC",
            Self::NODE_COLUMNS
        ))?;

        let nodes = stmt.query_map(params![idea_id], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Get counts of supporting items for badge display
    /// HIDDEN tier (code, debug, paste) returns 0 - they are excluded from UI
    pub fn get_supporting_counts(&self, _parent_id: &str) -> Result<(i32, i32, i32)> {
        // HIDDEN tier items (code, debug, paste) are excluded from the UI entirely
        // Return 0 for all counts so the tabs don't appear
        Ok((0, 0, 0))
    }

    /// Get count of items associated with a specific idea
    pub fn get_associated_count(&self, idea_id: &str) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE associated_idea_id = ?1",
            params![idea_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Get the Universe node (single root, is_universe = true)
    /// Prefers id='universe' if multiple universe nodes exist (legacy cleanup)
    pub fn get_universe(&self) -> Result<Option<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE is_universe = 1 ORDER BY CASE WHEN id = 'universe' THEN 0 ELSE 1 END LIMIT 1",
            Self::NODE_COLUMNS
        ))?;

        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            Ok(Some(Self::row_to_node(row)?))
        } else {
            Ok(None)
        }
    }

    /// Get all items (is_item = true) - openable content
    pub fn get_items(&self) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE is_item = 1 ORDER BY created_at DESC",
            Self::NODE_COLUMNS
        ))?;

        let nodes = stmt.query_map([], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Get only VISIBLE tier items for hierarchy building
    /// VISIBLE: insight, exploration, synthesis, question, planning, paper, code_*
    /// Excludes SUPPORTING (investigation, discussion, reference, creative)
    /// Excludes HIDDEN (debug, code, paste, trivial)
    pub fn get_visible_items(&self) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE is_item = 1
             AND (content_type IN ('insight', 'exploration', 'synthesis', 'question', 'planning', 'paper', 'bookmark')
                  OR content_type LIKE 'code_%')
             ORDER BY created_at DESC",
            Self::NODE_COLUMNS
        ))?;

        let nodes = stmt.query_map([], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Get items with pagination - for large datasets
    pub fn get_items_paginated(&self, limit: usize, offset: usize) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE is_item = 1 ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
            Self::NODE_COLUMNS
        ))?;

        let nodes = stmt.query_map(params![limit as i64, offset as i64], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Update a node's parent and depth
    pub fn update_node_hierarchy(&self, node_id: &str, parent_id: Option<&str>, depth: i32) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET parent_id = ?2, depth = ?3 WHERE id = ?1",
            params![node_id, parent_id, depth],
        )?;
        Ok(())
    }

    /// Update a node's child count
    pub fn update_child_count(&self, node_id: &str, child_count: i32) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET child_count = ?2 WHERE id = ?1",
            params![node_id, child_count],
        )?;
        Ok(())
    }

    /// Recalculate and update a node's child count from actual children
    pub fn recalculate_child_count(&self, node_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET child_count = (SELECT COUNT(*) FROM nodes WHERE parent_id = ?1) WHERE id = ?1",
            params![node_id],
        )?;
        Ok(())
    }

    /// Recalculate child_count for ALL categories (non-items) in one SQL statement
    pub fn recalculate_all_child_counts(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE nodes SET child_count = (
                SELECT COUNT(*) FROM nodes AS children WHERE children.parent_id = nodes.id
            ) WHERE is_item = 0",
            [],
        )?;
        Ok(updated)
    }

    /// Count children of a node
    #[allow(dead_code)]
    pub fn count_children(&self, parent_id: &str) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE parent_id = ?1",
            params![parent_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Get max depth in the hierarchy
    pub fn get_max_depth(&self) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
        let max_depth: i32 = conn.query_row(
            "SELECT COALESCE(MAX(depth), 0) FROM nodes",
            [],
            |row| row.get(0),
        )?;
        Ok(max_depth)
    }

    /// Delete all non-item nodes (intermediate hierarchy scaffolding)
    /// Skips protected nodes (Recent Notes container and descendants)
    pub fn delete_hierarchy_nodes(&self) -> Result<usize> {
        use crate::settings;

        let protected_ids = self.get_protected_node_ids();
        let conn = self.conn.lock().unwrap();

        if protected_ids.is_empty() {
            // No protection - delete all
            let deleted = conn.execute(
                "DELETE FROM nodes WHERE is_item = 0 AND is_universe = 0",
                [],
            )?;
            Ok(deleted)
        } else {
            // Build exclusion list
            let placeholders: Vec<String> = protected_ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect();
            let sql = format!(
                "DELETE FROM nodes WHERE is_item = 0 AND is_universe = 0 AND id NOT IN ({})",
                placeholders.join(", ")
            );

            let params: Vec<&str> = protected_ids.iter().map(|s| s.as_str()).collect();
            let deleted = conn.execute(&sql, rusqlite::params_from_iter(params))?;

            if settings::is_recent_notes_protected() {
                println!("[Hierarchy] Preserved {} protected nodes (Recent Notes)", protected_ids.len());
            }
            Ok(deleted)
        }
    }

    /// Clear parent_id on all items (for rebuild)
    /// Skips protected items (descendants of Recent Notes)
    pub fn clear_item_parents(&self) -> Result<()> {
        use crate::settings;

        let protected_ids = self.get_protected_node_ids();
        let conn = self.conn.lock().unwrap();

        if protected_ids.is_empty() {
            // No protection - clear all
            conn.execute(
                "UPDATE nodes SET parent_id = NULL WHERE is_item = 1",
                [],
            )?;
        } else {
            // Build exclusion list
            let placeholders: Vec<String> = protected_ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect();
            let sql = format!(
                "UPDATE nodes SET parent_id = NULL WHERE is_item = 1 AND id NOT IN ({})",
                placeholders.join(", ")
            );

            let params: Vec<&str> = protected_ids.iter().map(|s| s.as_str()).collect();
            conn.execute(&sql, rusqlite::params_from_iter(params))?;

            if settings::is_recent_notes_protected() {
                println!("[Hierarchy] Preserved parent_id on {} protected items", protected_ids.len());
            }
        }
        Ok(())
    }

    /// Delete hierarchy nodes at depth > min_depth (preserves FOS nodes at depth 1)
    /// Used when rebuilding hierarchy while preserving top-level structure
    pub fn delete_hierarchy_nodes_below_depth(&self, min_depth: i32) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute(
            "DELETE FROM nodes WHERE is_item = 0 AND is_universe = 0 AND depth > ?1",
            [min_depth],
        )?;
        Ok(deleted)
    }

    /// Clear parent_id only on items whose parent no longer exists
    /// Used after deleting hierarchy nodes to clean up orphaned references
    pub fn clear_orphaned_item_parents(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET parent_id = NULL
             WHERE is_item = 1 AND parent_id IS NOT NULL
             AND parent_id NOT IN (SELECT id FROM nodes)",
            [],
        )?;
        Ok(())
    }

    /// Get all category/topic names in the hierarchy (for duplicate prevention)
    /// Returns distinct names from non-item nodes
    pub fn get_all_category_names(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT COALESCE(cluster_label, title)
             FROM nodes
             WHERE is_item = 0"
        )?;

        let names: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(names)
    }

    // ==================== Clustering Operations ====================

    /// Get items that need clustering (needs_clustering = 1 AND is_item = 1)
    pub fn get_items_needing_clustering(&self) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE is_item = 1 AND needs_clustering = 1 ORDER BY created_at DESC",
            Self::NODE_COLUMNS
        ))?;

        let nodes = stmt.query_map([], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Count items that need clustering
    pub fn count_items_needing_clustering(&self) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND needs_clustering = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Count items that have been assigned to clusters
    pub fn count_clustered_items(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND cluster_id IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Get existing cluster info for AI context
    pub fn get_existing_clusters(&self) -> Result<Vec<(i32, String, i32)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT cluster_id, cluster_label, COUNT(*) as count
             FROM nodes
             WHERE is_item = 1 AND cluster_id IS NOT NULL AND needs_clustering = 0
             GROUP BY cluster_id, cluster_label
             ORDER BY count DESC"
        )?;

        let clusters = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?.collect::<Result<Vec<_>>>()?;

        Ok(clusters)
    }

    /// Find a topic node by cluster_label (for quick hierarchy add)
    pub fn find_topic_by_cluster_label(&self, cluster_label: &str) -> Result<Option<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE cluster_label = ? AND is_item = 0 AND depth > 0 LIMIT 1",
            Self::NODE_COLUMNS
        ))?;

        let mut rows = stmt.query([cluster_label])?;
        if let Some(row) = rows.next()? {
            Ok(Some(Self::row_to_node(row)?))
        } else {
            Ok(None)
        }
    }

    /// Get orphaned items (items with no parent_id) that have been clustered
    pub fn get_orphaned_clustered_items(&self) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE is_item = 1 AND parent_id IS NULL AND cluster_id IS NOT NULL ORDER BY created_at DESC",
            Self::NODE_COLUMNS
        ))?;

        let nodes = stmt.query_map([], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Update a node's parent and depth
    pub fn set_node_parent(&self, node_id: &str, parent_id: &str, depth: i32) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET parent_id = ?, depth = ? WHERE id = ?",
            rusqlite::params![parent_id, depth, node_id],
        )?;
        Ok(())
    }

    /// Increment a node's child_count
    pub fn increment_child_count(&self, node_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET child_count = child_count + 1 WHERE id = ?",
            [node_id],
        )?;
        Ok(())
    }

    /// Get next available cluster_id
    pub fn get_next_cluster_id(&self) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
        let max_id: Option<i32> = conn.query_row(
            "SELECT MAX(cluster_id) FROM nodes WHERE cluster_id >= 0",
            [],
            |row| row.get(0),
        ).ok();
        Ok(max_id.unwrap_or(-1) + 1)
    }

    /// Update clustering for a node and mark as clustered
    pub fn update_node_clustering(&self, node_id: &str, cluster_id: i32, cluster_label: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET cluster_id = ?2, cluster_label = ?3, needs_clustering = 0 WHERE id = ?1",
            params![node_id, cluster_id, cluster_label],
        )?;
        Ok(())
    }

    /// Mark items as needing re-clustering (e.g., after import)
    #[allow(dead_code)]
    pub fn mark_items_need_clustering(&self, node_ids: &[String]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        for id in node_ids {
            conn.execute(
                "UPDATE nodes SET needs_clustering = 1 WHERE id = ?1",
                params![id],
            )?;
        }
        Ok(())
    }

    /// Mark all items as needing clustering (for full rebuild)
    pub fn mark_all_items_need_clustering(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE nodes SET needs_clustering = 1, cluster_id = NULL, cluster_label = NULL WHERE is_item = 1",
            [],
        )?;
        Ok(updated)
    }

    /// Get clusters that need AI naming
    /// Returns clusters where the label looks like keyword-generated (contains comma)
    /// or is a generic "Cluster N" name
    pub fn get_clusters_needing_names(&self) -> Result<Vec<(i32, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT cluster_id, cluster_label FROM nodes
             WHERE cluster_id IS NOT NULL
               AND cluster_label IS NOT NULL
               AND (cluster_label LIKE '%,%' OR cluster_label LIKE 'Cluster %')
             ORDER BY cluster_id"
        )?;

        let clusters = stmt.query_map([], |row| {
            Ok((row.get::<_, i32>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(clusters)
    }

    /// Get sample items from a cluster for naming
    pub fn get_cluster_sample_items(&self, cluster_id: i32, limit: usize) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let query = format!(
            "SELECT {} FROM nodes WHERE cluster_id = ?1 AND is_item = 1 LIMIT ?2",
            Self::NODE_COLUMNS
        );
        let mut stmt = conn.prepare(&query)?;
        let nodes = stmt
            .query_map(params![cluster_id, limit as i64], Self::row_to_node)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(nodes)
    }

    /// Update cluster label for all items in a cluster
    pub fn update_cluster_label(&self, cluster_id: i32, new_label: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE nodes SET cluster_label = ?2 WHERE cluster_id = ?1",
            params![cluster_id, new_label],
        )?;
        Ok(updated)
    }

    // ==================== Multi-Path Edge Operations ====================

    /// Delete AI-generated BelongsTo edges for a node (preserves user-edited edges)
    /// Only deletes edges where edge_source = 'ai' or edge_source IS NULL (legacy)
    pub fn delete_belongs_to_edges(&self, node_id: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute(
            "DELETE FROM edges WHERE source_id = ?1 AND type = 'belongs_to' AND (edge_source = 'ai' OR edge_source IS NULL)",
            params![node_id],
        )?;
        Ok(deleted)
    }

    /// Get user-edited BelongsTo edges for a node (edge_source = 'user')
    /// Used during clustering to skip re-generating edges user has curated
    #[allow(dead_code)]
    pub fn get_user_belongs_to_edges(&self, node_id: &str) -> Result<Vec<Edge>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, source_id, target_id, type, label, weight, edge_source, evidence_id, confidence, created_at
             FROM edges WHERE source_id = ?1 AND type = 'belongs_to' AND edge_source = 'user'
             ORDER BY weight DESC"
        )?;

        let edges = stmt.query_map(params![node_id], |row| {
            Ok(Edge {
                id: row.get(0)?,
                source: row.get(1)?,
                target: row.get(2)?,
                edge_type: EdgeType::from_str(&row.get::<_, String>(3)?).unwrap_or(EdgeType::BelongsTo),
                label: row.get(4)?,
                weight: row.get(5)?,
                edge_source: row.get(6)?,
                evidence_id: row.get(7)?,
                confidence: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?.collect::<Result<Vec<_>>>()?;

        Ok(edges)
    }

    /// Find topic node ID for a cluster_id (e.g., returns "topic-0" for cluster_id 0)
    /// Topic nodes are created by hierarchy builder with IDs like "topic-{cluster_id}"
    pub fn find_topic_node_for_cluster(&self, cluster_id: i32) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        // Topic nodes have cluster_id set and are not items (intermediate hierarchy nodes)
        let topic_id = format!("topic-{}", cluster_id);

        // Check if this topic node exists
        let exists: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM nodes WHERE id = ?1",
            params![topic_id],
            |row| row.get(0),
        ).unwrap_or(false);

        if exists {
            Ok(Some(topic_id))
        } else {
            // Fallback: find any non-item node with this cluster_id
            let result: Option<String> = conn.query_row(
                "SELECT id FROM nodes WHERE cluster_id = ?1 AND is_item = 0 LIMIT 1",
                params![cluster_id],
                |row| row.get(0),
            ).ok();
            Ok(result)
        }
    }

    /// Get all BelongsTo edges for a node (category associations)
    pub fn get_belongs_to_edges(&self, node_id: &str) -> Result<Vec<Edge>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, source_id, target_id, type, label, weight, edge_source, evidence_id, confidence, created_at
             FROM edges WHERE source_id = ?1 AND type = 'belongs_to'
             ORDER BY weight DESC"
        )?;

        let edges = stmt.query_map(params![node_id], |row| {
            Ok(Edge {
                id: row.get(0)?,
                source: row.get(1)?,
                target: row.get(2)?,
                edge_type: EdgeType::from_str(&row.get::<_, String>(3)?).unwrap_or(EdgeType::BelongsTo),
                label: row.get(4)?,
                weight: row.get(5)?,
                edge_source: row.get(6)?,
                evidence_id: row.get(7)?,
                confidence: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?.collect::<Result<Vec<_>>>()?;

        Ok(edges)
    }

    /// Get all items that belong to a cluster (via BelongsTo edges)
    pub fn get_items_in_cluster_via_edges(&self, cluster_id: i32, min_weight: Option<f64>) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let topic_id = format!("topic-{}", cluster_id);
        let placeholder_id = format!("cluster-{}", cluster_id);
        let min_w = min_weight.unwrap_or(0.0);

        let mut stmt = conn.prepare(
            "SELECT n.id, n.type, n.title, n.url, n.content, n.position_x, n.position_y, n.created_at, n.updated_at,
                    n.cluster_id, n.cluster_label, n.ai_title, n.summary, n.tags, n.emoji, n.is_processed, n.depth, n.is_item, n.is_universe, n.parent_id, n.child_count, n.conversation_id, n.sequence_index, n.is_pinned, n.last_accessed_at, n.latest_child_date, n.is_private, n.privacy_reason
             FROM nodes n
             JOIN edges e ON n.id = e.source_id
             WHERE (e.target_id = ?1 OR e.target_id = ?2)
               AND e.type = 'belongs_to'
               AND (e.weight IS NULL OR e.weight >= ?3)
             ORDER BY e.weight DESC"
        )?;

        let nodes = stmt.query_map(params![topic_id, placeholder_id, min_w], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    // ==================== Conversation Context Operations ====================

    // ==================== Recursive Hierarchy Operations ====================

    /// Get child count (recursive) for a node - counts all items in subtree
    #[allow(dead_code)]
    pub fn get_recursive_item_count(&self, node_id: &str) -> Result<i32> {
        let conn = self.conn.lock().unwrap();

        // If the node is an item itself, return 1
        let is_item: bool = conn.query_row(
            "SELECT is_item FROM nodes WHERE id = ?1",
            params![node_id],
            |row| row.get::<_, i32>(0).map(|v| v != 0),
        ).unwrap_or(false);

        if is_item {
            return Ok(1);
        }

        // Otherwise, count items in subtree using recursive CTE
        let count: i32 = conn.query_row(
            "WITH RECURSIVE subtree(id) AS (
                SELECT id FROM nodes WHERE parent_id = ?1
                UNION ALL
                SELECT n.id FROM nodes n JOIN subtree s ON n.parent_id = s.id
            )
            SELECT COUNT(*) FROM nodes WHERE id IN (SELECT id FROM subtree) AND is_item = 1",
            params![node_id],
            |row| row.get(0),
        ).unwrap_or(0);

        Ok(count)
    }

    /// Get topic info for children of a node (for recursive hierarchy building)
    /// Returns (id, label, item_count) for each child
    #[allow(dead_code)]
    pub fn get_children_topic_info(&self, parent_id: &str) -> Result<Vec<(String, String, i32)>> {
        let children = self.get_children(parent_id)?;

        let mut topic_info = Vec::new();
        for child in children {
            // Get item count (either direct if it's an item, or count children)
            let item_count = if child.is_item {
                1
            } else {
                child.child_count.max(1) // At least 1 for display purposes
            };

            let label = child.cluster_label
                .or(child.ai_title.clone())
                .unwrap_or_else(|| child.title.clone());

            topic_info.push((child.id, label, item_count));
        }

        Ok(topic_info)
    }

    /// Update parent_id for a node
    pub fn update_parent(&self, node_id: &str, parent_id: &str) -> Result<()> {
        // Safety: prevent self-referential nodes (causes infinite loops)
        if node_id == parent_id {
            println!("[DB] WARNING: Prevented self-referential parent_id for node '{}'", node_id);
            return Ok(());
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET parent_id = ?2 WHERE id = ?1",
            params![node_id, parent_id],
        )?;
        Ok(())
    }

    /// Set depth for a single node (does NOT cascade to descendants)
    pub fn set_node_depth(&self, node_id: &str, depth: i32) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET depth = ?2 WHERE id = ?1",
            params![node_id, depth],
        )?;
        Ok(())
    }

    /// Get children by their labels (cluster_label or title)
    #[allow(dead_code)]
    pub fn get_children_by_labels(&self, parent_id: &str, labels: &[String]) -> Result<Vec<Node>> {
        let children = self.get_children(parent_id)?;

        let matching: Vec<Node> = children
            .into_iter()
            .filter(|child| {
                let label = child.cluster_label
                    .as_ref()
                    .or(child.ai_title.as_ref())
                    .unwrap_or(&child.title);
                labels.contains(label)
            })
            .collect();

        Ok(matching)
    }

    /// Insert a new hierarchy node (intermediate grouping node)
    #[allow(dead_code)]
    pub fn insert_hierarchy_node(&self, id: &str, title: &str, parent_id: Option<&str>, depth: i32, child_count: i32) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let node = Node {
            id: id.to_string(),
            node_type: NodeType::Cluster,
            title: title.to_string(),
            url: None,
            content: Some(format!("{} items", child_count)),
            position: Position { x: 0.0, y: 0.0 },
            created_at: now,
            updated_at: now,
            cluster_id: None,
            cluster_label: Some(title.to_string()),
            depth,
            is_item: false,
            is_universe: false,
            parent_id: parent_id.map(|s| s.to_string()),
            child_count,
            ai_title: None,
            summary: None,
            tags: None,
            emoji: Some(self.get_level_emoji(depth)),
            is_processed: false,
            conversation_id: None,
            sequence_index: None,
            is_pinned: false,
            last_accessed_at: None,
            latest_child_date: None,
            is_private: None,
            privacy_reason: None,
            source: None,
            pdf_available: None,
            content_type: None,
            associated_idea_id: None,
            privacy: None,
        };

        self.insert_node(&node)
    }

    /// Get emoji for a hierarchy level
    #[allow(dead_code)]
    fn get_level_emoji(&self, depth: i32) -> String {
        match depth {
            0 => "🌌".to_string(),  // Universe
            1 => "🌀".to_string(),  // Domain/Galaxy
            2 => "🌍".to_string(),  // Region
            3 => "🗂️".to_string(), // Topic
            _ => "📁".to_string(),  // Generic folder
        }
    }

    /// Get all messages belonging to a conversation, ordered by sequence_index
    pub fn get_conversation_messages(&self, conversation_id: &str) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE conversation_id = ?1 ORDER BY sequence_index ASC",
            Self::NODE_COLUMNS
        ))?;

        let nodes = stmt.query_map(params![conversation_id], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    // ==================== Date Propagation Operations ====================

    /// Propagate latest_child_date from leaves up through the hierarchy
    /// Processes bottom-up: deepest nodes first, bubbles up to Universe
    /// Leaves (child_count = 0): latest_child_date = created_at
    /// Groups (child_count > 0): latest_child_date = MAX(children's latest_child_date)
    pub fn propagate_latest_dates(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Get max depth in hierarchy
        let max_depth: i32 = conn.query_row(
            "SELECT COALESCE(MAX(depth), 0) FROM nodes",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        println!("Propagating latest dates from depth {} up to 0...", max_depth);

        // Process bottom-up (deepest first)
        for depth in (0..=max_depth).rev() {
            // For leaves (child_count = 0): set latest_child_date = created_at
            let leaves_updated = conn.execute(
                "UPDATE nodes SET latest_child_date = created_at
                 WHERE depth = ?1 AND child_count = 0",
                params![depth],
            )?;

            // For groups (child_count > 0): set to MAX of children's latest_child_date
            let groups_updated = conn.execute(
                "UPDATE nodes SET latest_child_date = (
                    SELECT MAX(c.latest_child_date)
                    FROM nodes c
                    WHERE c.parent_id = nodes.id
                 )
                 WHERE depth = ?1 AND child_count > 0",
                params![depth],
            )?;

            if leaves_updated > 0 || groups_updated > 0 {
                println!("  Depth {}: {} leaves, {} groups updated", depth, leaves_updated, groups_updated);
            }
        }

        println!("✓ Latest dates propagated to all nodes");
        Ok(())
    }

    // ==================== Quick Access Operations (Sidebar) ====================

    /// Pin or unpin a node
    pub fn set_node_pinned(&self, node_id: &str, pinned: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET is_pinned = ?2 WHERE id = ?1",
            params![node_id, pinned as i32],
        )?;
        Ok(())
    }

    /// Set content type for classification (idea, code, debug, paste)
    pub fn set_content_type(&self, node_id: &str, content_type: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET content_type = ?2 WHERE id = ?1",
            params![node_id, content_type],
        )?;
        Ok(())
    }

    /// Batch set content_type for multiple nodes (uses transaction for speed)
    pub fn set_content_types_batch(&self, updates: &[(String, String)]) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;

        let mut updated = 0;
        for (node_id, content_type) in updates {
            tx.execute(
                "UPDATE nodes SET content_type = ?2 WHERE id = ?1",
                params![node_id, content_type],
            )?;
            updated += 1;
        }

        tx.commit()?;
        Ok(updated)
    }

    /// Clear all content_type values (for reclassification)
    pub fn clear_all_content_types(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET content_type = NULL WHERE is_item = 1",
            [],
        )?;
        Ok(())
    }

    /// Set the associated idea ID for a supporting item
    pub fn set_associated_idea(&self, node_id: &str, idea_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET associated_idea_id = ?2 WHERE id = ?1",
            params![node_id, idea_id],
        )?;
        Ok(())
    }

    /// Clear the associated idea ID for a node
    pub fn clear_associated_idea(&self, node_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET associated_idea_id = NULL WHERE id = ?1",
            params![node_id],
        )?;
        Ok(())
    }

    /// Update a node's parent_id (reparent a node)
    pub fn update_node_parent(&self, node_id: &str, new_parent_id: &str) -> Result<()> {
        // Safety: prevent self-referential nodes (causes infinite loops)
        if node_id == new_parent_id {
            println!("[DB] WARNING: Prevented self-referential parent_id for node '{}'", node_id);
            return Ok(());
        }
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        conn.execute(
            "UPDATE nodes SET parent_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![node_id, new_parent_id, now],
        )?;
        Ok(())
    }

    /// Increment depth of a node and all its descendants by a given amount
    pub fn increment_subtree_depth_by(&self, root_id: &str, increment: i32) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Use recursive CTE to find all descendants and update their depth
        conn.execute(
            "WITH RECURSIVE descendants(id) AS (
                SELECT ?1
                UNION ALL
                SELECT n.id FROM nodes n
                JOIN descendants d ON n.parent_id = d.id
            )
            UPDATE nodes SET depth = depth + ?2
            WHERE id IN (SELECT id FROM descendants)",
            params![root_id, increment],
        )?;
        Ok(())
    }

    /// Update last accessed timestamp for a node
    pub fn touch_node(&self, node_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        conn.execute(
            "UPDATE nodes SET last_accessed_at = ?2 WHERE id = ?1",
            params![node_id, now],
        )?;
        Ok(())
    }

    /// Clear last accessed timestamp for a node (remove from recents)
    pub fn clear_recent(&self, node_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET last_accessed_at = NULL WHERE id = ?1",
            params![node_id],
        )?;
        Ok(())
    }

    /// Get pinned nodes (for Sidebar Pinned tab)
    /// Only returns VISIBLE tier items
    pub fn get_pinned_nodes(&self) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes
             WHERE is_pinned = 1 AND ({})
             ORDER BY last_accessed_at DESC",
            Self::NODE_COLUMNS, Self::VISIBLE_CONTENT_TYPES
        ))?;

        let nodes = stmt.query_map([], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    /// Get recently accessed nodes (for Sidebar Recent tab)
    /// Only returns VISIBLE tier items
    pub fn get_recent_nodes(&self, limit: i32) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes
             WHERE last_accessed_at IS NOT NULL AND ({})
             ORDER BY last_accessed_at DESC LIMIT ?1",
            Self::NODE_COLUMNS, Self::VISIBLE_CONTENT_TYPES
        ))?;

        let nodes = stmt.query_map(params![limit], Self::row_to_node)?.collect::<Result<Vec<_>>>()?;
        Ok(nodes)
    }

    // ==================== Embedding Operations ====================

    /// Update a node's embedding (for semantic similarity)
    pub fn update_node_embedding(&self, node_id: &str, embedding: &[f32]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        // Convert f32 slice to raw bytes (little-endian)
        let bytes: Vec<u8> = embedding.iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        conn.execute(
            "UPDATE nodes SET embedding = ?2 WHERE id = ?1",
            params![node_id, bytes],
        )?;
        Ok(())
    }

    /// Get a node's embedding
    pub fn get_node_embedding(&self, node_id: &str) -> Result<Option<Vec<f32>>> {
        let conn = self.conn.lock().unwrap();
        let bytes: Option<Vec<u8>> = conn.query_row(
            "SELECT embedding FROM nodes WHERE id = ?1",
            params![node_id],
            |row| row.get(0),
        ).ok();

        Ok(bytes.map(|b| bytes_to_embedding(&b)))
    }

    /// Get all nodes that have embeddings (for similarity search)
    /// Returns (node_id, embedding) pairs - no content_type filtering
    pub fn get_nodes_with_embeddings(&self) -> Result<Vec<(String, Vec<f32>)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, embedding FROM nodes WHERE embedding IS NOT NULL"
        )?;

        let results = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let bytes: Vec<u8> = row.get(1)?;
            Ok((id, bytes_to_embedding(&bytes)))
        })?.collect::<Result<Vec<_>>>()?;

        Ok(results)
    }

    /// Get count of nodes with embeddings
    pub fn count_nodes_with_embeddings(&self) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE embedding IS NOT NULL",
            [],
            |row| row.get(0),
        )
    }

    /// Get all nodes that need embeddings
    /// Includes ALL items (for association matching) AND category nodes (with title)
    pub fn get_nodes_needing_embeddings(&self) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        // Include all items (needed for association matching) and non-items with titles
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE (is_item = 1 OR (is_item = 0 AND title IS NOT NULL)) AND embedding IS NULL",
            Self::NODE_COLUMNS
        ))?;

        let rows = stmt.query_map([], Self::row_to_node)?;
        rows.collect()
    }

    /// Create semantic "Related" edges between nodes based on embedding similarity
    /// Uses two-pass approach for fair top-K selection:
    /// - Pass 1: Compute all similarities (upper triangle), store candidates for BOTH nodes
    /// - Pass 2: Each node selects top-K, dedupe, batch insert
    /// Gives +0.2 bonus to siblings (same parent) so within-view edges are prioritized
    /// Uses lower threshold (min_similarity - 0.2) for category-to-category edges
    /// Returns the number of edges created
    pub fn create_semantic_edges(&self, min_similarity: f32, max_edges_per_node: usize) -> Result<usize> {
        use crate::similarity::cosine_similarity;
        use std::collections::{HashMap, HashSet};

        // Get all nodes with embeddings
        let nodes_with_embeddings = self.get_nodes_with_embeddings()?;
        let n = nodes_with_embeddings.len();
        if n < 2 {
            return Ok(0);
        }

        eprintln!("[Edges] Starting semantic edge generation for {} nodes", n);

        // Build parent_id and is_item lookup
        let (parent_map, is_item_map): (
            HashMap<String, Option<String>>,
            HashMap<String, bool>
        ) = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare("SELECT id, parent_id, is_item FROM nodes WHERE embedding IS NOT NULL")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, bool>(2)?
                ))
            })?;
            let mut parents = HashMap::new();
            let mut items = HashMap::new();
            for row in rows.filter_map(|r| r.ok()) {
                parents.insert(row.0.clone(), row.1);
                items.insert(row.0, row.2);
            }
            (parents, items)
        };

        const SIBLING_BONUS: f32 = 0.2;
        const CATEGORY_THRESHOLD_REDUCTION: f32 = 0.2;

        // Pass 1: Collect ALL candidate edges above threshold (store for BOTH nodes)
        // This ensures every node gets fair top-K selection regardless of position
        let mut candidates: HashMap<String, Vec<(String, f32, f32)>> = HashMap::new(); // node_id -> [(other_id, raw_sim, boosted_sim)]

        for (i, (node_id, embedding)) in nodes_with_embeddings.iter().enumerate() {
            if i % 1000 == 0 {
                eprintln!("[Edges] Pass 1: Comparing node {}/{} ({:.1}%)", i, n, (i as f64 / n as f64) * 100.0);
            }

            let node_parent = parent_map.get(node_id).and_then(|p| p.clone());
            let node_is_item = *is_item_map.get(node_id).unwrap_or(&true);

            // Upper triangle only - but store for BOTH nodes
            for (other_id, other_emb) in nodes_with_embeddings[i+1..].iter() {
                let raw_sim = cosine_similarity(embedding, other_emb);
                let other_parent = parent_map.get(other_id).and_then(|p| p.clone());
                let other_is_item = *is_item_map.get(other_id).unwrap_or(&true);

                // Boost score if same parent (siblings will be visible together)
                let is_sibling = node_parent.is_some() && node_parent == other_parent;
                let boosted_sim = if is_sibling { raw_sim + SIBLING_BONUS } else { raw_sim };

                // Use lower threshold for category-to-category edges
                let is_category_edge = !node_is_item && !other_is_item;
                let effective_threshold = if is_category_edge {
                    min_similarity - CATEGORY_THRESHOLD_REDUCTION
                } else {
                    min_similarity
                };

                if raw_sim >= effective_threshold {
                    // Store for BOTH nodes so each gets fair top-K selection
                    candidates.entry(node_id.clone()).or_default().push((other_id.clone(), raw_sim, boosted_sim));
                    candidates.entry(other_id.clone()).or_default().push((node_id.clone(), raw_sim, boosted_sim));
                }
            }
        }

        eprintln!("[Edges] Pass 1 complete: {} nodes with candidates", candidates.len());

        // Pass 2: Truncate each node to top-K, dedupe, and batch insert
        let now = chrono::Utc::now().timestamp_millis();
        let mut all_edges: Vec<Edge> = Vec::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut edges_created = 0;
        const BATCH_SIZE: usize = 10_000;

        for (node_id, mut node_candidates) in candidates {
            // Sort by boosted similarity descending
            node_candidates.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
            node_candidates.truncate(max_edges_per_node);

            for (other_id, raw_similarity, _boosted) in node_candidates {
                // Canonical order for deduplication (smaller ID first)
                let key = if node_id < other_id {
                    (node_id.clone(), other_id.clone())
                } else {
                    (other_id.clone(), node_id.clone())
                };

                if seen.insert(key.clone()) {
                    all_edges.push(Edge {
                        id: format!("semantic-{}-{}", key.0, key.1),
                        source: key.0,
                        target: key.1,
                        edge_type: EdgeType::Related,
                        label: Some(format!("{:.0}% similar", raw_similarity * 100.0)),
                        weight: Some(raw_similarity as f64),
                        edge_source: Some("ai".to_string()),
                        evidence_id: None,
                        confidence: Some(raw_similarity as f64),
                        created_at: now,
                    });
                }
            }

            // Batch insert every BATCH_SIZE edges
            if all_edges.len() >= BATCH_SIZE {
                edges_created += self.insert_edges_batch(&all_edges)?;
                all_edges.clear();
                eprintln!("[Edges] Pass 2: {} edges created so far", edges_created);
            }
        }

        // Insert remaining edges
        if !all_edges.is_empty() {
            edges_created += self.insert_edges_batch(&all_edges)?;
        }

        eprintln!("[Edges] Complete: {} edges from {} nodes", edges_created, n);
        Ok(edges_created)
    }

    /// Delete all AI-generated semantic edges (for re-generation)
    pub fn delete_semantic_edges(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute(
            "DELETE FROM edges WHERE type = 'related' AND edge_source = 'ai'",
            [],
        )?;
        Ok(deleted)
    }

    /// Count AI-generated semantic edges
    pub fn count_semantic_edges(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE type = 'related' AND edge_source = 'ai'",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    // ==================== Settings Panel Operations ====================

    /// Delete all nodes
    pub fn delete_all_nodes(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute("DELETE FROM nodes", [])?;
        Ok(deleted)
    }

    /// Delete all edges
    pub fn delete_all_edges(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute("DELETE FROM edges", [])?;
        Ok(deleted)
    }

    /// Delete all edges where node is source or target
    pub fn delete_edges_for_node(&self, node_id: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute(
            "DELETE FROM edges WHERE source_id = ?1 OR target_id = ?1",
            params![node_id],
        )?;
        Ok(deleted)
    }

    /// Clear parent_id references to a node (set to NULL)
    /// Used before deleting a node to avoid foreign key violations
    pub fn clear_parent_references(&self, node_id: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE nodes SET parent_id = NULL WHERE parent_id = ?1",
            params![node_id],
        )?;
        Ok(updated)
    }

    /// Delete empty items (items with no meaningful content/raw data)
    pub fn delete_empty_items(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute(
            "DELETE FROM nodes WHERE is_item = 1 AND (
                content IS NULL
                OR TRIM(content) = ''
                OR LENGTH(TRIM(content)) < 10
                OR TRIM(REPLACE(REPLACE(REPLACE(REPLACE(content, 'Human:', ''), 'Assistant:', ''), 'A:', ''), char(10), '')) = ''
            )",
            [],
        )?;
        Ok(deleted)
    }

    /// Delete incomplete conversations (has human query but no real Claude response)
    /// Detects:
    /// - Conversations with [H] but no [A] (our format)
    /// - Conversations with Human: but no Assistant: (legacy)
    /// - Conversations with "*No response*" placeholder
    pub fn delete_incomplete_conversations(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute(
            "DELETE FROM nodes WHERE is_item = 1 AND content IS NOT NULL AND (
                -- Our format: has [H] but no [A]
                (content LIKE '%[H]%' AND content NOT LIKE '%[A]%')
                -- Legacy format: has Human: but no Assistant:
                OR (content LIKE '%Human:%' AND content NOT LIKE '%Assistant:%' AND content NOT LIKE '%[A]%')
                -- Placeholder for missing response
                OR content LIKE '%*No response*%'
            )",
            [],
        )?;
        Ok(deleted)
    }

    /// Reset AI processing flag (mark all items as unprocessed)
    pub fn reset_ai_processing(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE nodes SET is_processed = 0, ai_title = NULL, summary = NULL, tags = NULL, emoji = NULL WHERE is_item = 1",
            [],
        )?;
        Ok(updated)
    }

    /// Clear all embeddings
    pub fn clear_all_embeddings(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE nodes SET embedding = NULL",
            [],
        )?;
        Ok(updated)
    }

    /// Get database stats for settings panel
    pub fn get_stats(&self) -> Result<(usize, usize, usize, usize, usize, usize, usize, usize)> {
        let conn = self.conn.lock().unwrap();
        let total_nodes: usize = conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))?;
        let total_items: usize = conn.query_row("SELECT COUNT(*) FROM nodes WHERE is_item = 1", [], |r| r.get(0))?;
        let processed: usize = conn.query_row("SELECT COUNT(*) FROM nodes WHERE is_processed = 1", [], |r| r.get(0))?;
        let with_embeddings: usize = conn.query_row("SELECT COUNT(*) FROM nodes WHERE embedding IS NOT NULL", [], |r| r.get(0))?;
        // Additional stats for API cost estimation
        let unprocessed: usize = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND is_processed = 0", [], |r| r.get(0))?;
        let unclustered: usize = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND cluster_id IS NULL", [], |r| r.get(0))?;
        let orphan_items: usize = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 1 AND parent_id IS NULL AND cluster_id IS NOT NULL", [], |r| r.get(0))?;
        let topics: usize = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_item = 0 AND is_universe = 0 AND depth > 0", [], |r| r.get(0))?;
        Ok((total_nodes, total_items, processed, with_embeddings, unprocessed, unclustered, orphan_items, topics))
    }

    // ==================== Hierarchy Flattening Operations ====================

    /// Flatten empty passthrough levels in the hierarchy
    /// Finds nodes that are single-child intermediate nodes (like "Uncategorized and Related")
    /// and reparents their children directly to grandparent, then deletes the empty node
    /// Returns the number of nodes flattened
    pub fn flatten_empty_levels(&self) -> Result<usize> {
        let mut flattened_count = 0;

        // Find passthrough nodes: non-item, non-universe nodes that have exactly 1 child
        // or nodes with "Uncategorized" in the title that are intermediate pass-throughs
        // Process deepest first to avoid orphaning nodes
        let passthrough_nodes: Vec<(String, Option<String>)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT n.id, n.parent_id
                 FROM nodes n
                 WHERE n.is_item = 0
                   AND n.is_universe = 0
                   AND (
                     n.child_count = 1
                     OR n.title LIKE '%Uncategorized%'
                     OR n.cluster_label LIKE '%Uncategorized%'
                   )
                 ORDER BY n.depth DESC"
            )?;

            let results: Vec<(String, Option<String>)> = stmt.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?.filter_map(|r| r.ok()).collect();
            results
        };

        println!("[Flatten] Found {} potential passthrough nodes", passthrough_nodes.len());

        for (node_id, grandparent_id) in passthrough_nodes {
            let conn = self.conn.lock().unwrap();

            // Get the children of this node
            let children: Vec<String> = {
                let mut stmt = conn.prepare(
                    "SELECT id FROM nodes WHERE parent_id = ?1"
                )?;
                let results: Vec<String> = stmt.query_map(params![&node_id], |row| row.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                results
            };

            // Skip if no children (would make no difference)
            if children.is_empty() {
                continue;
            }

            // Skip if node has more than 1 child and doesn't have "Uncategorized" in name
            // (we only auto-flatten single-child nodes or explicitly named uncategorized ones)
            if children.len() > 1 {
                let has_uncategorized: bool = conn.query_row(
                    "SELECT (title LIKE '%Uncategorized%' OR cluster_label LIKE '%Uncategorized%') FROM nodes WHERE id = ?1",
                    params![&node_id],
                    |row| row.get(0),
                ).unwrap_or(false);

                if !has_uncategorized {
                    continue;
                }
            }

            println!("[Flatten] Flattening node '{}' with {} children", node_id, children.len());

            // Reparent all children to grandparent
            for child_id in &children {
                conn.execute(
                    "UPDATE nodes SET parent_id = ?2 WHERE id = ?1",
                    params![child_id, &grandparent_id],
                )?;
            }

            // Delete the passthrough node
            conn.execute("DELETE FROM nodes WHERE id = ?1", params![&node_id])?;

            // Update grandparent's child count if it exists
            if let Some(ref gp_id) = grandparent_id {
                let new_count: i32 = conn.query_row(
                    "SELECT COUNT(*) FROM nodes WHERE parent_id = ?1",
                    params![gp_id],
                    |row| row.get(0),
                ).unwrap_or(0);

                conn.execute(
                    "UPDATE nodes SET child_count = ?2 WHERE id = ?1",
                    params![gp_id, new_count],
                )?;
            }

            flattened_count += 1;
        }

        // Recalculate depths for all affected nodes
        if flattened_count > 0 {
            self.update_all_depths()?;
        }

        println!("[Flatten] Flattened {} passthrough nodes", flattened_count);
        Ok(flattened_count)
    }

    /// Recalculate depth for all nodes based on parent relationships
    fn update_all_depths(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Set universe to depth 0
        conn.execute(
            "UPDATE nodes SET depth = 0 WHERE is_universe = 1",
            [],
        )?;

        // Use recursive update starting from universe
        conn.execute_batch(
            "WITH RECURSIVE depth_calc(id, depth) AS (
                SELECT id, 0 FROM nodes WHERE is_universe = 1
                UNION ALL
                SELECT n.id, d.depth + 1
                FROM nodes n
                JOIN depth_calc d ON n.parent_id = d.id
            )
            UPDATE nodes SET depth = (
                SELECT depth FROM depth_calc WHERE depth_calc.id = nodes.id
            )
            WHERE id IN (SELECT id FROM depth_calc)"
        )?;

        Ok(())
    }

    // ==================== Tidy Database Operations ====================

    /// Merge children that have the same name as their parent
    /// Example: "Consciousness" → "Consciousness" (7 children) becomes "Consciousness" (7 children)
    /// Skips protected nodes (Recent Notes)
    pub fn merge_same_name_children(&self) -> Result<usize> {
        let protected_ids = self.get_protected_node_ids();
        let conn = self.conn.lock().unwrap();

        // Find parent-child pairs where labels match (case-insensitive)
        // Use COALESCE to check cluster_label, ai_title, title
        let duplicates: Vec<(String, String)> = {
            let mut stmt = conn.prepare(
                "SELECT p.id as parent_id, c.id as child_id
                 FROM nodes p
                 JOIN nodes c ON c.parent_id = p.id
                 WHERE c.is_item = 0
                   AND c.is_universe = 0
                   AND LOWER(COALESCE(c.cluster_label, c.ai_title, c.title)) =
                       LOWER(COALESCE(p.cluster_label, p.ai_title, p.title))"
            )?;
            let results: Vec<_> = stmt.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?.filter_map(|r| r.ok()).collect();
            results
        };

        // Filter out protected nodes
        let duplicates: Vec<(String, String)> = duplicates
            .into_iter()
            .filter(|(parent_id, child_id)| !protected_ids.contains(parent_id) && !protected_ids.contains(child_id))
            .collect();

        let count = duplicates.len();

        for (parent_id, child_id) in &duplicates {
            // Reparent all grandchildren to parent
            conn.execute(
                "UPDATE nodes SET parent_id = ?1 WHERE parent_id = ?2",
                params![parent_id, child_id],
            )?;

            // Delete the redundant child
            conn.execute("DELETE FROM nodes WHERE id = ?1", params![child_id])?;
        }

        // Fix child counts for affected parents
        if count > 0 {
            for (parent_id, _) in &duplicates {
                let new_count: i32 = conn.query_row(
                    "SELECT COUNT(*) FROM nodes WHERE parent_id = ?1",
                    params![parent_id],
                    |row| row.get(0),
                ).unwrap_or(0);
                conn.execute(
                    "UPDATE nodes SET child_count = ?2 WHERE id = ?1",
                    params![parent_id, new_count],
                )?;
            }
        }

        println!("[Tidy] Merged {} same-name children", count);
        Ok(count)
    }

    /// Flatten single-child chains: reparent child to grandparent, delete middle node
    /// Loops until no single-child chains remain. Returns total nodes removed.
    /// Optimized with batching and transactions for performance.
    /// Skips protected nodes (Recent Notes)
    pub fn flatten_single_child_chains(&self) -> Result<usize> {
        let protected_ids = self.get_protected_node_ids();
        let mut total_flattened = 0;

        loop {
            let conn = self.conn.lock().unwrap();

            // Find ALL single-child chains in one query
            // Returns: middle_node_id, grandparent_id, child_id
            let chains: Vec<(String, Option<String>, String)> = {
                let mut stmt = conn.prepare(
                    "SELECT parent.id, parent.parent_id, child.id
                     FROM nodes parent
                     JOIN nodes child ON child.parent_id = parent.id
                     WHERE parent.is_item = 0
                       AND parent.is_universe = 0
                       AND parent.child_count = 1"
                )?;
                let results: Vec<_> = stmt.query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?.filter_map(|r| r.ok()).collect();
                results
            };

            // Filter out protected nodes
            let chains: Vec<(String, Option<String>, String)> = chains
                .into_iter()
                .filter(|(middle_id, _, child_id)| !protected_ids.contains(middle_id) && !protected_ids.contains(child_id))
                .collect();

            if chains.is_empty() {
                break;
            }

            // Batch reparent: move each child to its grandparent
            for (_middle_id, grandparent_id, child_id) in &chains {
                conn.execute(
                    "UPDATE nodes SET parent_id = ?2 WHERE id = ?1",
                    params![child_id, grandparent_id],
                )?;
            }

            // Batch delete all middle nodes
            for (middle_id, _, _) in &chains {
                conn.execute("DELETE FROM nodes WHERE id = ?1", params![middle_id])?;
            }

            total_flattened += chains.len();
        }

        // Fix depths after all flattening
        if total_flattened > 0 {
            drop(self.conn.lock()); // Release lock before calling update_all_depths
            self.update_all_depths()?;
            // Child counts will be fixed by fix_all_child_counts() later
        }

        println!("[Tidy] Flattened {} single-child chains", total_flattened);
        Ok(total_flattened)
    }

    /// Remove empty category nodes (non-item, non-universe with 0 children)
    /// Skips protected nodes (Recent Notes)
    pub fn remove_empty_categories(&self) -> Result<usize> {
        let protected_ids = self.get_protected_node_ids();
        let conn = self.conn.lock().unwrap();

        if protected_ids.is_empty() {
            let count = conn.execute(
                "DELETE FROM nodes WHERE is_item = 0 AND is_universe = 0 AND child_count = 0",
                [],
            )?;
            println!("[Tidy] Removed {} empty categories", count);
            Ok(count)
        } else {
            // Build exclusion list
            let placeholders: Vec<String> = protected_ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect();
            let sql = format!(
                "DELETE FROM nodes WHERE is_item = 0 AND is_universe = 0 AND child_count = 0 AND id NOT IN ({})",
                placeholders.join(", ")
            );
            let params: Vec<&str> = protected_ids.iter().map(|s| s.as_str()).collect();
            let count = conn.execute(&sql, rusqlite::params_from_iter(params))?;
            println!("[Tidy] Removed {} empty categories (protected {} nodes)", count, protected_ids.len());
            Ok(count)
        }
    }

    /// Fix all child_count fields by actually counting VISIBLE tier children
    pub fn fix_all_child_counts(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();

        // Find all mismatches
        // Count only VISIBLE tier children (insight, idea, exploration, synthesis, question, planning, paper, code_*)
        let mismatches: Vec<(String, i32, i32)> = {
            let mut stmt = conn.prepare(
                "SELECT n.id, n.child_count,
                        (SELECT COUNT(*) FROM nodes c
                         WHERE c.parent_id = n.id
                           AND (c.content_type IS NULL OR c.content_type IN ('insight', 'idea', 'exploration', 'synthesis', 'question', 'planning', 'paper', 'bookmark') OR c.content_type LIKE 'code_%')) as actual
                 FROM nodes n
                 WHERE n.child_count != (SELECT COUNT(*) FROM nodes c
                                         WHERE c.parent_id = n.id
                                           AND (c.content_type IS NULL OR c.content_type IN ('insight', 'idea', 'exploration', 'synthesis', 'question', 'planning', 'paper', 'bookmark') OR c.content_type LIKE 'code_%'))"
            )?;
            let results: Vec<_> = stmt.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?.filter_map(|r| r.ok()).collect();
            results
        };

        let count = mismatches.len();

        for (node_id, _old, actual) in &mismatches {
            conn.execute(
                "UPDATE nodes SET child_count = ?2 WHERE id = ?1",
                params![node_id, actual],
            )?;
        }

        println!("[Tidy] Fixed {} child counts (excludes supporting items)", count);
        Ok(count)
    }

    /// Fix all depths via BFS from Universe. Returns count of nodes with wrong depth.
    pub fn fix_all_depths(&self) -> Result<usize> {
        // First, count how many have wrong depth
        let wrong_count: usize = {
            let conn = self.conn.lock().unwrap();

            // Build correct depths via CTE and count mismatches
            let count: i32 = conn.query_row(
                "WITH RECURSIVE depth_calc(id, correct_depth) AS (
                    SELECT id, 0 FROM nodes WHERE is_universe = 1
                    UNION ALL
                    SELECT n.id, d.correct_depth + 1
                    FROM nodes n
                    JOIN depth_calc d ON n.parent_id = d.id
                )
                SELECT COUNT(*) FROM nodes n
                JOIN depth_calc d ON n.id = d.id
                WHERE n.depth != d.correct_depth",
                [],
                |row| row.get(0),
            ).unwrap_or(0);
            count as usize
        };

        if wrong_count > 0 {
            self.update_all_depths()?;
        }

        println!("[Tidy] Fixed {} node depths", wrong_count);
        Ok(wrong_count)
    }

    /// Reparent orphan nodes (parent_id points to non-existent node) to Universe
    /// Skips protected nodes (Recent Notes descendants)
    pub fn reparent_orphans(&self) -> Result<usize> {
        let protected_ids = self.get_protected_node_ids();
        let conn = self.conn.lock().unwrap();

        // Find universe id
        let universe_id: Option<String> = conn.query_row(
            "SELECT id FROM nodes WHERE is_universe = 1",
            [],
            |row| row.get(0),
        ).ok();

        let universe_id = match universe_id {
            Some(id) => id,
            None => {
                println!("[Tidy] No universe node found, skipping orphan reparenting");
                return Ok(0);
            }
        };

        // Find and reparent orphans (excluding protected)
        if protected_ids.is_empty() {
            let count = conn.execute(
                "UPDATE nodes
                 SET parent_id = ?1
                 WHERE parent_id IS NOT NULL
                   AND parent_id NOT IN (SELECT id FROM nodes)
                   AND is_universe = 0",
                params![&universe_id],
            )?;
            println!("[Tidy] Reparented {} orphan nodes to Universe", count);
            Ok(count)
        } else {
            let placeholders: Vec<String> = protected_ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 2)).collect();
            let sql = format!(
                "UPDATE nodes
                 SET parent_id = ?1
                 WHERE parent_id IS NOT NULL
                   AND parent_id NOT IN (SELECT id FROM nodes)
                   AND is_universe = 0
                   AND id NOT IN ({})",
                placeholders.join(", ")
            );
            let mut params: Vec<&str> = vec![&universe_id];
            params.extend(protected_ids.iter().map(|s| s.as_str()));
            let count = conn.execute(&sql, rusqlite::params_from_iter(params))?;
            println!("[Tidy] Reparented {} orphan nodes to Universe (protected {} nodes)", count, protected_ids.len());
            Ok(count)
        }
    }

    /// Delete edges where source or target node doesn't exist
    pub fn prune_dead_edges(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "DELETE FROM edges
             WHERE source_id NOT IN (SELECT id FROM nodes)
                OR target_id NOT IN (SELECT id FROM nodes)",
            [],
        )?;
        println!("[Tidy] Pruned {} dead edges", count);
        Ok(count)
    }

    /// Remove duplicate edges (same source, target, type), keep oldest
    pub fn deduplicate_edges(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();

        // Find and delete duplicates, keeping the one with minimum rowid
        let count = conn.execute(
            "DELETE FROM edges
             WHERE rowid NOT IN (
                 SELECT MIN(rowid)
                 FROM edges
                 GROUP BY source_id, target_id, type
             )",
            [],
        )?;

        println!("[Tidy] Removed {} duplicate edges", count);
        Ok(count)
    }

    /// Increment depth of a node and all its descendants by 1
    /// Uses recursive CTE to do it in a single query (avoids lock issues)
    pub fn increment_subtree_depth(&self, node_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Use recursive CTE to find all descendants and increment their depth
        conn.execute(
            "WITH RECURSIVE subtree(id) AS (
                SELECT ?1
                UNION ALL
                SELECT n.id FROM nodes n JOIN subtree s ON n.parent_id = s.id
            )
            UPDATE nodes SET depth = depth + 1 WHERE id IN (SELECT id FROM subtree)",
            params![node_id],
        )?;

        Ok(())
    }

    /// Decrement depth of a subtree by 1 (used when moving nodes up a level)
    pub fn decrement_subtree_depth(&self, node_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Use recursive CTE to find all descendants and decrement their depth
        conn.execute(
            "WITH RECURSIVE subtree(id) AS (
                SELECT ?1
                UNION ALL
                SELECT n.id FROM nodes n JOIN subtree s ON n.parent_id = s.id
            )
            UPDATE nodes SET depth = depth - 1 WHERE id IN (SELECT id FROM subtree)",
            params![node_id],
        )?;

        Ok(())
    }

    /// Increment depth of multiple subtrees by 1
    /// Uses level-by-level iteration instead of recursive CTE (much faster with many roots)
    pub fn increment_multiple_subtrees_depth(&self, root_ids: &[String]) -> Result<()> {
        if root_ids.is_empty() {
            return Ok(());
        }

        let conn = self.conn.lock().unwrap();

        // Level-by-level: update roots, then their children, then grandchildren, etc.
        // Each level uses indexed parent_id lookups instead of recursive CTE traversal
        let mut current_ids = root_ids.to_vec();
        let mut total_updated = 0;
        let mut level = 0;
        let start = std::time::Instant::now();

        const MAX_LEVELS: usize = 20;
        while !current_ids.is_empty() && level < MAX_LEVELS {
            let level_start = std::time::Instant::now();
            let level_count = current_ids.len();

            // Batch updates to avoid SQLite variable limits (max ~999 params)
            for batch in current_ids.chunks(500) {
                let placeholders: String = (1..=batch.len())
                    .map(|i| format!("?{}", i))
                    .collect::<Vec<_>>()
                    .join(", ");

                let update_sql = format!(
                    "UPDATE nodes SET depth = depth + 1 WHERE id IN ({})",
                    placeholders
                );

                let params: Vec<&dyn rusqlite::ToSql> = batch.iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();

                total_updated += conn.execute(&update_sql, params.as_slice())?;
            }

            // Get children IDs for next level (also batched)
            let mut next_level = Vec::new();
            for batch in current_ids.chunks(500) {
                let placeholders: String = (1..=batch.len())
                    .map(|i| format!("?{}", i))
                    .collect::<Vec<_>>()
                    .join(", ");

                let select_sql = format!(
                    "SELECT id FROM nodes WHERE parent_id IN ({})",
                    placeholders
                );

                let params: Vec<&dyn rusqlite::ToSql> = batch.iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();

                let mut stmt = conn.prepare(&select_sql)?;
                let children: Vec<String> = stmt.query_map(params.as_slice(), |row| row.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                next_level.extend(children);
            }

            // Log progress for this level
            let level_elapsed = level_start.elapsed().as_millis();
            println!("[DepthUpdate] Level {}: {} nodes updated, {} children found ({} ms)",
                level, level_count, next_level.len(), level_elapsed);

            current_ids = next_level;
            level += 1;
        }

        if level >= MAX_LEVELS {
            println!("[DepthUpdate] WARNING: Hit max level limit ({}) - possible cycle in parent_id references!", MAX_LEVELS);
        }

        let total_elapsed = start.elapsed().as_millis();
        println!("[DepthUpdate] Complete: {} levels, {} total nodes in {} ms",
            level, total_updated, total_elapsed);

        Ok(())
    }

    /// Set depth for reparented nodes to correct values (parent.depth + 1)
    /// Unlike increment, this SETS the correct depth regardless of current value.
    /// For items (leaf nodes), just sets depth. For categories, cascades to descendants.
    pub fn set_reparented_nodes_depth(&self, node_ids: &[String], new_depth: i32) -> Result<()> {
        if node_ids.is_empty() {
            return Ok(());
        }

        let conn = self.conn.lock().unwrap();
        let start = std::time::Instant::now();

        // First, set depth for the root nodes
        for batch in node_ids.chunks(500) {
            let placeholders: String = (1..=batch.len())
                .map(|i| format!("?{}", i))
                .collect::<Vec<_>>()
                .join(", ");

            let update_sql = format!(
                "UPDATE nodes SET depth = {} WHERE id IN ({})",
                new_depth, placeholders
            );

            let params: Vec<&dyn rusqlite::ToSql> = batch.iter()
                .map(|s| s as &dyn rusqlite::ToSql)
                .collect();

            conn.execute(&update_sql, params.as_slice())?;
        }

        // Now cascade to descendants level by level
        let mut current_ids = node_ids.to_vec();
        let mut current_depth = new_depth;
        let mut level = 0;
        const MAX_LEVELS: usize = 20;

        while !current_ids.is_empty() && level < MAX_LEVELS {
            // Get children IDs
            let mut next_level = Vec::new();
            for batch in current_ids.chunks(500) {
                let placeholders: String = (1..=batch.len())
                    .map(|i| format!("?{}", i))
                    .collect::<Vec<_>>()
                    .join(", ");

                let select_sql = format!(
                    "SELECT id FROM nodes WHERE parent_id IN ({})",
                    placeholders
                );

                let params: Vec<&dyn rusqlite::ToSql> = batch.iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();

                let mut stmt = conn.prepare(&select_sql)?;
                let children: Vec<String> = stmt.query_map(params.as_slice(), |row| row.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                next_level.extend(children);
            }

            if next_level.is_empty() {
                break;
            }

            // Set depth for children = current_depth + 1
            current_depth += 1;
            for batch in next_level.chunks(500) {
                let placeholders: String = (1..=batch.len())
                    .map(|i| format!("?{}", i))
                    .collect::<Vec<_>>()
                    .join(", ");

                let update_sql = format!(
                    "UPDATE nodes SET depth = {} WHERE id IN ({})",
                    current_depth, placeholders
                );

                let params: Vec<&dyn rusqlite::ToSql> = batch.iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();

                conn.execute(&update_sql, params.as_slice())?;
            }

            current_ids = next_level;
            level += 1;
        }

        let total_elapsed = start.elapsed().as_millis();
        println!("[SetDepth] Set {} nodes to depth {}, cascaded {} levels in {} ms",
            node_ids.len(), new_depth, level, total_elapsed);

        Ok(())
    }

    // ========== Tag Operations ==========

    /// Count total tags
    pub fn count_tags(&self) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM tags", [], |row| row.get(0))
    }

    /// Insert a new tag
    pub fn insert_tag(&self, tag: &Tag) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tags (id, title, parent_tag_id, depth, item_count, pinned, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                tag.id,
                tag.title,
                tag.parent_tag_id,
                tag.depth,
                tag.item_count,
                tag.pinned as i32,
                tag.created_at,
                tag.updated_at,
            ],
        )?;
        Ok(())
    }

    /// Get a tag by ID
    pub fn get_tag(&self, id: &str) -> Result<Option<Tag>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, parent_tag_id, depth, item_count, pinned, created_at, updated_at
             FROM tags WHERE id = ?1"
        )?;
        let tag = stmt.query_row(params![id], |row| {
            Ok(Tag {
                id: row.get(0)?,
                title: row.get(1)?,
                parent_tag_id: row.get(2)?,
                depth: row.get(3)?,
                item_count: row.get(4)?,
                pinned: row.get::<_, i32>(5)? != 0,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        }).optional()?;
        Ok(tag)
    }

    /// Get all tags
    pub fn get_all_tags(&self) -> Result<Vec<Tag>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, parent_tag_id, depth, item_count, pinned, created_at, updated_at
             FROM tags ORDER BY depth, item_count DESC"
        )?;
        let tags = stmt.query_map([], |row| {
            Ok(Tag {
                id: row.get(0)?,
                title: row.get(1)?,
                parent_tag_id: row.get(2)?,
                depth: row.get(3)?,
                item_count: row.get(4)?,
                pinned: row.get::<_, i32>(5)? != 0,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?.filter_map(|r| r.ok()).collect();
        Ok(tags)
    }

    /// Get tags by depth range (e.g., 0..=1 for L0 and L1 tags)
    pub fn get_tags_by_depth(&self, min_depth: i32, max_depth: i32) -> Result<Vec<Tag>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, parent_tag_id, depth, item_count, pinned, created_at, updated_at
             FROM tags WHERE depth >= ?1 AND depth <= ?2
             ORDER BY item_count DESC"
        )?;
        let tags = stmt.query_map(params![min_depth, max_depth], |row| {
            Ok(Tag {
                id: row.get(0)?,
                title: row.get(1)?,
                parent_tag_id: row.get(2)?,
                depth: row.get(3)?,
                item_count: row.get(4)?,
                pinned: row.get::<_, i32>(5)? != 0,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?.filter_map(|r| r.ok()).collect();
        Ok(tags)
    }

    /// Update tag centroid (embedding)
    pub fn update_tag_centroid(&self, tag_id: &str, centroid: &[f32]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let bytes: Vec<u8> = centroid.iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        conn.execute(
            "UPDATE tags SET centroid = ?2, updated_at = ?3 WHERE id = ?1",
            params![tag_id, bytes, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }

    /// Get tag centroid (embedding)
    pub fn get_tag_centroid(&self, tag_id: &str) -> Result<Option<Vec<f32>>> {
        let conn = self.conn.lock().unwrap();
        let bytes: Option<Vec<u8>> = conn.query_row(
            "SELECT centroid FROM tags WHERE id = ?1",
            params![tag_id],
            |row| row.get(0),
        ).optional()?.flatten();
        Ok(bytes.map(|b| bytes_to_embedding(&b)))
    }

    /// Update tag item count
    pub fn update_tag_item_count(&self, tag_id: &str) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM item_tags WHERE tag_id = ?1",
            params![tag_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "UPDATE tags SET item_count = ?2, updated_at = ?3 WHERE id = ?1",
            params![tag_id, count, chrono::Utc::now().timestamp()],
        )?;
        Ok(count)
    }

    /// Insert an item-tag assignment
    pub fn insert_item_tag(&self, item_id: &str, tag_id: &str, confidence: f64, source: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO item_tags (item_id, tag_id, confidence, source)
             VALUES (?1, ?2, ?3, ?4)",
            params![item_id, tag_id, confidence, source],
        )?;
        Ok(())
    }

    /// Insert an item-tag assignment only if it doesn't already exist
    /// Returns true if inserted, false if already exists
    pub fn insert_item_tag_if_not_exists(&self, item_id: &str, tag_id: &str, confidence: f64, source: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "INSERT OR IGNORE INTO item_tags (item_id, tag_id, confidence, source)
             VALUES (?1, ?2, ?3, ?4)",
            params![item_id, tag_id, confidence, source],
        )?;
        Ok(rows > 0)
    }

    /// Get all tags for an item
    pub fn get_item_tags(&self, item_id: &str) -> Result<Vec<(Tag, f64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT t.id, t.title, t.parent_tag_id, t.depth, t.item_count, t.pinned,
                    t.created_at, t.updated_at, it.confidence
             FROM item_tags it
             JOIN tags t ON it.tag_id = t.id
             WHERE it.item_id = ?1
             ORDER BY it.confidence DESC"
        )?;
        let tags = stmt.query_map(params![item_id], |row| {
            Ok((
                Tag {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    parent_tag_id: row.get(2)?,
                    depth: row.get(3)?,
                    item_count: row.get(4)?,
                    pinned: row.get::<_, i32>(5)? != 0,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                },
                row.get::<_, f64>(8)?,
            ))
        })?.filter_map(|r| r.ok()).collect();
        Ok(tags)
    }

    /// Get shared tag IDs between two items (for similarity bonus)
    pub fn get_shared_tag_ids(&self, item_a_id: &str, item_b_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT a.tag_id FROM item_tags a
             INNER JOIN item_tags b ON a.tag_id = b.tag_id
             WHERE a.item_id = ?1 AND b.item_id = ?2"
        )?;
        let tags = stmt.query_map(params![item_a_id, item_b_id], |row| {
            row.get::<_, String>(0)
        })?.filter_map(|r| r.ok()).collect();
        Ok(tags)
    }

    /// Count shared tags between two items (faster than get_shared_tag_ids)
    pub fn count_shared_tags(&self, item_a_id: &str, item_b_id: &str) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM item_tags a
             INNER JOIN item_tags b ON a.tag_id = b.tag_id
             WHERE a.item_id = ?1 AND b.item_id = ?2",
            params![item_a_id, item_b_id],
            |row| row.get(0),
        )
    }

    /// Get all items for a tag
    pub fn get_tag_items(&self, tag_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT item_id FROM item_tags WHERE tag_id = ?1 ORDER BY confidence DESC"
        )?;
        let items = stmt.query_map(params![tag_id], |row| {
            row.get::<_, String>(0)
        })?.filter_map(|r| r.ok()).collect();
        Ok(items)
    }

    /// Get all item IDs that have any of the given tag titles (case-insensitive)
    /// Used for tag-based export filtering
    pub fn get_items_with_any_tags(&self, tag_titles: &[String]) -> Result<std::collections::HashSet<String>> {
        if tag_titles.is_empty() {
            return Ok(std::collections::HashSet::new());
        }

        let conn = self.conn.lock().unwrap();
        // Build query with placeholders for each tag title
        let placeholders: Vec<String> = tag_titles.iter().enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        let query = format!(
            "SELECT DISTINCT it.item_id
             FROM item_tags it
             JOIN tags t ON it.tag_id = t.id
             WHERE LOWER(t.title) IN ({})",
            placeholders.join(", ")
        );

        let mut stmt = conn.prepare(&query)?;
        let params: Vec<String> = tag_titles.iter().map(|t| t.to_lowercase()).collect();
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();

        let items = stmt.query_map(param_refs.as_slice(), |row| {
            row.get::<_, String>(0)
        })?.filter_map(|r| r.ok()).collect();
        Ok(items)
    }

    /// Delete all tags (for reset/rebuild)
    pub fn delete_all_tags(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        // item_tags will cascade delete due to foreign key
        let count = conn.execute("DELETE FROM tags", [])?;
        Ok(count)
    }

    /// Get cluster statistics for bootstrap (cluster_id, cluster_label, item_count)
    pub fn get_cluster_statistics(&self) -> Result<Vec<(i32, String, i32)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT cluster_id, cluster_label, COUNT(*) as item_count
             FROM nodes
             WHERE is_item = 1 AND cluster_id IS NOT NULL AND cluster_label IS NOT NULL
             GROUP BY cluster_id, cluster_label
             ORDER BY item_count DESC"
        )?;
        let stats = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i32>(2)?,
            ))
        })?.filter_map(|r| r.ok()).collect();
        Ok(stats)
    }

    /// Get items by cluster_id (for computing centroids)
    pub fn get_items_by_cluster(&self, cluster_id: i32) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let query = format!(
            "SELECT {} FROM nodes WHERE is_item = 1 AND cluster_id = ?1",
            Self::NODE_COLUMNS
        );
        let mut stmt = conn.prepare(&query)?;
        let nodes = stmt.query_map(params![cluster_id], Self::row_to_node)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(nodes)
    }

    /// Get all item-tag associations as a HashMap for efficient lookup during clustering
    /// Returns HashMap<item_id, Vec<tag_id>>
    pub fn get_all_item_tags_map(&self) -> Result<std::collections::HashMap<String, Vec<String>>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT item_id, tag_id FROM item_tags")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for row in rows.filter_map(|r| r.ok()) {
            map.entry(row.0).or_default().push(row.1);
        }
        Ok(map)
    }

    // ==========================================================================
    // Database Metadata / State Tracking
    // ==========================================================================

    /// Get a metadata value by key
    pub fn get_metadata(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let result: std::result::Result<String, _> = conn.query_row(
            "SELECT value FROM db_metadata WHERE key = ?1",
            params![key],
            |row| row.get(0),
        );
        match result {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Set a metadata value (insert or update)
    pub fn set_metadata(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        conn.execute(
            "INSERT INTO db_metadata (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
            params![key, value, now],
        )?;
        Ok(())
    }

    /// Get the current pipeline state
    /// Returns: fresh, imported, processed, clustered, hierarchized, complete
    pub fn get_pipeline_state(&self) -> String {
        self.get_metadata("pipeline_state")
            .ok()
            .flatten()
            .unwrap_or_else(|| "fresh".to_string())
    }

    /// Set the pipeline state
    pub fn set_pipeline_state(&self, state: &str) -> Result<()> {
        self.set_metadata("pipeline_state", state)
    }

    /// Get all metadata as key-value pairs
    pub fn get_all_metadata(&self) -> Result<Vec<(String, String, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT key, value, updated_at FROM db_metadata ORDER BY key")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    // ==================== Paper Methods ====================

    /// Insert a new paper record
    pub fn insert_paper(
        &self,
        node_id: &str,
        openaire_id: Option<&str>,
        doi: Option<&str>,
        authors: Option<&str>,
        publication_date: Option<&str>,
        journal: Option<&str>,
        publisher: Option<&str>,
        abstract_text: Option<&str>,
        abstract_formatted: Option<&str>,
        abstract_sections: Option<&str>,
        pdf_url: Option<&str>,
        subjects: Option<&str>,
        access_right: Option<&str>,
        content_hash: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        conn.execute(
            "INSERT INTO papers (node_id, openaire_id, doi, authors, publication_date, journal, publisher, abstract, abstract_formatted, abstract_sections, pdf_url, subjects, access_right, content_hash, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![node_id, openaire_id, doi, authors, publication_date, journal, publisher, abstract_text, abstract_formatted, abstract_sections, pdf_url, subjects, access_right, content_hash, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Get paper metadata by node ID
    pub fn get_paper_by_node_id(&self, node_id: &str) -> Result<Option<Paper>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, node_id, openaire_id, doi, authors, publication_date, journal, publisher, abstract, abstract_formatted, abstract_sections, pdf_url, pdf_available, doc_format, subjects, access_right, created_at
             FROM papers WHERE node_id = ?1",
            params![node_id],
            |row| {
                Ok(Paper {
                    id: row.get(0)?,
                    node_id: row.get(1)?,
                    openaire_id: row.get(2)?,
                    doi: row.get(3)?,
                    authors: row.get(4)?,
                    publication_date: row.get(5)?,
                    journal: row.get(6)?,
                    publisher: row.get(7)?,
                    abstract_text: row.get(8)?,
                    abstract_formatted: row.get(9)?,
                    abstract_sections: row.get(10)?,
                    pdf_url: row.get(11)?,
                    pdf_available: row.get::<_, i32>(12)? != 0,
                    doc_format: row.get(13)?,
                    subjects: row.get(14)?,
                    access_right: row.get(15)?,
                    created_at: row.get(16)?,
                })
            },
        ).optional()
    }

    /// Check if a paper with this OpenAIRE ID already exists
    pub fn paper_exists_by_openaire_id(&self, openaire_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM papers WHERE openaire_id = ?1",
            params![openaire_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get all OpenAIRE IDs for batch duplicate checking (O(1) lookup)
    pub fn get_all_openaire_ids(&self) -> Result<std::collections::HashSet<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT openaire_id FROM papers WHERE openaire_id IS NOT NULL")?;
        let ids = stmt.query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    /// Get all DOIs for batch duplicate checking (O(1) lookup)
    pub fn get_all_paper_dois(&self) -> Result<std::collections::HashSet<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT LOWER(doi) FROM papers WHERE doi IS NOT NULL AND doi != ''")?;
        let ids = stmt.query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    /// Get all content hashes for batch duplicate checking (O(1) lookup)
    pub fn get_all_content_hashes(&self) -> Result<std::collections::HashSet<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT content_hash FROM papers WHERE content_hash IS NOT NULL")?;
        let ids = stmt.query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    /// Update content_hash for a paper (for backfilling existing papers)
    pub fn update_paper_content_hash(&self, node_id: &str, content_hash: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE papers SET content_hash = ?2 WHERE node_id = ?1",
            params![node_id, content_hash],
        )?;
        Ok(())
    }

    /// Get papers that need content_hash backfilled
    /// Returns (node_id, title, abstract_text) tuples
    pub fn get_papers_needing_content_hash(&self) -> Result<Vec<(String, String, Option<String>)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT p.node_id, n.title, p.abstract FROM papers p
             JOIN nodes n ON p.node_id = n.id
             WHERE p.content_hash IS NULL"
        )?;
        let papers = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?))
        })?
        .filter_map(|r| r.ok())
        .collect();
        Ok(papers)
    }

    /// Count papers matching OpenAIRE IDs (for showing "X already imported")
    pub fn count_papers_by_openaire_ids(&self, openaire_ids: &[String]) -> Result<i32> {
        if openaire_ids.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().unwrap();
        // Use IN clause with placeholders
        let placeholders: Vec<_> = (0..openaire_ids.len()).map(|i| format!("?{}", i + 1)).collect();
        let sql = format!(
            "SELECT COUNT(*) FROM papers WHERE openaire_id IN ({})",
            placeholders.join(", ")
        );
        let params: Vec<&dyn rusqlite::ToSql> = openaire_ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let count: i32 = conn.query_row(&sql, params.as_slice(), |row| row.get(0))?;
        Ok(count)
    }

    /// Get PDF blob for a paper
    pub fn get_paper_pdf(&self, node_id: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT pdf_blob FROM papers WHERE node_id = ?1 AND pdf_available = 1",
            params![node_id],
            |row| row.get(0),
        ).optional()
    }

    /// Store PDF blob for a paper
    pub fn update_paper_pdf(&self, node_id: &str, pdf_blob: &[u8]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        // Update papers table with doc_format = 'pdf'
        conn.execute(
            "UPDATE papers SET pdf_blob = ?1, pdf_available = 1, doc_format = 'pdf' WHERE node_id = ?2",
            params![pdf_blob, node_id],
        )?;
        // Also sync to nodes table (denormalized for graph display)
        conn.execute(
            "UPDATE nodes SET pdf_available = 1 WHERE id = ?1",
            params![node_id],
        )?;
        Ok(())
    }

    /// Get document blob and format for a paper (supports PDF, DOCX, DOC)
    /// Returns None if blob is not available (including NULL blob with pdf_available=1)
    pub fn get_paper_document(&self, node_id: &str) -> Result<Option<(Vec<u8>, String)>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT pdf_blob, doc_format FROM papers WHERE node_id = ?1 AND pdf_available = 1 AND pdf_blob IS NOT NULL",
            params![node_id],
            |row| {
                let blob: Vec<u8> = row.get(0)?;
                let format: String = row.get::<_, Option<String>>(1)?.unwrap_or_else(|| "pdf".to_string());
                Ok((blob, format))
            },
        ).optional()
    }

    /// Store document blob with format for a paper
    pub fn update_paper_document(&self, node_id: &str, doc_blob: &[u8], format: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        // Update papers table
        conn.execute(
            "UPDATE papers SET pdf_blob = ?1, pdf_available = 1, doc_format = ?2 WHERE node_id = ?3",
            params![doc_blob, format, node_id],
        )?;
        // Also sync to nodes table (denormalized for graph display)
        conn.execute(
            "UPDATE nodes SET pdf_available = 1 WHERE id = ?1",
            params![node_id],
        )?;
        Ok(())
    }

    /// Update the pdf_source field for a paper
    pub fn update_paper_pdf_source(&self, node_id: &str, source: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE papers SET pdf_source = ?1 WHERE node_id = ?2",
            params![source, node_id],
        )?;
        Ok(())
    }

    /// Check if a paper has a PDF available
    pub fn has_paper_pdf(&self, node_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let available: Option<i32> = conn.query_row(
            "SELECT pdf_available FROM papers WHERE node_id = ?1",
            params![node_id],
            |row| row.get(0),
        ).optional()?;
        Ok(available == Some(1))
    }

    /// Sync pdf_available from papers table to nodes table
    pub fn sync_paper_pdf_status(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();

        let count = conn.execute(
            "UPDATE nodes SET pdf_available = 1
             WHERE content_type = 'paper'
               AND id IN (SELECT node_id FROM papers WHERE pdf_available = 1)",
            [],
        )?;

        Ok(count)
    }

    /// Sync paper dates from papers.publication_date to nodes.created_at
    /// Re-parses publication_date strings and updates nodes
    /// Papers with missing/unparseable dates get 0 (unknown)
    /// Returns (updated_count, unknown_count)
    pub fn sync_paper_dates(&self) -> Result<(usize, usize)> {
        let conn = self.conn.lock().unwrap();

        // Get ALL papers (including those with NULL publication_date)
        let mut stmt = conn.prepare(
            "SELECT p.node_id, p.publication_date, n.created_at
             FROM papers p
             JOIN nodes n ON n.id = p.node_id"
        )?;

        let papers: Vec<(String, Option<String>, i64)> = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?.filter_map(|r| r.ok()).collect();

        let mut updated = 0;
        let mut unknown = 0;

        for (node_id, pub_date, current_created_at) in papers {
            // Try to parse the publication date
            let parsed_ts = pub_date
                .as_ref()
                .filter(|d| !d.is_empty())
                .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|dt| dt.and_utc().timestamp_millis())
                .unwrap_or(0);  // 0 = unknown date

            // Update if dates differ significantly (more than 1 day) or setting to unknown
            if (parsed_ts - current_created_at).abs() > 86_400_000 || (parsed_ts == 0 && current_created_at != 0) {
                conn.execute(
                    "UPDATE nodes SET created_at = ?1 WHERE id = ?2",
                    params![parsed_ts, node_id],
                )?;
                if parsed_ts == 0 {
                    unknown += 1;
                } else {
                    updated += 1;
                }
            }
        }

        Ok((updated, unknown))
    }

    /// Get count of papers
    pub fn get_paper_count(&self) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM papers", [], |row| row.get(0))
    }

    /// Get count of papers with PDFs
    pub fn get_paper_pdf_count(&self) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM papers WHERE pdf_available = 1", [], |row| row.get(0))
    }

    /// Reformat all paper abstracts with section detection
    pub fn reformat_all_paper_abstracts(&self) -> Result<usize> {
        use crate::format_abstract::{format_abstract, strip_html_tags};

        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT node_id, abstract FROM papers WHERE abstract IS NOT NULL"
        )?;

        let papers: Vec<(String, String)> = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .filter_map(|r| r.ok())
        .collect();

        drop(stmt);

        let mut count = 0;
        for (node_id, abstract_text) in papers {
            // Strip HTML from raw abstract
            let clean_abstract = strip_html_tags(&abstract_text);
            let formatted = format_abstract(&clean_abstract);

            // Always update - clean raw abstract and add formatted version if structured
            let abstract_formatted = if formatted.had_structure {
                Some(formatted.markdown.as_str())
            } else {
                None
            };
            let sections_json = if formatted.sections.is_empty() {
                None
            } else {
                serde_json::to_string(&formatted.sections).ok()
            };

            conn.execute(
                "UPDATE papers SET abstract = ?1, abstract_formatted = ?2, abstract_sections = ?3 WHERE node_id = ?4",
                params![clean_abstract, abstract_formatted, sections_json, node_id],
            )?;
            count += 1;
        }

        Ok(count)
    }

    /// Find duplicate papers by title for cleanup
    /// Returns (node_id, title, doi, abstract_text, node_content) where title appears more than once
    pub fn find_duplicate_papers_by_title(&self) -> Result<Vec<(String, String, Option<String>, Option<String>, Option<String>)>> {
        let conn = self.conn.lock().unwrap();

        // Find titles that appear more than once
        let mut stmt = conn.prepare(
            "SELECT n.id, n.title, p.doi, p.abstract, n.content
             FROM nodes n
             JOIN papers p ON n.id = p.node_id
             WHERE n.title IN (
                 SELECT title FROM nodes WHERE content_type = 'paper' GROUP BY title HAVING COUNT(*) > 1
             )
             ORDER BY n.title"
        )?;

        let papers: Vec<(String, String, Option<String>, Option<String>, Option<String>)> = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

        Ok(papers)
    }

    /// Delete a paper and its node
    pub fn delete_paper_and_node(&self, node_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        // Cascade will handle papers table due to foreign key
        conn.execute("DELETE FROM nodes WHERE id = ?1", params![node_id])?;
        Ok(())
    }
}

/// Convert raw bytes to f32 embedding vector
fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks(4)
        .map(|chunk| {
            if chunk.len() == 4 {
                f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
            } else {
                0.0
            }
        })
        .collect()
}
