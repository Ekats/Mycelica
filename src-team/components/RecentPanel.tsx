import { useState, useEffect } from "react";
import { ChevronLeft } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useTeamStore } from "../stores/teamStore";
import type { Node } from "../types";

export default function RecentPanel() {
  const { setShowRecent, navigateToNodeParent } = useTeamStore();
  const [recent, setRecent] = useState<Node[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    setLoading(true);
    invoke<Node[]>("team_get_recent", { limit: 20 })
      .then(setRecent)
      .catch((e) => console.error("Failed to load recent:", e))
      .finally(() => setLoading(false));
  }, []);

  const formatTime = (ts: number) => {
    const d = new Date(ts);
    const now = Date.now();
    const diff = Math.floor((now - ts) / 1000);
    if (diff < 60) return "just now";
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    return d.toLocaleDateString();
  };

  return (
    <div className="flex flex-col border-r" style={{
      width: 280,
      minWidth: 280,
      background: "var(--bg-secondary)",
      borderColor: "var(--border)",
    }}>
      <div className="flex items-center gap-2 px-3 py-2 border-b" style={{ borderColor: "var(--border)" }}>
        <h3 className="text-sm font-medium flex-1">Recent Activity</h3>
        <button className="btn-secondary p-1" onClick={() => setShowRecent(false)}>
          <ChevronLeft size={14} />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <p className="text-xs px-3 py-4" style={{ color: "var(--text-secondary)" }}>Loading...</p>
        ) : recent.length === 0 ? (
          <p className="text-xs px-3 py-4" style={{ color: "var(--text-secondary)" }}>No recent activity</p>
        ) : (
          recent.map((node) => (
            <div
              key={node.id}
              className="px-3 py-2 border-b cursor-pointer hover:opacity-80"
              style={{ borderColor: "var(--bg-tertiary)" }}
              onClick={() => navigateToNodeParent(node.id)}
            >
              <p className="text-sm truncate">{node.aiTitle || node.title}</p>
              <div className="flex items-center gap-2 mt-0.5">
                {node.author && (
                  <span className="text-[10px]" style={{ color: "#f59e0b" }}>{node.author}</span>
                )}
                {node.contentType && (
                  <span className="text-[10px]" style={{ color: "var(--text-secondary)" }}>{node.contentType}</span>
                )}
                <span className="text-[10px] ml-auto" style={{ color: "var(--text-secondary)" }}>
                  {formatTime(node.updatedAt)}
                </span>
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
