from fastapi import FastAPI, UploadFile, File, Depends, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy import select, func
from typing import List
import tempfile
import os

from .database import init_db, get_session
from .models import Conversation, Message, Node, Edge
from .parser import parse_claude_export, create_db_objects
from .analysis import analyze_conversations, search_similar

# Cache for analysis results
_analysis_cache = None

app = FastAPI(title="Mycelica API", version="0.1.0")

# Allow frontend to connect
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)


@app.on_event("startup")
async def startup():
    await init_db()


@app.get("/health")
async def health_check():
    return {"status": "healthy"}


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
    """Run full analysis: embeddings, clustering, and edge detection."""
    global _analysis_cache

    # Get all conversations with messages
    result = await session.execute(
        select(Conversation).order_by(Conversation.updated_at.desc())
    )
    conversations = result.scalars().all()

    # Build conversation dicts with messages
    conv_dicts = []
    for conv in conversations:
        msg_result = await session.execute(
            select(Message)
            .where(Message.conversation_id == conv.id)
            .order_by(Message.created_at)
            .limit(20)  # First 20 messages for analysis
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
                {"content": m.content, "role": m.role}
                for m in messages
            ]
        })

    # Run analysis
    _analysis_cache = analyze_conversations(conv_dicts)

    return {
        "status": "success",
        "nodes": len(_analysis_cache["nodes"]),
        "edges": len(_analysis_cache["edges"]),
        "clusters": len(_analysis_cache["clusters"])
    }


@app.get("/graph/analyzed")
async def get_analyzed_graph():
    """Get the analyzed graph with clusters and edges."""
    global _analysis_cache

    if _analysis_cache is None:
        raise HTTPException(
            status_code=400,
            detail="Analysis not run yet. Call POST /analyze first."
        )

    return {
        "nodes": _analysis_cache["nodes"],
        "edges": _analysis_cache["edges"],
        "clusters": _analysis_cache["clusters"]
    }


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


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8000)
