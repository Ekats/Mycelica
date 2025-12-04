from fastapi import FastAPI, UploadFile, File, Depends, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy import select, func
from typing import List
from pydantic import BaseModel
from dotenv import load_dotenv
import tempfile
import os

# Load environment variables from .env file
load_dotenv()

from .database import init_db, get_session
from .models import Conversation, Message, Node, Edge, ClusterName
from .parser import parse_claude_export, create_db_objects
from .analysis import (
    analyze_conversations, analyze_conversations_hierarchical, search_similar,
    regenerate_all_tags, analyze_all_messages, get_analysis_status, extract_message_pairs,
    load_message_analysis_from_db, run_full_ai_analysis, load_topic_analysis_from_db
)
from . import data_service

# Cache for analysis results
_analysis_cache = None

# In-memory storage for API key (for this session only)
_api_key = None

app = FastAPI(title="Mycelica API", version="0.1.0")

# Allow frontend to connect
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)


# Pydantic models for request/response
class ApiKeyRequest(BaseModel):
    api_key: str

class ApiKeyStatus(BaseModel):
    has_key: bool
    key_preview: str = ""


@app.on_event("startup")
async def startup():
    await init_db()


@app.get("/health")
async def health_check():
    return {"status": "healthy"}


@app.post("/api-key")
async def set_api_key(request: ApiKeyRequest):
    """Set Anthropic API key for AI naming (stored in memory only)."""
    global _api_key
    
    # Basic validation
    if not request.api_key.startswith("sk-"):
        raise HTTPException(status_code=400, detail="Invalid API key format")
    
    # Store in memory (not persisted)
    _api_key = request.api_key
    
    # Set environment variable for the analysis module
    os.environ["ANTHROPIC_API_KEY"] = request.api_key
    
    return {
        "status": "success", 
        "message": "API key set successfully",
        "key_preview": request.api_key[:8] + "..." + request.api_key[-4:]
    }


@app.get("/api-key/status")
async def get_api_key_status():
    """Check if API key is set."""
    global _api_key
    
    has_key = _api_key is not None or os.getenv("ANTHROPIC_API_KEY") is not None
    
    if has_key:
        key = _api_key or os.getenv("ANTHROPIC_API_KEY")
        preview = key[:8] + "..." + key[-4:] if key else ""
    else:
        preview = ""
    
    return ApiKeyStatus(
        has_key=has_key,
        key_preview=preview
    )


@app.delete("/api-key")
async def clear_api_key():
    """Clear the API key."""
    global _api_key
    
    _api_key = None
    if "ANTHROPIC_API_KEY" in os.environ:
        del os.environ["ANTHROPIC_API_KEY"]
    
    return {"status": "success", "message": "API key cleared"}


@app.get("/stats")
async def get_stats(session: AsyncSession = Depends(get_session)):
    """Get database statistics."""
    conv_count = await session.scalar(select(func.count(Conversation.id)))
    msg_count = await session.scalar(select(func.count(Message.id)))
    return {
        "conversations": conv_count or 0,
        "messages": msg_count or 0
    }


@app.post("/import")
async def import_conversations(
    file: UploadFile = File(...),
    session: AsyncSession = Depends(get_session)
):
    """Import conversations from Claude.ai export."""
    # Save uploaded file temporarily
    suffix = '.zip' if file.filename.endswith('.zip') else '.json'
    with tempfile.NamedTemporaryFile(delete=False, suffix=suffix) as tmp:
        content = await file.read()
        tmp.write(content)
        tmp_path = tmp.name

    try:
        # Parse the export
        parsed = parse_claude_export(tmp_path)
        conversations = create_db_objects(parsed)

        # Insert into database
        for conv in conversations:
            session.add(conv)

        await session.commit()

        return {
            "status": "success",
            "imported": len(conversations),
            "total_messages": sum(c.message_count for c in conversations)
        }
    except Exception as e:
        await session.rollback()
        raise HTTPException(status_code=400, detail=str(e))
    finally:
        os.unlink(tmp_path)


