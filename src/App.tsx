import { useEffect, useRef, useState, Component, ErrorInfo, ReactNode } from 'react';
import { Graph } from './components/graph/Graph';
import { LeafView } from './components/leaf/LeafView';
import { Sidebar } from './components/sidebar/Sidebar';
import { SettingsPanel } from './components/settings/SettingsPanel';
import { useGraph } from './hooks/useGraph';
import { useGraphStore } from './stores/graphStore';
import './App.css';

// Error boundary to catch React errors
interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
}

class ErrorBoundary extends Component<{ children: ReactNode }, ErrorBoundaryState> {
  constructor(props: { children: ReactNode }) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error('React error:', error, errorInfo);
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="h-screen w-screen flex items-center justify-center bg-red-900 p-8">
          <div className="text-center max-w-2xl">
            <div className="text-4xl mb-4">💥</div>
            <h1 className="text-2xl font-bold text-white mb-4">Something went wrong</h1>
            <pre className="text-left bg-black/50 p-4 rounded text-red-200 text-sm overflow-auto max-h-64">
              {this.state.error?.message}
              {'\n\n'}
              {this.state.error?.stack}
            </pre>
            <button
              onClick={() => window.location.reload()}
              className="mt-4 px-4 py-2 bg-white text-red-900 rounded font-medium"
            >
              Reload App
            </button>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}

function App() {
  const containerRef = useRef<HTMLDivElement>(null);
  const [dimensions, setDimensions] = useState({ width: 800, height: 600 });
  const [loading, setLoading] = useState(true);
  const [showSettings, setShowSettings] = useState(false);
  const { nodes, edges, reload } = useGraph();
  const { viewMode, leafNodeId, closeLeaf } = useGraphStore();

  useEffect(() => {
    // Skip if still loading (container not in DOM yet)
    if (loading) return;

    const updateDimensions = () => {
      if (containerRef.current) {
        const rect = containerRef.current.getBoundingClientRect();
        // Only update if we have valid dimensions (flex layout complete)
        if (rect.width > 0 && rect.height > 0) {
          setDimensions({ width: rect.width, height: rect.height });
        }
      }
    };

    // Use ResizeObserver for reliable dimension tracking
    const observer = new ResizeObserver(updateDimensions);
    if (containerRef.current) {
      observer.observe(containerRef.current);
    }

    // Also check on animation frame for initial render
    requestAnimationFrame(updateDimensions);

    window.addEventListener('resize', updateDimensions);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', updateDimensions);
    };
  }, [loading]);

  // Track loading state - short timeout for initial render
  useEffect(() => {
    const timer = setTimeout(() => {
      setLoading(false);
    }, 500);
    return () => clearTimeout(timer);
  }, []);

  // Also stop loading when nodes arrive
  useEffect(() => {
    if (nodes.size > 0) {
      setLoading(false);
    }
  }, [nodes]);

  if (loading) {
    return (
      <div className="h-screen w-screen flex items-center justify-center bg-gray-900">
        <div className="text-center">
          <div className="text-4xl mb-4">🍄</div>
          <h1 className="text-2xl font-bold text-white mb-2">Loading Mycelica...</h1>
          <p className="text-gray-400">Retrieving your knowledge graph</p>
          <p className="text-gray-500 text-sm mt-4">
            Debug: {nodes.size} nodes, {edges.size} edges loaded
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="h-screen w-screen flex bg-gray-900">
      <Sidebar onOpenSettings={() => setShowSettings(true)} />
      <main ref={containerRef} className="flex-1 relative overflow-hidden">
        {viewMode === 'leaf' && leafNodeId ? (
          <LeafView nodeId={leafNodeId} onBack={closeLeaf} />
        ) : (
          <Graph width={dimensions.width} height={dimensions.height} />
        )}
      </main>
      <SettingsPanel
        open={showSettings}
        onClose={() => setShowSettings(false)}
        onDataChanged={reload}
      />
    </div>
  );
}

function AppWithErrorBoundary() {
  return (
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  );
}

export default AppWithErrorBoundary;
