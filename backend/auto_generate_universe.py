#!/usr/bin/env python3
"""
Automatically generate universe categories by clustering existing galaxy clusters
based on their semantic content and keywords
"""

import sqlite3
import json
from datetime import datetime
from sklearn.feature_extraction.text import TfidfVectorizer
from sklearn.cluster import KMeans
from sklearn.metrics.pairwise import cosine_similarity
import numpy as np

def load_galaxy_clusters():
    """Load existing galaxy cluster names and keywords from database"""
    conn = sqlite3.connect('mycelica.db')
    cursor = conn.cursor()
    
    cursor.execute('''
        SELECT cluster_id, name FROM cluster_names 
        WHERE level = 'galaxy' 
        ORDER BY cluster_id
    ''')
    
    clusters = []
    for cluster_id, name in cursor.fetchall():
        clusters.append({
            'id': cluster_id,
            'name': name,
            'text_content': name  # Use the cluster name as content for now
        })
    
    conn.close()
    return clusters

def cluster_galaxy_into_universe(galaxy_clusters, n_universe_clusters=8):
    """Cluster galaxy clusters into universe-level categories"""
    
    # Prepare text data for clustering
    texts = [cluster['text_content'] for cluster in galaxy_clusters]
    
    # Create TF-IDF embeddings of cluster names
    vectorizer = TfidfVectorizer(
        max_features=200,
        stop_words='english',
        ngram_range=(1, 2),
        min_df=1
    )
    
    embeddings = vectorizer.fit_transform(texts).toarray()
    
    # K-means clustering to group galaxies into universes
    kmeans = KMeans(n_clusters=n_universe_clusters, random_state=42, n_init=10)
    universe_labels = kmeans.fit_predict(embeddings)
    
    # Group clusters by universe
    universe_groups = {}
    for i, galaxy_cluster in enumerate(galaxy_clusters):
        universe_id = universe_labels[i]
        if universe_id not in universe_groups:
            universe_groups[universe_id] = []
        universe_groups[universe_id].append(galaxy_cluster)
    
    return universe_groups, embeddings, vectorizer

def generate_universe_name(galaxy_clusters_in_universe):
    """Generate a meaningful name for a universe based on its galaxy clusters"""
    
    # Combine all cluster names
    combined_text = ' '.join([cluster['name'] for cluster in galaxy_clusters_in_universe])
    
    # Extract key terms
    words = combined_text.lower().split()
    word_freq = {}
    
    # Common stop words for this domain
    stop_words = {'the', 'and', 'or', 'but', 'in', 'on', 'at', 'to', 'for', 'of', 'with', 'by', '&'}
    
    for word in words:
        word = word.strip('.,()[]{}')
        if len(word) > 3 and word not in stop_words:
            word_freq[word] = word_freq.get(word, 0) + 1
    
    # Get most common meaningful words
    top_words = sorted(word_freq.items(), key=lambda x: x[1], reverse=True)[:3]
    
    # Create category name based on patterns
    galaxy_names = [cluster['name'] for cluster in galaxy_clusters_in_universe]
    
    # Pattern matching for better names
    if any('development' in name.lower() or 'programming' in name.lower() or 'code' in name.lower() 
           for name in galaxy_names):
        return "Software Development"
    elif any('system' in name.lower() or 'admin' in name.lower() or 'database' in name.lower()
             for name in galaxy_names):
        return "System Administration" 
    elif any('hardware' in name.lower() or 'electronic' in name.lower() or 'device' in name.lower()
             for name in galaxy_names):
        return "Hardware & Electronics"
    elif any('health' in name.lower() or 'personal' in name.lower() or 'wellness' in name.lower()
             for name in galaxy_names):
        return "Personal & Lifestyle"
    elif any('ai' in name.lower() or 'data' in name.lower() or 'machine' in name.lower()
             for name in galaxy_names):
        return "AI & Data"
    elif any('game' in name.lower() or 'creative' in name.lower() or 'design' in name.lower()
             for name in galaxy_names):
        return "Creative & Gaming"
    elif any('language' in name.lower() or 'translation' in name.lower() or 'estonian' in name.lower()
             for name in galaxy_names):
        return "Language & Communication"
    elif any('work' in name.lower() or 'job' in name.lower() or 'career' in name.lower()
             for name in galaxy_names):
        return "Work & Career"
    else:
        # Fallback to most common words
        if top_words:
            return f"{top_words[0][0].title()} & {top_words[1][0].title() if len(top_words) > 1 else 'More'}"
        else:
            return "General Topics"

def save_universe_categories(universe_groups):
    """Save discovered universe categories to database"""
    
    conn = sqlite3.connect('mycelica.db')
    cursor = conn.cursor()
    
    # Clear existing universe categories
    cursor.execute('DELETE FROM cluster_names WHERE level = "universe"')
    
    current_time = datetime.utcnow().isoformat()
    
    universe_categories = []
    colors = ["#4a9eff", "#ff6b4a", "#4aff6b", "#ff4a9e", "#9eff4a", "#4aff9e", "#ff9e4a", "#9e4aff"]
    
    for universe_id, galaxy_clusters in universe_groups.items():
        universe_name = generate_universe_name(galaxy_clusters)
        galaxy_ids = [cluster['id'] for cluster in galaxy_clusters]
        
        universe_data = {
            "description": f"Automatically discovered category containing {len(galaxy_clusters)} topics",
            "color": colors[universe_id % len(colors)],
            "galaxy_clusters": galaxy_ids,
            "cluster_count": len(galaxy_clusters),
            "sample_topics": [cluster['name'] for cluster in galaxy_clusters[:3]]
        }
        
        # Use universe_id + 1000 to avoid conflicts with galaxy IDs
        cursor.execute('''
            INSERT INTO cluster_names (cluster_id, level, name, keywords, created_at, is_manual)
            VALUES (?, ?, ?, ?, ?, ?)
        ''', (universe_id + 1000, 'universe', universe_name, 
              json.dumps(universe_data), current_time, 1))
        
        universe_categories.append({
            'id': universe_id + 1000,
            'name': universe_name,
            'galaxy_count': len(galaxy_clusters),
            'galaxies': [cluster['name'] for cluster in galaxy_clusters]
        })
    
    conn.commit()
    conn.close()
    
    return universe_categories

def auto_generate_universe_categories():
    """Main function to automatically generate universe categories"""
    
    print("🔍 Loading existing galaxy clusters...")
    galaxy_clusters = load_galaxy_clusters()
    print(f"   Found {len(galaxy_clusters)} galaxy clusters")
    
    print("🤖 Clustering galaxies into universe categories...")
    universe_groups, embeddings, vectorizer = cluster_galaxy_into_universe(galaxy_clusters)
    print(f"   Created {len(universe_groups)} universe categories")
    
    print("💾 Saving universe categories to database...")
    universe_categories = save_universe_categories(universe_groups)
    
    print("✅ Universe categories auto-generated!")
    print("\n📊 Discovered Categories:")
    for category in universe_categories:
        print(f"   • {category['name']} ({category['galaxy_count']} topics)")
        print(f"     Examples: {', '.join(category['galaxies'][:3])}...")
        print()

if __name__ == '__main__':
    auto_generate_universe_categories()