import { useState, useEffect, useCallback } from "react";
import { X, Trash2, Link, FolderInput, Pencil, Globe, RefreshCw, Loader2, ExternalLink } from "lucide-react";
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

interface LeafViewProps {
  nodeId: string;
}

export default function LeafView({ nodeId }: LeafViewProps) {
  const {
    nodes, personalNodes, edges, personalEdges,
    closeLeafView, updateNode, deleteNode, deletePersonalNode,
    updatePersonalNode, navigateToNodeParent, config,
    fetchedContent, isFetching, fetchUrlContent, loadFetchedContent,
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

  const teamNode = nodes.get(nodeId);
  const personalNode = personalNodes.get(nodeId);
  const isPersonal = !teamNode && !!personalNode;
  const node = teamNode || personalNode;

  // Reset state when node changes
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
  }, [nodeId, teamNode, personalNode]);

  // Load cached fetched content on mount / node change
  useEffect(() => {
    loadFetchedContent(nodeId);
  }, [nodeId, loadFetchedContent]);

  // Gather edges
  const nodeEdges: DisplayEdge[] = [];
  for (const e of edges) {
    if (e.source === nodeId || e.target === nodeId) {
      nodeEdges.push({ ...e, type: e.type, isPersonal: false });
    }
  }
  for (const pe of personalEdges) {
    if (pe.sourceId === nodeId || pe.targetId === nodeId) {
      nodeEdges.push({
        id: pe.id, source: pe.sourceId, target: pe.targetId,
        type: pe.edgeType, reason: pe.reason, isPersonal: true,
      });
    }
  }

  const resolveNodeName = (id: string): string => {
    const n = nodes.get(id);
    if (n) return n.aiTitle || n.title;
    const pn = personalNodes.get(id);
    if (pn) return pn.title;
    return id.slice(0, 8) + "...";
  };

  const handleTitleSave = useCallback(async () => {
    if (!teamNode) return;
    setEditingTitle(false);
    if (titleDraft !== (teamNode.aiTitle || teamNode.title)) {
      await updateNode(nodeId, { title: titleDraft, author: config?.author });
    }
  }, [nodeId, teamNode, titleDraft, updateNode, config]);

  const handleContentSave = useCallback(async () => {
    if (!teamNode) return;
    setEditingContent(false);
    if (contentDraft !== (teamNode.content || "")) {
      await updateNode(nodeId, { content: contentDraft, author: config?.author });
    }
  }, [nodeId, teamNode, contentDraft, updateNode, config]);

  const handleDelete = useCallback(async () => {
    if (isPersonal) await deletePersonalNode(nodeId);
    else await deleteNode(nodeId);
    closeLeafView();
  }, [nodeId, isPersonal, deleteNode, deletePersonalNode, closeLeafView]);

  // Category assigner
  const loadAllCategories = useCallback(() => {
    const { nodes: allNodes, localCategories } = useTeamStore.getState();
    const cats: Node[] = [];
    for (const n of allNodes.values()) {
      if ((!n.isItem || localCategories.has(n.id)) && n.id !== nodeId) cats.push(n);
    }
    cats.sort((a, b) => (a.aiTitle || a.title).localeCompare(b.aiTitle || b.title));
    setCategoryResults(cats.slice(0, 100));
  }, [nodeId]);

  const handleCategorySearch = useCallback(async (value: string) => {
    setCategoryQuery(value);
    if (!value.trim()) { loadAllCategories(); return; }
    try {
      const { localCategories } = useTeamStore.getState();
      const results = await invoke<Node[]>("team_search", { query: value, limit: 100 });
      setCategoryResults(results.filter((r) => (!r.isItem || localCategories.has(r.id)) && r.id !== nodeId));
    } catch { setCategoryResults([]); }
  }, [nodeId, loadAllCategories]);

  const handleAssignCategory = useCallback(async (categoryId: string) => {
    if (isPersonal) {
      const { createPersonalEdge } = useTeamStore.getState();
      await createPersonalEdge(categoryId, nodeId, "contains");
    } else {
      await updateNode(nodeId, { parent_id: categoryId, author: config?.author });
    }
    setShowCategoryAssigner(false);
    setCategoryQuery("");
    setCategoryResults([]);
  }, [nodeId, isPersonal, updateNode, config]);

  const formatTime = (ts: number) => {
    const diff = Math.floor((Date.now() - ts) / 1000);
    if (diff < 60) return "just now";
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    return new Date(ts).toLocaleDateString();
  };

  if (!node) {
    return (
      <div className="flex flex-col border-l leaf-view-panel" style={{
        width: 380, minWidth: 380, background: "var(--bg-secondary)", borderColor: "var(--border)",
      }}>
        <div className="flex items-center justify-between px-4 py-3 border-b" style={{ borderColor: "var(--border)" }}>
          <span className="text-sm" style={{ color: "var(--text-secondary)" }}>Node not found</span>
          <button className="btn-secondary p-1" onClick={closeLeafView}><X size={16} /></button>
        </div>
      </div>
    );
  }

  const title = teamNode ? (teamNode.aiTitle || teamNode.title) : personalNode!.title;
  const content = teamNode?.content || personalNode?.content;
  const author = teamNode?.author;
  const contentType = teamNode?.contentType || personalNode?.contentType;
  const nodeTags = teamNode?.tags || personalNode?.tags;

  // Detect URLs in title + content
  const allText = `${title} ${content || ""}`;
  const detectedUrls: string[] = allText.match(/https?:\/\/[^\s<>"')\]]+/g) || [];
  const cachedContent = fetchedContent.get(nodeId);
  const isFetchingThis = isFetching === nodeId;

  return (
    <div className="flex flex-col border-l overflow-hidden leaf-view-panel" style={{
      width: 380, minWidth: 380, background: "var(--bg-secondary)", borderColor: "var(--border)",
    }}>
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
        <button className="btn-secondary p-1" onClick={closeLeafView}><X size={16} /></button>
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
        <div className="flex items-center gap-3 mt-1 text-[11px]" style={{ color: "var(--text-secondary)" }}>
          <span>Created {formatTime(node.createdAt)}</span>
          <span>Updated {formatTime(node.updatedAt)}</span>
        </div>
      </div>

      {/* Content area */}
      <div className="flex-1 overflow-y-auto px-4 py-3">
        {/* Content text */}
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
        {cachedContent && (
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
                  onClick={() => fetchUrlContent(nodeId, cachedContent.url)}
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
              style={{ height: 300, borderColor: "var(--border)", background: "#fff" }}
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
        {!cachedContent && detectedUrls.length > 0 && (() => {
          const primaryUrl = detectedUrls[0];
          let hostname = primaryUrl;
          try { hostname = new URL(primaryUrl).hostname; } catch {}
          return (
            <div className="mt-3 flex flex-col gap-1">
              <button
                className="btn-secondary flex items-center gap-1.5 text-xs w-full justify-center"
                onClick={() => fetchUrlContent(nodeId, primaryUrl)}
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
                  onClick={() => fetchUrlContent(nodeId, url)}
                  disabled={isFetchingThis}
                >
                  {url}
                </button>
              ))}
            </div>
          );
        })()}

        {/* Tags */}
        {nodeTags && (
          <div className="flex flex-wrap gap-1 mt-3">
            {nodeTags.split(",").map((tag) => tag.trim()).filter(Boolean).map((tag) => (
              <span key={tag} className="text-[11px] px-2 py-0.5 rounded-full"
                style={{ background: "var(--bg-tertiary)", color: "var(--text-secondary)" }}>
                {tag}
              </span>
            ))}
          </div>
        )}

        {/* Parent category */}
        {teamNode?.parentId && (
          <div className="mt-3">
            <span className="text-xs uppercase font-medium" style={{ color: "var(--text-secondary)" }}>Parent: </span>
            <button
              className="text-xs hover:underline"
              style={{ color: "var(--accent)" }}
              onClick={() => { closeLeafView(); navigateToNodeParent(nodeId); }}
            >
              {resolveNodeName(teamNode.parentId)}
            </button>
          </div>
        )}

        {/* Connections */}
        {nodeEdges.length > 0 && (
          <div className="mt-4">
            <h3 className="text-xs font-medium uppercase mb-2" style={{ color: "var(--text-secondary)" }}>
              Connections ({nodeEdges.length})
            </h3>
            <div className="flex flex-col gap-1.5">
              {nodeEdges.map((e) => {
                const otherId = e.source === nodeId ? e.target : e.source;
                const direction = e.source === nodeId ? "\u2192" : "\u2190";
                return (
                  <div
                    key={e.id}
                    className="flex items-center gap-2 text-xs px-2 py-1.5 rounded cursor-pointer hover:opacity-80"
                    style={{
                      background: "var(--bg-tertiary)",
                      borderLeft: `3px solid ${e.isPersonal ? "#14b8a6" : "#4b5563"}`,
                    }}
                    onClick={() => { closeLeafView(); navigateToNodeParent(otherId); }}
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

      {/* Footer actions */}
      <div className="flex items-center gap-2 px-4 py-3 border-t flex-wrap" style={{ borderColor: "var(--border)" }}>
        <button className="btn-secondary flex items-center gap-1 text-xs"
          onClick={() => setShowEditModal(true)}>
          <Pencil size={12} /> Edit
        </button>
        <button className="btn-secondary flex items-center gap-1 text-xs"
          onClick={() => { setShowEdgeCreator(!showEdgeCreator); setShowCategoryAssigner(false); }}>
          <Link size={12} /> Add Edge
        </button>
        <button className="btn-secondary flex items-center gap-1 text-xs"
          onClick={() => { setShowCategoryAssigner(!showCategoryAssigner); setShowEdgeCreator(false); }}>
          <FolderInput size={12} /> Assign Category
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
          <Trash2 size={12} /> Delete
        </button>
      </div>

      {/* Edge Creator */}
      {showEdgeCreator && (
        <EdgeCreator sourceId={nodeId} onClose={() => setShowEdgeCreator(false)} />
      )}

      {/* Category Assigner */}
      {showCategoryAssigner && (
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
      {showEditModal && node && (
        <EditNodeModal
          node={{ title, content: content || undefined, contentType: contentType || undefined, tags: nodeTags || undefined }}
          nodeId={nodeId}
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
          <button className="btn-secondary p-1" onClick={onClose}><X size={16} /></button>
        </div>

        <div className="mb-3">
          <label className="block text-xs mb-1" style={{ color: "var(--text-secondary)" }}>Type</label>
          <select value={contentType} onChange={(e) => setContentType(e.target.value)} className="w-full">
            {CONTENT_TYPES.map((t) => (<option key={t} value={t}>{t}</option>))}
          </select>
        </div>

        <div className="mb-3">
          <label className="block text-xs mb-1" style={{ color: "var(--text-secondary)" }}>Title *</label>
          <input type="text" className="w-full" value={title}
            onChange={(e) => setTitle(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && !e.shiftKey && handleSave()}
            autoFocus />
        </div>

        <div className="mb-3">
          <label className="block text-xs mb-1" style={{ color: "var(--text-secondary)" }}>Note</label>
          <textarea className="w-full h-20 resize-none" value={content}
            onChange={(e) => setContent(e.target.value)} />
        </div>

        <div className="mb-4">
          <label className="block text-xs mb-1" style={{ color: "var(--text-secondary)" }}>Tags (comma-separated)</label>
          <input type="text" className="w-full" value={tags}
            onChange={(e) => setTags(e.target.value)} />
        </div>

        <div className="flex justify-end gap-2">
          <button className="btn-secondary" onClick={onClose}>Cancel</button>
          <button className="btn-primary" onClick={handleSave} disabled={!title.trim() || saving}>
            {saving ? "Saving..." : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}
