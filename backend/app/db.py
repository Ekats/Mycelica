"""
db.py - Unified Database Layer for Mycelica

Cute internal API for all database operations.
No more scattered sqlite3.connect() calls!
"""

import sqlite3
import json
from typing import Dict, List, Any, Optional
from contextlib import contextmanager
from datetime import datetime

DB_PATH = 'mycelica.db'


# =============================================================================
# CONNECTION MANAGEMENT
# =============================================================================

@contextmanager
def get_connection():
    """Context manager for database connections. Use this everywhere!

    Usage:
        with get_connection() as conn:
            cursor = conn.cursor()
            cursor.execute(...)
    """
    conn = sqlite3.connect(DB_PATH)
    try:
        yield conn
        conn.commit()
    except Exception as e:
        conn.rollback()
        raise e
    finally:
        conn.close()


def init_all_tables():
    """Initialize ALL application tables in one place."""
    with get_connection() as conn:
        cursor = conn.cursor()

        # Message analysis table (base version - compatible with existing)
        cursor.execute('''
            CREATE TABLE IF NOT EXISTS message_analysis (
                message_id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                title TEXT,
                summary TEXT,
                tags TEXT,
                is_analyzed INTEGER DEFAULT 0,
                analyzed_at TIMESTAMP,
                user_query_preview TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
        ''')

        # Add new columns if they don't exist (schema migration)
        # This handles existing databases gracefully
        new_columns = [
            ('significance_score', 'REAL DEFAULT 0'),
            ('is_finding', 'INTEGER DEFAULT 0'),
            ('is_quote', 'INTEGER DEFAULT 0'),
            ('is_poem', 'INTEGER DEFAULT 0')
        ]
        for col_name, col_type in new_columns:
            try:
                cursor.execute(f'ALTER TABLE message_analysis ADD COLUMN {col_name} {col_type}')
            except sqlite3.OperationalError:
                pass  # Column already exists

        # Topic analysis table (base version - compatible with existing)
        cursor.execute('''
            CREATE TABLE IF NOT EXISTS topic_analysis (
                topic_id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                title TEXT,
                summary TEXT,
                tags TEXT,
                message_count INTEGER,
                is_analyzed INTEGER DEFAULT 0,
                analyzed_at TIMESTAMP,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
        ''')

        # Add new columns to topic_analysis if they don't exist
        topic_new_columns = [
            ('child_count', 'INTEGER DEFAULT 0'),
            ('significance_score', 'REAL DEFAULT 0'),
            ('findings_count', 'INTEGER DEFAULT 0')
        ]
        for col_name, col_type in topic_new_columns:
            try:
                cursor.execute(f'ALTER TABLE topic_analysis ADD COLUMN {col_name} {col_type}')
            except sqlite3.OperationalError:
                pass  # Column already exists

        # Topic tags table
        cursor.execute('''
            CREATE TABLE IF NOT EXISTS topic_tags (
                topic_id TEXT PRIMARY KEY,
                tags TEXT NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                is_ai_generated INTEGER DEFAULT 1
            )
        ''')

        # Cluster names table
        cursor.execute('''
            CREATE TABLE IF NOT EXISTS cluster_names (
                cluster_id INTEGER,
                level TEXT NOT NULL,
                name TEXT NOT NULL,
                keywords TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                is_manual INTEGER DEFAULT 1,
                PRIMARY KEY (cluster_id, level)
            )
        ''')

        # Manual message titles table
        cursor.execute('''
            CREATE TABLE IF NOT EXISTS message_titles (
                message_id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                is_manual INTEGER DEFAULT 1,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
        ''')

        # Create indexes (only for base columns that always exist)
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_message_analysis_conv ON message_analysis(conversation_id)')
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_message_analysis_analyzed ON message_analysis(is_analyzed)')
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_topic_analysis_conv ON topic_analysis(conversation_id)')

        # Try to create index on new columns (may fail if columns don't exist yet)
        try:
            cursor.execute('CREATE INDEX IF NOT EXISTS idx_message_analysis_finding ON message_analysis(is_finding)')
        except sqlite3.OperationalError:
            pass

        print("Database tables initialized")


# =============================================================================
# MESSAGE ANALYSIS OPERATIONS
# =============================================================================

def load_all_message_analysis() -> Dict[str, Dict[str, Any]]:
    """Load all analyzed messages as a dict keyed by message_id."""
    analysis_map = {}

    with get_connection() as conn:
        cursor = conn.cursor()
        cursor.execute('''
            SELECT message_id, title, summary, tags, is_analyzed, analyzed_at,
                   significance_score, is_finding, is_quote, is_poem
            FROM message_analysis
            WHERE is_analyzed = 1
        ''')

        for row in cursor.fetchall():
            msg_id, title, summary, tags_json, is_analyzed, analyzed_at, \
            significance, is_finding, is_quote, is_poem = row

            try:
                tags = json.loads(tags_json) if tags_json else []
            except json.JSONDecodeError:
                tags = []

            analysis_map[msg_id] = {
                "title": title,
                "summary": summary,
                "tags": tags,
                "is_analyzed": bool(is_analyzed),
                "analyzed_at": analyzed_at,
                "significance_score": significance or 0,
                "is_finding": bool(is_finding),
                "is_quote": bool(is_quote),
                "is_poem": bool(is_poem)
            }

    if analysis_map:
        print(f"Loaded {len(analysis_map)} analyzed messages from database")

    return analysis_map


