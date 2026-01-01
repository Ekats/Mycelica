# TODO

Minor hierarchy improvements identified from post-audit analysis (2024-12-31).

---

## Low Priority

### 1. Rename Catch-All Categories

7 categories have generic names that could be more descriptive:

| Current Name | Location | Suggested Action |
|--------------|----------|------------------|
| "Mixed" | Various depths | Rename based on actual content themes |
| "Tangents" | Various depths | Rename to reflect specific tangent topics |

**How to fix:** Run AI naming pass on these specific categories, or manually rename via `update_node`.

**File:** `src-tauri/src/ai_client.rs` (AI naming logic)

---

### 2. Merge Duplicate "Job Search Strategy" Categories

Two categories with identical names exist at same hierarchy level.

**How to fix:**
```sql
-- Find duplicates
SELECT id, title, parent_id, depth FROM nodes
WHERE title = 'Job Search Strategy' AND is_item = 0;

-- Merge children into one, delete the other
UPDATE nodes SET parent_id = '<keep_id>' WHERE parent_id = '<delete_id>';
DELETE FROM nodes WHERE id = '<delete_id>';
```

---

### 3. Add "Rebuild" to Project Detection Stopwords

"Rebuild" is being detected as a potential project name but it's a generic action word.

**File:** `src-tauri/src/ai_client.rs`

**Location:** Project detection prompt (around line 913)

**Fix:** Add to the exclusion examples in the prompt:
```
Exclude:
- Generic action words (Rebuild, Update, Fix)
```

---

## Completed (2024-12-31)

- [x] Privacy threshold 0.3 → 0.5 (captures Personal tier)
- [x] Configurable clustering thresholds (0.75/0.60 defaults, adjustable in Settings)
- [x] Project detection prompt updated for crDroid/custom ROMs
- [x] Synopsis text wrapping in graph nodes
