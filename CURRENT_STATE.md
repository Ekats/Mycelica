# Mycelica

A graph-based knowledge system where reasoning is visible as structure.

Nodes (thoughts, sources, conversations) connect via explicit edges (supports, contradicts, evolved-from), making it possible to audit how conclusions were reached rather than just see endpoints.

Built with Rust/Tauri, React, SQLite, and OpenAI embeddings.

---

## Analysis Architecture

### What Mycelica Currently Analyzes

### 1. Import Pipeline (Current: Claude Conversations)

Claude JSON export → Exchange Nodes (Human + Assistant pairs)

- Pairs human questions with assistant responses into single "exchange" nodes
- Extracts: title (first 60 chars of question), content, timestamps
- Creates conversation containers with exchange counts

### 2. AI Processing (Claude Haiku)

For each item node, generates:

| Field    | Description                                            |
|----------|--------------------------------------------------------|
| ai_title | Concise 5-10 word descriptive title                    |
| summary  | 50-100 word summary of key points                      |
| tags     | 3-5 specific tags (technologies, concepts, task types) |
| emoji    | Single representative emoji                            |

### 3. Clustering (Claude Haiku)

- Groups items into fine-grained topics
- Assigns cluster_id and cluster_label
- Uses AI-processed summaries + tags for context
- Creates multi-path associations with strength weights

### 4. Hierarchy Grouping (Claude Sonnet)

- Recursively groups topics into 5-12 parent categories
- Detects project names (e.g., "Mycelica") as umbrella categories
- Creates navigable tree: Universe → Galaxies → Domains → Topics → Items
- Target: 8-15 children per level for usability

### 5. Embeddings (OpenAI text-embedding-3-small)

- 1536-dimensional vectors from ai_title + summary
- Stored as BLOB in SQLite
- Generated for both items AND category nodes

### 6. Semantic Similarity

- Cosine similarity between embeddings
- Creates "Related" edges (min 50% similarity)
- Sibling bonus (+20%) for nodes with same parent
- Lower threshold for category-to-category edges

### 7. Full-Text Search

- FTS5 index on title + content
- Keyword search across all nodes

---

## Potential Future Analysis

### Data Sources to Import

| Source                | Format          | Analysis Potential                      |
|-----------------------|-----------------|-----------------------------------------|
| Browser bookmarks     | JSON/HTML       | URL metadata, tags, folder structure    |
| Browser history       | SQLite          | Visit frequency, time patterns, domains |
| Obsidian/Notion notes | Markdown        | Wikilinks, backlinks, frontmatter       |
| Email threads         | MBOX/EML        | Participants, topics, sentiment         |
| Slack/Discord         | JSON export     | Channel context, threads, reactions     |
| GitHub issues/PRs     | API/JSON        | Code context, labels, assignees         |
| PDFs/documents        | Text extraction | Highlighted passages, annotations       |
| Voice memos           | Transcription   | Speech patterns, topics                 |
| Calendar events       | ICS             | Time-based clustering, attendees        |
| Code files            | AST parsing     | Dependencies, function signatures       |

### Analysis Capabilities to Add

#### Content Analysis

- **Sentiment analysis** – track emotional arc of conversations
- **Entity extraction** – people, organizations, projects, dates
- **Topic modeling** (LDA/BERTopic) – unsupervised theme discovery
- **Summarization levels** – one-liner, paragraph, full
- **Language detection** – multilingual knowledge bases

#### Relationship Analysis

- **Temporal patterns** – when topics cluster, decay curves
- **Citation/reference tracking** – what links to what
- **Contradiction detection** – conflicting information across nodes
- **Knowledge gaps** – questions without answers
- **Expertise mapping** – who knows what

#### Structural Analysis

- **Centrality metrics** – which nodes are hubs
- **Community detection** – natural knowledge clusters
- **Path analysis** – how topics connect
- **Evolution tracking** – how understanding changes over time

#### Predictive

- **Related content suggestions** – "you might also want to see..."
- **Missing connection prediction** – edges that should exist
- **Query anticipation** – what you'll search for next

---

## Architecture Insight

The current pipeline:

```
Import → AI Process → Cluster → Hierarchy → Embeddings → Edges
```

Is designed for extensibility:

- `NodeType` enum can add: Bookmark, Note, Email, Code, etc.
- `EdgeType` enum can add: References, Contradicts, Precedes, etc.
- Embedding dimension is configurable (currently 1536)
- Hierarchy depth is fully dynamic

### Future: Browser Integration

Potential browser integration could add:

- Live page content capture
- Real-time browsing context as nodes
- Tab-to-node conversion
- Automatic relationship inference from navigation patterns
