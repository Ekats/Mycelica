# Mycelica Certainty Scoring System

## Problem Statement

When importing conversation history into Mycelica's knowledge graph, not all content is equally reliable. Some exchanges represent:
- Confirmed decisions that were implemented
- Speculative plans that were never realized
- Abandoned investigations
- Contested conclusions where user and AI disagreed

Currently, Mycelica classifies **content type** (idea, code, debug, paste) but not **epistemic status**. This plan adds certainty scoring to surface verified knowledge and deprioritize stale speculation.

## Goals

1. Score nodes on multiple certainty dimensions (not a single magic number)
2. Minimize API costs — use embedding geometry first, LLM only for ambiguous cases
3. Enable filtering/ranking by certainty in search and navigation
4. Link plans to their eventual realizations (or mark as superseded)
5. Cross-reference claims against codebase for grounding

---

## Certainty Dimensions

### Core Scores (all floats 0.0-1.0)

| Dimension | Measures | Detection Method |
|-----------|----------|------------------|
| `agreement` | Did user and AI converge? | Embedding direction projection |
| `certainty` | How confident was the conclusion? | Embedding direction projection |
| `resolution` | Was it concluded or abandoned? | Final message analysis |
| `grounding` | Can it be verified against code? | Codebase cross-reference |
| `validity` | Is referenced code still current? | File modification dates |

### Epistemic Type (categorical)

| Type | Description | Example |
|------|-------------|---------|
| `decision` | Explicit choice made | "We'll use HAC clustering" |
| `observation` | Something discovered/confirmed | "Found the bug — it was the batch size" |
| `exploration` | Trying approaches, no conclusion | "What if we used a different algorithm?" |
| `speculation` | Hypothetical, untested | "Maybe consciousness is quantum" |
| `plan` | Future intent | "Next I'll implement the UI" |

### Plan Status (for epistemic_type = plan)

| Status | Meaning |
|--------|---------|
| `pending` | Not yet realized, still plausible |
| `realized` | Later observation confirms implementation |
| `superseded` | Later decision contradicts this plan |
| `stale` | >90 days old with no follow-up |

---

## Embedding-Based Detection

### Semantic Direction Vectors

The embedding model encodes semantic relationships. We exploit this by defining **directions** in embedding space:

```
agreement_direction = center(agreement_phrases) - center(disagreement_phrases)
certainty_direction = center(certain_phrases) - center(uncertain_phrases)
resolution_direction = center(resolved_phrases) - center(ongoing_phrases)
action_direction = center(action_phrases) - center(discussion_phrases)
```

Scoring any text = project its embedding onto the direction vector.

### Why Few Phrases Work

**The phrases are seeds, not dictionaries.**

The embedding model already learned that "maybe", "perhaps", "possibly", "might be", "could be" are semantically similar — they cluster together in 384-dimensional space. Computing `mean(["maybe", "possibly", "not sure"])` finds the approximate center of that cluster.

Adding 50 more synonyms moves the center by ~0.01. Diminishing returns.

**What matters:**
1. **Coverage of semantic subtypes** — "maybe" (hedging) vs "I wonder" (curiosity) vs "not yet" (temporal) are different flavors. One example of each > 20 synonyms of one.
2. **Avoiding outliers** — One phrase the model places weirdly skews the center. 8 solid phrases > 30 noisy ones.
3. **Balanced poles** — Similar number of positive/negative phrases for stable direction.

**Validation approach:**
```bash
# Test that known phrases score correctly
mycelica-cli certainty test-phrase "definitely" --direction certainty
# Should output: 0.85+ 

mycelica-cli certainty test-phrase "maybe" --direction certainty  
# Should output: 0.30-

mycelica-cli certainty test-phrase "I think so" --direction certainty
# Should output: 0.50-0.65 (middle ground)
```

If edge cases score wrong, add targeted phrases to shift the center — don't exhaustively list synonyms.

### Distance Interpretation

Scores are continuous, not binary. The projection gives natural gradation:

| Score | Interpretation | Example phrases |
|-------|----------------|-----------------|
| 0.90+ | Strong signal | "definitely", "absolutely", "confirmed" |
| 0.70-0.89 | Moderate | "I think", "pretty sure", "looks like" |
| 0.50-0.69 | Weak/neutral | "might", "could", "possibly" |
| 0.30-0.49 | Leaning negative | "not sure", "uncertain", "hard to say" |
| <0.30 | Strong negative | "no idea", "wrong", "doubt it" |

This granularity comes free from the embedding geometry — no calibration needed.

### Bootstrap Phrases

