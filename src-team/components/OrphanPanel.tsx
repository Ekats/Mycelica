import { useState, useEffect } from "react";
import { ChevronLeft } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useTeamStore } from "../stores/teamStore";
import type { Node } from "../types";

export default function OrphanPanel() {
  const { setShowOrphans, setSelectedNodeId } = useTeamStore();
  const [orphans, setOrphans] = useState<Node[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    setLoading(true);
    invoke<Node[]>("team_get_orphans", { limit: 100 })
      .then(setOrphans)
      .catch((e) => console.error("Failed to load orphans:", e))
      .finally(() => setLoading(false));
  }, []);

  return (
    <div className="flex flex-col border-r" style={{
      width: 256,
      minWidth: 256,
      background: "var(--bg-secondary)",
      borderColor: "var(--border)",
    }}>
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2 border-b" style={{ borderColor: "var(--border)" }}>
        <h3 className="text-sm font-medium flex-1">Orphans ({orphans.length})</h3>
        <button className="btn-secondary p-1" onClick={() => setShowOrphans(false)}>
          <ChevronLeft size={14} />
        </button>
      </div>

      {/* List */}
      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <p className="text-xs px-3 py-4" style={{ color: "var(--text-secondary)" }}>Loading...</p>
        ) : orphans.length === 0 ? (
          <p className="text-xs px-3 py-4" style={{ color: "var(--text-secondary)" }}>No orphan nodes</p>
        ) : (
          orphans.map((node) => (
            <div
              key={node.id}
              className="px-3 py-2 border-b cursor-pointer hover:opacity-80"
              style={{ borderColor: "var(--bg-tertiary)" }}
              onClick={() => setSelectedNodeId(node.id)}
            >
              <p className="text-sm truncate">{node.aiTitle || node.title}</p>
              <div className="flex items-center gap-2 mt-0.5">
                {node.author && (
                  <span className="text-[10px]" style={{ color: "#f59e0b" }}>{node.author}</span>
                )}
                {node.contentType && (
                  <span className="text-[10px]" style={{ color: "var(--text-secondary)" }}>{node.contentType}</span>
                )}
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
