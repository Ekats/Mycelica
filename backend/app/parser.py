import json
import zipfile
from pathlib import Path
from datetime import datetime
from typing import List, Dict, Any
from .models import Conversation, Message


def parse_datetime(dt_string: str) -> datetime:
    """Parse ISO format datetime from Claude export."""
    # Handle various formats
    if dt_string.endswith('Z'):
        dt_string = dt_string[:-1] + '+00:00'
    return datetime.fromisoformat(dt_string.replace('Z', '+00:00'))


def parse_claude_export(file_path: str) -> List[Dict[str, Any]]:
    """
    Parse a Claude.ai export file (ZIP or JSON).
    Returns list of conversation dicts ready for database insertion.
    """
    path = Path(file_path)

    if path.suffix == '.zip':
        with zipfile.ZipFile(path, 'r') as zf:
            with zf.open('conversations.json') as f:
                data = json.load(f)
    else:
        with open(path, 'r', encoding='utf-8') as f:
            data = json.load(f)

    conversations = []

    for conv in data:
        # Build conversation object
        conversation = {
            'id': conv['uuid'],
            'title': conv.get('name') or 'Untitled',
            'summary': conv.get('summary'),
            'created_at': parse_datetime(conv['created_at']),
            'updated_at': parse_datetime(conv['updated_at']),
            'messages': []
        }

        # Parse messages
        for msg in conv.get('chat_messages', []):
            # Get content - try 'text' first, then 'content'
            content = msg.get('text') or ''
            if not content and msg.get('content'):
                # Content might be a list of content blocks
                if isinstance(msg['content'], list):
                    content = '\n'.join(
                        block.get('text', '')
                        for block in msg['content']
                        if isinstance(block, dict)
                    )
                else:
                    content = str(msg['content'])

            message = {
                'id': msg['uuid'],
                'role': msg['sender'],  # 'human' or 'assistant'
                'content': content,
                'created_at': parse_datetime(msg['created_at'])
            }
            conversation['messages'].append(message)

        conversation['message_count'] = len(conversation['messages'])
        conversations.append(conversation)

    return conversations


def create_db_objects(parsed_data: List[Dict[str, Any]]) -> List[Conversation]:
    """Convert parsed data into SQLAlchemy model objects."""
    db_conversations = []

    for conv_data in parsed_data:
        conversation = Conversation(
            id=conv_data['id'],
            title=conv_data['title'],
            summary=conv_data['summary'],
            created_at=conv_data['created_at'],
            updated_at=conv_data['updated_at'],
            message_count=conv_data['message_count']
        )

        for msg_data in conv_data['messages']:
            message = Message(
                id=msg_data['id'],
                conversation_id=conv_data['id'],
                role=msg_data['role'],
                content=msg_data['content'],
                created_at=msg_data['created_at']
            )
            conversation.messages.append(message)

        db_conversations.append(conversation)

    return db_conversations
