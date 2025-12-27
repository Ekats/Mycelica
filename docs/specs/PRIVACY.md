# Privacy System

> Generated from `src-tauri/src/commands/privacy.rs` and `src-tauri/src/classification.rs`.

Mycelica uses AI-powered privacy classification to enable sharing subsets of your knowledge graph without exposing personal information.

---

## Overview

The privacy system has two complementary components:

1. **Privacy Scoring** (0.0-1.0 scale): AI-analyzed shareability rating
2. **Content Classification** (visibility tiers): Structural content filtering

These work together: privacy scoring determines *what* can be shared, content classification determines *how* content appears in the graph.

---

## Privacy Scoring

Each node can have a `privacy` score from 0.0 (highly private) to 1.0 (fully public).

### Privacy Tiers

| Score Range | Level | Description | Examples |
|-------------|-------|-------------|----------|
| 0.0 - 0.2 | Highly private | Real names, health, finances | "My therapist said...", "My salary is..." |
| 0.3 - 0.4 | Personal | Work grievances, emotional venting | "I hate my job at [Company]", "Feeling anxious about..." |
| 0.5 - 0.6 | Semi-private | Named projects in neutral context | "Working on Project X at [Company]" |
| 0.7 - 0.8 | Low risk | Technical content with minor context | "Implementing auth for my app" |
| 0.9 - 1.0 | Public | Generic concepts, tutorials | "How React hooks work" |

### Database Fields

> See `SCHEMA.md` for full schema.

- `privacy` (REAL): 0.0-1.0 continuous score
- `privacy_reason` (TEXT): AI explanation of rating
- `is_private` (INTEGER): Deprecated legacy boolean — use `privacy` float instead

---

## Privacy Scoring Commands

### Batch Scoring

```typescript
// Score all unscored items
await invoke('score_privacy_all_items', { forceRescore: false });

// Force rescore everything
await invoke('score_privacy_all_items', { forceRescore: true });
```

Uses batching (25 items per API call) to efficiently process large collections.

### Single Node Analysis

```typescript
const result = await invoke<PrivacyResult>('analyze_node_privacy', { nodeId: 'abc123' });
// { isPrivate: boolean, reason: string | null }
```

### Category-Level Analysis

```typescript
// Analyze categories and propagate to descendants
await invoke('analyze_categories_privacy', { showcaseMode: false });
```

If a category is marked private, all its children inherit that status automatically. This is much faster than scanning individual items.

### Manual Override

```typescript
// Manually set privacy (propagates to descendants)
const result = await invoke<SetPrivacyResult>('set_node_privacy', {
  nodeId: 'abc123',
  isPrivate: true
});
// Returns { affectedIds: string[] } including all propagated descendants
```

---

## Privacy Filtering

### Thresholds

Different operations use different privacy thresholds:

| Operation | Threshold | Meaning |
|-----------|-----------|---------|
| Default clustering | 0.3 | Items below 0.3 excluded from clustering |
| Demo export | 0.7 | Only include clearly public content |
| Showcase mode | 0.9 | Very strict, only generic/educational |

### Export with Privacy Filter

```typescript
// Export database excluding private content
const path = await invoke<string>('export_shareable_db', {
  minPrivacy: 0.7,  // Threshold
  includeTags: ['mycelica', 'tutorial']  // Optional tag whitelist
});
```

The export:
1. Copies the database
2. Deletes nodes with `privacy < threshold`
3. Also deletes nodes with `is_private = 1` (legacy)
4. Removes orphaned edges
5. Vacuums to reclaim space

### Preview Export

```typescript
const preview = await invoke<ExportPreview>('get_export_preview', {
  minPrivacy: 0.7,
  includeTags: ['mycelica']
});
// { included: 150, excluded: 350, unscored: 50 }
```

---

## Showcase Mode

A stricter privacy analysis mode for creating demo databases:

```typescript
await invoke('analyze_all_privacy', { showcaseMode: true });
```

**Normal mode** marks as private:
- Health, medical, mental health topics
- Relationship, dating, family discussions
- Financial details
- Complaints about employers/coworkers
- Personal emotional struggles
- Personal file paths, usernames

**Showcase mode** additionally filters:
- ANY personal names, locations, employers
- Personal projects (except Mycelica itself)
- Career discussions, job searching
- Daily life, routines, preferences
- "I want", "I need", "I feel" statements

Showcase mode keeps ONLY:
- Mycelica development itself
- Pure philosophy, epistemology
- Abstract technical architecture
- AI/ML concepts
- Pure code examples (no personal context)
- Educational explanations

---

## Content Classification

Separate from privacy scoring, content classification determines visibility in the graph UI.

### Visibility Tiers

