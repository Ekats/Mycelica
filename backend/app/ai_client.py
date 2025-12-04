"""
ai_client.py - Unified AI Client for Mycelica

One cute place for ALL AI-powered operations.
No more scattered Anthropic client creation!
"""

import os
import json
import asyncio
from typing import Dict, List, Any, Optional

# Try to import Anthropic
try:
    from anthropic import Anthropic
    ANTHROPIC_AVAILABLE = True
except ImportError:
    ANTHROPIC_AVAILABLE = False
    print("Anthropic SDK not available - AI features disabled")


# =============================================================================
# CLIENT MANAGEMENT
# =============================================================================

def get_client() -> Optional[Any]:
    """Get Anthropic client if available and configured."""
    if not ANTHROPIC_AVAILABLE:
        return None

    api_key = os.getenv('ANTHROPIC_API_KEY')
    if not api_key:
        return None

    return Anthropic(api_key=api_key)


def is_ai_available() -> bool:
    """Check if AI features are available."""
    return ANTHROPIC_AVAILABLE and os.getenv('ANTHROPIC_API_KEY') is not None


# =============================================================================
# CORE AI CALLING FUNCTION
# =============================================================================

async def call_ai(
    prompt: str,
    max_tokens: int = 300,
    model: str = "claude-3-5-haiku-20241022"
) -> Optional[str]:
    """
    The ONE function to call AI. Use this everywhere!

    Args:
        prompt: The prompt to send
        max_tokens: Max response tokens
        model: Model to use

    Returns:
        Response text or None if failed
    """
    client = get_client()
    if not client:
        return None

    try:
        response = await asyncio.to_thread(
            client.messages.create,
            model=model,
            max_tokens=max_tokens,
            messages=[{"role": "user", "content": prompt}]
        )
        return response.content[0].text.strip()
    except Exception as e:
        print(f"AI call failed: {e}")
        return None


async def call_ai_json(
    prompt: str,
    max_tokens: int = 300,
    model: str = "claude-3-5-haiku-20241022"
) -> Optional[Dict]:
    """
    Call AI and parse response as JSON.

    Args:
        prompt: The prompt (should ask for JSON response)
        max_tokens: Max response tokens
        model: Model to use

    Returns:
        Parsed JSON dict or None if failed
    """
    response = await call_ai(prompt, max_tokens, model)
    if not response:
        return None

    try:
        # Handle markdown code blocks
        text = response
        if text.startswith("```"):
            text = text.split("```")[1]
            if text.startswith("json"):
                text = text[4:]

        return json.loads(text)
    except json.JSONDecodeError as e:
        print(f"JSON parse error: {e}")
        return None


# =============================================================================
# SPECIALIZED AI OPERATIONS
# =============================================================================

async def analyze_message_pair(
    user_query: str,
    assistant_response: str
) -> Dict[str, Any]:
    """
    Analyze a Q&A pair to extract title, summary, tags, and significance.

    Returns dict with: title, summary, tags, significance_score, is_finding, is_quote, is_poem
    """
    # Truncate for API efficiency
    user_preview = user_query[:1500] if len(user_query) > 1500 else user_query
    assistant_preview = assistant_response[:1500] if len(assistant_response) > 1500 else assistant_response

    prompt = f"""Analyze this Q&A exchange and provide a structured analysis.

USER QUESTION:
{user_preview}

ASSISTANT RESPONSE:
{assistant_preview}

Provide the following in JSON format:
1. "title": A concise title (5-10 words) capturing the main topic/task
2. "summary": A brief summary (50-100 words) of what was discussed/accomplished
3. "tags": 3-5 specific tags (technologies, concepts, task types)
4. "significance_score": 0.0-1.0 rating of how significant/insightful this exchange is
5. "is_finding": true if this contains a key discovery, insight, or breakthrough
6. "is_quote": true if the user shared a memorable quote or saying
7. "is_poem": true if this contains poetry or creative writing

Be specific with tags - use actual technology names (Python, React), specific concepts (state management), or task types (debugging).

Respond ONLY with valid JSON:
{{"title": "...", "summary": "...", "tags": [...], "significance_score": 0.5, "is_finding": false, "is_quote": false, "is_poem": false}}"""

    result = await call_ai_json(prompt)

    if result:
        return {
            "title": result.get("title", user_query[:50]),
            "summary": result.get("summary", user_query[:100]),
            "tags": result.get("tags", [])[:5],
            "significance_score": float(result.get("significance_score", 0.3)),
            "is_finding": bool(result.get("is_finding", False)),
            "is_quote": bool(result.get("is_quote", False)),
            "is_poem": bool(result.get("is_poem", False))
        }

    # Fallback without AI
    return {
        "title": user_query[:50] + "..." if len(user_query) > 50 else user_query,
        "summary": user_query[:100] + "..." if len(user_query) > 100 else user_query,
        "tags": [],
        "significance_score": 0.3,
        "is_finding": False,
        "is_quote": False,
        "is_poem": False
    }


