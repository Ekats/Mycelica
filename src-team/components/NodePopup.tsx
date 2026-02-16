import { useState, useEffect, useCallback } from "react";
import { X, Trash2, Link, FolderInput, FolderOpen, Pencil, Maximize2, Globe, RefreshCw, Loader2, ExternalLink } from "lucide-react";
import ReactMarkdown from "react-markdown";
import { invoke } from "@tauri-apps/api/core";
import { useTeamStore } from "../stores/teamStore";
import EdgeCreator from "./EdgeCreator";
import { ConversationRenderer, isConversationContent } from "./ConversationRenderer";
import type { ContentType, DisplayEdge, Node } from "../types";

const CONTENT_TYPES: ContentType[] = [
  "concept", "question", "decision", "reference", "idea",
  "insight", "exploration", "synthesis", "planning",
];

export default function NodePopup() {
  const {
    nodes, personalNodes, selectedNodeId, edges, personalEdges,
    setSelectedNodeId, updateNode, deleteNode, deletePersonalNode, updatePersonalNode,
    navigateToCategory, openLeafView,
    fetchedContent, isFetching, fetchUrlContent, loadFetchedContent,
    mergedBodies, mergeGroupIds,
  } = useTeamStore();

  const [editingTitle, setEditingTitle] = useState(false);
  const [editingContent, setEditingContent] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");
  const [contentDraft, setContentDraft] = useState("");
  const [showEdgeCreator, setShowEdgeCreator] = useState(false);
  const [showCategoryAssigner, setShowCategoryAssigner] = useState(false);
  const [categoryQuery, setCategoryQuery] = useState("");
  const [categoryResults, setCategoryResults] = useState<Node[]>([]);
  const [showEditModal, setShowEditModal] = useState(false);

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
    setShowEditModal(false);
  }, [selectedNodeId, teamNode, personalNode]);

  // Load cached fetched content, auto-fetch if URL present and not cached
  useEffect(() => {
    if (!selectedNodeId) return;
    loadFetchedContent(selectedNodeId).then(() => {
      const { fetchedContent: fc, isFetching: fetching } = useTeamStore.getState();
      if (fc.has(selectedNodeId) || fetching) return;
      // Detect URLs in node text
      const node = useTeamStore.getState().nodes.get(selectedNodeId);
      const pNode = useTeamStore.getState().personalNodes.get(selectedNodeId);
      const t = node ? (node.aiTitle || node.title) : pNode?.title || "";
      const c = node?.content || pNode?.content || "";
      const urls = `${c} ${t}`.match(/https?:\/\/[^\s<>"')\]]+/g);
      if (urls?.length) fetchUrlContent(selectedNodeId, urls[0]);
    });
  }, [selectedNodeId, loadFetchedContent, fetchUrlContent]);

  // Gather edges for this node (including all IDs in a merge group)
  const nodeEdges: DisplayEdge[] = [];
  if (selectedNodeId) {
    const groupIds = mergeGroupIds.get(selectedNodeId) || [selectedNodeId];
    const groupSet = new Set(groupIds);
    for (const e of edges) {
      if (groupSet.has(e.source) || groupSet.has(e.target)) {
        nodeEdges.push({ ...e, type: e.type, isPersonal: false });
      }
    }
    for (const pe of personalEdges) {
      if (groupSet.has(pe.sourceId) || groupSet.has(pe.targetId)) {
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
    if (!selectedNodeId) return;
    if (isPersonal) {
      await deletePersonalNode(selectedNodeId);
    } else {
      await deleteNode(selectedNodeId);
    }
  }, [selectedNodeId, isPersonal, deleteNode, deletePersonalNode]);

  const loadAllCategories = useCallback(() => {
    const { nodes: allNodes, localCategories } = useTeamStore.getState();
    const cats: Node[] = [];
    for (const n of allNodes.values()) {
      if ((!n.isItem || localCategories.has(n.id)) && n.id !== selectedNodeId) {
        cats.push(n);
      }
    }
    cats.sort((a, b) => (a.aiTitle || a.title).localeCompare(b.aiTitle || b.title));
    setCategoryResults(cats.slice(0, 100));
  }, [selectedNodeId]);

  const handleCategorySearch = useCallback(async (value: string) => {
    setCategoryQuery(value);
    if (!value.trim()) { loadAllCategories(); return; }
    try {
      const { localCategories } = useTeamStore.getState();
      const results = await invoke<Node[]>("team_search", { query: value, limit: 100 });
      setCategoryResults(results.filter((r) => (!r.isItem || localCategories.has(r.id)) && r.id !== selectedNodeId));
    } catch { setCategoryResults([]); }
  }, [selectedNodeId, loadAllCategories]);

  const handleAssignCategory = useCallback(async (categoryId: string) => {
    if (!selectedNodeId) return;
    if (isPersonal) {
      // Personal nodes can't be PATCHed on server — create a "contains" edge instead
      const { createPersonalEdge } = useTeamStore.getState();
      await createPersonalEdge(categoryId, selectedNodeId, "contains");
    } else {
      await updateNode(selectedNodeId, { parent_id: categoryId, author: useTeamStore.getState().config?.author });
    }
    setShowCategoryAssigner(false);
    setCategoryQuery("");
    setCategoryResults([]);
  }, [selectedNodeId, isPersonal, updateNode]);

  if (!node) return null;

  const title = teamNode ? (teamNode.aiTitle || teamNode.title) : personalNode!.title;
  const rawContent = teamNode ? teamNode.content : personalNode!.content;
  const content = (selectedNodeId && mergedBodies.get(selectedNodeId)) || rawContent;
  const author = teamNode?.author;
  const contentType = teamNode?.contentType || personalNode?.contentType;
  const nodeTags = teamNode?.tags || personalNode?.tags;

  // URL detection + fetched content (prefer content over truncated title)
  const allText = `${content || ""} ${title}`;
  const detectedUrls: string[] = [...new Set(allText.match(/https?:\/\/[^\s<>"')\]]+/g) || [])];
  const cachedContent = selectedNodeId ? fetchedContent.get(selectedNodeId) : undefined;
  const isFetchingThis = isFetching === selectedNodeId;

  const formatTime = (ts: number) => {
    const diff = Math.floor((Date.now() - ts) / 1000);
    if (diff < 60) return "just now";
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    return new Date(ts).toLocaleDateString();
  };

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
            title={isPersonal ? "Use Edit to modify personal nodes" : "Click to edit"}
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
        ) : content && isConversationContent(content) ? (
          <ConversationRenderer content={content} />
        ) : content ? (
          <div
            className="prose prose-invert prose-sm max-w-none cursor-pointer hover:opacity-90
              prose-headings:text-white prose-headings:font-semibold prose-headings:mt-3 prose-headings:mb-1
              prose-p:text-gray-300 prose-p:leading-relaxed prose-p:my-2
              prose-a:text-amber-400 prose-a:no-underline hover:prose-a:underline
              prose-strong:text-white prose-em:text-gray-200
              prose-code:text-amber-300 prose-code:bg-gray-950 prose-code:px-1 prose-code:py-0.5 prose-code:rounded prose-code:text-xs prose-code:font-mono prose-code:border prose-code:border-gray-700
              prose-pre:bg-gray-950 prose-pre:border prose-pre:border-gray-600 prose-pre:rounded-lg prose-pre:my-3 prose-pre:p-3 prose-pre:overflow-x-auto
              [&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_pre_code]:text-green-400 [&_pre_code]:border-0
              prose-blockquote:border-l-2 prose-blockquote:border-amber-500/50 prose-blockquote:bg-gray-800/30 prose-blockquote:py-1 prose-blockquote:px-3 prose-blockquote:my-3 prose-blockquote:italic prose-blockquote:text-gray-400
              prose-ul:my-2 prose-ul:pl-4 prose-ol:my-2 prose-ol:pl-4
              prose-li:text-gray-300 prose-li:my-0.5
              prose-hr:border-gray-700 prose-hr:my-4"
            onClick={() => !isPersonal && setEditingContent(true)}
          >
            <ReactMarkdown>{content}</ReactMarkdown>
          </div>
        ) : (
          <p className="text-sm italic" style={{ color: "var(--text-secondary)" }}
            onClick={() => !isPersonal && setEditingContent(true)}>
            {isPersonal ? "No content" : "Click to add content..."}
          </p>
        )}

        {/* Fetched URL content */}
        {cachedContent && selectedNodeId && (
          <div className="mt-3">
            <div className="flex items-center justify-between mb-1">
              <span className="text-[11px] uppercase font-medium" style={{ color: "var(--text-secondary)" }}>
                Fetched Content
              </span>
              <div className="flex items-center gap-1">
                <span className="text-[10px]" style={{ color: "var(--text-secondary)" }}>
                  {formatTime(cachedContent.fetchedAt)}
                </span>
                <button
                  className="btn-secondary p-0.5"
                  title="Refresh"
                  onClick={() => fetchUrlContent(selectedNodeId, cachedContent.url)}
                  disabled={isFetchingThis}
                >
                  {isFetchingThis ? <Loader2 size={10} className="animate-spin" /> : <RefreshCw size={10} />}
                </button>
              </div>
            </div>
            {cachedContent.title && (
              <p className="text-xs font-medium mb-1">{cachedContent.title}</p>
            )}
            <iframe
              srcDoc={`<!DOCTYPE html><html><head><meta charset="utf-8"><style>body{font-family:system-ui,sans-serif;font-size:14px;line-height:1.6;color:#222;background:#fff;padding:12px;margin:0}img{max-width:100%;height:auto}pre{overflow-x:auto;background:#f5f5f5;padding:8px;border-radius:4px}a{color:#2563eb}</style></head><body>${cachedContent.html}</body></html>`}
              sandbox=""
              className="w-full rounded border"
              style={{ height: 250, borderColor: "var(--border)", background: "#fff" }}
              title="Fetched content"
            />
            {cachedContent.textContent.length < 50 && (
              <p className="text-[10px] mt-1 italic" style={{ color: "var(--text-secondary)" }}>
                Content may require a browser (JavaScript-rendered page)
              </p>
            )}
          </div>
        )}

        {/* Fetch URL button(s) */}
        {!cachedContent && detectedUrls.length > 0 && selectedNodeId && (() => {
          const primaryUrl = detectedUrls[0];
          let hostname = primaryUrl;
          try { hostname = new URL(primaryUrl).hostname; } catch {}
          return (
            <div className="mt-3 flex flex-col gap-1">
              <button
                className="btn-secondary flex items-center gap-1.5 text-xs w-full justify-center"
                onClick={() => fetchUrlContent(selectedNodeId, primaryUrl)}
                disabled={isFetchingThis}
              >
                {isFetchingThis ? (
                  <Loader2 size={12} className="animate-spin" />
                ) : (
                  <Globe size={12} />
                )}
                {isFetchingThis ? "Fetching..." : `Fetch content from ${hostname}`}
              </button>
              {detectedUrls.slice(1).map((url) => (
                <button
                  key={url}
                  className="text-[11px] hover:underline text-left truncate"
                  style={{ color: "var(--accent)" }}
                  onClick={() => fetchUrlContent(selectedNodeId, url)}
                  disabled={isFetchingThis}
                >
                  {url}
                </button>
              ))}
            </div>
          );
        })()}

        {/* Tags — skip for signal nodes (tags are JSON metadata, not user-facing) */}
        {nodeTags && !nodeTags.startsWith("{") && (
          <div className="flex flex-wrap gap-1 mt-3">
            {nodeTags.split(",").map((tag) => tag.trim()).filter(Boolean).map((tag) => (
              <span key={tag} className="text-[11px] px-2 py-0.5 rounded-full"
                style={{ background: "var(--bg-tertiary)", color: "var(--text-secondary)" }}>
                {tag}
              </span>
            ))}
          </div>
        )}

        {/* Edges — sorted by weight descending */}
        {nodeEdges.length > 0 && (() => {
          const groupSet = new Set(mergeGroupIds.get(selectedNodeId!) || [selectedNodeId!]);
          const sorted = [...nodeEdges].sort((a, b) => (b.weight ?? 0) - (a.weight ?? 0));
          return (
          <div className="mt-4">
            <h3 className="text-xs font-medium uppercase mb-2" style={{ color: "var(--text-secondary)" }}>
              Connections ({sorted.length})
            </h3>
            <div className="flex flex-col gap-1.5">
              {sorted.map((e) => {
                const otherId = groupSet.has(e.source) ? e.target : e.source;
                const direction = groupSet.has(e.source) ? "\u2192" : "\u2190";
                const pct = e.weight != null ? `${Math.round(e.weight * 100)}%` : null;
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
                    {pct && <span className="font-mono text-[10px]" style={{ color: "var(--accent)" }}>{pct}</span>}
                    <span style={{ color: "var(--text-secondary)" }}>{e.type}</span>
                    {e.isPersonal && (
                      <span className="text-[10px]" style={{ color: "#14b8a6" }}>(personal)</span>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
          );
        })()}
      </div>

      {/* Footer */}
      <div className="flex items-center gap-2 px-4 py-3 border-t flex-wrap" style={{ borderColor: "var(--border)" }}>
        {teamNode && !teamNode.isItem && (
          <button className="btn-secondary flex items-center gap-1 text-xs"
            onClick={() => { if (selectedNodeId) navigateToCategory(selectedNodeId); }}>
            <FolderOpen size={12} />
            Drill In{teamNode.childCount > 0 ? ` (${teamNode.childCount})` : ""}
          </button>
        )}
        <button className="btn-secondary flex items-center gap-1 text-xs"
          onClick={() => { if (selectedNodeId) openLeafView(selectedNodeId); }}>
          <Maximize2 size={12} />
          Expand
        </button>
        <button className="btn-secondary flex items-center gap-1 text-xs"
          onClick={() => { setShowEdgeCreator(!showEdgeCreator); setShowCategoryAssigner(false); }}>
          <Link size={12} />
          Add Edge
        </button>
        <button className="btn-secondary flex items-center gap-1 text-xs"
          onClick={() => setShowEditModal(true)}>
          <Pencil size={12} />
          Edit
        </button>
        <button className="btn-secondary flex items-center gap-1 text-xs"
          onClick={() => { setShowCategoryAssigner(!showCategoryAssigner); setShowEdgeCreator(false); }}>
          <FolderInput size={12} />
          Assign Category
        </button>
        {detectedUrls.length > 0 && (
          <button className="btn-secondary flex items-center gap-1 text-xs"
            onClick={async () => {
              const { openUrl } = await import("@tauri-apps/plugin-opener");
              openUrl(detectedUrls[0]);
            }}>
            <ExternalLink size={12} /> Open URL
          </button>
        )}
        <button className="btn-danger flex items-center gap-1 text-xs ml-auto" onClick={handleDelete}>
          <Trash2 size={12} />
          Delete
        </button>
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
            placeholder="Search or browse categories..."
            value={categoryQuery}
            onChange={(e) => handleCategorySearch(e.target.value)}
            onFocus={() => { if (!categoryQuery && categoryResults.length === 0) loadAllCategories(); }}
            style={{ padding: "4px 6px" }}
            autoFocus
          />
          {categoryResults.length > 0 && (
            <div className="flex flex-col gap-0.5 max-h-48 overflow-y-auto rounded" style={{ background: "var(--bg-tertiary)" }}>
              {categoryResults.slice(0, 100).map((r) => (
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

      {/* Edit Node Modal */}
      {showEditModal && selectedNodeId && node && (
        <EditNodeModal
          node={{
            title: title,
            content: content || undefined,
            contentType: contentType || undefined,
            tags: nodeTags || undefined,
          }}
          nodeId={selectedNodeId}
          isPersonal={isPersonal}
          onClose={() => setShowEditModal(false)}
          onSavePersonal={updatePersonalNode}
          onSaveTeam={updateNode}
        />
      )}
    </div>
  );
}

function EditNodeModal({ node, nodeId, isPersonal, onClose, onSavePersonal, onSaveTeam }: {
  node: { title: string; content?: string; contentType?: string; tags?: string };
  nodeId: string;
  isPersonal: boolean;
  onClose: () => void;
  onSavePersonal: (id: string, updates: { title?: string; content?: string; contentType?: string; tags?: string }) => Promise<void>;
  onSaveTeam: (id: string, req: { title?: string; content?: string; tags?: string; content_type?: string; author?: string }) => Promise<void>;
}) {
  const [title, setTitle] = useState(node.title);
  const [content, setContent] = useState(node.content || "");
  const [contentType, setContentType] = useState(node.contentType || "concept");
  const [tags, setTags] = useState(node.tags || "");
  const [saving, setSaving] = useState(false);

  const handleSave = useCallback(async () => {
    setSaving(true);
    try {
      if (isPersonal) {
        await onSavePersonal(nodeId, {
          title: title.trim() || undefined,
          content: content.trim() || undefined,
          contentType: contentType || undefined,
          tags: tags.trim() || undefined,
        });
      } else {
        await onSaveTeam(nodeId, {
          title: title.trim() || undefined,
          content: content.trim() || undefined,
          content_type: contentType || undefined,
          tags: tags.trim() || undefined,
          author: useTeamStore.getState().config?.author,
        });
      }
      onClose();
    } finally {
      setSaving(false);
    }
  }, [nodeId, isPersonal, title, content, contentType, tags, onSavePersonal, onSaveTeam, onClose]);

  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal-content">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold">Edit {isPersonal ? "Personal" : "Team"} Node</h2>
          <button className="btn-secondary p-1" onClick={onClose}>
            <X size={16} />
          </button>
        </div>

        <div className="mb-3">
          <label className="block text-xs mb-1" style={{ color: "var(--text-secondary)" }}>Type</label>
          <select value={contentType} onChange={(e) => setContentType(e.target.value)} className="w-full">
            {CONTENT_TYPES.map((t) => (
              <option key={t} value={t}>{t}</option>
            ))}
          </select>
        </div>

        <div className="mb-3">
          <label className="block text-xs mb-1" style={{ color: "var(--text-secondary)" }}>Title *</label>
          <input
            type="text"
            className="w-full"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && !e.shiftKey && handleSave()}
            autoFocus
          />
        </div>

        <div className="mb-3">
          <label className="block text-xs mb-1" style={{ color: "var(--text-secondary)" }}>Note</label>
          <textarea
            className="w-full h-20 resize-none"
            value={content}
            onChange={(e) => setContent(e.target.value)}
          />
        </div>

        <div className="mb-4">
          <label className="block text-xs mb-1" style={{ color: "var(--text-secondary)" }}>Tags (comma-separated)</label>
          <input
            type="text"
            className="w-full"
            value={tags}
            onChange={(e) => setTags(e.target.value)}
          />
        </div>

        <div className="flex justify-end gap-2">
          <button className="btn-secondary" onClick={onClose}>Cancel</button>
          <button
            className="btn-primary"
            onClick={handleSave}
            disabled={!title.trim() || saving}
          >
            {saving ? "Saving..." : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}
