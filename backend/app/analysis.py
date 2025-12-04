"""
analysis.py - Conversation Analysis & Clustering

Now uses modular internal APIs:
- db.py for database operations
- ai_client.py for AI operations
- data_service.py for high-level data access
"""

import json
import numpy as np
from typing import List, Dict, Any, Tuple
from sklearn.cluster import AgglomerativeClustering
from sklearn.metrics.pairwise import cosine_similarity
from sklearn.feature_extraction.text import TfidfVectorizer
import colorsys
from datetime import datetime
import os
import asyncio

# Import our cute internal APIs
from . import db
from . import ai_client
from . import data_service

print("Analysis module loaded - using modular architecture")


def generate_title_from_content(conversation: Dict[str, Any]) -> str:
    """Generate a title from conversation content for untitled conversations."""
    messages = conversation.get('messages', [])

    # Get first user message as basis for title
    for msg in messages[:5]:
        if msg.get('role') == 'human':
            content = msg.get('content', '').strip()
            if content:
                # Take first line or first 50 chars
                first_line = content.split('\n')[0]
                if len(first_line) > 50:
                    return first_line[:47] + '...'
                return first_line

    # Fallback: first message content
    if messages:
        content = messages[0].get('content', '').strip()
        if content:
            first_line = content.split('\n')[0]
            if len(first_line) > 50:
                return first_line[:47] + '...'
            return first_line

    return 'Empty conversation'


def get_conversation_text(conversation: Dict[str, Any]) -> str:
    """Extract text from conversation for embedding."""
    title = conversation.get('title', '').strip()

    # Don't use title if it's "Untitled" or empty - rely on content only
    parts = []
    if title and title.lower() != 'untitled':
        parts.append(title)

    if conversation.get('summary'):
        parts.append(conversation['summary'])

    # Include message content - more content for better clustering
    messages = conversation.get('messages', [])
    for msg in messages[:50]:  # First 50 messages
        content = msg.get('content', '')[:1000]  # First 1000 chars
        if content:
            parts.append(content)

    return ' '.join(parts)


def is_empty_conversation(conversation: Dict[str, Any]) -> bool:
    """Check if conversation has no meaningful content."""
    text = get_conversation_text(conversation)
    # Filter if less than 50 chars of actual content
    return len(text.strip()) < 50


def generate_embeddings(conversations: List[Dict[str, Any]]) -> np.ndarray:
    """Generate TF-IDF embeddings for all conversations."""
    texts = [get_conversation_text(conv) for conv in conversations]

    # Custom stop words - common but meaningless terms
    custom_stops = [
        'pro', 'plus', 'max', 'mini', 'lite', 'ultra', 'super',
        '3d', '2d', '4k', 'hd', 'vs', 'new', 'old', 'best', 'good',
        'need', 'want', 'like', 'use', 'using', 'used', 'make', 'made',
        'just', 'really', 'actually', 'basically', 'simply',
        'thing', 'things', 'something', 'anything', 'everything',
        'way', 'ways', 'time', 'times', 'day', 'days',
        'know', 'think', 'see', 'look', 'get', 'got', 'going',
        'yeah', 'yes', 'okay', 'ok', 'sure', 'right', 'well',
        'untitled', 'example', 'sample', 'test'
    ]

    # Combine english stop words with custom ones
    from sklearn.feature_extraction.text import ENGLISH_STOP_WORDS
    all_stops = list(ENGLISH_STOP_WORDS) + custom_stops

    # Use TF-IDF for embeddings
    vectorizer = TfidfVectorizer(
        max_features=1000,
        stop_words=all_stops,
        ngram_range=(1, 2),
        min_df=1,
        max_df=0.8  # Lower to filter more common terms
    )

    embeddings = vectorizer.fit_transform(texts).toarray()
    return embeddings


def cluster_conversations(
    embeddings: np.ndarray,
    distance_threshold: float = 0.8
) -> np.ndarray:
    """Cluster conversations based on embedding similarity."""
    if len(embeddings) < 2:
        return np.array([0] * len(embeddings))

    # Use cosine distance for clustering
    from sklearn.metrics.pairwise import pairwise_distances
    distances = pairwise_distances(embeddings, metric='cosine')

    clustering = AgglomerativeClustering(
        n_clusters=None,
        distance_threshold=distance_threshold,
        metric='precomputed',
        linkage='average'
    )
    labels = clustering.fit_predict(distances)
    return labels


def extract_cluster_keywords(
    conversations: List[Dict[str, Any]],
    cluster_labels: np.ndarray,
    top_k: int = 15
) -> Dict[int, List[str]]:
    """Extract keywords for each cluster using TF-IDF."""
    cluster_texts = {}

    for conv, label in zip(conversations, cluster_labels):
        if label not in cluster_texts:
            cluster_texts[label] = []
        # Use same text extraction (already ignores "Untitled")
        cluster_texts[label].append(get_conversation_text(conv))

    cluster_keywords = {}

    # Same stop words as embeddings
    bad_keywords = [
        'pro', 'plus', 'max', 'mini', 'lite', 'ultra', 'super',
        '3d', '2d', '4k', 'hd', 'vs', 'new', 'old', 'best', 'good',
        'need', 'want', 'like', 'use', 'using', 'used', 'make', 'made',
        'just', 'really', 'actually', 'basically', 'simply',
        'thing', 'things', 'something', 'anything', 'everything',
        'way', 'ways', 'time', 'times', 'day', 'days',
        'know', 'think', 'see', 'look', 'get', 'got', 'going',
        'yeah', 'yes', 'okay', 'ok', 'sure', 'right', 'well',
        'untitled', 'example', 'sample', 'test',
        'dumbdroid', 'oneplus', 'buds'  # Brand names that aren't meaningful topics
    ]

    for label, texts in cluster_texts.items():
        combined = ' '.join(texts)

        vectorizer = TfidfVectorizer(
            max_features=200,
            stop_words='english',
            ngram_range=(1, 2)
        )

        try:
            tfidf = vectorizer.fit_transform([combined])
            feature_names = vectorizer.get_feature_names_out()
            scores = tfidf.toarray()[0]

            # Get all keywords with scores
            top_indices = scores.argsort()[-top_k*3:][::-1]
            good_keywords = []
            seo_keywords = []  # Brand names etc - keep for SEO but bury

            for i in top_indices:
                if scores[i] > 0:
                    kw = feature_names[i]
                    # Check if it's a "bad" keyword (brand name, generic term)
                    if any(bad in kw.lower() for bad in bad_keywords):
                        seo_keywords.append(kw)
                    else:
                        good_keywords.append(kw)

            # Combine: good keywords first, then SEO keywords buried at end
            keywords = good_keywords[:top_k] + seo_keywords[:5]

            cluster_keywords[label] = keywords if keywords else [f"Cluster {label}"]
        except Exception as e:
            print(f"Error extracting keywords for cluster {label}: {e}")
            cluster_keywords[label] = [f"Cluster {label}"]

    return cluster_keywords