#### Agreement Direction
**Positive (agreement):**
- "yes exactly", "that's right", "perfect", "makes sense"
- "I agree", "that works", "good idea", "let's do that"

**Negative (disagreement):**
- "no that's wrong", "I disagree", "not quite", "wait no"
- "that's not what I meant", "you misunderstood"

#### Certainty Direction
**Positive (certain):**
- "definitely", "absolutely", "I'm sure", "it's clear that"
- "the answer is", "this proves", "confirmed"

**Negative (uncertain):**
- "maybe", "I'm not sure", "possibly", "it might be"
- "I wonder if", "hard to say", "unclear"

#### Resolution Direction
**Positive (resolved):**
- "done", "fixed it", "that works now", "problem solved"
- "shipped", "implemented", "finished"

**Negative (ongoing):**
- "still working on", "not yet", "in progress", "TODO"
- "need to figure out", "will revisit"

#### Action Direction
**Positive (action):**
- "I implemented", "the code is", "here's the fix"
- "committed", "deployed", "built"

**Negative (discussion):**
- "we should consider", "what about", "theoretically"
- "the idea is", "conceptually"

### Convergence Score

Track embedding distance between user and AI across a conversation:

```
convergence = negative slope of distance over time
```

- Decreasing distance = converging (agreement)
- Increasing distance = diverging (disagreement or topic shift)

### Final Message Analysis

The last human message in a conversation strongly signals resolution:

| Pattern | Signal |
|---------|--------|
| Short + thanks/acknowledgment | Resolved |
| Question | Unresolved |
| "nevermind", "forget it" | Abandoned |
| Continues to different topic | Pivoted |

---

## Code Grounding

### Function/Type Cross-Reference

Extract identifiers mentioned in conversation:
- Function names: `generate_embedding`, `build_hierarchy`
- Type names: `Node`, `ClusterConfig`
- File paths: `src/clustering.rs`

Query codebase database:
- `grounding = found_identifiers / mentioned_identifiers`

### Temporal Validity

Compare node creation date vs file modification dates:
- File unchanged since conversation → `validity = 1.0`
- File significantly modified → `validity = similarity(old_content, new_content)`
- File deleted → `validity = 0.0`

---

## Plan Linking

### Detection Algorithm

For each node with `epistemic_type = plan`:

1. Find later nodes with embedding similarity > 0.6
2. Check their epistemic type:
   - If `observation` with high certainty → mark plan as `realized`
   - If `decision` that contradicts → mark plan as `superseded`
3. If no matches and age > 90 days → mark as `stale`

### Edge Creation

When a plan is realized:
```
plan_node --[realized_by]--> observation_node
```

This creates traversable proof: "I said I would do X, and here's where I confirmed I did."

---

## LLM Refinement (Ollama)

### When to Use LLM

Only for genuinely ambiguous cases:
- Embedding agreement score between 0.4-0.6
- Resolution signal unclear
- Convergence score contradicts final message signal

### Conversation-Level Analysis

Don't analyze individual nodes — analyze full conversations:

```
Analyze this conversation for epistemic signals. Return JSON:

CONVERSATION:
{full_exchange}

1. "agreement_level": 0.0-1.0 (did human and AI converge?)
2. "resolution": "resolved" | "abandoned" | "ongoing" | "pivoted"
3. "epistemic_type": "decision" | "exploration" | "observation" | "speculation" | "plan"
4. "key_claims": [list of concrete claims made]

JSON only.
```

### Cost Estimate

- ~300 conversations (not 16k nodes)
- 3-4 seconds per conversation on qwen2.5:7b
- Total: ~15-20 minutes one-time

---

## Database Schema

### New Columns on `nodes`

```sql
ALTER TABLE nodes ADD COLUMN certainty_agreement REAL DEFAULT 0.5;
ALTER TABLE nodes ADD COLUMN certainty_certainty REAL DEFAULT 0.5;
ALTER TABLE nodes ADD COLUMN certainty_resolution REAL DEFAULT 0.5;
ALTER TABLE nodes ADD COLUMN certainty_grounded REAL DEFAULT 0.0;
ALTER TABLE nodes ADD COLUMN certainty_validity REAL DEFAULT 0.5;
ALTER TABLE nodes ADD COLUMN certainty_composite REAL DEFAULT 0.5;

ALTER TABLE nodes ADD COLUMN epistemic_type TEXT;  
-- 'decision', 'observation', 'exploration', 'speculation', 'plan'

ALTER TABLE nodes ADD COLUMN plan_status TEXT;
-- 'pending', 'realized', 'superseded', 'stale'

ALTER TABLE nodes ADD COLUMN realized_by TEXT;
-- node_id that realized this plan (FK to nodes.id)

ALTER TABLE nodes ADD COLUMN llm_verified INTEGER DEFAULT 0;
-- 1 if certainty was refined by LLM
```