@app.get("/conversations")
async def list_conversations(
    skip: int = 0,
    limit: int = 100,
    session: AsyncSession = Depends(get_session)
):
    """List all conversations."""
    result = await session.execute(
        select(Conversation)
        .order_by(Conversation.updated_at.desc())
        .offset(skip)
        .limit(limit)
    )
    conversations = result.scalars().all()

    return [
        {
            "id": c.id,
            "title": c.title,
            "summary": c.summary,
            "created_at": c.created_at.isoformat(),
            "updated_at": c.updated_at.isoformat(),
            "message_count": c.message_count
        }
        for c in conversations
    ]


@app.get("/conversations/{conversation_id}")
async def get_conversation(
    conversation_id: str,
    session: AsyncSession = Depends(get_session)
):
    """Get a single conversation with all messages."""
    result = await session.execute(
        select(Conversation).where(Conversation.id == conversation_id)
    )
    conversation = result.scalar_one_or_none()

    if not conversation:
        raise HTTPException(status_code=404, detail="Conversation not found")

    # Get messages
    msg_result = await session.execute(
        select(Message)
        .where(Message.conversation_id == conversation_id)
        .order_by(Message.created_at)
    )
    messages = msg_result.scalars().all()

    return {
        "id": conversation.id,
        "title": conversation.title,
        "summary": conversation.summary,
        "created_at": conversation.created_at.isoformat(),
        "updated_at": conversation.updated_at.isoformat(),
        "message_count": conversation.message_count,
        "messages": [
            {
                "id": m.id,
                "role": m.role,
                "content": m.content,
                "created_at": m.created_at.isoformat()
            }
            for m in messages
        ]
    }


@app.get("/graph")
async def get_graph(session: AsyncSession = Depends(get_session)):
    """Get graph data for visualization (basic, without analysis)."""
    result = await session.execute(
        select(Conversation).order_by(Conversation.updated_at.desc())
    )
    conversations = result.scalars().all()

    nodes = []
    for conv in conversations:
        nodes.append({
            "id": conv.id,
            "label": conv.title[:50] + "..." if len(conv.title) > 50 else conv.title,
            "title": conv.title,
            "size": min(5 + conv.message_count * 0.5, 30),
            "created_at": conv.created_at.isoformat(),
            "message_count": conv.message_count
        })

    return {
        "nodes": nodes,
        "edges": []
    }


@app.post("/analyze")
async def run_analysis(session: AsyncSession = Depends(get_session)):
    """Run hierarchical analysis: message-level clustering with zoom levels."""
    global _analysis_cache

    # Get all conversations with ALL messages (not limited to 20)
    result = await session.execute(
        select(Conversation).order_by(Conversation.updated_at.desc())
    )
    conversations = result.scalars().all()

    # Build conversation dicts with all messages for proper analysis
    conv_dicts = []
    for conv in conversations:
        msg_result = await session.execute(
            select(Message)
            .where(Message.conversation_id == conv.id)
            .order_by(Message.created_at)
            # No limit - we need all messages for proper Q&A pair extraction
        )
        messages = msg_result.scalars().all()

        conv_dicts.append({
            "id": conv.id,
            "title": conv.title,
            "summary": conv.summary,
            "created_at": conv.created_at.isoformat(),
            "updated_at": conv.updated_at.isoformat(),
            "message_count": conv.message_count,
            "messages": [
                {
                    "content": m.content, 
                    "role": m.role,
                    "timestamp": m.created_at.isoformat()
                }
                for m in messages
            ]
        })

    print(f"\n=== Starting Hierarchical Analysis ===")
    print(f"Total conversations: {len(conv_dicts)}")
    print(f"Total messages: {sum(len(c['messages']) for c in conv_dicts)}")
    
    # Run new hierarchical analysis
    _analysis_cache = await analyze_conversations_hierarchical(conv_dicts)

    total_nodes = sum(len(_analysis_cache["nodes"][level]) for level in _analysis_cache["nodes"])
    
    return {
        "status": "success",
        "analysis_type": "hierarchical",
        "total_nodes": total_nodes,
        "zoom_levels": {
            "galaxy": len(_analysis_cache["nodes"]["galaxy"]),
            "cluster": len(_analysis_cache["nodes"]["cluster"]),
            "topic": len(_analysis_cache["nodes"]["topic"]),
            "message": len(_analysis_cache["nodes"]["message"])
        },
        "total_message_pairs": _analysis_cache["metadata"]["total_pairs"],
        "total_conversations": _analysis_cache["metadata"]["total_conversations"]
    }