def generate_cluster_colors(n_clusters: int) -> Dict[int, str]:
    """Generate distinct colors for each cluster."""
    colors = {}

    for i in range(n_clusters):
        # Use HSV for even color distribution
        hue = i / max(n_clusters, 1)
        saturation = 0.7
        value = 0.9

        r, g, b = colorsys.hsv_to_rgb(hue, saturation, value)
        colors[i] = f"#{int(r*255):02x}{int(g*255):02x}{int(b*255):02x}"

    return colors


def find_similar_pairs(
    embeddings: np.ndarray,
    cluster_labels: np.ndarray = None,
    cluster_keywords: Dict[int, List[str]] = None,
    threshold: float = 0.2,
    max_edges: int = 500
) -> List[Tuple[int, int, float]]:
    """Find pairs of similar conversations for edges."""
    similarities = cosine_similarity(embeddings)

    edges = []
    n = len(embeddings)

    for i in range(n):
        for j in range(i + 1, n):
            sim = similarities[i][j]

            # Add keyword bonus if in same cluster or sharing keywords
            keyword_bonus = 0.0
            if cluster_labels is not None and cluster_keywords is not None:
                cluster_i = cluster_labels[i]
                cluster_j = cluster_labels[j]

                # Same cluster bonus
                if cluster_i == cluster_j:
                    keyword_bonus += 0.15
                else:
                    # Check for shared keywords between clusters
                    kw_i = set(cluster_keywords.get(cluster_i, []))
                    kw_j = set(cluster_keywords.get(cluster_j, []))
                    shared = kw_i & kw_j
                    if shared:
                        keyword_bonus += 0.1 * min(len(shared), 3)  # Up to 0.3 bonus

            combined_sim = sim + keyword_bonus
            if combined_sim >= threshold:
                edges.append((i, j, float(combined_sim)))

    # Sort by similarity and take top edges
    edges.sort(key=lambda x: x[2], reverse=True)
    return edges[:max_edges]


# NEW: Message-level analysis functions