### New Table: `semantic_directions`

```sql
CREATE TABLE semantic_directions (
    name TEXT PRIMARY KEY,           -- 'agreement', 'certainty', etc.
    vector BLOB NOT NULL,            -- 384-dim float array
    positive_phrases TEXT NOT NULL,  -- JSON array
    negative_phrases TEXT NOT NULL,  -- JSON array
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    embedding_model TEXT NOT NULL    -- 'all-MiniLM-L6-v2'
);
```

Cache computed directions. Invalidate if embedding model changes.

---

## CLI Commands

### `mycelica-cli certainty bootstrap`

Compute and cache semantic direction vectors.

```
Options:
  --force    Recompute even if cached
```

### `mycelica-cli certainty score`

Score all unscored nodes.

```
Options:
  --limit N          Process N nodes
  --conversation-id  Score specific conversation
  --use-llm          Force LLM refinement for all
  --skip-llm         Embedding-only scoring
```

### `mycelica-cli certainty link-plans`

Find and link realized/superseded plans.

```
Options:
  --dry-run   Show what would be linked
  --stale-days N   Mark plans older than N days as stale (default: 90)
```

### `mycelica-cli certainty stats`

Show certainty distribution.

```
Output:
  Nodes scored: 14,230 / 16,480
  LLM-verified: 892 (6.3%)
  
  Epistemic types:
    decision:    2,341 (16%)
    observation: 4,567 (32%)
    exploration: 3,891 (27%)
    speculation: 1,234 (9%)
    plan:        2,197 (15%)
  
  Plan status:
    realized:   891 (41%)
    superseded: 234 (11%)
    stale:      567 (26%)
    pending:    505 (23%)
  
  Mean certainty by type:
    decision:    0.78
    observation: 0.82
    exploration: 0.45
    speculation: 0.31
    plan:        0.52
```

---

## Implementation Phases

### Phase 1: Schema & Direction Vectors (2-3 hours)

1. Add database columns
2. Create `semantic_directions` table
3. Implement `SemanticDirections::build()`
4. Add `certainty bootstrap` command
5. Test direction vector quality with known examples

### Phase 2: Embedding-Based Scoring (3-4 hours)

1. Implement `score_agreement()`, `score_certainty()`, etc.
2. Implement `conversation_convergence()`
3. Implement `final_message_signal()`
4. Add `certainty score --skip-llm` command
5. Test on subset, validate scores make sense

### Phase 3: Code Grounding (2-3 hours)

1. Implement identifier extraction from content
2. Cross-reference against code intelligence database
3. Implement file modification date comparison
4. Add `grounding` and `validity` scores

### Phase 4: LLM Refinement (2-3 hours)

1. Implement conversation-level LLM prompt
2. Add routing logic (when to use LLM)
3. Add `certainty score --use-llm` option
4. Parse LLM response, update scores

### Phase 5: Plan Linking (2-3 hours)

1. Implement plan→realization detection
2. Create `realized_by` edges
3. Implement superseded detection
4. Implement stale marking
5. Add `certainty link-plans` command

### Phase 6: Composite Score & UI (2-3 hours)

1. Implement weighted composite score by epistemic type
2. Add certainty to search ranking
3. Add visual indicators in graph (opacity, borders)
4. Add certainty filter in GUI

---

## Composite Score Formula

```rust
fn compute_composite(scores: &CertaintyScores, epistemic: EpistemicType) -> f32 {
    match epistemic {
        Observation => {
            // Facts weight toward grounding
            scores.agreement * 0.15 +
            scores.certainty * 0.15 +
            scores.resolution * 0.20 +
            scores.grounding * 0.35 +
            scores.validity * 0.15
        }
        Decision => {
            // Decisions weight toward agreement + resolution
            scores.agreement * 0.30 +
            scores.certainty * 0.20 +
            scores.resolution * 0.30 +
            scores.grounding * 0.15 +
            scores.validity * 0.05
        }
        Plan => {
            // Plans can't be grounded yet
            scores.agreement * 0.40 +
            scores.certainty * 0.30 +
            scores.resolution * 0.25 +
            scores.grounding * 0.05 +
            scores.validity * 0.00
        }
        Exploration => {
            // Explorations are inherently uncertain
            (scores.agreement * 0.30 +
             scores.certainty * 0.30 +
             scores.resolution * 0.40).min(0.7)
        }
        Speculation => {
            // Cap speculation certainty
            (scores.agreement * 0.40 +
             scores.certainty * 0.40 +
             scores.resolution * 0.20).min(0.5)
        }
    }
}
```

