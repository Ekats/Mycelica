//! Persistent tags system for stable clustering across rebuilds
//!
//! Tags are generated from existing item tags (nodes.tags JSON field).
//! Process:
//! 1. Collect unique tags from items
//! 2. Embed and cluster similar tag strings
//! 3. Pick canonical names (most frequent variant)
//! 4. Build controlled vocabulary from frequent tags
//! 5. Create persistent tags with hierarchies

use crate::db::{Database, Tag};
use crate::local_embeddings;
use crate::similarity::{cosine_similarity, compute_centroid};
use chrono::Utc;
use std::collections::HashMap;

/// Generate tags from existing item tag vocabulary
/// Called once after first successful clustering (when tags table is empty)
/// Returns number of tags created
pub fn generate_tags_from_item_vocabulary(db: &Database) -> Result<usize, String> {
    // 1. Skip if tags already exist
    let existing_count = db.count_tags().map_err(|e| e.to_string())?;
    if existing_count > 0 {
        println!("[Tags] Tags already exist ({}), skipping bootstrap", existing_count);
        return Ok(0);
    }

    // 2. Collect all tags from items
    let items = db.get_items().map_err(|e| e.to_string())?;

    // tag_string -> (count, list of item_ids)
    let mut tag_occurrences: HashMap<String, (usize, Vec<String>)> = HashMap::new();

    for item in &items {
        if let Some(tags_json) = &item.tags {
            if let Ok(tags) = serde_json::from_str::<Vec<String>>(tags_json) {
                for tag in tags {
                    let tag_lower = tag.trim().to_lowercase();
                    if tag_lower.is_empty() || tag_lower.len() < 2 {
                        continue;
                    }
                    let entry = tag_occurrences.entry(tag_lower).or_insert((0, Vec::new()));
                    entry.0 += 1;
                    entry.1.push(item.id.clone());
                }
            }
        }
    }

    if tag_occurrences.is_empty() {
        println!("[Tags] No tags found in items");
        return Ok(0);
    }

    println!("[Tags] Found {} unique tag strings across items", tag_occurrences.len());

    // 3. Embed each unique tag
    let tag_strings: Vec<&str> = tag_occurrences.keys().map(|s| s.as_str()).collect();

    println!("[Tags] Embedding {} tags...", tag_strings.len());
    let embeddings = local_embeddings::generate_batch(&tag_strings)
        .map_err(|e| format!("Failed to embed tags: {}", e))?;

    // Build tag -> embedding map
    let tag_embeddings: HashMap<&str, &Vec<f32>> = tag_strings
        .iter()
        .zip(embeddings.iter())
        .map(|(&tag, emb)| (tag, emb))
        .collect();

    // 4. Cluster similar tags (cosine > 0.75) using union-find
    let mut parent: HashMap<&str, &str> = HashMap::new();
    for &tag in &tag_strings {
        parent.insert(tag, tag);
    }

    fn find<'a>(parent: &HashMap<&'a str, &'a str>, x: &'a str) -> &'a str {
        let mut current = x;
        while parent[current] != current {
            current = parent[current];
        }
        current
    }

    // Compare all pairs and merge similar ones
    for i in 0..tag_strings.len() {
        for j in (i + 1)..tag_strings.len() {
            let tag_a = tag_strings[i];
            let tag_b = tag_strings[j];

            let emb_a = tag_embeddings[tag_a];
            let emb_b = tag_embeddings[tag_b];

            let sim = cosine_similarity(emb_a, emb_b);
            if sim > 0.60 {
                let root_a = find(&parent, tag_a);
                let root_b = find(&parent, tag_b);
                if root_a != root_b {
                    // Merge smaller into larger (by occurrence count)
                    let count_a = tag_occurrences[root_a].0;
                    let count_b = tag_occurrences[root_b].0;
                    if count_a >= count_b {
                        parent.insert(root_b, root_a);
                    } else {
                        parent.insert(root_a, root_b);
                    }
                }
            }
        }
    }

    // 5. Group tags by cluster and pick canonical (most frequent)
    let mut clusters: HashMap<&str, Vec<&str>> = HashMap::new();
    for &tag in &tag_strings {
        let root = find(&parent, tag);
        clusters.entry(root).or_default().push(tag);
    }

    println!("[Tags] Clustered into {} canonical groups", clusters.len());

    // For each cluster, pick canonical name and sum occurrences
    // canonical -> (total_count, all_item_ids, centroid)
    let mut canonical_tags: Vec<(String, usize, Vec<String>, Vec<f32>)> = Vec::new();

    for (root, members) in &clusters {
        // Find most frequent variant as canonical
        let canonical = members
            .iter()
            .max_by_key(|&&tag| tag_occurrences[tag].0)
            .unwrap_or(root);

        // Sum total occurrences and collect all item IDs
        let mut total_count = 0;
        let mut all_items: Vec<String> = Vec::new();
        for &member in members {
            let (count, items) = &tag_occurrences[member];
            total_count += count;
            all_items.extend(items.clone());
        }

        // Deduplicate items (an item might have multiple variant tags)
        all_items.sort();
        all_items.dedup();

        // Compute centroid from item embeddings (not tag string embedding)
        let item_embeddings: Vec<Vec<f32>> = all_items.iter()
            .filter_map(|item_id| db.get_node_embedding(item_id).ok().flatten())
            .collect();

        let centroid = if item_embeddings.len() >= 2 {
            let refs: Vec<&[f32]> = item_embeddings.iter().map(|e| e.as_slice()).collect();
            compute_centroid(&refs).unwrap_or_else(|| tag_embeddings[*canonical].clone())
        } else {
            tag_embeddings[*canonical].clone()
        };

        canonical_tags.push((canonical.to_string(), all_items.len(), all_items, centroid));
    }

    // 6. Filter: only tags appearing in 5+ items
    // Long-tail distribution means most meaningful tags have 5-10 items
    let threshold = 5_usize;

    println!("[Tags] Threshold: {} items (fixed minimum)", threshold);

    let mut qualified_tags: Vec<_> = canonical_tags
        .into_iter()
        .filter(|(_, count, _, _)| *count >= threshold)
        .collect();

    // Sort by count descending for hierarchy building
    qualified_tags.sort_by(|a, b| b.1.cmp(&a.1));

    println!("[Tags] {} tags meet threshold", qualified_tags.len());

    if qualified_tags.is_empty() {
        println!("[Tags] No tags meet threshold");
        return Ok(0);
    }

    // 7. Build tag hierarchy (similar canonical tags become parent/child)
    // Larger tag can be parent if similarity > 0.6
    let mut parent_assignments: Vec<Option<usize>> = vec![None; qualified_tags.len()];

    for i in 0..qualified_tags.len() {
        let (_, count_i, _, centroid_i) = &qualified_tags[i];

        for j in 0..i {
            let (_, count_j, _, centroid_j) = &qualified_tags[j];

            let sim = cosine_similarity(centroid_i, centroid_j);
            if sim > 0.6 && count_j > count_i && parent_assignments[i].is_none() {
                parent_assignments[i] = Some(j);
                break;
            }
        }
    }

    // Calculate depths
    fn calc_depth(idx: usize, assignments: &[Option<usize>], memo: &mut HashMap<usize, i32>) -> i32 {
        if let Some(&d) = memo.get(&idx) {
            return d;
        }
        let depth = match assignments[idx] {
            None => 0,
            Some(parent_idx) => 1 + calc_depth(parent_idx, assignments, memo),
        };
        memo.insert(idx, depth);
        depth.min(3) // Cap at 3
    }

    let mut depth_memo: HashMap<usize, i32> = HashMap::new();
    let depths: Vec<i32> = (0..qualified_tags.len())
        .map(|i| calc_depth(i, &parent_assignments, &mut depth_memo))
        .collect();

    // 8. Create Tag records
    let now = Utc::now().timestamp();
    let mut tag_ids: Vec<String> = Vec::new();
    let mut tags_created = 0;

    for (i, (canonical, count, _, centroid)) in qualified_tags.iter().enumerate() {
        let tag_id = format!("tag-{}", canonical.replace(' ', "-"));
        tag_ids.push(tag_id.clone());

        let parent_tag_id = parent_assignments[i].map(|p| tag_ids[p].clone());

        let tag = Tag {
            id: tag_id.clone(),
            title: canonical.clone(),
            parent_tag_id,
            depth: depths[i],
            item_count: *count as i32,
            pinned: false,
            created_at: now,
            updated_at: now,
        };

        if let Err(e) = db.insert_tag(&tag) {
            println!("[Tags] Failed to insert tag {}: {}", canonical, e);
            continue;
        }

        if let Err(e) = db.update_tag_centroid(&tag_id, centroid) {
            println!("[Tags] Failed to store centroid for {}: {}", canonical, e);
        }

        tags_created += 1;
    }

    println!("[Tags] Created {} tags", tags_created);

    // 9. Create item_tags mappings
    let mut assignments_created = 0;
    for (i, (_, _, item_ids, _)) in qualified_tags.iter().enumerate() {
        let tag_id = &tag_ids[i];
        for item_id in item_ids {
            if let Err(e) = db.insert_item_tag(item_id, tag_id, 1.0, "vocabulary") {
                println!("[Tags] Failed to assign {} to {}: {}", item_id, tag_id, e);
            } else {
                assignments_created += 1;
            }
        }
    }

    println!("[Tags] Created {} item-tag assignments", assignments_created);

    // 10. Additional centroid-based assignment (items near tag centroids)
    // This gives items more tags beyond just their direct vocabulary matches
    let mut centroid_assignments = 0;
    for item in &items {
        let item_embedding = match db.get_node_embedding(&item.id) {
            Ok(Some(emb)) => emb,
            _ => continue,
        };

        for (i, (_, _, _, centroid)) in qualified_tags.iter().enumerate() {
            let sim = cosine_similarity(&item_embedding, centroid);
            if sim > 0.40 {
                let tag_id = &tag_ids[i];
                // INSERT OR IGNORE - don't overwrite vocabulary assignments (confidence 1.0)
                if let Ok(true) = db.insert_item_tag_if_not_exists(&item.id, tag_id, sim as f64, "centroid") {
                    centroid_assignments += 1;
                }
            }
        }
    }

    println!("[Tags] Added {} centroid-based assignments", centroid_assignments);

    // Update item counts
    for tag_id in &tag_ids {
        let _ = db.update_tag_item_count(tag_id);
    }

    Ok(tags_created)
}

/// Get tag similarity bonus for clustering
/// Returns bonus to add to similarity score (0.0 if no shared tags)
pub fn get_tag_similarity_bonus(db: &Database, item_a_id: &str, item_b_id: &str) -> f64 {
    match db.count_shared_tags(item_a_id, item_b_id) {
        Ok(count) => count as f64 * 0.08, // +0.08 per shared tag
        Err(_) => 0.0,
    }
}

/// Convert a string to title case
fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Get tag anchors for uber-category creation
/// Returns titles of L0/L1 tags with 15+ items (substantial tags only)
pub fn get_tag_anchors(db: &Database) -> Vec<String> {
    // Only substantial tags (15+ items) should influence uber-category names
    // Smaller tags still provide clustering bonus but don't anchor categories
    let threshold = 15;

    match db.get_tags_by_depth(0, 1) {
        Ok(tags) => tags
            .into_iter()
            .filter(|t| t.item_count >= threshold)
            .map(|t| title_case(&t.title))
            .collect(),
        Err(_) => Vec::new(),
    }
}