async def generate_topic_summary(
    message_summaries: List[Dict[str, str]]
) -> Dict[str, Any]:
    """
    Generate a topic-level summary from individual message summaries.

    Returns dict with: title, summary, tags, significance_score, findings_count
    """
    # Combine message summaries for context
    summaries_text = "\n".join([
        f"- {msg.get('title', 'Untitled')}: {msg.get('summary', '')}"
        for msg in message_summaries[:10]
    ])

    all_tags = []
    findings_count = 0
    total_significance = 0

    for msg in message_summaries:
        all_tags.extend(msg.get("tags", []))
        if msg.get("is_finding"):
            findings_count += 1
        total_significance += msg.get("significance_score", 0.3)

    avg_significance = total_significance / len(message_summaries) if message_summaries else 0.3
    tags_text = ", ".join(list(dict.fromkeys(all_tags))[:15])

    prompt = f"""Based on these {len(message_summaries)} related conversation exchanges, create a unified topic summary.

INDIVIDUAL MESSAGE SUMMARIES:
{summaries_text}

TAGS FROM MESSAGES: {tags_text}

Create a JSON response with:
1. "title": A concise title (5-10 words) for this entire topic
2. "summary": A unified summary (80-120 words) capturing the overall theme and progression
3. "tags": 4-6 tags that best represent the entire topic
4. "connection_text": 1-2 sentences explaining how these messages connect/relate to each other

Respond ONLY with valid JSON:
{{"title": "...", "summary": "...", "tags": [...], "connection_text": "..."}}"""

    result = await call_ai_json(prompt, max_tokens=400)

    if result:
        return {
            "title": result.get("title", message_summaries[0].get("title", "Discussion")),
            "summary": result.get("summary", ""),
            "tags": result.get("tags", [])[:6],
            "connection_text": result.get("connection_text", ""),
            "significance_score": avg_significance,
            "findings_count": findings_count
        }

    # Fallback without AI
    return {
        "title": message_summaries[0].get("title", "Discussion") if message_summaries else "Discussion",
        "summary": "; ".join([m.get("summary", "")[:100] for m in message_summaries[:3]]),
        "tags": list(dict.fromkeys(all_tags))[:5],
        "connection_text": "",
        "significance_score": avg_significance,
        "findings_count": findings_count
    }


async def generate_cluster_name(
    sample_pairs: List[Dict[str, str]]
) -> str:
    """
    Generate a meaningful name for a cluster based on sample content.

    Args:
        sample_pairs: List of dicts with 'user_query' and 'assistant_response'

    Returns:
        Cluster name (2-4 words)
    """
    sample_text = "\n\n".join([
        f"Q: {pair.get('user_query', '')[:200]}...\nA: {pair.get('assistant_response', '')[:200]}..."
        for pair in sample_pairs[:5]
    ])

    prompt = f"""Here are some Q&A pairs from the same topic cluster:

{sample_text}

Generate a concise, descriptive name (2-4 words) for this topic cluster.
Focus on the main technical/subject matter, not generic words.
Examples: "ESP32 Programming", "Mental Health", "3D Printing", "Language Learning"

Name:"""

    response = await call_ai(prompt, max_tokens=15)

    if response:
        return response.strip().strip('"').strip("'")

    return "Discussion"