| Tier | Content Types | Display |
|------|---------------|---------|
| **Visible** | insight, exploration, synthesis, question, planning | Shown in graph |
| **Supporting** | investigation, discussion, reference, creative | Lazy-loaded in Leaf view |
| **Hidden** | debug, code, paste, trivial | Excluded from graph |

### Content Type Definitions

**VISIBLE (original thought):**
- `insight` - Realization, conclusion, crystallized understanding
- `exploration` - Researching, thinking out loud, no firm conclusion
- `synthesis` - Summarizing, connecting threads
- `question` - Inquiry that frames investigation
- `planning` - Roadmap, TODO, intentions

**SUPPORTING (lazy-loaded):**
- `investigation` - Problem-solving focused on fixing
- `discussion` - Back-and-forth Q&A without synthesis
- `reference` - Factual lookup, definitions
- `creative` - Fiction, poetry, roleplay

**HIDDEN (excluded):**
- `debug` - Error messages, stack traces
- `code` - Code blocks, implementations
- `paste` - Logs, terminal output, data dumps
- `trivial` - Greetings, acknowledgments, fragments

### Classification Method

Content is classified using pattern matching (not AI) for speed and consistency. The classifier checks:

1. Content length (< 100 chars = trivial)
2. Conversational patterns (`Human:`, `A:`, question marks)
3. Specific keyword patterns for each type
4. Code-like patterns (indentation, syntax markers)
5. Debug patterns (stack traces, error formats)

---

## Privacy + Classification Interaction

Both systems work together:

```
                    ┌─────────────────────────────────┐
                    │         Node Content            │
                    └────────────────┬────────────────┘
                                     │
              ┌──────────────────────┼──────────────────────┐
              │                      │                      │
              ▼                      ▼                      ▼
     ┌────────────────┐    ┌────────────────┐    ┌────────────────┐
     │    Privacy     │    │  Content Type  │    │   Visibility   │
     │   0.0 - 1.0    │    │  (13 types)    │    │   (3 tiers)    │
     └────────────────┘    └────────────────┘    └────────────────┘
              │                      │                      │
              └──────────────────────┼──────────────────────┘
                                     ▼
     ┌───────────────────────────────────────────────────────────────┐
     │ Display Decision:                                             │
     │ - Privacy < threshold? → Don't export                         │
     │ - Hidden content type? → Don't show in graph                  │
     │ - Supporting type? → Show only in Leaf view                   │
     │ - Visible type + public? → Show everywhere                    │
     └───────────────────────────────────────────────────────────────┘
```

---

## Protected Content

### Recent Notes Protection

User-created notes under "Recent Notes" can be protected from AI processing:

```typescript
// Check protection status
const protected = await invoke<boolean>('get_protect_recent_notes');

// Enable/disable protection
await invoke('set_protect_recent_notes', { protected: true });
```

When enabled:
- Recent Notes are excluded from `process_nodes`
- They keep their original content without AI-generated titles/summaries
- They can still be manually edited

---

## Events

Privacy operations emit progress events:

```typescript
import { listen } from '@tauri-apps/api/event';

// Privacy scanning progress
listen<PrivacyProgressEvent>('privacy-progress', (event) => {
  const { current, total, nodeTitle, isPrivate, reason, status } = event.payload;
  console.log(`[${current}/${total}] ${nodeTitle}: ${isPrivate ? '🔒' : '✓'}`);
});

// Privacy scoring progress
listen<PrivacyScoringProgress>('privacy-scoring-progress', (event) => {
  const { currentBatch, totalBatches, itemsScored, status } = event.payload;
});
```

### Event Statuses

- `processing` - Currently analyzing
- `success` - Analysis complete for this item
- `error` - Analysis failed (API error, parse error)
- `complete` - All items processed
- `cancelled` - User cancelled operation

---

## API Cost Considerations

Privacy analysis uses Claude Haiku (claude-haiku-4-5) for cost efficiency:
- ~$0.0001-0.0003 per item analyzed
- Batched scoring: 25 items per API call
- Token usage tracked in settings

For large collections:
1. Use category-level analysis first (fewer API calls)
2. Inherit privacy to descendants automatically
3. Only scan individual items that need it

---

## Best Practices

1. **Start with categories**: Analyze topic categories first, not individual items
2. **Use thresholds wisely**:
   - 0.3 for internal use (exclude only highly sensitive)
   - 0.7 for sharing (exclude most personal context)
   - 0.9 for demos (only generic content)
3. **Preview before export**: Check counts with `get_export_preview`
4. **Manual overrides**: Mark sensitive categories private manually for instant propagation
5. **Tag filtering**: Combine privacy + tags for precise exports

---

*Last updated: 2025-12-26*
