"""
data_service.py - High-Level Data Service for Mycelica

Combines db.py and ai_client.py for convenient data operations.
The ONE place for "get me the data I need" logic.
"""

from typing import Dict, List, Any, Optional, Tuple
from . import db
from . import ai_client


# =============================================================================
# CACHED DATA LOADERS
# Cache all the data we need in memory for fast access
# =============================================================================

_cache = {
    "message_analysis": None,
    "topic_analysis": None,
    "topic_tags": None,
    "cluster_names": None,
    "manual_titles": None,
    "findings": None
}


def refresh_cache():
    """Reload all cached data from database."""
    global _cache
    _cache["message_analysis"] = db.load_all_message_analysis()
    _cache["topic_analysis"] = db.load_all_topic_analysis()
    _cache["topic_tags"] = db.load_all_topic_tags()
    _cache["cluster_names"] = db.load_cluster_names('galaxy')
    _cache["manual_titles"] = db.load_manual_titles()
    _cache["findings"] = db.get_findings()
    print("Data cache refreshed")


def get_message_analysis() -> Dict[str, Dict[str, Any]]:
    """Get cached message analysis data."""
    if _cache["message_analysis"] is None:
        _cache["message_analysis"] = db.load_all_message_analysis()
    return _cache["message_analysis"]


def get_topic_analysis() -> Dict[str, Dict[str, Any]]:
    """Get cached topic analysis data."""
    if _cache["topic_analysis"] is None:
        _cache["topic_analysis"] = db.load_all_topic_analysis()
    return _cache["topic_analysis"]


def get_topic_tags() -> Dict[str, List[str]]:
    """Get cached topic tags data."""
    if _cache["topic_tags"] is None:
        _cache["topic_tags"] = db.load_all_topic_tags()
    return _cache["topic_tags"]


def get_cluster_names() -> Dict[int, str]:
    """Get cached cluster names."""
    if _cache["cluster_names"] is None:
        _cache["cluster_names"] = db.load_cluster_names('galaxy')
    return _cache["cluster_names"]


def get_manual_titles() -> Dict[str, str]:
    """Get cached manual titles."""
    if _cache["manual_titles"] is None:
        _cache["manual_titles"] = db.load_manual_titles()
    return _cache["manual_titles"]


def get_findings() -> List[Dict[str, Any]]:
    """Get cached findings (significant discoveries)."""
    if _cache["findings"] is None:
        _cache["findings"] = db.get_findings()
    return _cache["findings"]


# =============================================================================
# LABEL RESOLUTION - The ONE place for "what label to show"
# =============================================================================

def resolve_message_label(
    message_id: str,
    user_query: str,
    max_length: int = 50
) -> Tuple[str, bool]:
    """
    Resolve the best label for a message.

    Priority:
    1. AI-generated analysis title
    2. Manual title
    3. Truncated user query

    Returns: (label, is_analyzed)
    """
    message_analysis = get_message_analysis()
    manual_titles = get_manual_titles()

    # Check AI analysis first
    if message_id in message_analysis:
        analysis = message_analysis[message_id]
        if analysis.get("title"):
            return analysis["title"], True

    # Check manual titles
    if message_id in manual_titles:
        return manual_titles[message_id], False

    # Fallback to truncated query
    if len(user_query) > max_length:
        return user_query[:max_length - 3] + "...", False

    return user_query, False


def resolve_topic_label(
    topic_id: str,
    first_message_id: str,
    first_user_query: str,
    max_length: int = 40
) -> Tuple[str, Optional[str], bool]:
    """
    Resolve the best label and summary for a topic.

    Priority:
    1. Topic-level AI analysis (aggregated)
    2. First message's AI analysis
    3. Manual title for first message
    4. Truncated first user query

    Returns: (label, summary, is_analyzed)
    """
    topic_analysis = get_topic_analysis()
    message_analysis = get_message_analysis()
    manual_titles = get_manual_titles()

    full_topic_id = topic_id if topic_id.startswith("topic_") else f"topic_{topic_id}"

    # Check topic-level analysis first (aggregated from all messages)
    if full_topic_id in topic_analysis:
        t_analysis = topic_analysis[full_topic_id]
        return (
            t_analysis.get("title", "Untitled"),
            t_analysis.get("summary"),
            t_analysis.get("is_analyzed", False)
        )

    # Fall back to first message's analysis
    if first_message_id in message_analysis:
        analysis = message_analysis[first_message_id]
        return (
            analysis.get("title", first_user_query[:max_length]),
            analysis.get("summary"),
            analysis.get("is_analyzed", False)
        )

    # Fall back to manual title
    if first_message_id in manual_titles:
        return manual_titles[first_message_id], None, False

    # Fallback to truncated query
    if len(first_user_query) > max_length:
        return first_user_query[:max_length - 3] + "...", None, False

    return first_user_query, None, False


