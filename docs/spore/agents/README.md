# Spore Agent Launch Guide

How to run Coder + Verifier agents against the Mycelica knowledge graph.

## Prerequisites

```bash
# Install CLI with MCP feature
cd /home/ekats/Repos/Mycelica/src-tauri
cargo +nightly install --path . --bin mycelica-cli --features mcp --force

# Verify MCP server works
echo '{"jsonrpc":"2.0","method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}},"id":1}' \
  | mycelica-cli mcp-server --stdio --agent-role coder --agent-id test
# Should return JSON with server info
```

## Launching the Coder

```bash
cd /home/ekats/Repos/Mycelica

# 1. Set up MCP config
cp docs/spore/agents/mcp-coder.json .mcp.json

# 2. Launch with prompt
claude -p "$(cat docs/spore/agents/coder.md)

Your task: <describe what to implement>"
```

Or interactively:
```bash
cp docs/spore/agents/mcp-coder.json .mcp.json
claude
# Paste contents of docs/spore/agents/coder.md as first message, followed by the task
```

## Launching the Verifier

```bash
cd /home/ekats/Repos/Mycelica

# 1. Set up MCP config
cp docs/spore/agents/mcp-verifier.json .mcp.json

# 2. Launch with prompt
claude -p "$(cat docs/spore/agents/verifier.md)

Check the Coder's recent implementation. The implementation node ID is: <id>"
```

## The Bounce Loop

This is human-orchestrated for V1. You launch agents one at a time and read the graph between sessions.

1. **Launch Coder** with a task. Coder writes code and creates an implementation node.
2. **Read the graph** to find the implementation node:
   ```bash
   mycelica-cli search "Implemented:"
   mycelica-cli node get <id> --full
   ```
3. **Launch Verifier** with the implementation node ID.
4. **Verifier records results:**
   - `supports` edge = PASS (loop done for this item)
   - `contradicts` edge = FAIL (bounce needed)
5. **If fail:** Relaunch Coder with: "The Verifier found issues with your implementation node `<id>`. Check for `contradicts` edges and fix the problems."
6. **Repeat** steps 3-5 until a `supports` edge exists.

## Reading the Deliberation Trail

```bash
# See recent verification edges
mycelica-cli spore query-edges --type contradicts,supports --since 2026-02-17

# See all edges on a specific node
mycelica-cli nav edges <node-id> --direction incoming

# Full explanation of an edge
mycelica-cli spore explain-edge <edge-id>

# Spore dashboard
mycelica-cli spore status
```

## Validation Test

**Task**: "Add a `--json` flag to `mycelica-cli db stats` that outputs stats as JSON instead of formatted text."

1. Launch Coder with this task
2. After Coder finishes: `mycelica-cli search "Implemented:"`
3. Launch Verifier with the implementation node ID
4. Evaluate:
   - Did both agents use MCP tools without errors?
   - Do operational nodes have meaningful content?
   - Are edges typed correctly with reasonable confidence?
   - Could a third party follow the deliberation trail?
   - If a bounce occurred: does the fix node supersede the old one?

## System Agents

### spore:orchestrator

`spore:orchestrator` is a **system agent**, not an LLM agent. It runs on the host machine
and manages the automated Coder-Verifier bounce loop via `mycelica-cli spore orchestrate`.

It creates:
- **Task nodes** (`node_class=operational`) to track orchestration runs
- **Escalation nodes** (`node_class=meta`, `meta_type=escalation`) when max bounces are reached
- **Tracks edges** to record run status (success/failure/incomplete) with metadata
- **Flags edges** from escalation nodes to the last failed implementation

It does **not** appear in the MCP permission matrix because it never connects as an MCP client.
Its `agent_id` is `spore:orchestrator` for attribution in the graph.

## Known V1 Limitations

- ~~No automated bounce discovery~~ — `--target-agent` filter on `query-edges` (Phase 6)
- ~~No run tracking~~ — `spore runs list/get/rollback` with `json_extract` on edge metadata (Phase 6)
- ~~No escalation detection~~ — automatic after max bounces (Phase 6)
- ~~Manual MCP config swapping~~ — orchestrator writes temp configs with `--mcp-config` (Phase 6)