---

## Visualization

### Graph View

| Signal | Visual |
|--------|--------|
| Composite certainty | Node opacity (faded = uncertain) |
| Epistemic type | Border style (solid = observation, dashed = plan) |
| Plan status | Badge icon (✓ realized, ⚠ stale, ✗ superseded) |
| LLM-verified | Subtle indicator (dot in corner) |

### Search Results

Sort by: `relevance * certainty_composite`

Show certainty inline:
```
[0.92] Found the clustering bug          decision, resolved
[0.71] Should switch to HAC algorithm    plan, realized ✓
[0.34] Maybe try a neural approach?      speculation
```

### Leaf View

Add certainty panel:
```
┌─────────────────────────────────────┐
│ Certainty                           │
├─────────────────────────────────────┤
│ Agreement:   ████████░░  0.82       │
│ Certainty:   ███████░░░  0.71       │
│ Resolution:  █████████░  0.94       │
│ Grounded:    ██████░░░░  0.63       │
│ Valid:       █████████░  0.91       │
├─────────────────────────────────────┤
│ Type: decision                      │
│ Composite: 0.81                     │
│ [LLM verified]                      │
└─────────────────────────────────────┘
```

---

## Testing Strategy

### Unit Tests

1. Direction vector stability (add/remove phrases, vector should barely move)
2. Known agreement phrases score > 0.7 on agreement direction
3. Known disagreement phrases score < 0.3
4. Convergence calculation on synthetic conversations

### Integration Tests

1. Score a real conversation, verify scores plausible
2. Plan linking finds obvious realizations
3. Code grounding correctly identifies implemented vs speculative

### Manual Validation

Sample 50 nodes across certainty range, manually verify:
- High certainty nodes (>0.8) are actually reliable
- Low certainty nodes (<0.3) are actually speculative/abandoned
- Plans marked "realized" actually were implemented

---

## Open Questions

1. **Direction vector drift**: If we add many more phrases, does the direction improve or get noisier? (Likely: minimal change after ~15 well-chosen phrases per pole)

2. **Cross-conversation linking**: Should plans link to realizations in *different* conversations? (Probably yes, but needs embedding similarity across all nodes, not just within-conversation)

3. **Retroactive invalidation**: If code changes significantly, should we re-score affected nodes automatically?

4. **User override**: Should users be able to manually set certainty? (Sovereign data principle suggests yes)

5. **Embedding model upgrade**: If we switch to a better model, all directions need recomputing. Migration path?

6. **Iterative refinement**: When a phrase scores unexpectedly (e.g., "I suppose" scores 0.7 when it should be ~0.5), add targeted counter-examples to shift the cluster center rather than exhaustively listing synonyms.

---

## Success Criteria

1. Search for "clustering" surfaces implemented approaches before abandoned explorations
2. Plans from 6 months ago that were never mentioned again rank lower than recent confirmations
3. Speculative discussions are clearly distinguishable from verified decisions
4. Zero additional API costs for 80%+ of nodes (embedding-only scoring)
5. Full certainty scoring completes in <30 minutes for 16k nodes

---

## Theoretical Endnote: Toward Grounded Epistemics

This system approximates something humans do naturally: weighting knowledge by how it was acquired and whether it held up over time.

The current design uses three grounding layers:

1. **Conversational** — agreement, resolution, certainty signals from the dialogue itself
2. **Codebase** — cross-reference against what actually exists in source files
3. **Temporal** — staleness, supersession, realization tracking

But there's a fourth layer we're not yet touching: **version control as epistemological record**.

Git history is a timestamped, immutable log of what was actually built. A commit message saying "implement HAC clustering" is stronger evidence than a conversation saying "I'll implement HAC clustering." A revert is evidence of a failed approach. A file surviving 50 commits without major changes suggests stability.

```
Future grounding hierarchy:

Speculation < Plan < Decision < Observation < Code exists < Code committed < Code survived
```

The deeper you can ground a claim, the higher its certainty floor. Conversations are context. Code is evidence. Git is proof.

This plan implements layers 1-3. Layer 4 (git integration) remains future work — parsing history, correlating commits to conversations by time and semantic similarity, tracking survival through refactors. It's significant engineering, but the architecture here is designed to accommodate it: `certainty_grounded` and `certainty_validity` can incorporate git signals without restructuring.

The end goal isn't a truth oracle. It's a system where you can ask "what do I actually know?" and get an answer weighted by evidence, not recency or verbosity. Knowledge graphs that can distinguish "I thought about this once" from "I built this and it's still running" are rare. That's what we're building toward.
