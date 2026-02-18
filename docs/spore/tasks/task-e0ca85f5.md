# Task: Add a --format flag to the 'spore status' command that accep

- **Run:** e0ca85f5
- **Agent:** coder
- **Bounce:** 1/1
- **Generated:** 2026-02-18 00:13:49 UTC

## Task

Add a --format flag to the 'spore status' command that accepts 'compact' (default, current behavior) or 'full'. In 'full' mode, also show: (1) the top 5 most-connected nodes by edge count, (2) edge type distribution counts, and (3) a list of recent operational nodes from agent activity (last 24h). Use existing Database methods - get_edges_for_node for edge counts, existing SQL patterns for aggregates.

## Graph Context

Relevant nodes found by search + Dijkstra traversal from the task description.
Use `mycelica_node_get` or `mycelica_read_content` to read full content of any node.

_No relevant nodes found in the graph._

## Checklist

- [ ] Read relevant context nodes above before starting
- [ ] Record implementation as operational node when done
- [ ] Link implementation to modified code nodes with edges
