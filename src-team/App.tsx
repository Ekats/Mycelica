import { useEffect } from "react";
import { useTeamStore } from "./stores/teamStore";
import Toolbar from "./components/Toolbar";
import Breadcrumb from "./components/Breadcrumb";
import GraphView from "./components/GraphView";
import RecentPanel from "./components/RecentPanel";
import NodePopup from "./components/NodePopup";
import LeafView from "./components/LeafView";
import QuickAdd from "./components/QuickAdd";
import Settings from "./components/Settings";

export default function App() {
  const {
    showRecent, showSettings, showQuickAdd, selectedNodeId, error,
    leafViewNodeId,
    loadSettings, loadPositions, loadPersonalData, refresh,
    setShowQuickAdd, clearError, navigateBack, breadcrumbs,
  } = useTeamStore();

  useEffect(() => {
    // Load settings first, then refresh (settings are async — config must be ready before refresh)
    loadSettings().then(() => {
      const { config } = useTeamStore.getState();
      if (config?.server_url) refresh();
    });
    loadPositions();
    loadPersonalData();
  }, [loadSettings, loadPositions, loadPersonalData, refresh]);

  // Ctrl+N for quick-add, Backspace to navigate back
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === "n") {
        e.preventDefault();
        setShowQuickAdd(true);
      }
      if (e.key === "Escape") {
        if (useTeamStore.getState().leafViewNodeId) {
          useTeamStore.getState().closeLeafView();
          return;
        }
        setShowQuickAdd(false);
      }
      if (e.key === "Backspace" && breadcrumbs.length > 0 &&
          !(e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLSelectElement)) {
        e.preventDefault();
        navigateBack();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [setShowQuickAdd, navigateBack, breadcrumbs.length]);

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

      <Breadcrumb />

      <div className="flex flex-1 overflow-hidden">
        {showRecent && <RecentPanel />}
        <GraphView />
        {leafViewNodeId && <LeafView nodeId={leafViewNodeId} />}
      </div>

      {selectedNodeId && !leafViewNodeId && <NodePopup />}
      {showQuickAdd && <QuickAdd />}
      {showSettings && <Settings />}
    </div>
  );
}
