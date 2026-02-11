import { useEffect } from "react";
import { useTeamStore } from "./stores/teamStore";
import Toolbar from "./components/Toolbar";
import GraphView from "./components/GraphView";
import RecentPanel from "./components/RecentPanel";
import NodePopup from "./components/NodePopup";
import QuickAdd from "./components/QuickAdd";
import Settings from "./components/Settings";

export default function App() {
  const {
    showRecent, showSettings, showQuickAdd, selectedNodeId, error,
    loadSettings, loadPositions, loadPersonalData, refresh,
    setShowQuickAdd, clearError,
  } = useTeamStore();

  useEffect(() => {
    loadSettings();
    loadPositions();
    loadPersonalData();
  }, [loadSettings, loadPositions, loadPersonalData]);

  // Ctrl+N for quick-add
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === "n") {
        e.preventDefault();
        setShowQuickAdd(true);
      }
      if (e.key === "Escape") {
        setShowQuickAdd(false);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [setShowQuickAdd]);

  // Auto-refresh on startup (connection failure is silent — shown via status dot)
  useEffect(() => {
    const { config } = useTeamStore.getState();
    if (config?.server_url) {
      refresh();
    }
  }, [refresh]);

  return (
    <div className="flex flex-col h-screen">
      <Toolbar />

      {error && (
        <div className="flex items-center gap-2 px-4 py-2 text-sm"
          style={{ background: "#7f1d1d", color: "#fca5a5" }}>
          <span className="flex-1">{error}</span>
          <button className="btn-secondary text-xs" onClick={clearError}>Dismiss</button>
        </div>
      )}

      <div className="flex flex-1 overflow-hidden">
        {showRecent && <RecentPanel />}
        <GraphView />
      </div>

      {selectedNodeId && <NodePopup />}
      {showQuickAdd && <QuickAdd />}
      {showSettings && <Settings />}
    </div>
  );
}