async def generate_topic_tags(
    sample_pairs: List[Dict[str, str]],
    max_tags: int = 5
) -> List[str]:
    """
    Generate meaningful tags for a topic.

    Args:
        sample_pairs: List of dicts with content
        max_tags: Maximum number of tags

    Returns:
        List of tags
    """
    sample_text = "\n\n".join([
        f"User: {pair.get('user_query', '')[:300]}\nAssistant: {pair.get('assistant_response', '')[:300]}"
        for pair in sample_pairs[:3]
    ])

    prompt = f"""Analyze this conversation and generate 3-5 specific, descriptive tags.

Conversation:
{sample_text}

Generate tags that describe:
- The main technology, tool, or subject (e.g., "Python", "React", "AutoHotkey", "ESP32")
- The type of task (e.g., "debugging", "UI design", "API integration", "data parsing")
- The specific feature or concept (e.g., "hotkeys", "state management", "serial communication")

Return ONLY the tags, comma-separated. Be specific - avoid generic words like "code", "help", "feature", "add", "text".

Tags:"""

    response = await call_ai(prompt, max_tokens=50)

    if response:
        tags = [t.strip() for t in response.split(',') if t.strip()]
        return tags[:max_tags]

    return []


async def determine_content_complexity(
    sample_text: str
) -> Dict[str, Any]:
    """
    Analyze content to determine its complexity and recommended hierarchy depth.

    Used for dynamic depth feature - coding/philosophy = more levels, fitness = fewer.

    Returns dict with: complexity_score (0-1), recommended_depth (1-5), category_type
    """
    prompt = f"""Analyze this content and determine its complexity for organizing in a mind map.

Content sample:
{sample_text[:1000]}

Rate the content on:
1. "complexity_score": 0.0-1.0 (0=simple/practical, 1=complex/theoretical)
2. "recommended_depth": 1-5 levels of hierarchy needed
3. "category_type": one of ["technical", "philosophical", "creative", "practical", "reference"]

Guidelines:
- Coding, philosophy, theories → higher complexity (0.7-1.0), depth 4-5
- Fitness, recipes, simple how-tos → lower complexity (0.1-0.4), depth 1-2
- Creative writing, poetry → medium complexity (0.5-0.7), depth 2-3

Respond ONLY with valid JSON:
{{"complexity_score": 0.5, "recommended_depth": 3, "category_type": "technical"}}"""

    result = await call_ai_json(prompt, max_tokens=100)

    if result:
        return {
            "complexity_score": float(result.get("complexity_score", 0.5)),
            "recommended_depth": int(result.get("recommended_depth", 3)),
            "category_type": result.get("category_type", "general")
        }

    return {
        "complexity_score": 0.5,
        "recommended_depth": 3,
        "category_type": "general"
    }


# =============================================================================
# BATCH OPERATIONS
# =============================================================================

async def batch_analyze_messages(
    pairs: List[Dict[str, Any]],
    batch_size: int = 5,
    on_progress: callable = None
) -> List[Dict[str, Any]]:
    """
    Analyze multiple message pairs in batches.

    Args:
        pairs: List of pairs with 'user_query' and 'assistant_response'
        batch_size: Number of concurrent API calls per batch
        on_progress: Optional callback(current, total, result)

    Returns:
        List of analysis results
    """
    results = []
    total = len(pairs)

    for batch_start in range(0, total, batch_size):
        batch_end = min(batch_start + batch_size, total)
        batch = pairs[batch_start:batch_end]

        # Process batch concurrently
        tasks = [
            analyze_message_pair(p['user_query'], p['assistant_response'])
            for p in batch
        ]

        batch_results = await asyncio.gather(*tasks)

        for i, result in enumerate(batch_results):
            results.append(result)
            if on_progress:
                on_progress(batch_start + i + 1, total, result)

        # Small delay between batches
        if batch_end < total:
            await asyncio.sleep(0.5)

    return results
