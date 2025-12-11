use rusqlite::{Connection, Result, params};
use std::path::Path;
use std::sync::Mutex;
use super::models::{Node, Edge, NodeType, EdgeType, Position};

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Database { conn: Mutex::new(conn) };
        db.init()?;
        Ok(db)
    }

    #[allow(dead_code)]
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Database { conn: Mutex::new(conn) };
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
                last_accessed_at INTEGER
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

            CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(source_id);
            CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(target_id);
            CREATE INDEX IF NOT EXISTS idx_edges_type ON edges(type);
            CREATE INDEX IF NOT EXISTS idx_nodes_type ON nodes(type);
            -- Note: indexes for depth, is_item, parent_id are created after migration

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

        // Create indexes for dynamic hierarchy columns (after migrations ensure columns exist)
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_depth ON nodes(depth)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_is_item ON nodes(is_item)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_parent ON nodes(parent_id)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_needs_clustering ON nodes(needs_clustering)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_conversation ON nodes(conversation_id)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_pinned ON nodes(is_pinned)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_nodes_last_accessed ON nodes(last_accessed_at)", [])?;

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
            "INSERT INTO nodes (id, type, title, url, content, position_x, position_y, created_at, updated_at, cluster_id, cluster_label, ai_title, summary, tags, emoji, is_processed, depth, is_item, is_universe, parent_id, child_count, conversation_id, sequence_index, is_pinned, last_accessed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
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
        })
    }

    /// Standard SELECT columns for nodes (excludes embedding - use dedicated functions)
    const NODE_COLUMNS: &'static str = "id, type, title, url, content, position_x, position_y, created_at, updated_at, cluster_id, cluster_label, ai_title, summary, tags, emoji, is_processed, depth, is_item, is_universe, parent_id, child_count, conversation_id, sequence_index, is_pinned, last_accessed_at";

    pub fn get_all_nodes(&self) -> Result<Vec<Node>> {
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
             conversation_id = ?21, sequence_index = ?22, is_pinned = ?23, last_accessed_at = ?24 WHERE id = ?1",
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
            ],
        )?;
        Ok(())
    }

    pub fn delete_node(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM nodes WHERE id = ?1", params![id])?;
        Ok(())
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

    pub fn delete_edge(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM edges WHERE id = ?1", params![id])?;
        Ok(())
    }

    // Search
    pub fn search_nodes(&self, query: &str) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT n.id, n.type, n.title, n.url, n.content, n.position_x, n.position_y, n.created_at, n.updated_at, n.cluster_id, n.cluster_label, n.ai_title, n.summary, n.tags, n.emoji, n.is_processed, n.depth, n.is_item, n.is_universe, n.parent_id, n.child_count, n.conversation_id, n.sequence_index, n.is_pinned, n.last_accessed_at
             FROM nodes n
             JOIN nodes_fts fts ON n.rowid = fts.rowid
             WHERE nodes_fts MATCH ?1
             ORDER BY rank"
        )?;

        let nodes = stmt.query_map(params![query], |row| {
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
            })
        })?.collect::<Result<Vec<_>>>()?;

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
    pub fn update_node_ai(&self, node_id: &str, ai_title: &str, summary: &str, tags: &str, emoji: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET ai_title = ?2, summary = ?3, tags = ?4, emoji = ?5, is_processed = 1 WHERE id = ?1",
            params![node_id, ai_title, summary, tags, emoji],
        )?;
        Ok(())
    }

    // Get items that haven't been processed by AI yet
    pub fn get_unprocessed_nodes(&self) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, type, title, url, content, position_x, position_y, created_at, updated_at, cluster_id, cluster_label, ai_title, summary, tags, emoji, is_processed, depth, is_item, is_universe, parent_id, child_count, conversation_id, sequence_index, is_pinned, last_accessed_at, embedding
             FROM nodes WHERE is_item = 1 AND (is_processed = 0 OR is_processed IS NULL) ORDER BY created_at DESC"
        )?;

        let nodes = stmt.query_map([], |row| {
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
            })
        })?.collect::<Result<Vec<_>>>()?;

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
        let mut stmt = conn.prepare(
            "SELECT id, type, title, url, content, position_x, position_y, created_at, updated_at,
                    cluster_id, cluster_label, ai_title, summary, tags, emoji, is_processed, depth, is_item, is_universe, parent_id, child_count, conversation_id, sequence_index, is_pinned, last_accessed_at, embedding
             FROM nodes WHERE depth = ?1 ORDER BY child_count DESC, title"
        )?;

        let nodes = stmt.query_map(params![depth], |row| {
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
            })
        })?.collect::<Result<Vec<_>>>()?;

        Ok(nodes)
    }

    /// Get children of a specific parent node
    pub fn get_children(&self, parent_id: &str) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, type, title, url, content, position_x, position_y, created_at, updated_at,
                    cluster_id, cluster_label, ai_title, summary, tags, emoji, is_processed, depth, is_item, is_universe, parent_id, child_count, conversation_id, sequence_index, is_pinned, last_accessed_at, embedding
             FROM nodes WHERE parent_id = ?1 ORDER BY child_count DESC, title"
        )?;

        let nodes = stmt.query_map(params![parent_id], |row| {
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
            })
        })?.collect::<Result<Vec<_>>>()?;

        Ok(nodes)
    }

    /// Get the Universe node (single root, is_universe = true)
    pub fn get_universe(&self) -> Result<Option<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, type, title, url, content, position_x, position_y, created_at, updated_at,
                    cluster_id, cluster_label, ai_title, summary, tags, emoji, is_processed, depth, is_item, is_universe, parent_id, child_count, conversation_id, sequence_index, is_pinned, last_accessed_at, embedding
             FROM nodes WHERE is_universe = 1 LIMIT 1"
        )?;

        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            Ok(Some(Node {
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
            }))
        } else {
            Ok(None)
        }
    }

    /// Get all items (is_item = true) - openable content
    pub fn get_items(&self) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, type, title, url, content, position_x, position_y, created_at, updated_at,
                    cluster_id, cluster_label, ai_title, summary, tags, emoji, is_processed, depth, is_item, is_universe, parent_id, child_count, conversation_id, sequence_index, is_pinned, last_accessed_at, embedding
             FROM nodes WHERE is_item = 1 ORDER BY created_at DESC"
        )?;

        let nodes = stmt.query_map([], |row| {
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
            })
        })?.collect::<Result<Vec<_>>>()?;

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
    pub fn delete_hierarchy_nodes(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute(
            "DELETE FROM nodes WHERE is_item = 0 AND is_universe = 0",
            [],
        )?;
        Ok(deleted)
    }

    /// Clear parent_id on all items (for rebuild)
    pub fn clear_item_parents(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET parent_id = NULL WHERE is_item = 1",
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
        let mut stmt = conn.prepare(
            "SELECT id, type, title, url, content, position_x, position_y, created_at, updated_at,
                    cluster_id, cluster_label, ai_title, summary, tags, emoji, is_processed, depth, is_item, is_universe, parent_id, child_count, conversation_id, sequence_index, is_pinned, last_accessed_at, embedding
             FROM nodes WHERE is_item = 1 AND needs_clustering = 1 ORDER BY created_at DESC"
        )?;

        let nodes = stmt.query_map([], |row| {
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
            })
        })?.collect::<Result<Vec<_>>>()?;

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
                    n.cluster_id, n.cluster_label, n.ai_title, n.summary, n.tags, n.emoji, n.is_processed, n.depth, n.is_item, n.is_universe, n.parent_id, n.child_count, n.conversation_id, n.sequence_index, n.is_pinned, n.last_accessed_at
             FROM nodes n
             JOIN edges e ON n.id = e.source_id
             WHERE (e.target_id = ?1 OR e.target_id = ?2)
               AND e.type = 'belongs_to'
               AND (e.weight IS NULL OR e.weight >= ?3)
             ORDER BY e.weight DESC"
        )?;

        let nodes = stmt.query_map(params![topic_id, placeholder_id, min_w], |row| {
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
            })
        })?.collect::<Result<Vec<_>>>()?;

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
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE nodes SET parent_id = ?2 WHERE id = ?1",
            params![node_id, parent_id],
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
        let mut stmt = conn.prepare(
            "SELECT id, type, title, url, content, position_x, position_y, created_at, updated_at,
                    cluster_id, cluster_label, ai_title, summary, tags, emoji, is_processed, depth, is_item, is_universe, parent_id, child_count, conversation_id, sequence_index, is_pinned, last_accessed_at, embedding
             FROM nodes WHERE conversation_id = ?1 ORDER BY sequence_index ASC"
        )?;

        let nodes = stmt.query_map(params![conversation_id], |row| {
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
            })
        })?.collect::<Result<Vec<_>>>()?;

        Ok(nodes)
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
    pub fn get_pinned_nodes(&self) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, type, title, url, content, position_x, position_y, created_at, updated_at,
                    cluster_id, cluster_label, ai_title, summary, tags, emoji, is_processed, depth, is_item, is_universe, parent_id, child_count, conversation_id, sequence_index, is_pinned, last_accessed_at, embedding
             FROM nodes WHERE is_pinned = 1 ORDER BY last_accessed_at DESC"
        )?;

        let nodes = stmt.query_map([], |row| {
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
            })
        })?.collect::<Result<Vec<_>>>()?;

        Ok(nodes)
    }

    /// Get recently accessed nodes (for Sidebar Recent tab)
    pub fn get_recent_nodes(&self, limit: i32) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, type, title, url, content, position_x, position_y, created_at, updated_at,
                    cluster_id, cluster_label, ai_title, summary, tags, emoji, is_processed, depth, is_item, is_universe, parent_id, child_count, conversation_id, sequence_index, is_pinned, last_accessed_at, embedding
             FROM nodes WHERE last_accessed_at IS NOT NULL ORDER BY last_accessed_at DESC LIMIT ?1"
        )?;

        let nodes = stmt.query_map(params![limit], |row| {
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
            })
        })?.collect::<Result<Vec<_>>>()?;

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
    /// Returns (node_id, embedding) pairs
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
    /// Includes both items (with ai_title) AND category nodes (with title)
    pub fn get_nodes_needing_embeddings(&self) -> Result<Vec<Node>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM nodes WHERE (ai_title IS NOT NULL OR (is_item = 0 AND title IS NOT NULL)) AND embedding IS NULL",
            Self::NODE_COLUMNS
        ))?;

        let rows = stmt.query_map([], Self::row_to_node)?;
        rows.collect()
    }

    /// Create semantic "Related" edges between nodes based on embedding similarity
    /// Gives +0.2 bonus to siblings (same parent) so within-view edges are prioritized
    /// Uses lower threshold (min_similarity - 0.2) for category-to-category edges
    /// Returns the number of edges created
    pub fn create_semantic_edges(&self, min_similarity: f32, max_edges_per_node: usize) -> Result<usize> {
        use crate::similarity::cosine_similarity;

        // Get all nodes with embeddings
        let nodes_with_embeddings = self.get_nodes_with_embeddings()?;
        if nodes_with_embeddings.len() < 2 {
            return Ok(0);
        }

        // Build parent_id and is_item lookup
        let (parent_map, is_item_map): (
            std::collections::HashMap<String, Option<String>>,
            std::collections::HashMap<String, bool>
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
            let mut parents = std::collections::HashMap::new();
            let mut items = std::collections::HashMap::new();
            for row in rows.filter_map(|r| r.ok()) {
                parents.insert(row.0.clone(), row.1);
                items.insert(row.0, row.2);
            }
            (parents, items)
        };

        let now = chrono::Utc::now().timestamp_millis();
        let mut edges_created = 0;
        const SIBLING_BONUS: f32 = 0.2;
        const CATEGORY_THRESHOLD_REDUCTION: f32 = 0.2;

        // For each node, find most similar nodes and create edges
        for (i, (node_id, embedding)) in nodes_with_embeddings.iter().enumerate() {
            let node_parent = parent_map.get(node_id).and_then(|p| p.clone());
            let node_is_item = *is_item_map.get(node_id).unwrap_or(&true);

            // Compute similarities with all other nodes, applying sibling bonus
            let mut similarities: Vec<(&String, f32, f32, bool)> = nodes_with_embeddings
                .iter()
                .skip(i + 1)  // Only compare with nodes after this one (avoid duplicates)
                .map(|(other_id, other_emb)| {
                    let raw_sim = cosine_similarity(embedding, other_emb);
                    let other_parent = parent_map.get(other_id).and_then(|p| p.clone());
                    let other_is_item = *is_item_map.get(other_id).unwrap_or(&true);

                    // Boost score if same parent (siblings will be visible together)
                    let is_sibling = node_parent.is_some() && node_parent == other_parent;
                    let boosted_sim = if is_sibling { raw_sim + SIBLING_BONUS } else { raw_sim };

                    // Use lower threshold for category-to-category edges
                    let is_category_edge = !node_is_item && !other_is_item;

                    (other_id, raw_sim, boosted_sim, is_category_edge)
                })
                .filter(|(_, raw_sim, _, is_category_edge)| {
                    let effective_threshold = if *is_category_edge {
                        min_similarity - CATEGORY_THRESHOLD_REDUCTION
                    } else {
                        min_similarity
                    };
                    *raw_sim >= effective_threshold
                })
                .collect();

            // Sort by boosted similarity descending and take top N
            similarities.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
            similarities.truncate(max_edges_per_node);

            // Create edges (store raw similarity as weight, not boosted)
            for (other_id, raw_similarity, _, _) in similarities {
                let edge_id = format!("semantic-{}-{}", node_id, other_id);

                // Check if edge already exists
                let conn = self.conn.lock().unwrap();
                let exists: bool = conn.query_row(
                    "SELECT COUNT(*) > 0 FROM edges WHERE id = ?1",
                    params![&edge_id],
                    |row| row.get(0),
                ).unwrap_or(false);
                drop(conn);

                if !exists {
                    let edge = Edge {
                        id: edge_id,
                        source: node_id.clone(),
                        target: other_id.clone(),
                        edge_type: EdgeType::Related,
                        label: Some(format!("{:.0}% similar", raw_similarity * 100.0)),
                        weight: Some(raw_similarity as f64),
                        edge_source: Some("ai".to_string()),
                        evidence_id: None,
                        confidence: Some(raw_similarity as f64),
                        created_at: now,
                    };

                    if self.insert_edge(&edge).is_ok() {
                        edges_created += 1;
                    }
                }
            }
        }

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