def save_message_analysis(
    message_id: str,
    conversation_id: str,
    title: str,
    summary: str,
    tags: List[str],
    user_query_preview: str = "",
    significance_score: float = 0,
    is_finding: bool = False,
    is_quote: bool = False,
    is_poem: bool = False
):
    """Save a single message analysis to the database."""
    with get_connection() as conn:
        cursor = conn.cursor()
        cursor.execute('''
            INSERT OR REPLACE INTO message_analysis
            (message_id, conversation_id, title, summary, tags, is_analyzed,
             analyzed_at, user_query_preview, significance_score, is_finding, is_quote, is_poem, created_at)
            VALUES (?, ?, ?, ?, ?, 1, datetime('now'), ?, ?, ?, ?, ?, datetime('now'))
        ''', (message_id, conversation_id, title, summary, json.dumps(tags),
              user_query_preview[:200], significance_score, int(is_finding), int(is_quote), int(is_poem)))


def get_analyzed_message_ids() -> set:
    """Get set of message IDs that have been analyzed."""
    with get_connection() as conn:
        cursor = conn.cursor()
        cursor.execute('SELECT message_id FROM message_analysis WHERE is_analyzed = 1')
        return {row[0] for row in cursor.fetchall()}


def get_findings() -> List[Dict[str, Any]]:
    """Get all messages marked as findings (significant discoveries)."""
    with get_connection() as conn:
        cursor = conn.cursor()
        cursor.execute('''
            SELECT message_id, conversation_id, title, summary, tags, significance_score
            FROM message_analysis
            WHERE is_finding = 1
            ORDER BY significance_score DESC
        ''')

        findings = []
        for row in cursor.fetchall():
            msg_id, conv_id, title, summary, tags_json, score = row
            findings.append({
                "message_id": msg_id,
                "conversation_id": conv_id,
                "title": title,
                "summary": summary,
                "tags": json.loads(tags_json) if tags_json else [],
                "significance_score": score
            })
        return findings


# =============================================================================
# TOPIC ANALYSIS OPERATIONS
# =============================================================================

def load_all_topic_analysis() -> Dict[str, Dict[str, Any]]:
    """Load all analyzed topics as a dict keyed by topic_id."""
    analysis_map = {}

    with get_connection() as conn:
        cursor = conn.cursor()
        cursor.execute('''
            SELECT topic_id, title, summary, tags, message_count, is_analyzed,
                   analyzed_at, child_count, significance_score, findings_count
            FROM topic_analysis
            WHERE is_analyzed = 1
        ''')

        for row in cursor.fetchall():
            topic_id, title, summary, tags_json, msg_count, is_analyzed, \
            analyzed_at, child_count, significance, findings_count = row

            try:
                tags = json.loads(tags_json) if tags_json else []
            except json.JSONDecodeError:
                tags = []

            analysis_map[topic_id] = {
                "title": title,
                "summary": summary,
                "tags": tags,
                "message_count": msg_count,
                "is_analyzed": bool(is_analyzed),
                "analyzed_at": analyzed_at,
                "child_count": child_count or 0,
                "significance_score": significance or 0,
                "findings_count": findings_count or 0
            }

    if analysis_map:
        print(f"Loaded {len(analysis_map)} analyzed topics from database")

    return analysis_map


def save_topic_analysis(
    topic_id: str,
    conversation_id: str,
    title: str,
    summary: str,
    tags: List[str],
    message_count: int,
    child_count: int = 0,
    significance_score: float = 0,
    findings_count: int = 0
):
    """Save a single topic analysis to the database."""
    with get_connection() as conn:
        cursor = conn.cursor()
        cursor.execute('''
            INSERT OR REPLACE INTO topic_analysis
            (topic_id, conversation_id, title, summary, tags, message_count,
             child_count, significance_score, findings_count, is_analyzed, analyzed_at, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, datetime('now'), datetime('now'))
        ''', (topic_id, conversation_id, title, summary, json.dumps(tags),
              message_count, child_count, significance_score, findings_count))


def get_analyzed_topic_ids() -> set:
    """Get set of topic IDs that have been analyzed."""
    with get_connection() as conn:
        cursor = conn.cursor()
        cursor.execute('SELECT topic_id FROM topic_analysis WHERE is_analyzed = 1')
        return {row[0] for row in cursor.fetchall()}


# =============================================================================
# TOPIC TAGS OPERATIONS
# =============================================================================

def load_all_topic_tags() -> Dict[str, List[str]]:
    """Load all topic tags as a dict keyed by topic_id."""
    tags_map = {}

    with get_connection() as conn:
        cursor = conn.cursor()
        cursor.execute('SELECT topic_id, tags FROM topic_tags')

        for topic_id, tags_json in cursor.fetchall():
            try:
                tags_map[topic_id] = json.loads(tags_json)
            except json.JSONDecodeError:
                pass

    if tags_map:
        print(f"Loaded {len(tags_map)} topic tags from database")

    return tags_map