@app.get("/graph/analyzed")
async def get_analyzed_graph():
    """Get the analyzed graph with clusters and edges (legacy format)."""
    global _analysis_cache

    if _analysis_cache is None:
        raise HTTPException(
            status_code=400,
            detail="Analysis not run yet. Call POST /analyze first."
        )

    # Return legacy format for backward compatibility
    # Use galaxy level as the main nodes
    return {
        "nodes": _analysis_cache["nodes"].get("galaxy", []),
        "edges": _analysis_cache["edges"],
        "clusters": _analysis_cache["metadata"]
    }


@app.get("/graph/hierarchical")
async def get_hierarchical_graph():
    """Get the full hierarchical graph structure."""
    global _analysis_cache

    if _analysis_cache is None:
        raise HTTPException(
            status_code=400,
            detail="Analysis not run yet. Call POST /analyze first."
        )

    return {
        "nodes": _analysis_cache["nodes"],
        "edges": _analysis_cache["edges"], 
        "metadata": _analysis_cache["metadata"]
    }


@app.get("/graph/zoom/{level}")
async def get_zoom_level(
    level: str,
    parent_id: str = None,
    session: AsyncSession = Depends(get_session)
):
    """Get nodes for a specific zoom level.
    
    Args:
        level: 'universe', 'galaxy', 'cluster', 'topic', or 'message'  
        parent_id: Optional parent ID to filter children
    """
    global _analysis_cache

    # Handle universe level - load from database
    if level == "universe":
        import sqlite3
        import json

        conn = sqlite3.connect('mycelica.db')
        cursor = conn.cursor()

        cursor.execute('''
            SELECT cluster_id, name, keywords FROM cluster_names
            WHERE level = 'universe'
            ORDER BY cluster_id
        ''')

        # Group by name to merge duplicate universe categories
        universe_by_name = {}
        for cluster_id, name, keywords_json in cursor.fetchall():
            # Handle bytes cluster_id
            if isinstance(cluster_id, bytes):
                cluster_id = int.from_bytes(cluster_id, byteorder='little')

            try:
                if keywords_json:
                    # Handle both string and bytes data
                    if isinstance(keywords_json, bytes):
                        keywords_json = keywords_json.decode('utf-8', errors='replace')
                    keywords_data = json.loads(keywords_json)
                else:
                    keywords_data = {}
            except (json.JSONDecodeError, UnicodeDecodeError, TypeError) as e:
                print(f"Warning: Failed to decode keywords for universe {cluster_id}: {e}")
                keywords_data = {}

            label = name or f"Universe {cluster_id}"

            if label not in universe_by_name:
                # First occurrence of this name
                universe_by_name[label] = {
                    "id": f"universe_{cluster_id}",
                    "label": label,
                    "type": "universe",
                    "cluster_id": cluster_id,
                    "cluster_ids": [cluster_id],  # Track all cluster_ids with this name
                    "color": keywords_data.get("color", "#4a9eff"),
                    "size": 80,
                    "galaxy_count": keywords_data.get("cluster_count", 0),
                    "description": keywords_data.get("description", ""),
                    "sample_topics": keywords_data.get("sample_topics", []),
                    "zoom_level": "universe"
                }
            else:
                # Merge with existing - add cluster_id to list and sum galaxy counts
                universe_by_name[label]["cluster_ids"].append(cluster_id)
                universe_by_name[label]["galaxy_count"] += keywords_data.get("cluster_count", 0)
                # Merge sample topics
                existing_topics = set(universe_by_name[label]["sample_topics"])
                for topic in keywords_data.get("sample_topics", []):
                    if topic not in existing_topics:
                        universe_by_name[label]["sample_topics"].append(topic)

        universe_nodes = list(universe_by_name.values())

        conn.close()

        return {
            "level": "universe",
            "parent_id": None,
            "nodes": universe_nodes,
            "count": len(universe_nodes)
        }

    # Handle other levels - from analysis cache
    if _analysis_cache is None:
        raise HTTPException(
            status_code=400,
            detail="Analysis not run yet. Call POST /analyze first."
        )

    valid_levels = ["galaxy", "cluster", "topic", "message"]
    if level not in valid_levels:
        raise HTTPException(
            status_code=400,
            detail=f"Invalid zoom level. Must be one of: universe, {valid_levels}"
        )

    nodes = _analysis_cache["nodes"].get(level, [])
    
    # Filter by parent if specified
    if parent_id is not None:
        if level == "galaxy":
            # Filter galaxies by parent universe(s)
            # parent_id can be comma-separated list of cluster_ids (for merged universes)
            import sqlite3
            import json

            conn = sqlite3.connect('mycelica.db')
            cursor = conn.cursor()

            # Parse comma-separated parent_ids
            parent_ids = [int(pid.strip()) for pid in str(parent_id).split(',')]

            galaxy_cluster_ids = set()
            for pid in parent_ids:
                # cluster_id is stored as bytes (little-endian int), so convert parent_id
                parent_id_bytes = pid.to_bytes(4, byteorder='little')
                cursor.execute('''
                    SELECT keywords FROM cluster_names
                    WHERE level = 'universe' AND cluster_id = ?
                ''', (parent_id_bytes,))

                result = cursor.fetchone()
                if result:
                    try:
                        keywords_json = result[0]
                        if isinstance(keywords_json, bytes):
                            keywords_json = keywords_json.decode('utf-8', errors='replace')
                        keywords_data = json.loads(keywords_json)
                        for gid in keywords_data.get("galaxy_clusters", []):
                            galaxy_cluster_ids.add(gid)
                    except (json.JSONDecodeError, UnicodeDecodeError, TypeError) as e:
                        print(f"Warning: Failed to decode keywords for parent {pid}: {e}")

            nodes = [n for n in nodes if n.get("cluster_id") in galaxy_cluster_ids]
            conn.close()
        elif level == "cluster":
            # Filter clusters by parent galaxy
            nodes = [n for n in nodes if n.get("parent_galaxy") == int(parent_id)]
        elif level == "topic":
            # Filter topics by parent galaxy (could extend to cluster parent)
            nodes = [n for n in nodes if n.get("parent_galaxy") == int(parent_id)]
        elif level == "message":
            # Filter messages by parent galaxy
            nodes = [n for n in nodes if n.get("parent_galaxy") == int(parent_id)]
    
    return {
        "level": level,
        "parent_id": parent_id,
        "nodes": nodes,
        "count": len(nodes)
    }


