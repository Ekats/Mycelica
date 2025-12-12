#!/usr/bin/env python3
"""Import Claude conversations directly into Mycelica database."""

import json
import sqlite3
import uuid
import math
from datetime import datetime
from pathlib import Path

def parse_timestamp(ts: str) -> int:
    """Parse ISO timestamp to Unix milliseconds."""
    try:
        dt = datetime.fromisoformat(ts.replace('Z', '+00:00'))
        return int(dt.timestamp() * 1000)
    except:
        return int(datetime.now().timestamp() * 1000)

def create_exchange_title(human_text: str) -> str:
    """Create title from human question (first line, truncated)."""
    clean = human_text.strip()
    first_line = clean.split('\n')[0]
    if len(first_line) > 60:
        return first_line[:60] + "..."
    return first_line

def pair_messages(messages: list) -> list:
    """Pair consecutive human + assistant messages into exchanges."""
    exchanges = []
    i = 0

    while i < len(messages):
        msg = messages[i]
        sender = msg.get('sender', '')
        text = msg.get('text', '') or msg.get('content', [{}])[0].get('text', '') or ''

        if sender == 'human':
            human_text = text
            human_time = parse_timestamp(msg.get('created_at', ''))

            # Look for following assistant response
            if i + 1 < len(messages) and messages[i + 1].get('sender') == 'assistant':
                i += 1
                assistant_msg = messages[i]
                assistant_text = assistant_msg.get('text', '') or ''
                # Handle content array format
                if not assistant_text and assistant_msg.get('content'):
                    content = assistant_msg['content']
                    if isinstance(content, list) and content:
                        assistant_text = content[0].get('text', '')
            else:
                assistant_text = '*No response*'

            exchanges.append({
                'title': create_exchange_title(human_text),
                'content': f"Human: {human_text}\n\nAssistant: {assistant_text}",
                'created_at': human_time,
            })
        else:
            # Orphan assistant message
            exchanges.append({
                'title': create_exchange_title(text[:100] if text else 'Response'),
                'content': f"Assistant: {text}",
                'created_at': parse_timestamp(msg.get('created_at', '')),
            })

        i += 1

    return exchanges

def main():
    import random

    # Paths
    json_path = Path("data/conversations.json")
    db_path = Path.home() / ".local/share/com.mycelica.app/mycelica.db"

    # Limit: set to None for all, or a number for testing
    MAX_EXCHANGES = None  # Import all

    print(f"Reading: {json_path}")
    print(f"Database: {db_path}")
    print(f"Max exchanges: {MAX_EXCHANGES or 'unlimited'}")

    with open(json_path) as f:
        conversations = json.load(f)

    # Exclude specific conversations
    EXCLUDE_IDS = {
        'fb73ed99-5131-4c2c-bd81-4a156634d344',  # Navigating direct instructions (personal)
    }

    # Only include conversations with "mycelica" in title (case insensitive)
    FILTER_MYCELICA = False  # Import all conversations

    conversations = [c for c in conversations if c.get('uuid') not in EXCLUDE_IDS]

    # Filter to mycelica-related conversations only
    if FILTER_MYCELICA:
        conversations = [
            c for c in conversations
            if 'mycelica' in (c.get('name') or '').lower()
        ]
        print(f"Filtered to {len(conversations)} Mycelica conversations")

    # Shuffle for random sampling
    random.shuffle(conversations)

    print(f"Found {len(conversations)} conversations (excluded {len(EXCLUDE_IDS)})")

    conn = sqlite3.connect(db_path)
    cur = conn.cursor()

    # Check existing nodes
    cur.execute("SELECT COUNT(*) FROM nodes WHERE conversation_id IS NOT NULL")
    existing = cur.fetchone()[0]
    if existing > 0:
        print(f"Warning: {existing} conversation nodes already exist. Skipping import.")
        print("To reimport, clear the database first.")
        return

    conversations_imported = 0
    exchanges_imported = 0
    skipped = 0

    n_convos = len(conversations)
    radius = 300.0

    for i, conv in enumerate(conversations):
        conv_id = conv.get('uuid', str(uuid.uuid4()))

        # Check if already exists
        cur.execute("SELECT 1 FROM nodes WHERE id = ?", (conv_id,))
        if cur.fetchone():
            skipped += 1
            continue

        # Position in circle
        angle = (2.0 * math.pi * i) / max(n_convos, 1)
        x = radius * math.cos(angle)
        y = radius * math.sin(angle)

        created_at = parse_timestamp(conv.get('created_at', ''))
        updated_at = parse_timestamp(conv.get('updated_at', '')) if conv.get('updated_at') else created_at

        messages = conv.get('chat_messages', [])
        exchanges = pair_messages(messages)
        exchange_count = len(exchanges)

        # Create conversation container (not an item, won't be clustered)
        cur.execute("""
            INSERT INTO nodes (
                id, type, title, url, content, position_x, position_y,
                created_at, updated_at, cluster_id, cluster_label,
                ai_title, summary, tags, emoji, is_processed,
                depth, is_item, is_universe, parent_id, child_count,
                conversation_id, sequence_index, is_pinned, last_accessed_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """, (
            conv_id, 'context', conv.get('name') or 'Untitled', None,
            f"{exchange_count} exchanges", x, y,
            created_at, updated_at, None, None,
            None, conv.get('summary'), None, '💬', 0,
            0, 0, 0, None, exchange_count,  # is_item=0, is_universe=0
            None, None, 0, None
        ))

        conversations_imported += 1

        # Create exchange nodes
        exchange_radius = 80.0
        for idx, exchange in enumerate(exchanges):
            exchange_id = f"{conv_id}-ex-{idx}"

            ex_angle = (2.0 * math.pi * idx) / max(exchange_count, 1)
            ex_x = x + exchange_radius * math.cos(ex_angle)
            ex_y = y + exchange_radius * math.sin(ex_angle)

            cur.execute("""
                INSERT INTO nodes (
                    id, type, title, url, content, position_x, position_y,
                    created_at, updated_at, cluster_id, cluster_label,
                    ai_title, summary, tags, emoji, is_processed,
                    depth, is_item, is_universe, parent_id, child_count,
                    conversation_id, sequence_index, is_pinned, last_accessed_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """, (
                exchange_id, 'thought', exchange['title'], None,
                exchange['content'], ex_x, ex_y,
                exchange['created_at'], exchange['created_at'], None, None,
                None, None, None, '💬', 0,
                0, 1, 0, None, 0,  # is_item=1 (will be clustered)
                conv_id, idx, 0, None
            ))

            exchanges_imported += 1

            # Check limit
            if MAX_EXCHANGES and exchanges_imported >= MAX_EXCHANGES:
                break

        if (i + 1) % 50 == 0:
            print(f"  Processed {i + 1}/{n_convos} conversations...")
            conn.commit()

        # Check limit
        if MAX_EXCHANGES and exchanges_imported >= MAX_EXCHANGES:
            print(f"  Reached {MAX_EXCHANGES} exchange limit, stopping.")
            break

    conn.commit()
    conn.close()

    print(f"\n✓ Import complete!")
    print(f"  Conversations: {conversations_imported}")
    print(f"  Exchanges: {exchanges_imported}")
    print(f"  Skipped: {skipped}")

if __name__ == "__main__":
    main()