def resolve_topic_keywords(
    topic_id: str,
    fallback_texts: List[str] = None,
    max_keywords: int = 5
) -> List[str]:
    """
    Resolve the best keywords for a topic.

    Priority:
    1. Topic-level AI analysis tags
    2. Saved topic tags
    3. Extracted from texts (if provided)
    """
    from .analysis import extract_keywords_from_texts  # Avoid circular import

    topic_analysis = get_topic_analysis()
    topic_tags = get_topic_tags()

    full_topic_id = topic_id if topic_id.startswith("topic_") else f"topic_{topic_id}"

    # Check topic analysis first
    if full_topic_id in topic_analysis:
        tags = topic_analysis[full_topic_id].get("tags", [])
        if tags:
            return tags[:max_keywords]

    # Check saved tags
    if full_topic_id in topic_tags:
        return topic_tags[full_topic_id][:max_keywords]

    # Extract from texts as fallback
    if fallback_texts:
        return extract_keywords_from_texts(fallback_texts, max_keywords)

    return []


# =============================================================================
# STATS AGGREGATION - Get summary stats for a container
# =============================================================================

def get_container_stats(
    message_ids: List[str]
) -> Dict[str, Any]:
    """
    Get aggregated stats for a container (cluster, topic, etc.)
    based on its child messages.

    Returns dict with: item_count, findings_count, quotes_count, poems_count, avg_significance
    """
    message_analysis = get_message_analysis()

    findings_count = 0
    quotes_count = 0
    poems_count = 0
    total_significance = 0
    analyzed_count = 0

    for msg_id in message_ids:
        if msg_id in message_analysis:
            analysis = message_analysis[msg_id]
            if analysis.get("is_finding"):
                findings_count += 1
            if analysis.get("is_quote"):
                quotes_count += 1
            if analysis.get("is_poem"):
                poems_count += 1
            total_significance += analysis.get("significance_score", 0)
            analyzed_count += 1

    return {
        "item_count": len(message_ids),
        "analyzed_count": analyzed_count,
        "findings_count": findings_count,
        "quotes_count": quotes_count,
        "poems_count": poems_count,
        "avg_significance": total_significance / analyzed_count if analyzed_count > 0 else 0,
        "has_gems": findings_count > 0 or quotes_count > 0 or poems_count > 0
    }


def get_child_preview(
    child_items: List[Dict[str, Any]],
    max_items: int = 3
) -> str:
    """
    Generate preview text showing what's inside a container.

    Args:
        child_items: List of child items with 'label' key
        max_items: Max items to show in preview

    Returns:
        Preview text like "ESP32 setup, WiFi config, +5 more"
    """
    if not child_items:
        return ""

    labels = [item.get("label", "Untitled") for item in child_items[:max_items]]

    if len(child_items) > max_items:
        remaining = len(child_items) - max_items
        return ", ".join(labels) + f", +{remaining} more"

    return ", ".join(labels)


# =============================================================================
# SIGNIFICANCE & SURFACING - "Surface the gems"
# =============================================================================

def get_surfaced_content(
    message_ids: List[str],
    max_findings: int = 3,
    max_quotes: int = 2
) -> Dict[str, List[Dict]]:
    """
    Get significant content to surface from a set of messages.
    Used to show gems at higher levels (not hiding them deep in hierarchy).

    Returns dict with: findings, quotes, poems
    """
    message_analysis = get_message_analysis()

    findings = []
    quotes = []
    poems = []

    for msg_id in message_ids:
        if msg_id in message_analysis:
            analysis = message_analysis[msg_id]

            item = {
                "message_id": msg_id,
                "title": analysis.get("title"),
                "summary": analysis.get("summary"),
                "significance_score": analysis.get("significance_score", 0)
            }

            if analysis.get("is_finding"):
                findings.append(item)
            if analysis.get("is_quote"):
                quotes.append(item)
            if analysis.get("is_poem"):
                poems.append(item)

    # Sort by significance and limit
    findings.sort(key=lambda x: x["significance_score"], reverse=True)
    quotes.sort(key=lambda x: x["significance_score"], reverse=True)
    poems.sort(key=lambda x: x["significance_score"], reverse=True)

    return {
        "findings": findings[:max_findings],
        "quotes": quotes[:max_quotes],
        "poems": poems[:2]
    }


# =============================================================================
# AI-POWERED ANALYSIS - High-level analysis operations
# =============================================================================

async def analyze_and_save_message(
    message_id: str,
    conversation_id: str,
    user_query: str,
    assistant_response: str
) -> Dict[str, Any]:
    """
    Analyze a message pair using AI and save to database.

    Returns the analysis result.
    """
    # Get analysis from AI
    result = await ai_client.analyze_message_pair(user_query, assistant_response)

    # Save to database
    db.save_message_analysis(
        message_id=message_id,
        conversation_id=conversation_id,
        title=result["title"],
        summary=result["summary"],
        tags=result["tags"],
        user_query_preview=user_query[:200],
        significance_score=result["significance_score"],
        is_finding=result["is_finding"],
        is_quote=result["is_quote"],
        is_poem=result["is_poem"]
    )

    # Update cache
    if _cache["message_analysis"] is not None:
        _cache["message_analysis"][message_id] = result

    return result