@app.get("/message/{message_id}")
async def get_message_details(message_id: str):
    """Get detailed information about a specific message pair."""
    global _analysis_cache

    if _analysis_cache is None:
        raise HTTPException(
            status_code=400,
            detail="Analysis not run yet. Call POST /analyze first."
        )

    # Find the message in the message-level nodes
    message_nodes = _analysis_cache["nodes"].get("message", [])
    message_node = next((n for n in message_nodes if n["id"] == message_id), None)
    
    if not message_node:
        raise HTTPException(
            status_code=404,
            detail="Message not found"
        )
    
    return message_node


@app.post("/regenerate-tags")
async def regenerate_tags():
    """Regenerate tags for all topics using AI."""
    global _analysis_cache

    if _analysis_cache is None:
        raise HTTPException(
            status_code=400,
            detail="Analysis not run yet. Call POST /analyze first."
        )

    # Check if API key is available
    api_key = os.getenv("ANTHROPIC_API_KEY")
    if not api_key:
        raise HTTPException(
            status_code=400,
            detail="Anthropic API key not set. Set it via POST /api-key first."
        )

    result = await regenerate_all_tags(_analysis_cache)
    return result


@app.get("/analyze-messages/status")
async def get_message_analysis_status():
    """Get the current status of message analysis (how many analyzed vs remaining)."""
    global _analysis_cache

    if _analysis_cache is None:
        raise HTTPException(
            status_code=400,
            detail="Analysis not run yet. Call POST /analyze first."
        )

    # Get pairs from cache metadata
    pairs = _analysis_cache.get("metadata", {}).get("pairs", [])
    if not pairs:
        # Re-extract pairs if not in cache
        from .parser import parse_claude_export
        conversations = parse_claude_export("conversations.json")
        pairs = extract_message_pairs(conversations)

    status = get_analysis_status(pairs)
    return status


