#!/usr/bin/env python3
"""
Create universe-level categories that group the existing galaxy clusters
"""

import sqlite3
import json
from datetime import datetime

# Universe-level categories grouping our 125 galaxy clusters
UNIVERSE_CATEGORIES = {
    "Technology & Development": {
        "description": "Programming, web development, software engineering",
        "clusters": [0, 3, 8, 19, 22, 33, 38, 43, 45, 52, 53, 59, 75, 93, 105, 106, 112, 114, 116, 117],
        "color": "#4a9eff"
    },
    "System Administration": {
        "description": "Operating systems, databases, networking, servers", 
        "clusters": [1, 15, 20, 21, 46, 58, 61, 82, 92, 94, 121],
        "color": "#ff6b4a"
    },
    "Hardware & Electronics": {
        "description": "3D printing, electronics, hardware design, devices",
        "clusters": [6, 18, 23, 24, 30, 36, 41, 71, 78, 104, 110, 111, 120],
        "color": "#4aff6b"
    },
    "AI & Data Science": {
        "description": "Artificial intelligence, machine learning, data analysis",
        "clusters": [7, 11, 19, 42, 76, 86, 107, 118, 119],
        "color": "#ff4a9e"
    },
    "Personal & Lifestyle": {
        "description": "Health, wellness, personal stories, cooking, exercise",
        "clusters": [9, 10, 13, 32, 47, 48, 55, 60, 63, 70, 88, 98, 99],
        "color": "#9eff4a"
    },
    "Work & Career": {
        "description": "Job applications, career development, professional work",
        "clusters": [4, 5, 17, 49, 50, 51, 75, 77],
        "color": "#4aff9e"
    },
    "Creative & Entertainment": {
        "description": "Gaming, creative writing, design, entertainment",
        "clusters": [26, 27, 54, 56, 62, 81, 85, 89, 109, 115],
        "color": "#ff9e4a"
    },
    "Language & Communication": {
        "description": "Languages, translation, communication, learning",
        "clusters": [35, 65, 67, 84, 119],
        "color": "#9e4aff"
    },
    "Home & DIY": {
        "description": "Home automation, DIY projects, heating, household",
        "clusters": [13, 14, 69, 72, 96],
        "color": "#4a9eff"
    },
    "Science & Research": {
        "description": "Physics, biology, research, academic work", 
        "clusters": [2, 27, 28, 37, 68, 76, 79, 100, 102],
        "color": "#ff4aff"
    },
    "Security & Privacy": {
        "description": "Digital security, privacy, authentication",
        "clusters": [12, 25, 40, 44, 80, 91, 95, 113],
        "color": "#4affff"
    },
    "Tools & Utilities": {
        "description": "Development tools, utilities, file management",
        "clusters": [39, 52, 57, 59, 73, 83, 103, 108, 122, 123, 124],
        "color": "#ffff4a"
    }
}

def create_universe_categories():
    """Create universe-level categories in the database"""
    
    conn = sqlite3.connect('mycelica.db')
    cursor = conn.cursor()
    
    # Add universe level to cluster_names table if not exists
    cursor.execute('DELETE FROM cluster_names WHERE level = "universe"')
    
    current_time = datetime.utcnow().isoformat()
    universe_id = 1000  # Start universe IDs at 1000 to avoid conflicts
    
    for category_name, category_data in UNIVERSE_CATEGORIES.items():
        cursor.execute('''
            INSERT INTO cluster_names (cluster_id, level, name, keywords, created_at, is_manual)
            VALUES (?, ?, ?, ?, ?, ?)
        ''', (universe_id, 'universe', category_name, 
              json.dumps({
                  "description": category_data["description"],
                  "color": category_data["color"], 
                  "galaxy_clusters": category_data["clusters"],
                  "cluster_count": len(category_data["clusters"])
              }), current_time, 1))
        
        universe_id += 1
    
    conn.commit()
    conn.close()
    
    print(f"✅ Created {len(UNIVERSE_CATEGORIES)} universe categories")
    
    # Print mapping for verification
    print("\nUniverse → Galaxy Mapping:")
    for category_name, category_data in UNIVERSE_CATEGORIES.items():
        print(f"  {category_name}: {len(category_data['clusters'])} clusters")

if __name__ == '__main__':
    create_universe_categories()