async def analyze_and_save_topic(
    topic_id: str,
    conversation_id: str,
    message_ids: List[str]
) -> Dict[str, Any]:
    """
    Generate and save a topic-level summary from its messages.

    Returns the analysis result.
    """
    message_analysis = get_message_analysis()

    # Gather message summaries
    message_summaries = []
    for msg_id in message_ids:
        if msg_id in message_analysis:
            message_summaries.append(message_analysis[msg_id])

    if not message_summaries:
        return {"title": "Untitled", "summary": "", "tags": []}

    # Get topic summary from AI
    result = await ai_client.generate_topic_summary(message_summaries)

    # Save to database
    db.save_topic_analysis(
        topic_id=topic_id,
        conversation_id=conversation_id,
        title=result["title"],
        summary=result["summary"],
        tags=result["tags"],
        message_count=len(message_ids),
        child_count=len(message_ids),
        significance_score=result.get("significance_score", 0),
        findings_count=result.get("findings_count", 0)
    )

    # Update cache
    if _cache["topic_analysis"] is not None:
        _cache["topic_analysis"][topic_id] = result

    return result


async def run_full_analysis(
    pairs: List[Dict[str, Any]],
    topic_data: Dict[str, List[Dict]],
    on_message_progress: callable = None,
    on_topic_progress: callable = None
) -> Dict[str, Any]:
    """
    Run complete AI analysis in correct order:
    1. First: Analyze all individual messages
    2. Then: Generate aggregated topic summaries

    Args:
        pairs: List of message pairs
        topic_data: Dict mapping topic_id to list of pairs
        on_message_progress: Optional callback(current, total, result)
        on_topic_progress: Optional callback(current, total, result)

    Returns:
        Dict with status and counts
    """
    print("\n" + "="*60)
    print("FULL AI ANALYSIS")
    print("="*60)

    # Phase 1: Analyze individual messages
    analyzed_ids = db.get_analyzed_message_ids()
    unanalyzed_pairs = [p for p in pairs if p['id'] not in analyzed_ids]

    print(f"\nPhase 1: Messages - {len(unanalyzed_pairs)} to analyze ({len(analyzed_ids)} already done)")

    messages_analyzed = 0
    for i, pair in enumerate(unanalyzed_pairs):
        result = await analyze_and_save_message(
            message_id=pair['id'],
            conversation_id=pair['conversation_id'],
            user_query=pair['user_query'],
            assistant_response=pair['assistant_response']
        )
        messages_analyzed += 1

        if on_message_progress:
            on_message_progress(i + 1, len(unanalyzed_pairs), result)

        # Small delay to avoid rate limits
        if (i + 1) % 5 == 0:
            import asyncio
            await asyncio.sleep(0.5)

    # Refresh cache after messages
    refresh_cache()

    # Phase 2: Generate topic summaries
    analyzed_topic_ids = db.get_analyzed_topic_ids()
    topics_to_analyze = []

    for topic_id, topic_pairs in topic_data.items():
        full_topic_id = f"topic_{topic_id}"
        if full_topic_id not in analyzed_topic_ids:
            message_ids = [p['id'] for p in topic_pairs]
            topics_to_analyze.append((full_topic_id, topic_pairs[0].get('conversation_id', ''), message_ids))

    print(f"\nPhase 2: Topics - {len(topics_to_analyze)} to analyze ({len(analyzed_topic_ids)} already done)")

    topics_analyzed = 0
    for i, (topic_id, conv_id, message_ids) in enumerate(topics_to_analyze):
        result = await analyze_and_save_topic(topic_id, conv_id, message_ids)
        topics_analyzed += 1

        if on_topic_progress:
            on_topic_progress(i + 1, len(topics_to_analyze), result)

        # Small delay
        import asyncio
        await asyncio.sleep(0.3)

    # Final cache refresh
    refresh_cache()

    print(f"\nAnalysis complete: {messages_analyzed} messages, {topics_analyzed} topics")

    return {
        "status": "success",
        "messages_analyzed": messages_analyzed,
        "topics_analyzed": topics_analyzed,
        "messages_skipped": len(analyzed_ids),
        "topics_skipped": len(analyzed_topic_ids)
    }


# =============================================================================
# ANALYSIS STATUS
# =============================================================================

def get_analysis_status(pairs: List[Dict[str, Any]]) -> Dict[str, Any]:
    """Get current analysis status without running analysis."""
    analyzed_ids = db.get_analyzed_message_ids()
    total = len(pairs)
    analyzed = len([p for p in pairs if p['id'] in analyzed_ids])

    stats = db.get_analysis_stats()

    return {
        "total": total,
        "analyzed": analyzed,
        "remaining": total - analyzed,
        "percent_complete": round((analyzed / total * 100) if total > 0 else 100, 1),
        "findings_count": stats["findings_count"],
        "quotes_count": stats["quotes_count"],
        "poems_count": stats["poems_count"]
    }