@app.post("/analyze-messages")
async def analyze_messages_endpoint():
    """
    Run full AI analysis in 2 phases:
    1. Phase 1: Analyze individual messages (title, summary, tags)
    2. Phase 2: Generate aggregated topic summaries from message analyses

    Already analyzed items are skipped. Progress is logged to console.
    """
    global _analysis_cache

    if _analysis_cache is None:
        raise HTTPException(
            status_code=400,
            detail="Analysis not run yet. Call POST /analyze first."
        )

    # Check if API key is available
    api_key = os.getenv("ANTHROPIC_API_KEY")
    if not api_key:
        raise HTTPException(
            status_code=400,
            detail="Anthropic API key not set. Set it via POST /api-key or in .env file."
        )

    # Get pairs and topic_data from cache
    pairs = _analysis_cache.get("metadata", {}).get("pairs", [])
    topic_data = _analysis_cache.get("metadata", {}).get("topic_data", {})

    if not pairs:
        from .parser import parse_claude_export
        conversations = parse_claude_export("conversations.json")
        pairs = extract_message_pairs(conversations)

    # Run the full 2-phase analysis
    result = await run_full_ai_analysis(pairs, topic_data)

    return result


@app.get("/search")
async def search_conversations(
    q: str,
    limit: int = 10,
    session: AsyncSession = Depends(get_session)
):
    """Semantic search for conversations."""
    global _analysis_cache

    if _analysis_cache is None:
        raise HTTPException(
            status_code=400,
            detail="Analysis not run yet. Call POST /analyze first."
        )

    # Get conversations for search
    result = await session.execute(
        select(Conversation).order_by(Conversation.updated_at.desc())
    )
    conversations = result.scalars().all()

    conv_dicts = [
        {
            "id": c.id,
            "title": c.title,
            "summary": c.summary,
            "message_count": c.message_count
        }
        for c in conversations
    ]

    import numpy as np
    embeddings = np.array(_analysis_cache["embeddings"])

    results = search_similar(q, embeddings, conv_dicts, top_k=limit)

    return {
        "query": q,
        "results": [
            {
                "id": r["conversation"]["id"],
                "title": r["conversation"]["title"],
                "similarity": round(r["similarity"], 3),
                "message_count": r["conversation"]["message_count"]
            }
            for r in results
        ]
    }


@app.get("/graph/surfaced")
async def get_surfaced_gems(
    message_ids: str = "",  # Comma-separated message IDs
    max_findings: int = 5,
    max_quotes: int = 3
):
    """Get surfaced content (findings, quotes, poems) for a set of messages.

    This endpoint returns the "gems" - significant discoveries, memorable quotes,
    and poems - that should be surfaced at higher levels in the hierarchy.
    """
    if not message_ids:
        return {
            "findings": [],
            "quotes": [],
            "poems": [],
            "stats": {
                "total_findings": 0,
                "total_quotes": 0,
                "total_poems": 0
            }
        }

    ids_list = [id.strip() for id in message_ids.split(",") if id.strip()]

    # Get surfaced content from data service
    surfaced = data_service.get_surfaced_content(
        message_ids=ids_list,
        max_findings=max_findings,
        max_quotes=max_quotes
    )

    # Get container stats
    stats = data_service.get_container_stats(ids_list)

    return {
        "findings": surfaced["findings"],
        "quotes": surfaced["quotes"],
        "poems": surfaced["poems"],
        "stats": {
            "total_findings": stats["findings_count"],
            "total_quotes": stats["quotes_count"],
            "total_poems": stats["poems_count"],
            "analyzed_count": stats["analyzed_count"],
            "avg_significance": round(stats["avg_significance"], 2),
            "has_gems": stats["has_gems"]
        }
    }


@app.get("/graph/findings")
async def get_all_findings():
    """Get all findings (significant discoveries) across all messages."""
    findings = data_service.get_findings()
    return {
        "count": len(findings),
        "findings": findings
    }


@app.get("/graph/stats")
async def get_analysis_stats_endpoint():
    """Get overall analysis statistics."""
    from . import db
    stats = db.get_analysis_stats()
    return stats


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8000)
