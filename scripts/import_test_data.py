#!/usr/bin/env python3
"""Import test conversations into Mycelica SQLite database."""

import json
import sqlite3
import uuid
import math
from datetime import datetime
from pathlib import Path

# Paths
PROJECT_ROOT = Path(__file__).parent.parent
DATA_DIR = PROJECT_ROOT / "data"
DB_PATH = DATA_DIR / "mycelica.db"
TEST_DATA = DATA_DIR / "test_conversations.json"


def init_db(conn: sqlite3.Connection):
    """Create tables if they don't exist."""
    conn.executescript("""
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
            level INTEGER NOT NULL DEFAULT 3,
            parent_id TEXT,
            child_count INTEGER NOT NULL DEFAULT 0,
            ai_title TEXT,
            summary TEXT,
            tags TEXT,
            emoji TEXT,
            is_processed INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS edges (
            id TEXT PRIMARY KEY,
            source_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
            target_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
            type TEXT NOT NULL,
            label TEXT,
            created_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(source_id);
        CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(target_id);
        CREATE INDEX IF NOT EXISTS idx_nodes_type ON nodes(type);
        CREATE INDEX IF NOT EXISTS idx_nodes_level ON nodes(level);
        CREATE INDEX IF NOT EXISTS idx_nodes_parent ON nodes(parent_id);
    """)
    conn.commit()


def parse_timestamp(ts: str) -> int:
    """Parse ISO timestamp to Unix milliseconds."""
    try:
        dt = datetime.fromisoformat(ts.replace('Z', '+00:00'))
        return int(dt.timestamp() * 1000)
    except:
        return int(datetime.now().timestamp() * 1000)


def import_conversations(conn: sqlite3.Connection, conversations: list):
    """Import conversations as L3 Tree nodes with L4 Leaf message children.

    Hierarchy:
    L3 Tree  = Conversations (imported here)
    L4 Leaf  = Individual messages within conversations
    """

    # Clear existing data
    conn.execute("DELETE FROM edges")
    conn.execute("DELETE FROM nodes")

    nodes = []
    edges = []

    # Layout: arrange conversations in a circle
    n_convos = len(conversations)
    radius = 300

    for i, conv in enumerate(conversations):
        # Conversation becomes a L3 "context" node (Tree level)
        conv_id = conv['uuid']
        angle = (2 * math.pi * i) / n_convos
        x = radius * math.cos(angle)
        y = radius * math.sin(angle)

        messages = conv.get('chat_messages', [])
        # Count how many human messages we'll create
        child_count = sum(1 for msg in messages if msg.get('sender') == 'human')

        conv_node = {
            'id': conv_id,
            'type': 'context',
            'title': conv.get('name') or 'Untitled',
            'url': None,
            'content': conv.get('summary'),
            'position_x': x,
            'position_y': y,
            'created_at': parse_timestamp(conv.get('created_at', '')),
            'updated_at': parse_timestamp(conv.get('updated_at', '')),
            'level': 3,  # L3 Tree = Conversations
            'parent_id': None,  # Will be set when L2 World nodes are created
            'child_count': child_count,
        }
        nodes.append(conv_node)

        # Each message becomes a L4 "thought" node (Leaf level)
        msg_radius = 80

        for j, msg in enumerate(messages):
            if msg.get('sender') != 'human':
                continue  # Only create nodes for human messages (questions)

            msg_id = msg['uuid']
            msg_angle = (2 * math.pi * j) / max(len(messages), 1)
            msg_x = x + msg_radius * math.cos(msg_angle)
            msg_y = y + msg_radius * math.sin(msg_angle)

            # Truncate title from message text
            text = msg.get('text', '')[:100]
            if len(msg.get('text', '')) > 100:
                text += '...'

            msg_node = {
                'id': msg_id,
                'type': 'thought',
                'title': text or 'Message',
                'url': None,
                'content': msg.get('text'),
                'position_x': msg_x,
                'position_y': msg_y,
                'created_at': parse_timestamp(msg.get('created_at', '')),
                'updated_at': parse_timestamp(msg.get('updated_at', '')),
                'level': 4,  # L4 Leaf = Individual messages
                'parent_id': conv_id,  # Parent is the conversation
                'child_count': 0,  # Leaf nodes have no children
            }
            nodes.append(msg_node)

            # Edge from conversation to message
            edge = {
                'id': str(uuid.uuid4()),
                'source_id': conv_id,
                'target_id': msg_id,
                'type': 'contains',
                'label': None,
                'created_at': parse_timestamp(msg.get('created_at', '')),
            }
            edges.append(edge)

    # Insert nodes
    conn.executemany("""
        INSERT INTO nodes (id, type, title, url, content, position_x, position_y, created_at, updated_at, level, parent_id, child_count)
        VALUES (:id, :type, :title, :url, :content, :position_x, :position_y, :created_at, :updated_at, :level, :parent_id, :child_count)
    """, nodes)

    # Insert edges
    conn.executemany("""
        INSERT INTO edges (id, source_id, target_id, type, label, created_at)
        VALUES (:id, :source_id, :target_id, :type, :label, :created_at)
    """, edges)

    conn.commit()
    return len(nodes), len(edges)


def main():
    print(f"Loading test data from {TEST_DATA}")
    with open(TEST_DATA) as f:
        conversations = json.load(f)

    print(f"Found {len(conversations)} conversations")

    # Ensure data directory exists
    DATA_DIR.mkdir(exist_ok=True)

    print(f"Creating database at {DB_PATH}")
    conn = sqlite3.connect(DB_PATH)
    init_db(conn)

    n_nodes, n_edges = import_conversations(conn, conversations)
    print(f"Imported {n_nodes} nodes and {n_edges} edges")

    # Quick verification
    cursor = conn.execute("SELECT type, COUNT(*) FROM nodes GROUP BY type")
    print("\nNodes by type:")
    for row in cursor:
        print(f"  {row[0]}: {row[1]}")

    cursor = conn.execute("SELECT level, COUNT(*) FROM nodes GROUP BY level ORDER BY level")
    print("\nNodes by hierarchy level:")
    level_names = {0: 'Universe', 1: 'Galaxy', 2: 'World', 3: 'Tree', 4: 'Leaf'}
    for row in cursor:
        level = row[0]
        count = row[1]
        name = level_names.get(level, f'Unknown-L{level}')
        print(f"  L{level} {name}: {count}")

    conn.close()
    print("\nDone!")


if __name__ == "__main__":
    main()