def save_topic_tags(topic_id: str, tags: List[str], is_ai: bool = True):
    """Save tags for a topic."""
    with get_connection() as conn:
        cursor = conn.cursor()
        cursor.execute('''
            INSERT OR REPLACE INTO topic_tags (topic_id, tags, created_at, is_ai_generated)
            VALUES (?, ?, datetime('now'), ?)
        ''', (topic_id, json.dumps(tags), 1 if is_ai else 0))


# =============================================================================
# CLUSTER NAMES OPERATIONS
# =============================================================================

def load_cluster_names(level: str = 'galaxy') -> Dict[int, str]:
    """Load cluster names for a specific level."""
    names = {}

    with get_connection() as conn:
        cursor = conn.cursor()
        cursor.execute('''
            SELECT cluster_id, name FROM cluster_names
            WHERE level = ?
            ORDER BY cluster_id
        ''', (level,))

        for cluster_id, name in cursor.fetchall():
            # Handle bytes cluster_id
            if isinstance(cluster_id, bytes):
                cluster_id = int.from_bytes(cluster_id, byteorder='little')
            names[cluster_id] = name

    if names:
        print(f"Loaded {len(names)} cluster names for level '{level}'")

    return names


def save_cluster_name(cluster_id: int, level: str, name: str, keywords: Dict = None, is_manual: bool = True):
    """Save a cluster name."""
    with get_connection() as conn:
        cursor = conn.cursor()
        cursor.execute('''
            INSERT OR REPLACE INTO cluster_names (cluster_id, level, name, keywords, created_at, is_manual)
            VALUES (?, ?, ?, ?, datetime('now'), ?)
        ''', (cluster_id, level, name, json.dumps(keywords or {}), 1 if is_manual else 0))


def load_universe_categories() -> List[Dict[str, Any]]:
    """Load universe-level categories with their metadata."""
    categories = []

    with get_connection() as conn:
        cursor = conn.cursor()
        cursor.execute('''
            SELECT cluster_id, name, keywords FROM cluster_names
            WHERE level = 'universe'
            ORDER BY cluster_id
        ''')

        for cluster_id, name, keywords_json in cursor.fetchall():
            # Handle bytes cluster_id
            if isinstance(cluster_id, bytes):
                cluster_id = int.from_bytes(cluster_id, byteorder='little')

            try:
                keywords_data = json.loads(keywords_json) if keywords_json else {}
            except (json.JSONDecodeError, TypeError):
                keywords_data = {}

            categories.append({
                "cluster_id": cluster_id,
                "name": name,
                "color": keywords_data.get("color", "#4a9eff"),
                "galaxy_count": keywords_data.get("cluster_count", 0),
                "description": keywords_data.get("description", ""),
                "sample_topics": keywords_data.get("sample_topics", []),
                "galaxy_clusters": keywords_data.get("galaxy_clusters", [])
            })

    return categories


# =============================================================================
# MANUAL TITLES OPERATIONS
# =============================================================================

def load_manual_titles() -> Dict[str, str]:
    """Load all manually written message titles."""
    titles = {}

    with get_connection() as conn:
        cursor = conn.cursor()
        cursor.execute('SELECT message_id, title FROM message_titles WHERE is_manual = 1')

        for msg_id, title in cursor.fetchall():
            titles[msg_id] = title

    if titles:
        print(f"Loaded {len(titles)} manual message titles")

    return titles


def save_manual_title(message_id: str, title: str):
    """Save a manual title for a message."""
    with get_connection() as conn:
        cursor = conn.cursor()
        cursor.execute('''
            INSERT OR REPLACE INTO message_titles (message_id, title, is_manual, created_at)
            VALUES (?, ?, 1, datetime('now'))
        ''', (message_id, title))


# =============================================================================
# STATS & UTILITIES
# =============================================================================

def get_analysis_stats() -> Dict[str, Any]:
    """Get overall analysis statistics."""
    with get_connection() as conn:
        cursor = conn.cursor()

        cursor.execute('SELECT COUNT(*) FROM message_analysis WHERE is_analyzed = 1')
        analyzed_messages = cursor.fetchone()[0]

        cursor.execute('SELECT COUNT(*) FROM topic_analysis WHERE is_analyzed = 1')
        analyzed_topics = cursor.fetchone()[0]

        cursor.execute('SELECT COUNT(*) FROM message_analysis WHERE is_finding = 1')
        findings_count = cursor.fetchone()[0]

        cursor.execute('SELECT COUNT(*) FROM message_analysis WHERE is_quote = 1')
        quotes_count = cursor.fetchone()[0]

        cursor.execute('SELECT COUNT(*) FROM message_analysis WHERE is_poem = 1')
        poems_count = cursor.fetchone()[0]

        return {
            "analyzed_messages": analyzed_messages,
            "analyzed_topics": analyzed_topics,
            "findings_count": findings_count,
            "quotes_count": quotes_count,
            "poems_count": poems_count
        }


# Initialize tables on module import
init_all_tables()