def extract_message_pairs(conversations: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Extract individual Q&A pairs from conversations."""
    pairs = []
    
    for conv in conversations:
        messages = conv.get('messages', [])
        conv_id = conv['id']
        
        # Extract pairs (user question + assistant response)
        for i in range(0, len(messages) - 1):
            current_msg = messages[i]
            next_msg = messages[i + 1]
            
            # Look for user -> assistant pairs
            if current_msg.get('role') == 'human' and next_msg.get('role') == 'assistant':
                user_text = current_msg.get('content', '').strip()
                assistant_text = next_msg.get('content', '').strip()
                
                if user_text and assistant_text and len(user_text) > 20:
                    pair = {
                        'id': f"{conv_id}_pair_{i//2}",
                        'conversation_id': conv_id,
                        'conversation_title': conv.get('title', 'Untitled'),
                        'user_query': user_text,
                        'assistant_response': assistant_text,
                        'pair_index': i // 2,
                        'timestamp': current_msg.get('timestamp'),
                        'combined_text': f"Q: {user_text} A: {assistant_text}"
                    }
                    pairs.append(pair)
    
    print(f"Extracted {len(pairs)} message pairs from {len(conversations)} conversations")
    return pairs


def generate_pair_embeddings(pairs: List[Dict[str, Any]]) -> np.ndarray:
    """Generate embeddings for message pairs."""
    texts = [pair['combined_text'] for pair in pairs]
    
    # Custom stop words for message content
    custom_stops = [
        'claude', 'sure', 'help', 'question', 'answer', 'thanks', 'please',
        'could', 'would', 'should', 'might', 'maybe', 'probably',
        'understand', 'explain', 'tell', 'show', 'give', 'provide',
        'great', 'good', 'excellent', 'perfect', 'nice', 'awesome'
    ]
    
    from sklearn.feature_extraction.text import ENGLISH_STOP_WORDS
    all_stops = list(ENGLISH_STOP_WORDS) + custom_stops
    
    vectorizer = TfidfVectorizer(
        max_features=800,
        stop_words=all_stops,
        ngram_range=(1, 2),
        min_df=1,
        max_df=0.7
    )
    
    embeddings = vectorizer.fit_transform(texts).toarray()
    return embeddings


def hierarchical_cluster_pairs(
    pairs: List[Dict[str, Any]], 
    embeddings: np.ndarray
) -> Dict[str, Any]:
    """Create hierarchical clusters at multiple zoom levels."""
    if len(pairs) < 2:
        return {"galaxy": [], "cluster": [], "topic": [], "message": pairs}
    
    # Level 1: Galaxy view (broad topics, ~8-12 clusters)
    galaxy_threshold = 0.85
    galaxy_labels = cluster_conversations(embeddings, distance_threshold=galaxy_threshold)
    
    # Level 2: Cluster view (sub-topics within each galaxy cluster)
    cluster_data = {}
    topic_data = {}
    
    # For each galaxy cluster, create sub-clusters
    for galaxy_id in set(galaxy_labels):
        galaxy_pairs = [pairs[i] for i, label in enumerate(galaxy_labels) if label == galaxy_id]
        galaxy_embeddings = embeddings[[i for i, label in enumerate(galaxy_labels) if label == galaxy_id]]
        
        if len(galaxy_pairs) > 1:
            # Level 2: Cluster within galaxy (threshold 0.75)
            cluster_labels = cluster_conversations(galaxy_embeddings, distance_threshold=0.75)
            
            for cluster_id in set(cluster_labels):
                full_cluster_id = f"g{galaxy_id}_c{cluster_id}"
                cluster_pairs = [galaxy_pairs[i] for i, label in enumerate(cluster_labels) if label == cluster_id]
                cluster_embeddings = galaxy_embeddings[[i for i, label in enumerate(cluster_labels) if label == cluster_id]]
                
                cluster_data[full_cluster_id] = cluster_pairs
                
                # Level 3: Topics within cluster (threshold 0.65)
                if len(cluster_pairs) > 1:
                    topic_labels = cluster_conversations(cluster_embeddings, distance_threshold=0.65)
                    
                    for topic_id in set(topic_labels):
                        full_topic_id = f"g{galaxy_id}_c{cluster_id}_t{topic_id}"
                        topic_pairs = [cluster_pairs[i] for i, label in enumerate(topic_labels) if label == topic_id]
                        topic_data[full_topic_id] = topic_pairs
                else:
                    topic_data[full_cluster_id] = cluster_pairs
        else:
            cluster_data[f"g{galaxy_id}_c0"] = galaxy_pairs
            topic_data[f"g{galaxy_id}_c0_t0"] = galaxy_pairs
    
    return {
        "galaxy_labels": galaxy_labels,
        "cluster_data": cluster_data,
        "topic_data": topic_data,
        "all_pairs": pairs
    }


def load_topic_tags_from_db() -> Dict[str, List[str]]:
    """Load saved topic tags from database. Uses db module."""
    return data_service.get_topic_tags()


async def build_hierarchical_nodes(hierarchy: Dict[str, Any], embeddings: np.ndarray) -> Dict[str, List[Dict]]:
    """Build nodes for each zoom level."""
    galaxy_labels = hierarchy["galaxy_labels"]
    cluster_data = hierarchy["cluster_data"]
    topic_data = hierarchy["topic_data"]
    pairs = hierarchy["all_pairs"]
    
    # Generate colors for galaxy clusters
    n_galaxy = len(set(galaxy_labels))
    galaxy_colors = generate_cluster_colors(n_galaxy)
    
    # Galaxy level nodes (broad topic clusters) with AI naming
    galaxy_nodes = []
    
    # Generate AI names for clusters
    ai_names = await generate_ai_cluster_names(hierarchy)
    
    for galaxy_id in set(galaxy_labels):
        galaxy_pairs = [pairs[i] for i, label in enumerate(galaxy_labels) if label == galaxy_id]
        
        # Use AI-generated name, fallback to generic
        ai_name = ai_names.get(galaxy_id, f"Topic {galaxy_id}")
        
        # Keep old keywords for backward compatibility
        galaxy_keywords_fallback = extract_pair_cluster_keywords(pairs, galaxy_labels)
        keywords = galaxy_keywords_fallback.get(galaxy_id, [ai_name])
        
        galaxy_nodes.append({
            "id": f"galaxy_{galaxy_id}",
            "label": ai_name,
            "type": "galaxy", 
            "cluster_id": int(galaxy_id),  # Convert numpy int64 to Python int
            "color": galaxy_colors[galaxy_id],
            "size": min(20 + len(galaxy_pairs) * 2, 80),
            "pair_count": len(galaxy_pairs),
            "keywords": keywords,
            "zoom_level": "galaxy"
        })
    
    # Cluster level nodes (sub-topics)
    cluster_nodes = []
    for cluster_id, cluster_pairs in cluster_data.items():
        galaxy_id = int(cluster_id.split('_')[0][1:])  # Extract galaxy ID
        
        # Extract keywords from cluster pairs
        cluster_texts = [pair['combined_text'] for pair in cluster_pairs]
        keywords = extract_keywords_from_texts(cluster_texts)
        
        cluster_nodes.append({
            "id": f"cluster_{cluster_id}",
            "label": ", ".join(keywords[:2]) if keywords else f"Cluster {cluster_id}",
            "type": "cluster",
            "parent_galaxy": int(galaxy_id),  # Convert numpy int64 to Python int
            "color": galaxy_colors[galaxy_id],
            "size": min(10 + len(cluster_pairs) * 1.5, 50),
            "pair_count": len(cluster_pairs),
            "keywords": keywords,
            "zoom_level": "cluster"
        })
    
    # Load all cached data from our cute internal APIs
    manual_titles = data_service.get_manual_titles()
    saved_tags = data_service.get_topic_tags()
    message_analysis = data_service.get_message_analysis()
    topic_analysis = data_service.get_topic_analysis()

    # Topic level nodes (specific discussions)
    topic_nodes = []
    for topic_id, topic_pairs in topic_data.items():
        galaxy_id = int(topic_id.split('_')[0][1:])  # Extract galaxy ID

        # Get conversation info from first pair
        first_pair = topic_pairs[0]
        first_pair_id = first_pair['id']
        full_topic_id = f"topic_{topic_id}"

        # Priority for topics: Topic analysis > message analysis > manual title > truncated query
        label = None
        summary = None
        keywords = None
        is_analyzed = False

        # First check topic-level analysis (aggregated from all messages)
        if full_topic_id in topic_analysis:
            t_analysis = topic_analysis[full_topic_id]
            label = t_analysis.get("title")
            summary = t_analysis.get("summary")
            keywords = t_analysis.get("tags", [])
            is_analyzed = t_analysis.get("is_analyzed", False)
        # Fall back to first message's analysis
        elif first_pair_id in message_analysis:
            analysis = message_analysis[first_pair_id]
            label = analysis.get("title")
            summary = analysis.get("summary")
            is_analyzed = analysis.get("is_analyzed", False)
            if analysis.get("tags"):
                keywords = analysis["tags"]
        # Fall back to manual titles
        elif first_pair_id in manual_titles:
            label = manual_titles[first_pair_id]

        # Default label if none found
        if not label:
            label = first_pair['user_query'][:40] + "..." if len(first_pair['user_query']) > 40 else first_pair['user_query']

        # Default keywords if none found
        if not keywords:
            if full_topic_id in saved_tags:
                keywords = saved_tags[full_topic_id]
            else:
                keywords = extract_keywords_from_texts([p['combined_text'] for p in topic_pairs], top_k=5)

        topic_nodes.append({
            "id": full_topic_id,
            "label": label,
            "type": "topic",
            "parent_galaxy": int(galaxy_id),  # Convert numpy int64 to Python int
            "color": galaxy_colors[galaxy_id],
            "size": min(5 + len(topic_pairs), 25),
            "pair_count": len(topic_pairs),
            "zoom_level": "topic",
            "conversation_id": first_pair.get('conversation_id'),
            "conversation_title": first_pair.get('conversation_title'),
            "timestamp": first_pair.get('timestamp'),
            "keywords": keywords,
            "summary": summary,
            "is_analyzed": is_analyzed
        })

    # Message level nodes (individual Q&A pairs)
    message_nodes = []
    for i, pair in enumerate(pairs):
        galaxy_id = galaxy_labels[i]
        msg_id = pair['id']

        # Priority: AI analysis > manual title > truncated query
        label = None
        summary = None
        tags = []
        is_analyzed = False

        if msg_id in message_analysis:
            analysis = message_analysis[msg_id]
            label = analysis.get("title")
            summary = analysis.get("summary")
            tags = analysis.get("tags", [])
            is_analyzed = analysis.get("is_analyzed", False)
        elif msg_id in manual_titles:
            label = manual_titles[msg_id]
        else:
            label = pair['user_query'][:50] + "..." if len(pair['user_query']) > 50 else pair['user_query']

        message_nodes.append({
            "id": msg_id,
            "label": label,
            "type": "message",
            "parent_galaxy": int(galaxy_id),  # Convert numpy int64 to Python int
            "color": galaxy_colors[galaxy_id],
            "size": 8,
            "user_query": pair['user_query'],
            "assistant_response": pair['assistant_response'],
            "conversation_id": pair['conversation_id'],
            "conversation_title": pair['conversation_title'],
            "timestamp": pair['timestamp'],
            "zoom_level": "message",
            "summary": summary,
            "tags": tags,
            "is_analyzed": is_analyzed
        })
    
    return {
        "galaxy": galaxy_nodes,
        "cluster": cluster_nodes,
        "topic": topic_nodes,
        "message": message_nodes
    }


def extract_pair_cluster_keywords(pairs: List[Dict[str, Any]], labels: np.ndarray, top_k: int = 10) -> Dict[int, List[str]]:
    """Extract keywords for clusters of message pairs."""
    cluster_texts = {}
    
    for pair, label in zip(pairs, labels):
        if label not in cluster_texts:
            cluster_texts[label] = []
        cluster_texts[label].append(pair['combined_text'])
    
    cluster_keywords = {}
    
    for label, texts in cluster_texts.items():
        keywords = extract_keywords_from_texts(texts, top_k)
        cluster_keywords[label] = keywords
    
    return cluster_keywords


def extract_keywords_from_texts(texts: List[str], top_k: int = 10) -> List[str]:
    """Extract keywords from a list of texts."""
    if not texts:
        return []
    
    combined = ' '.join(texts)
    
    bad_keywords = [
        'claude', 'sure', 'help', 'question', 'answer', 'thanks', 'please',
        'could', 'would', 'should', 'might', 'maybe', 'probably',
        'understand', 'explain', 'tell', 'show', 'give', 'provide'
    ]
    
    try:
        vectorizer = TfidfVectorizer(
            max_features=100,
            stop_words='english',
            ngram_range=(1, 2),
            min_df=1
        )
        
        tfidf = vectorizer.fit_transform([combined])
        feature_names = vectorizer.get_feature_names_out()
        scores = tfidf.toarray()[0]
        
        top_indices = scores.argsort()[-top_k*2:][::-1]
        good_keywords = []
        
        for i in top_indices:
            if scores[i] > 0:
                kw = feature_names[i]
                if not any(bad in kw.lower() for bad in bad_keywords):
                    good_keywords.append(kw)
                    if len(good_keywords) >= top_k:
                        break
        
        return good_keywords if good_keywords else ["Discussion"]
    except:
        return ["Discussion"]


async def generate_ai_cluster_name(pairs: List[Dict[str, Any]]) -> str:
    """Generate meaningful cluster name using AI."""
    if not ANTHROPIC_AVAILABLE:
        # Fallback to improved keyword extraction
        texts = [pair['combined_text'] for pair in pairs[:5]]
        keywords = extract_keywords_from_texts(texts, 2)
        return ' '.join(keywords[:2]).title() if keywords else "Discussion"
    
    api_key = os.getenv('ANTHROPIC_API_KEY')
    if not api_key:
        print("Warning: ANTHROPIC_API_KEY not set, using fallback naming")
        texts = [pair['combined_text'] for pair in pairs[:5]]
        keywords = extract_keywords_from_texts(texts, 2)
        return ' '.join(keywords[:2]).title() if keywords else "Discussion"
    
    try:
        # Sample 3-5 representative pairs
        sample_pairs = pairs[:min(5, len(pairs))]
        
        # Format sample content for AI
        sample_text = "\n\n".join([
            f"Q: {pair['user_query'][:200]}...\nA: {pair['assistant_response'][:200]}..."
            for pair in sample_pairs
        ])
        
        client = Anthropic(api_key=api_key)
        
        prompt = f"""Here are some Q&A pairs from the same topic cluster:

{sample_text}

Generate a concise, descriptive name (2-4 words) for this topic cluster.
Focus on the main technical/subject matter, not generic words.
Examples: "ESP32 Programming", "Mental Health", "3D Printing", "Language Learning"

Name:"""

        response = await asyncio.to_thread(
            client.messages.create,
            model="claude-3-5-haiku-20241022",
            max_tokens=15,
            messages=[{"role": "user", "content": prompt}]
        )
        
        ai_name = response.content[0].text.strip().strip('"').strip("'")
        print(f"AI generated name: '{ai_name}' for cluster with {len(pairs)} pairs")
        return ai_name if ai_name else "Discussion"
        
    except Exception as e:
        print(f"AI naming failed: {e}, using fallback")
        texts = [pair['combined_text'] for pair in pairs[:5]]
        keywords = extract_keywords_from_texts(texts, 2)
        return ' '.join(keywords[:2]).title() if keywords else "Discussion"


def load_cluster_names_from_db() -> Dict[int, str]:
    """Load manual cluster names from database. Uses db module."""
    return data_service.get_cluster_names()

async def generate_ai_cluster_names(hierarchy: Dict[str, Any]) -> Dict[int, str]:
    """Generate AI names for all galaxy clusters."""
    # First try to load manual names from database
    manual_names = load_cluster_names_from_db()
    
    # If we have manual names, use them (disabled until credits available)
    if manual_names:
        print(f"🔒 Using {len(manual_names)} manual cluster names (AI naming disabled)")
        return manual_names
    
    # Fallback to AI generation (when credits available and enabled)
    galaxy_labels = hierarchy["galaxy_labels"]
    pairs = hierarchy["all_pairs"]
    
    # Group pairs by galaxy cluster
    galaxy_clusters = {}
    for i, pair in enumerate(pairs):
        galaxy_id = galaxy_labels[i]
        if galaxy_id not in galaxy_clusters:
            galaxy_clusters[galaxy_id] = []
        galaxy_clusters[galaxy_id].append(pair)
    
    # Generate names for each cluster
    ai_names = {}
    print(f"\nGenerating AI names for {len(galaxy_clusters)} clusters...")
    
    for galaxy_id, cluster_pairs in galaxy_clusters.items():
        ai_name = await generate_ai_cluster_name(cluster_pairs)
        ai_names[galaxy_id] = ai_name
    
    print("AI cluster naming complete!")
    return ai_names


async def analyze_conversations_hierarchical(conversations: List[Dict[str, Any]]) -> Dict[str, Any]:
    """New hierarchical analysis pipeline for message-level clustering."""
    if not conversations:
        return {"nodes": {"galaxy": [], "cluster": [], "topic": [], "message": []}, "edges": {}, "metadata": {}}
    
    # Filter out empty conversations
    conversations = [c for c in conversations if not is_empty_conversation(c)]
    if not conversations:
        return {"nodes": {"galaxy": [], "cluster": [], "topic": [], "message": []}, "edges": {}, "metadata": {}}
    
    print(f"\n=== Hierarchical Analysis Pipeline ===")
    print(f"Processing {len(conversations)} conversations...")
    
    # Step 1: Extract message pairs
    pairs = extract_message_pairs(conversations)
    if not pairs:
        return {"nodes": {"galaxy": [], "cluster": [], "topic": [], "message": []}, "edges": {}, "metadata": {}}
    
    # Step 2: Generate embeddings for pairs
    print("Generating embeddings for message pairs...")
    embeddings = generate_pair_embeddings(pairs)
    
    # Step 3: Create hierarchical clusters
    print("Creating hierarchical clusters...")
    hierarchy = hierarchical_cluster_pairs(pairs, embeddings)
    
    # Step 4: Build nodes for each zoom level
    print("Building hierarchical nodes...")
    nodes = await build_hierarchical_nodes(hierarchy, embeddings)
    
    # Step 5: Calculate similarities for edges (simplified for now)
    print("Calculating edges...")
    edges = {}
    # TODO: Add edge calculation for each zoom level
    
    print(f"""\n=== Analysis Complete ==="
    - Message pairs: {len(pairs)}
    - Galaxy clusters: {len(nodes['galaxy'])}
    - Sub-clusters: {len(nodes['cluster'])}
    - Topics: {len(nodes['topic'])}
    - Individual messages: {len(nodes['message'])}
    """)
    
    return {
        "nodes": nodes,
        "edges": edges,
        "metadata": {
            "total_pairs": len(pairs),
            "total_conversations": len(conversations),
            "zoom_levels": ["galaxy", "cluster", "topic", "message"],
            "pairs": pairs,  # Include pairs for AI analysis endpoint
            "topic_data": hierarchy.get("topic_data", {})  # Include topic_data for topic analysis
        }
    }


# Keep original function for backward compatibility
def analyze_conversations(conversations: List[Dict[str, Any]]) -> Dict[str, Any]:
    """Backward compatible analysis - now calls hierarchical version."""
    return analyze_conversations_hierarchical(conversations)


async def generate_ai_tags_for_topic(pairs: List[Dict[str, Any]], max_tags: int = 5) -> List[str]:
    """Generate meaningful tags for a topic using AI."""
    if not ANTHROPIC_AVAILABLE:
        return extract_keywords_from_texts([p['combined_text'] for p in pairs], max_tags)

    api_key = os.getenv('ANTHROPIC_API_KEY')
    if not api_key:
        return extract_keywords_from_texts([p['combined_text'] for p in pairs], max_tags)

    try:
        # Sample content from pairs
        sample_pairs = pairs[:min(3, len(pairs))]
        sample_text = "\n\n".join([
            f"User: {pair['user_query'][:300]}\nAssistant: {pair['assistant_response'][:300]}"
            for pair in sample_pairs
        ])

        client = Anthropic(api_key=api_key)

        prompt = f"""Analyze this conversation and generate 3-5 specific, descriptive tags.

Conversation:
{sample_text}

Generate tags that describe:
- The main technology, tool, or subject (e.g., "Python", "React", "AutoHotkey", "ESP32")
- The type of task (e.g., "debugging", "UI design", "API integration", "data parsing")
- The specific feature or concept (e.g., "hotkeys", "state management", "serial communication")

Return ONLY the tags, comma-separated. Be specific - avoid generic words like "code", "help", "feature", "add", "text".

Tags:"""

        response = await asyncio.to_thread(
            client.messages.create,
            model="claude-3-5-haiku-20241022",
            max_tokens=50,
            messages=[{"role": "user", "content": prompt}]
        )

        tags_text = response.content[0].text.strip()
        tags = [t.strip() for t in tags_text.split(',') if t.strip()]
        return tags[:max_tags] if tags else ["Discussion"]

    except Exception as e:
        print(f"AI tag generation failed: {e}")
        return extract_keywords_from_texts([p['combined_text'] for p in pairs], max_tags)


def save_topic_tags_to_db(topic_id: str, tags: List[str], is_ai: bool = True):
    """Save topic tags to database. Uses db module."""
    db.save_topic_tags(topic_id, tags, is_ai)


async def regenerate_all_tags(analysis_cache: Dict[str, Any]) -> Dict[str, Any]:
    """Regenerate tags for all topics using AI and save to database."""
    if not analysis_cache:
        return {"error": "No analysis cache available"}

    topic_nodes = analysis_cache["nodes"].get("topic", [])
    message_nodes = analysis_cache["nodes"].get("message", [])

    # Build a map of topic_id to its message pairs
    # Group messages by their topic (based on conversation_id)
    conv_messages = {}
    for msg in message_nodes:
        conv_id = msg.get("conversation_id")
        if conv_id not in conv_messages:
            conv_messages[conv_id] = []
        conv_messages[conv_id].append({
            "user_query": msg.get("user_query", ""),
            "assistant_response": msg.get("assistant_response", ""),
            "combined_text": f"Q: {msg.get('user_query', '')} A: {msg.get('assistant_response', '')}"
        })

    updated_count = 0
    print(f"\n=== Regenerating Tags for {len(topic_nodes)} Topics ===")

    for i, topic in enumerate(topic_nodes):
        conv_id = topic.get("conversation_id")
        topic_id = topic.get("id", conv_id)  # Use topic ID or conversation ID

        if conv_id and conv_id in conv_messages:
            pairs = conv_messages[conv_id]
            new_tags = await generate_ai_tags_for_topic(pairs)
            topic["keywords"] = new_tags
            updated_count += 1

            # Save to database for persistence
            save_topic_tags_to_db(topic_id, new_tags, is_ai=True)

            if (i + 1) % 50 == 0:
                print(f"  Processed {i + 1}/{len(topic_nodes)} topics...")

    print(f"✅ Regenerated and saved tags for {updated_count} topics")

    return {
        "status": "success",
        "topics_updated": updated_count,
        "sample_tags": [t["keywords"] for t in topic_nodes[:5]]
    }


def search_similar(
    query: str,
    embeddings: np.ndarray,
    conversations: List[Dict[str, Any]],
    top_k: int = 10
) -> List[Dict[str, Any]]:
    """Find conversations most similar to a query using keyword matching."""
    # Simple keyword-based search
    query_lower = query.lower()

    results = []
    for i, conv in enumerate(conversations):
        text = get_conversation_text(conv).lower()
        # Simple relevance score based on keyword presence
        score = 0
        for word in query_lower.split():
            if word in text:
                score += text.count(word) * 0.1

        if score > 0:
            results.append({
                "conversation": conv,
                "similarity": min(score, 1.0)
            })

    # Sort by score
    results.sort(key=lambda x: x['similarity'], reverse=True)
    return results[:top_k]


# =============================================================================
# MESSAGE ANALYSIS - AI-powered comprehensive analysis for each message pair
# =============================================================================

def init_message_analysis_table():
    """Create the message_analysis table. Uses db module (tables auto-init on import)."""
    pass  # db module auto-initializes tables on import


def load_message_analysis_from_db() -> Dict[str, Dict[str, Any]]:
    """Load all analyzed message data from database. Uses data_service cache."""
    return data_service.get_message_analysis()


def save_message_analysis_to_db(
    message_id: str,
    conversation_id: str,
    title: str,
    summary: str,
    tags: List[str],
    user_query_preview: str = ""
):
    """Save message analysis to database. Uses db module."""
    db.save_message_analysis(
        message_id=message_id,
        conversation_id=conversation_id,
        title=title,
        summary=summary,
        tags=tags,
        user_query_preview=user_query_preview
    )


def get_unanalyzed_message_ids() -> set:
    """Get set of message IDs that have already been analyzed. Uses db module."""
    return db.get_analyzed_message_ids()


async def analyze_single_message_pair(
    message_id: str,
    conversation_id: str,
    user_query: str,
    assistant_response: str
) -> Dict[str, Any]:
    """Analyze a single message pair using AI to get title, summary, and tags."""

    api_key = os.getenv('ANTHROPIC_API_KEY')
    if not api_key or not ANTHROPIC_AVAILABLE:
        # Fallback: generate basic data without AI
        return {
            "title": user_query[:50] + "..." if len(user_query) > 50 else user_query,
            "summary": user_query[:100] + "..." if len(user_query) > 100 else user_query,
            "tags": extract_keywords_from_texts([f"{user_query} {assistant_response}"], 5)
        }

    try:
        client = Anthropic(api_key=api_key)

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
3. "tags": 3-5 specific tags describing technologies, concepts, or task types

Be specific with tags - use actual technology names (Python, React, Docker), specific concepts (state management, API design), or task types (debugging, refactoring).

Respond ONLY with valid JSON, no markdown:
{{"title": "...", "summary": "...", "tags": ["tag1", "tag2", "tag3"]}}"""

        response = await asyncio.to_thread(
            client.messages.create,
            model="claude-3-5-haiku-20241022",
            max_tokens=300,
            messages=[{"role": "user", "content": prompt}]
        )

        response_text = response.content[0].text.strip()

        # Parse JSON response
        # Handle potential markdown code blocks
        if response_text.startswith("```"):
            response_text = response_text.split("```")[1]
            if response_text.startswith("json"):
                response_text = response_text[4:]

        result = json.loads(response_text)

        return {
            "title": result.get("title", user_query[:50]),
            "summary": result.get("summary", user_query[:100]),
            "tags": result.get("tags", [])[:5]
        }

    except json.JSONDecodeError as e:
        print(f"JSON parse error for {message_id}: {e}")
        # Fallback
        return {
            "title": user_query[:50] + "..." if len(user_query) > 50 else user_query,
            "summary": user_query[:100] + "..." if len(user_query) > 100 else user_query,
            "tags": extract_keywords_from_texts([f"{user_query} {assistant_response}"], 5)
        }
    except Exception as e:
        print(f"AI analysis error for {message_id}: {e}")
        return {
            "title": user_query[:50] + "..." if len(user_query) > 50 else user_query,
            "summary": user_query[:100] + "..." if len(user_query) > 100 else user_query,
            "tags": extract_keywords_from_texts([f"{user_query} {assistant_response}"], 5)
        }


async def analyze_all_messages(
    pairs: List[Dict[str, Any]],
    batch_size: int = 10,
    progress_callback = None
) -> Dict[str, Any]:
    """
    Analyze all unanalyzed message pairs using AI.

    Args:
        pairs: List of message pairs from extract_message_pairs()
        batch_size: Number of concurrent API calls per batch
        progress_callback: Optional callback(current, total, message_id) for progress updates

    Returns:
        Dict with status, analyzed count, skipped count, and sample results
    """

    # Get already analyzed messages
    analyzed_ids = get_unanalyzed_message_ids()

    # Filter to only unanalyzed pairs
    unanalyzed_pairs = [p for p in pairs if p['id'] not in analyzed_ids]

    total = len(unanalyzed_pairs)
    already_done = len(analyzed_ids)

    print(f"\n{'='*60}")
    print(f"📝 MESSAGE ANALYSIS (Individual Summaries)")
    print(f"{'='*60}")
    print(f"Total message pairs: {len(pairs)}")
    print(f"Already analyzed: {already_done}")
    print(f"To analyze: {total}")
    print(f"{'='*60}")

    if total == 0:
        print(f"✅ All messages already analyzed!")
        return {
            "status": "success",
            "message": "All messages already analyzed",
            "analyzed": 0,
            "skipped": already_done,
            "total": len(pairs)
        }

    analyzed_count = 0
    errors = []

    # Process in batches
    for batch_start in range(0, total, batch_size):
        batch_end = min(batch_start + batch_size, total)
        batch = unanalyzed_pairs[batch_start:batch_end]
        batch_num = (batch_start // batch_size) + 1
        total_batches = (total + batch_size - 1) // batch_size

        print(f"\n📦 Batch {batch_num}/{total_batches} (messages {batch_start + 1}-{batch_end} of {total})")

        # Process batch concurrently
        tasks = []
        for pair in batch:
            task = analyze_single_message_pair(
                message_id=pair['id'],
                conversation_id=pair['conversation_id'],
                user_query=pair['user_query'],
                assistant_response=pair['assistant_response']
            )
            tasks.append((pair, task))

        # Wait for all tasks in batch
        for idx, (pair, task) in enumerate(tasks):
            try:
                result = await task

                # Save to database
                save_message_analysis_to_db(
                    message_id=pair['id'],
                    conversation_id=pair['conversation_id'],
                    title=result['title'],
                    summary=result['summary'],
                    tags=result['tags'],
                    user_query_preview=pair['user_query'][:200]
                )

                analyzed_count += 1
                progress_pct = round((analyzed_count / total) * 100, 1)
                title_preview = result['title'][:40] + "..." if len(result['title']) > 40 else result['title']
                print(f"  [{analyzed_count}/{total}] ({progress_pct}%) ✓ {title_preview}")

                if progress_callback:
                    progress_callback(batch_start + analyzed_count, total, pair['id'])

            except Exception as e:
                errors.append({"message_id": pair['id'], "error": str(e)})
                print(f"  [{batch_start + idx + 1}/{total}] ✗ Error: {e}")

        # Small delay between batches to avoid rate limits
        if batch_end < total:
            await asyncio.sleep(0.5)

    print(f"✅ Analyzed {analyzed_count} messages")
    if errors:
        print(f"⚠️ {len(errors)} errors occurred")

    return {
        "status": "success",
        "analyzed": analyzed_count,
        "skipped": already_done,
        "errors": len(errors),
        "total": len(pairs)
    }


def get_analysis_status(pairs: List[Dict[str, Any]]) -> Dict[str, Any]:
    """Get the current analysis status without performing analysis."""

    analyzed_ids = get_unanalyzed_message_ids()

    total = len(pairs)
    analyzed = len([p for p in pairs if p['id'] in analyzed_ids])

    return {
        "total": total,
        "analyzed": analyzed,
        "remaining": total - analyzed,
        "percent_complete": round((analyzed / total * 100) if total > 0 else 100, 1)
    }


# =============================================================================
# TOPIC ANALYSIS - Aggregated summaries generated from individual message analyses
# =============================================================================

def init_topic_analysis_table():
    """Create the topic_analysis table. Uses db module (tables auto-init on import)."""
    pass  # db module auto-initializes tables on import


def load_topic_analysis_from_db() -> Dict[str, Dict[str, Any]]:
    """Load all analyzed topic data from database. Uses data_service cache."""
    return data_service.get_topic_analysis()


def save_topic_analysis_to_db(
    topic_id: str,
    conversation_id: str,
    title: str,
    summary: str,
    tags: List[str],
    message_count: int
):
    """Save topic analysis to database. Uses db module."""
    db.save_topic_analysis(
        topic_id=topic_id,
        conversation_id=conversation_id,
        title=title,
        summary=summary,
        tags=tags,
        message_count=message_count
    )


def get_analyzed_topic_ids() -> set:
    """Get set of topic IDs that have already been analyzed. Uses db module."""
    return db.get_analyzed_topic_ids()


async def generate_topic_summary(
    topic_id: str,
    conversation_id: str,
    message_summaries: List[Dict[str, str]]
) -> Dict[str, Any]:
    """
    Generate a topic-level summary from individual message summaries.
    This is called AFTER all individual messages have been analyzed.
    """

    api_key = os.getenv('ANTHROPIC_API_KEY')
    if not api_key or not ANTHROPIC_AVAILABLE:
        # Fallback: combine message summaries
        all_tags = []
        for msg in message_summaries:
            all_tags.extend(msg.get("tags", []))
        unique_tags = list(dict.fromkeys(all_tags))[:5]

        return {
            "title": message_summaries[0].get("title", "Discussion") if message_summaries else "Discussion",
            "summary": "; ".join([m.get("summary", "")[:100] for m in message_summaries[:3]]),
            "tags": unique_tags
        }

    try:
        client = Anthropic(api_key=api_key)

        # Combine message summaries for context
        summaries_text = "\n".join([
            f"- {msg.get('title', 'Untitled')}: {msg.get('summary', '')}"
            for msg in message_summaries[:10]  # Limit to 10 messages
        ])

        all_tags = []
        for msg in message_summaries:
            all_tags.extend(msg.get("tags", []))
        tags_text = ", ".join(list(dict.fromkeys(all_tags))[:15])

        prompt = f"""Based on these {len(message_summaries)} related conversation exchanges, create a unified topic summary.

INDIVIDUAL MESSAGE SUMMARIES:
{summaries_text}

TAGS FROM MESSAGES: {tags_text}

Create a JSON response with:
1. "title": A concise title (5-10 words) for this entire topic/conversation thread
2. "summary": A unified summary (80-120 words) that captures the overall theme and progression of the discussion
3. "tags": 4-6 tags that best represent the entire topic (combine and prioritize from individual tags)

Respond ONLY with valid JSON:
{{"title": "...", "summary": "...", "tags": ["tag1", "tag2", ...]}}"""

        response = await asyncio.to_thread(
            client.messages.create,
            model="claude-3-5-haiku-20241022",
            max_tokens=400,
            messages=[{"role": "user", "content": prompt}]
        )

        response_text = response.content[0].text.strip()

        # Handle markdown code blocks
        if response_text.startswith("```"):
            response_text = response_text.split("```")[1]
            if response_text.startswith("json"):
                response_text = response_text[4:]

        result = json.loads(response_text)

        return {
            "title": result.get("title", message_summaries[0].get("title", "Discussion")),
            "summary": result.get("summary", ""),
            "tags": result.get("tags", [])[:6]
        }

    except Exception as e:
        print(f"    ⚠️ Topic summary generation error for {topic_id}: {e}")
        # Fallback
        all_tags = []
        for msg in message_summaries:
            all_tags.extend(msg.get("tags", []))
        return {
            "title": message_summaries[0].get("title", "Discussion") if message_summaries else "Discussion",
            "summary": "; ".join([m.get("summary", "")[:100] for m in message_summaries[:3]]),
            "tags": list(dict.fromkeys(all_tags))[:5]
        }


async def analyze_all_topics(
    topic_data: Dict[str, List[Dict]],
    message_analysis: Dict[str, Dict[str, Any]]
) -> Dict[str, Any]:
    """
    Generate summaries for all topics based on their individual message analyses.
    Must be called AFTER analyze_all_messages has completed.
    """

    # Check which topics are already analyzed
    analyzed_topic_ids = get_analyzed_topic_ids()

    # Build list of topics to analyze
    topics_to_analyze = []
    for topic_id, topic_pairs in topic_data.items():
        full_topic_id = f"topic_{topic_id}"
        if full_topic_id not in analyzed_topic_ids:
            # Get message summaries for this topic
            message_summaries = []
            for pair in topic_pairs:
                msg_id = pair['id']
                if msg_id in message_analysis:
                    message_summaries.append(message_analysis[msg_id])

            # Only analyze if we have message summaries
            if message_summaries:
                topics_to_analyze.append({
                    "topic_id": full_topic_id,
                    "conversation_id": topic_pairs[0].get('conversation_id', ''),
                    "message_summaries": message_summaries
                })

    total = len(topics_to_analyze)
    already_done = len(analyzed_topic_ids)

    print(f"\n{'='*60}")
    print(f"📁 TOPIC ANALYSIS (Phase 2 - Aggregated Summaries)")
    print(f"{'='*60}")
    print(f"Total topics: {len(topic_data)}")
    print(f"Already analyzed: {already_done}")
    print(f"To analyze: {total}")

    if total == 0:
        print(f"✅ All topics already analyzed!")
        return {
            "status": "success",
            "message": "All topics already analyzed",
            "analyzed": 0,
            "skipped": already_done,
            "total": len(topic_data)
        }

    analyzed_count = 0
    errors = []

    for i, topic_info in enumerate(topics_to_analyze):
        topic_id = topic_info["topic_id"]
        conv_id = topic_info["conversation_id"]
        msg_summaries = topic_info["message_summaries"]

        print(f"  [{i+1}/{total}] Analyzing {topic_id} ({len(msg_summaries)} messages)...")

        try:
            result = await generate_topic_summary(
                topic_id=topic_id,
                conversation_id=conv_id,
                message_summaries=msg_summaries
            )

            # Save to database
            save_topic_analysis_to_db(
                topic_id=topic_id,
                conversation_id=conv_id,
                title=result['title'],
                summary=result['summary'],
                tags=result['tags'],
                message_count=len(msg_summaries)
            )

            analyzed_count += 1
            print(f"    ✓ Title: {result['title'][:50]}...")

        except Exception as e:
            errors.append({"topic_id": topic_id, "error": str(e)})
            print(f"    ✗ Error: {e}")

        # Small delay to avoid rate limits
        if i < total - 1:
            await asyncio.sleep(0.3)

    print(f"\n✅ Topic analysis complete: {analyzed_count} topics analyzed")
    if errors:
        print(f"⚠️ {len(errors)} errors occurred")

    return {
        "status": "success",
        "analyzed": analyzed_count,
        "skipped": already_done,
        "errors": len(errors),
        "total": len(topic_data)
    }


async def run_full_ai_analysis(
    pairs: List[Dict[str, Any]],
    topic_data: Dict[str, List[Dict]]
) -> Dict[str, Any]:
    """
    Run complete AI analysis in the correct order:
    1. First: Analyze all individual messages
    2. Then: Generate aggregated topic summaries from message analyses
    """

    print(f"\n{'='*60}")
    print(f"🧠 FULL AI ANALYSIS")
    print(f"{'='*60}")
    print(f"This process runs in 2 phases:")
    print(f"  Phase 1: Individual message analysis (title, summary, tags)")
    print(f"  Phase 2: Aggregated topic summaries (from message summaries)")
    print(f"{'='*60}\n")

    # Phase 1: Analyze individual messages
    print(f"{'='*60}")
    print(f"📝 PHASE 1: MESSAGE ANALYSIS (Individual Summaries)")
    print(f"{'='*60}")

    message_result = await analyze_all_messages(pairs, batch_size=5)

    # Load the updated message analysis
    message_analysis = load_message_analysis_from_db()

    # Phase 2: Generate topic summaries (only if we have message analyses)
    if message_analysis:
        topic_result = await analyze_all_topics(topic_data, message_analysis)
    else:
        print(f"\n⚠️ Skipping topic analysis - no message analyses available")
        topic_result = {"status": "skipped", "reason": "No message analyses available"}

    return {
        "status": "success",
        "phase1_messages": message_result,
        "phase2_topics": topic_result
    }
