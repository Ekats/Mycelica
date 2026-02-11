import { useState, useEffect, useCallback } from "react";
import { X, Trash2, Link, FolderInput } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useTeamStore } from "../stores/teamStore";
import EdgeCreator from "./EdgeCreator";
import type { DisplayEdge, Node } from "../types";

export default function NodePopup() {
  const {
    nodes, personalNodes, selectedNodeId, edges, personalEdges,
    setSelectedNodeId, updateNode, deleteNode,
  } = useTeamStore();

  const [editingTitle, setEditingTitle] = useState(false);
  const [editingContent, setEditingContent] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");
  const [contentDraft, setContentDraft] = useState("");
  const [showEdgeCreator, setShowEdgeCreator] = useState(false);
  const [showCategoryAssigner, setShowCategoryAssigner] = useState(false);
  const [categoryQuery, setCategoryQuery] = useState("");
  const [categoryResults, setCategoryResults] = useState<Node[]>([]);

  const teamNode = selectedNodeId ? nodes.get(selectedNodeId) : null;
  const personalNode = selectedNodeId ? personalNodes.get(selectedNodeId) : null;
  const isPersonal = !teamNode && !!personalNode;
  const node = teamNode || personalNode;

  useEffect(() => {
    if (teamNode) {
      setTitleDraft(teamNode.aiTitle || teamNode.title);
      setContentDraft(teamNode.content || "");
    } else if (personalNode) {
      setTitleDraft(personalNode.title);
      setContentDraft(personalNode.content || "");
    }
    setEditingTitle(false);
    setEditingContent(false);
    setShowEdgeCreator(false);
    setShowCategoryAssigner(false);
    setCategoryQuery("");
    setCategoryResults([]);
  }, [selectedNodeId, teamNode, personalNode]);

  // Gather edges for this node
  const nodeEdges: DisplayEdge[] = [];
  if (selectedNodeId) {
    for (const e of edges) {
      if (e.source === selectedNodeId || e.target === selectedNodeId) {
        nodeEdges.push({ ...e, type: e.type, isPersonal: false });
      }
    }
    for (const pe of personalEdges) {
      if (pe.sourceId === selectedNodeId || pe.targetId === selectedNodeId) {
        nodeEdges.push({
          id: pe.id,
          source: pe.sourceId,
          target: pe.targetId,
          type: pe.edgeType,
          reason: pe.reason,
          isPersonal: true,
        });
      }
    }
  }

  const handleTitleSave = useCallback(async () => {
    if (!selectedNodeId || !teamNode) return;
    setEditingTitle(false);
    if (titleDraft !== (teamNode.aiTitle || teamNode.title)) {
      await updateNode(selectedNodeId, { title: titleDraft, author: useTeamStore.getState().config?.author });
    }
  }, [selectedNodeId, teamNode, titleDraft, updateNode]);

  const handleContentSave = useCallback(async () => {
    if (!selectedNodeId || !teamNode) return;
    setEditingContent(false);
    if (contentDraft !== (teamNode.content || "")) {
      await updateNode(selectedNodeId, { content: contentDraft, author: useTeamStore.getState().config?.author });
    }
  }, [selectedNodeId, teamNode, contentDraft, updateNode]);

  const handleDelete = useCallback(async () => {
    if (!selectedNodeId || isPersonal) return;
    await deleteNode(selectedNodeId);
  }, [selectedNodeId, isPersonal, deleteNode]);

  const handleCategorySearch = useCallback(async (value: string) => {
    setCategoryQuery(value);
    if (value.trim().length < 2) { setCategoryResults([]); return; }
    try {
      const { localCategories } = useTeamStore.getState();
      const results = await invoke<Node[]>("team_search", { query: value, limit: 10 });
      // Only show categories (not items), exclude self
      setCategoryResults(results.filter((r) => (!r.isItem || localCategories.has(r.id)) && r.id !== selectedNodeId));
    } catch { setCategoryResults([]); }
  }, [selectedNodeId]);

  const handleAssignCategory = useCallback(async (categoryId: string) => {
    if (!selectedNodeId) return;
    await updateNode(selectedNodeId, { parent_id: categoryId, author: useTeamStore.getState().config?.author });
    setShowCategoryAssigner(false);
    setCategoryQuery("");
    setCategoryResults([]);
  }, [selectedNodeId, updateNode]);

  if (!node) return null;

  const title = teamNode ? (teamNode.aiTitle || teamNode.title) : personalNode!.title;
  const content = teamNode ? teamNode.content : personalNode!.content;
  const author = teamNode?.author;
  const contentType = teamNode?.contentType || personalNode?.contentType;

  // Resolve edge target/source names
  const resolveNodeName = (id: string): string => {
    const n = nodes.get(id);
    if (n) return n.aiTitle || n.title;
    const pn = personalNodes.get(id);
    if (pn) return pn.title;
    return id.slice(0, 8) + "...";
  };

  return (
    <div className="fixed right-4 top-16 bottom-4 w-96 flex flex-col rounded-xl border overflow-hidden"
      style={{ background: "var(--bg-secondary)", borderColor: "var(--border)", zIndex: 50 }}>

      {/* Header */}
      <div className="flex items-center gap-2 px-4 py-3 border-b" style={{ borderColor: "var(--border)" }}>
        <div className="flex items-center gap-2 flex-1 min-w-0">
          {author && (
            <span className="text-xs px-2 py-0.5 rounded-full" style={{ background: "#374151", color: "#f59e0b" }}>
              {author}
            </span>
          )}
          {contentType && (
            <span className="text-xs px-2 py-0.5 rounded-full" style={{ background: "#374151", color: "var(--text-secondary)" }}>
              {contentType}
            </span>
          )}
          {isPersonal && (
            <span className="text-xs px-2 py-0.5 rounded-full" style={{ background: "#115e59", color: "#14b8a6" }}>
              personal
            </span>
          )}
        </div>
        <button className="btn-secondary p-1" onClick={() => setSelectedNodeId(null)}>
          <X size={16} />
        </button>
      </div>

      {/* Title */}
      <div className="px-4 py-3 border-b" style={{ borderColor: "var(--border)" }}>
        {editingTitle ? (
          <input
            className="w-full text-lg font-semibold"
            value={titleDraft}
            onChange={(e) => setTitleDraft(e.target.value)}
            onBlur={handleTitleSave}
            onKeyDown={(e) => e.key === "Enter" && handleTitleSave()}
            autoFocus
          />
        ) : (
          <h2
            className="text-lg font-semibold cursor-pointer hover:opacity-80"
            onClick={() => !isPersonal && setEditingTitle(true)}
            title={isPersonal ? "Personal nodes are local-only" : "Click to edit"}
          >
            {title}
          </h2>
        )}
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-4 py-3">
        {editingContent ? (
          <textarea
            className="w-full h-40 text-sm resize-none"
            value={contentDraft}
            onChange={(e) => setContentDraft(e.target.value)}
            onBlur={handleContentSave}
            autoFocus
          />
        ) : content ? (
          <p
            className="text-sm whitespace-pre-wrap cursor-pointer hover:opacity-80"
            style={{ color: "var(--text-secondary)" }}
            onClick={() => !isPersonal && setEditingContent(true)}
          >
            {content}
          </p>
        ) : (
          <p className="text-sm italic" style={{ color: "var(--text-secondary)" }}
            onClick={() => !isPersonal && setEditingContent(true)}>
            {isPersonal ? "No content" : "Click to add content..."}
          </p>
        )}

        {/* Edges */}
        {nodeEdges.length > 0 && (
          <div className="mt-4">
            <h3 className="text-xs font-medium uppercase mb-2" style={{ color: "var(--text-secondary)" }}>
              Connections ({nodeEdges.length})
            </h3>
            <div className="flex flex-col gap-1.5">
              {nodeEdges.map((e) => {
                const otherId = e.source === selectedNodeId ? e.target : e.source;
                const direction = e.source === selectedNodeId ? "\u2192" : "\u2190";
                return (
                  <div
                    key={e.id}
                    className="flex items-center gap-2 text-xs px-2 py-1.5 rounded cursor-pointer hover:opacity-80"
                    style={{
                      background: "var(--bg-tertiary)",
                      borderLeft: `3px solid ${e.isPersonal ? "#14b8a6" : "#4b5563"}`,
                    }}
                    onClick={() => setSelectedNodeId(otherId)}
                  >
                    <span style={{ color: "var(--text-secondary)" }}>{direction}</span>
                    <span className="flex-1 truncate">{resolveNodeName(otherId)}</span>
                    <span style={{ color: "var(--text-secondary)" }}>{e.type}</span>
                    {e.isPersonal && (
                      <span className="text-[10px]" style={{ color: "#14b8a6" }}>(personal)</span>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </div>

      {/* Footer */}
      <div className="flex items-center gap-2 px-4 py-3 border-t" style={{ borderColor: "var(--border)" }}>
        <button className="btn-secondary flex items-center gap-1 text-xs"
          onClick={() => { setShowEdgeCreator(!showEdgeCreator); setShowCategoryAssigner(false); }}>
          <Link size={12} />
          Add Edge
        </button>
        {!isPersonal && (
          <button className="btn-secondary flex items-center gap-1 text-xs"
            onClick={() => { setShowCategoryAssigner(!showCategoryAssigner); setShowEdgeCreator(false); }}>
            <FolderInput size={12} />
            Assign Category
          </button>
        )}
        {!isPersonal && (
          <button className="btn-danger flex items-center gap-1 text-xs ml-auto" onClick={handleDelete}>
            <Trash2 size={12} />
            Delete
          </button>
        )}
      </div>

      {showEdgeCreator && selectedNodeId && (
        <EdgeCreator sourceId={selectedNodeId} onClose={() => setShowEdgeCreator(false)} />
      )}

      {showCategoryAssigner && selectedNodeId && (
        <div className="border-t px-4 py-3" style={{ borderColor: "var(--border)", background: "var(--bg-primary)" }}>
          <div className="flex items-center justify-between mb-2">
            <h4 className="text-xs font-medium uppercase" style={{ color: "var(--text-secondary)" }}>Assign to Category</h4>
            <button className="btn-secondary p-0.5" onClick={() => setShowCategoryAssigner(false)}><X size={12} /></button>
          </div>
          {teamNode?.parentId && (
            <p className="text-xs mb-2" style={{ color: "var(--text-secondary)" }}>
              Current: {resolveNodeName(teamNode.parentId)}
            </p>
          )}
          <input
            type="text"
            className="w-full mb-1 text-xs"
            placeholder="Search categories..."
            value={categoryQuery}
            onChange={(e) => handleCategorySearch(e.target.value)}
            style={{ padding: "4px 6px" }}
            autoFocus
          />
          {categoryQuery && categoryResults.length > 0 && (
            <div className="flex flex-col gap-0.5 max-h-24 overflow-y-auto rounded" style={{ background: "var(--bg-tertiary)" }}>
              {categoryResults.slice(0, 5).map((r) => (
                <button
                  key={r.id}
                  className="text-left text-xs px-2 py-1 hover:opacity-80"
                  style={{ color: "var(--text-primary)" }}
                  onClick={() => handleAssignCategory(r.id)}
                >
                  {r.aiTitle || r.title}
                </button>
              ))}
            </div>
          )}
          {categoryQuery && categoryResults.length === 0 && (
            <p className="text-xs italic" style={{ color: "var(--text-secondary)" }}>No categories found</p>
          )}
        </div>
      )}
    </div>
  );
}
