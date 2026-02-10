import { useState, useCallback } from "react";
import { RefreshCw, Search, Network, Settings, Plus } from "lucide-react";
import { useTeamStore } from "../stores/teamStore";

export default function Toolbar() {
  const {
    isRefreshing, lastRefreshed, connected, nodes, showOrphans,
    refresh, search, setShowOrphans, setShowSettings, setShowQuickAdd,
    getOrphanCount,
  } = useTeamStore();

  const [searchInput, setSearchInput] = useState("");
  const orphanCount = getOrphanCount();

  const handleSearch = useCallback((value: string) => {
    setSearchInput(value);
    search(value);
  }, [search]);

  const timeAgo = lastRefreshed
    ? formatTimeAgo(lastRefreshed)
    : "never";

  return (
    <div className="flex items-center gap-3 px-4 py-2 border-b"
      style={{ background: "var(--bg-secondary)", borderColor: "var(--border)" }}>

      {/* Refresh */}
      <button
        className="btn-primary flex items-center gap-1.5"
        onClick={() => refresh()}
        disabled={isRefreshing}
      >
        <RefreshCw size={14} className={isRefreshing ? "animate-spin" : ""} />
        Refresh
      </button>

      {/* Search */}
      <div className="flex items-center gap-1.5 flex-1 max-w-md">
        <Search size={14} style={{ color: "var(--text-secondary)" }} />
        <input
          type="text"
          placeholder="Search nodes..."
          value={searchInput}
          onChange={(e) => handleSearch(e.target.value)}
          className="flex-1"
          style={{ fontSize: "13px", padding: "4px 8px" }}
        />
      </div>

      {/* Orphan badge */}
      <button
        className={`flex items-center gap-1.5 ${showOrphans ? "btn-primary" : "btn-secondary"}`}
        onClick={() => setShowOrphans(!showOrphans)}
      >
        <Network size={14} />
        Orphans
        {orphanCount > 0 && (
          <span className="inline-flex items-center justify-center rounded-full text-xs font-bold min-w-5 h-5 px-1"
            style={{ background: "#f59e0b", color: "#111827" }}>
            {orphanCount}
          </span>
        )}
      </button>

      {/* Quick add */}
      <button className="btn-secondary flex items-center gap-1"
        onClick={() => setShowQuickAdd(true)}
        title="Quick Add (Ctrl+N)"
      >
        <Plus size={14} />
      </button>

      {/* Settings */}
      <button className="btn-secondary" onClick={() => setShowSettings(true)} title="Settings">
        <Settings size={14} />
      </button>

      {/* Status */}
      <div className="flex items-center gap-2 text-xs ml-auto" style={{ color: "var(--text-secondary)" }}>
        <span className="inline-block w-2 h-2 rounded-full"
          style={{ background: connected ? "#10b981" : "#ef4444" }}
          title={connected ? "Connected" : "Server offline"} />
        <span>{nodes.size} nodes</span>
        <span>|</span>
        <span>{timeAgo}</span>
      </div>
    </div>
  );
}

function formatTimeAgo(date: Date): string {
  const seconds = Math.floor((Date.now() - date.getTime()) / 1000);
  if (seconds < 10) return "just now";
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ago`;
}
