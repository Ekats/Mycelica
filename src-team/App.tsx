import { useEffect, useMemo, useState } from "react";
import { useTeamStore } from "./stores/teamStore";
import Toolbar from "./components/Toolbar";
import Breadcrumb from "./components/Breadcrumb";
import GraphView from "./components/GraphView";
import RecentPanel from "./components/RecentPanel";
import NodePopup from "./components/NodePopup";
import LeafView from "./components/LeafView";
import SignalConversationRenderer from "./components/SignalConversationRenderer";
import QuickAdd from "./components/QuickAdd";
import Settings from "./components/Settings";

export default function App() {
  const {
    showRecent, showSettings, showQuickAdd, selectedNodeId, error,
    leafViewNodeId,
    loadSettings, loadPositions, loadPersonalData, loadLocalNodes, refresh,
    setShowQuickAdd, clearError, navigateBack, breadcrumbs,
    currentParentId, nodes,
  } = useTeamStore();

  const [signalViewMode, setSignalViewMode] = useState<'graph' | 'timeline'>('graph');

  const isSignalContainer = useMemo(() => {
    if (!currentParentId) return false;
    const parent = nodes.get(currentParentId);
    return parent?.source === 'signal' && !parent?.isItem;
  }, [currentParentId, nodes]);

  // Reset to graph when leaving a signal container
  useEffect(() => {
    if (!isSignalContainer) setSignalViewMode('graph');
  }, [isSignalContainer]);

  useEffect(() => {
    // Load settings first, then refresh (settings are async — config must be ready before refresh)
    loadSettings().then(() => {
      const { config } = useTeamStore.getState();
      if (config?.server_url) refresh();
    });
    loadPositions();
    loadPersonalData();
    loadLocalNodes();
  }, [loadSettings, loadPositions, loadPersonalData, loadLocalNodes, refresh]);

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

      <Breadcrumb
        isSignalContainer={isSignalContainer}
        signalViewMode={signalViewMode}
        onToggleSignalView={() => setSignalViewMode(m => m === 'graph' ? 'timeline' : 'graph')}
      />

      <div className="flex flex-1 overflow-hidden">
        {leafViewNodeId ? (
          <LeafView nodeId={leafViewNodeId} />
        ) : isSignalContainer && signalViewMode === 'timeline' ? (
          <SignalConversationRenderer containerId={currentParentId!} />
        ) : (
          <>
            {showRecent && <RecentPanel />}
            <GraphView />
          </>
        )}
      </div>

      {selectedNodeId && !leafViewNodeId && <NodePopup />}
      {showQuickAdd && <QuickAdd />}
      {showSettings && <Settings />}
    </div>
  );
}
