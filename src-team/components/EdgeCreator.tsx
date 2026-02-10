import { useState, useCallback } from "react";
import { X } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useTeamStore } from "../stores/teamStore";
import type { EdgeType, Node } from "../types";

const EDGE_TYPES: EdgeType[] = [
  "related", "reference", "because", "contains",
  "prerequisite", "contradicts", "supports", "evolved_from", "questions",
];

interface Props {
  sourceId: string;
  onClose: () => void;
}

export default function EdgeCreator({ sourceId, onClose }: Props) {
  const { createEdge, createPersonalEdge, config } = useTeamStore();

  const [mode, setMode] = useState<"team" | "personal">("team");
  const [edgeType, setEdgeType] = useState<EdgeType>("related");
  const [reason, setReason] = useState("");
  const [targetQuery, setTargetQuery] = useState("");
  const [targetId, setTargetId] = useState<string | null>(null);
  const [results, setResults] = useState<Node[]>([]);
  const [submitting, setSubmitting] = useState(false);

  const handleSearch = useCallback(async (value: string) => {
    setTargetQuery(value);
    if (value.trim().length < 2) {
      setResults([]);
      return;
    }
    try {
      const nodes = await invoke<Node[]>("team_search", { query: value, limit: 10 });
      setResults(nodes);
    } catch {
      setResults([]);
    }
  }, []);

  const handleSubmit = useCallback(async () => {
    if (!targetId) return;
    setSubmitting(true);
    try {
      if (mode === "team") {
        await createEdge({
          source: sourceId,
          target: targetId,
          edge_type: edgeType,
          reason: reason.trim() || undefined,
          author: config?.author,
        });
      } else {
        await createPersonalEdge(sourceId, targetId, edgeType, reason.trim() || undefined);
      }
      onClose();
    } finally {
      setSubmitting(false);
    }
  }, [mode, sourceId, targetId, edgeType, reason, config, createEdge, createPersonalEdge, onClose]);

  return (
    <div className="border-t px-4 py-3" style={{ borderColor: "var(--border)", background: "var(--bg-primary)" }}>
      <div className="flex items-center justify-between mb-2">
        <h4 className="text-xs font-medium uppercase" style={{ color: "var(--text-secondary)" }}>New Edge</h4>
        <button className="btn-secondary p-0.5" onClick={onClose}><X size={12} /></button>
      </div>

      {/* Team/Personal toggle */}
      <div className="flex gap-1 mb-2 p-0.5 rounded" style={{ background: "var(--bg-tertiary)" }}>
        <button
          className={`flex-1 py-1 rounded text-xs ${mode === "team" ? "btn-primary" : ""}`}
          onClick={() => setMode("team")}
        >Team</button>
        <button
          className={`flex-1 py-1 rounded text-xs`}
          style={mode === "personal" ? { background: "#14b8a6", color: "#111827" } : {}}
          onClick={() => setMode("personal")}
        >Personal</button>
      </div>

      <select value={edgeType} onChange={(e) => setEdgeType(e.target.value as EdgeType)}
        className="w-full mb-2 text-xs" style={{ padding: "4px 6px" }}>
        {EDGE_TYPES.map((t) => (
          <option key={t} value={t}>{t}</option>
        ))}
      </select>

      <input
        type="text"
        className="w-full mb-1 text-xs"
        placeholder="Search target node..."
        value={targetQuery}
        onChange={(e) => handleSearch(e.target.value)}
        style={{ padding: "4px 6px" }}
      />

      {targetQuery && results.length > 0 && (
        <div className="flex flex-col gap-0.5 max-h-24 overflow-y-auto rounded mb-2" style={{ background: "var(--bg-tertiary)" }}>
          {results.filter((r) => r.id !== sourceId).slice(0, 5).map((r) => (
            <button
              key={r.id}
              className="text-left text-xs px-2 py-1 hover:opacity-80"
              style={{
                background: targetId === r.id ? "var(--accent)" : "transparent",
                color: targetId === r.id ? "#111827" : "var(--text-primary)",
              }}
              onClick={() => setTargetId(r.id)}
            >
              {r.aiTitle || r.title}
            </button>
          ))}
        </div>
      )}

      <input
        type="text"
        className="w-full mb-2 text-xs"
        placeholder="Reason (optional)"
        value={reason}
        onChange={(e) => setReason(e.target.value)}
        style={{ padding: "4px 6px" }}
      />

      <button
        className="btn-primary w-full text-xs"
        onClick={handleSubmit}
        disabled={!targetId || submitting}
      >
        {submitting ? "Creating..." : "Create Edge"}
      </button>
    </div>
  );
}
