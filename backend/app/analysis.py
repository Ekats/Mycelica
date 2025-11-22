import json
import numpy as np
from typing import List, Dict, Any, Tuple
from sklearn.cluster import AgglomerativeClustering
from sklearn.metrics.pairwise import cosine_similarity
from sklearn.feature_extraction.text import TfidfVectorizer
import colorsys

print("Analysis module loaded - using TF-IDF embeddings with auto-titles")


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
    threshold: float = 0.3,
    max_edges: int = 500
) -> List[Tuple[int, int, float]]:
    """Find pairs of similar conversations for edges."""
    similarities = cosine_similarity(embeddings)

    edges = []
    n = len(embeddings)

    for i in range(n):
        for j in range(i + 1, n):
            sim = similarities[i][j]
            if sim >= threshold:
                edges.append((i, j, float(sim)))

    # Sort by similarity and take top edges
    edges.sort(key=lambda x: x[2], reverse=True)
    return edges[:max_edges]


def analyze_conversations(conversations: List[Dict[str, Any]]) -> Dict[str, Any]:
    """
    Full analysis pipeline: embeddings, clustering, keywords, edges.
    Returns graph-ready data structure.
    """
    if not conversations:
        return {"nodes": [], "edges": [], "clusters": []}

    # Filter out empty conversations
    original_count = len(conversations)
    conversations = [c for c in conversations if not is_empty_conversation(c)]
    filtered_count = original_count - len(conversations)

    if filtered_count > 0:
        print(f"Filtered out {filtered_count} empty/minimal conversations")

    if not conversations:
        return {"nodes": [], "edges": [], "clusters": []}

    print(f"Analyzing {len(conversations)} conversations...")

    # Generate embeddings
    print("Generating TF-IDF embeddings...")
    embeddings = generate_embeddings(conversations)
    print(f"Embedding shape: {embeddings.shape}")

    # Cluster
    print("Clustering conversations...")
    cluster_labels = cluster_conversations(embeddings)
    n_clusters = len(set(cluster_labels))
    print(f"Found {n_clusters} clusters")

    # Extract keywords per cluster
    print("Extracting cluster keywords...")
    cluster_keywords = extract_cluster_keywords(conversations, cluster_labels)

    # Generate colors
    colors = generate_cluster_colors(n_clusters)

    # Find similar pairs for edges
    print("Finding similar conversation pairs...")
    similar_pairs = find_similar_pairs(embeddings, threshold=0.2)
    print(f"Found {len(similar_pairs)} edges")

    # Build nodes
    nodes = []
    for i, conv in enumerate(conversations):
        cluster_id = int(cluster_labels[i])

        # Always generate title from content for consistency
        title = generate_title_from_content(conv)
        original_title = conv.get('title', '')

        # Keep original title if it's meaningful and different from auto-generated
        if original_title and original_title.lower() != 'untitled' and len(original_title) > 10:
            title = original_title

        label = title[:50] + "..." if len(title) > 50 else title

        nodes.append({
            "id": conv['id'],
            "label": label,
            "title": title,
            "cluster_id": cluster_id,
            "color": colors[cluster_id],
            "size": min(5 + conv.get('message_count', 0) * 0.5, 30),
            "message_count": conv.get('message_count', 0),
            "created_at": conv.get('created_at'),
            "keywords": cluster_keywords.get(cluster_id, [])
        })

    # Build edges
    edges = []
    for i, j, weight in similar_pairs:
        edges.append({
            "source": conversations[i]['id'],
            "target": conversations[j]['id'],
            "weight": weight
        })

    # Build cluster info
    clusters = []
    for cluster_id in range(n_clusters):
        count = sum(1 for label in cluster_labels if label == cluster_id)
        clusters.append({
            "id": cluster_id,
            "keywords": cluster_keywords.get(cluster_id, []),
            "color": colors[cluster_id],
            "count": count,
            "name": ", ".join(cluster_keywords.get(cluster_id, [])[:3])
        })

    # Sort clusters by size
    clusters.sort(key=lambda x: x['count'], reverse=True)

    print("Analysis complete!")

    return {
        "nodes": nodes,
        "edges": edges,
        "clusters": clusters,
        "embeddings": embeddings.tolist()
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
