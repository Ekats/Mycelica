# Mycelica Vision

> What if we organized external knowledge the way associative memory organizes internal knowledge?

## The Problem

Knowledge scatters. AI conversations disappear into chat history. Notes live in separate apps. Research papers sit in flat lists. Code and documentation drift apart. Ideas connect across domains but tools force you into folders or tags that capture one relationship at a time.

Real thinking is associative. One idea triggers another. Current tools don't support this.

## The Solution

Mycelica is a visual knowledge graph that builds structure from content automatically. Named after mycelium, the underground fungal network that connects forest ecosystems.

Import your data:
- AI conversations (Claude, ChatGPT exports)
- Research papers (via OpenAIRE)
- Notes and markdown files
- Source code (with call graphs and doc links)
- Web sessions (via browser extension)

Mycelica generates embeddings locally, computes semantic similarity edges between documents, then builds a navigable hierarchy from edge topology. Categories form because their contents actually connect. You explore by drilling down through levels, not by searching and scrolling through flat results.

## Core Principles

### Emergent Structure
Hierarchy comes from connectivity, not manual organization. Import 50,000 papers, get a browsable tree where related work clusters together automatically. The algorithm is deterministic: same edges produce the same tree.

### AI-Assisted, Auditable
AI generates category names after structure exists. All structural decisions trace back to measurable edge weights. You can inspect why documents clustered together (they share edges above threshold) and why clusters split (cohesion dropped below validation).

### Local-First
Your data lives on your device. No server, no account, no subscription. Works offline. Privacy by architecture, not policy.

## Current State

Working desktop application with GUI, CLI, and TUI interfaces. Tested on 53,000 research papers. Cross-platform builds for Linux and Windows. AGPL-3.0 licensed.

The adaptive tree algorithm reads hierarchical structure directly from edge weights using dendrogram extraction with threshold cuts validated for balance and cohesion.

## What Comes Next

Edge type classification (supports, contradicts, extends, replicates). Citation graph integration. Incremental hierarchy updates. More import sources. Better doc-to-code linking.

The longer term possibility: if the core approach proves useful, it could extend to shared knowledge bases and cross-instance queries. But that depends on getting the fundamentals right first.

## Success Metric

Using Mycelica daily. Finding old conversations and papers quickly. Discovering connections I had forgotten. Seeing the shape of what I know.

Everything else follows from that.

---

*For implementation details, see ALGORITHMS.md and ARCHITECTURE.md*