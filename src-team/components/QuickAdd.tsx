import { useState, useCallback, useMemo } from "react";
import { X } from "lucide-react";
import { useTeamStore } from "../stores/teamStore";
import type { ContentType } from "../types";

const CONTENT_TYPES: ContentType[] = [
  "concept", "question", "decision", "reference", "idea",
  "insight", "exploration", "synthesis", "planning",
];

const PERSONAL_COLOR = "#14b8a6";

export default function QuickAdd() {
  const {
    setShowQuickAdd, createNode, updateNode, config,
    search, searchResults, selectedNodeId, nodes,
  } = useTeamStore();

  // If a node is selected, pre-fill connection and detect if it's a category
  const selectedNode = selectedNodeId ? nodes.get(selectedNodeId) : null;
  const selectedIsCategory = selectedNode ? !selectedNode.isItem && selectedNode.childCount >= 0 : false;

  const [mode, setMode] = useState<"team" | "personal">("team");
  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");
  const [contentType, setContentType] = useState<ContentType>("concept");
  const [tags, setTags] = useState("");
  const [connectQuery, setConnectQuery] = useState("");
  const [connectTo, setConnectTo] = useState<string[]>(
    // Pre-fill with selected node if it's an item (peer connection)
    selectedNodeId && !selectedIsCategory ? [selectedNodeId] : []
  );
  const [parentId, setParentId] = useState<string | null>(
    // Pre-fill parent if selected node is a category
    selectedIsCategory && selectedNodeId ? selectedNodeId : null
  );
  const [submitting, setSubmitting] = useState(false);

  const parentName = useMemo(() => {
    if (!parentId) return null;
    const n = nodes.get(parentId);
    return n ? (n.aiTitle || n.title) : parentId.slice(0, 8);
  }, [parentId, nodes]);

  // Resolve display name for a connection id
  const resolveNodeName = useCallback((id: string): string => {
    const n = nodes.get(id);
    return n ? (n.aiTitle || n.title) : id.slice(0, 8);
  }, [nodes]);

  const handleConnectSearch = useCallback((value: string) => {
    setConnectQuery(value);
    if (value.trim().length >= 2) {
      search(value);
    }
  }, [search]);

  const handleSubmit = useCallback(async () => {
    if (!title.trim()) return;
    setSubmitting(true);
    try {
      if (mode === "team") {
        const newId = await createNode({
          title: title.trim(),
          content: content.trim() || undefined,
          content_type: contentType,
          tags: tags.trim() || undefined,
          author: config?.author,
          connects_to: connectTo.length > 0 ? connectTo : undefined,
        });
        if (parentId && newId) {
          await updateNode(newId, { parent_id: parentId, author: config?.author });
        }
      } else {
        const { createPersonalNode, createPersonalEdge } = useTeamStore.getState();
        const node = await createPersonalNode(
          title.trim(),
          content.trim() || undefined,
          contentType,
          tags.trim() || undefined,
        );
        for (const targetId of connectTo) {
          await createPersonalEdge(node.id, targetId, "related");
        }
      }
      setShowQuickAdd(false);
    } finally {
      setSubmitting(false);
    }
  }, [mode, title, content, contentType, tags, connectTo, parentId, config, createNode, updateNode, setShowQuickAdd]);

  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && setShowQuickAdd(false)}>
      <div className="modal-content">
        {/* Header */}
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold">Quick Add</h2>
          <button className="btn-secondary p-1" onClick={() => setShowQuickAdd(false)}>
            <X size={16} />
          </button>
        </div>

        {/* Team / Personal toggle */}
        <div className="flex gap-1 mb-4 p-1 rounded-lg" style={{ background: "var(--bg-tertiary)" }}>
          <button
            className={`flex-1 py-1.5 rounded text-sm font-medium ${mode === "team" ? "btn-primary" : ""}`}
            onClick={() => setMode("team")}
          >
            Team
          </button>
          <button
            className={`flex-1 py-1.5 rounded text-sm font-medium ${mode === "personal" ? "" : ""}`}
            style={mode === "personal" ? { background: PERSONAL_COLOR, color: "#111827" } : {}}
            onClick={() => setMode("personal")}
          >
            Personal
          </button>
        </div>

        {/* Parent category */}
        {mode === "team" && (
          <div className="mb-3">
            <label className="block text-xs mb-1" style={{ color: "var(--text-secondary)" }}>Parent category</label>
            {parentId ? (
              <div className="flex items-center gap-2 px-2 py-1.5 rounded text-sm"
                style={{ background: "var(--bg-tertiary)" }}>
                <span className="flex-1 truncate">{parentName}</span>
                <button className="hover:opacity-60" onClick={() => setParentId(null)}>
                  <X size={12} />
                </button>
              </div>
            ) : (
              <p className="text-xs italic" style={{ color: "var(--text-secondary)" }}>
                None (orphan) — select a category node before opening Quick Add to pre-fill
              </p>
            )}
          </div>
        )}

        {/* Content type */}
        <div className="mb-3">
          <label className="block text-xs mb-1" style={{ color: "var(--text-secondary)" }}>Type</label>
          <select
            value={contentType}
            onChange={(e) => setContentType(e.target.value as ContentType)}
            className="w-full"
          >
            {CONTENT_TYPES.map((t) => (
              <option key={t} value={t}>{t}</option>
            ))}
          </select>
        </div>

        {/* Title */}
        <div className="mb-3">
          <label className="block text-xs mb-1" style={{ color: "var(--text-secondary)" }}>Title *</label>
          <input
            type="text"
            className="w-full"
            placeholder="What's on your mind?"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && !e.shiftKey && handleSubmit()}
            autoFocus
          />
        </div>

        {/* Content */}
        <div className="mb-3">
          <label className="block text-xs mb-1" style={{ color: "var(--text-secondary)" }}>Note</label>
          <textarea
            className="w-full h-20 resize-none"
            placeholder="Optional details..."
            value={content}
            onChange={(e) => setContent(e.target.value)}
          />
        </div>

        {/* Tags */}
        <div className="mb-3">
          <label className="block text-xs mb-1" style={{ color: "var(--text-secondary)" }}>Tags (comma-separated)</label>
          <input
            type="text"
            className="w-full"
            placeholder="e.g., architecture, priority"
            value={tags}
            onChange={(e) => setTags(e.target.value)}
          />
        </div>

        {/* Connect to */}
        <div className="mb-4">
          <label className="block text-xs mb-1" style={{ color: "var(--text-secondary)" }}>Connect to</label>

          {/* Pre-filled connections shown as chips */}
          {connectTo.length > 0 && (
            <div className="flex flex-wrap gap-1 mb-1">
              {connectTo.map((id) => (
                <span key={id} className="text-xs px-2 py-0.5 rounded-full flex items-center gap-1"
                  style={{ background: "var(--bg-tertiary)" }}>
                  {resolveNodeName(id)}
                  <button className="hover:opacity-60" onClick={() => setConnectTo((p) => p.filter((x) => x !== id))}>
                    <X size={10} />
                  </button>
                </span>
              ))}
            </div>
          )}

          <input
            type="text"
            className="w-full mb-1"
            placeholder="Search existing nodes..."
            value={connectQuery}
            onChange={(e) => handleConnectSearch(e.target.value)}
          />
          {connectQuery && searchResults.length > 0 && (
            <div className="flex flex-col gap-0.5 max-h-32 overflow-y-auto rounded" style={{ background: "var(--bg-tertiary)" }}>
              {searchResults.slice(0, 6).map((r) => (
                <button
                  key={r.id}
                  className="text-left text-xs px-2 py-1.5 hover:opacity-80"
                  style={{
                    background: connectTo.includes(r.id) ? "var(--accent)" : "transparent",
                    color: connectTo.includes(r.id) ? "#111827" : "var(--text-primary)",
                  }}
                  onClick={() => {
                    setConnectTo((prev) =>
                      prev.includes(r.id)
                        ? prev.filter((x) => x !== r.id)
                        : [...prev, r.id]
                    );
                  }}
                >
                  {r.aiTitle || r.title}
                </button>
              ))}
            </div>
          )}
        </div>

        {/* Submit */}
        <div className="flex justify-end gap-2">
          <button className="btn-secondary" onClick={() => setShowQuickAdd(false)}>Cancel</button>
          <button
            className="btn-primary"
            onClick={handleSubmit}
            disabled={!title.trim() || submitting}
          >
            {submitting ? "Creating..." : mode === "team" ? "Create (Team)" : "Create (Personal)"}
          </button>
        </div>
      </div>
    </div>
  );
}
