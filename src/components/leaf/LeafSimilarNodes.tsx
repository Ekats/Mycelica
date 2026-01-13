import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Sparkles, ChevronLeft } from 'lucide-react';
import { getSimilarityColor } from '../../utils/similarityColor';

interface SimilarNode {
  id: string;
  title: string;
  emoji: string | null;
  summary: string | null;
  similarity: number;
  edgeType: string | null;  // "calls", "called by", "documents", etc.
}

interface HistoryEntry {
  id: string;
  title: string;
  emoji: string | null;
}

interface LeafSimilarNodesProps {
  nodeId: string;
  nodeTitle: string;
  nodeEmoji?: string;
  onNavigate: (nodeId: string) => void;
}

export function LeafSimilarNodes({ nodeId, nodeTitle, nodeEmoji, onNavigate }: LeafSimilarNodesProps) {
  const [similarNodes, setSimilarNodes] = useState<SimilarNode[]>([]);
  const [loading, setLoading] = useState(true);
  const [expanded, setExpanded] = useState(true);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const currentNodeRef = useRef<HistoryEntry | null>(null);

  // Update current node ref when props change
  useEffect(() => {
    currentNodeRef.current = { id: nodeId, title: nodeTitle, emoji: nodeEmoji || null };
  }, [nodeId, nodeTitle, nodeEmoji]);

  useEffect(() => {
    const fetchSimilar = async () => {
      setLoading(true);
      try {
        const similar = await invoke<SimilarNode[]>('get_similar_nodes', {
          nodeId,
          topN: 50,
          minSimilarity: 0.0,  // Show all similarities
        });
        setSimilarNodes(similar);
      } catch (err) {
        console.error('Failed to fetch similar nodes:', err);
        setSimilarNodes([]);
      } finally {
        setLoading(false);
      }
    };

    fetchSimilar();
  }, [nodeId]);

  const handleBack = () => {
    if (history.length > 0) {
      const prev = history[history.length - 1];
      setHistory(h => h.slice(0, -1));
      onNavigate(prev.id);
    }
  };

  const handleNavigate = (node: SimilarNode) => {
    // Save current node to history before navigating
    if (currentNodeRef.current) {
      setHistory(prev => [...prev, currentNodeRef.current!]);
    }
    onNavigate(node.id);
  };

  if (loading) {
    return (
      <div className="p-4">
        <div className="flex items-center gap-2 text-gray-400 text-sm">
          <Sparkles className="w-4 h-4 animate-pulse" />
          <span>Finding similar...</span>
        </div>
      </div>
    );
  }

  if (similarNodes.length === 0 && history.length === 0) {
    return null;
  }

  return (
    <div className="p-4">
      {/* Back button when there's history */}
      {history.length > 0 && (
        <button
          onClick={handleBack}
          className="flex items-center gap-2 w-full p-2 mb-3 rounded bg-gray-700/50 hover:bg-gray-700 text-gray-300 hover:text-white transition-colors"
        >
          <ChevronLeft className="w-4 h-4" />
          <span className="text-sm truncate">
            {history[history.length - 1].emoji || '📄'} {history[history.length - 1].title}
          </span>
          {history.length > 1 && (
            <span className="text-xs text-gray-500 ml-auto">+{history.length - 1}</span>
          )}
        </button>
      )}

      <button
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-2 text-sm font-medium text-gray-400 hover:text-gray-300 mb-3 w-full"
      >
        <Sparkles className="w-4 h-4 text-amber-400" />
        <span>Similar Nodes ({similarNodes.length})</span>
        <span className="ml-auto text-xs">{expanded ? '▼' : '▶'}</span>
      </button>

      {expanded && (
        <div className="space-y-1">
          {similarNodes.map((node) => (
            <button
              key={node.id}
              onClick={() => handleNavigate(node)}
              className="w-full flex items-start gap-2 p-2 rounded hover:bg-gray-700/50 text-left transition-colors group"
            >
              <span className="text-base shrink-0">{node.emoji || '📄'}</span>
              <div className="min-w-0 flex-1">
                <div className="text-sm text-white truncate group-hover:text-amber-300 transition-colors">
                  {node.title}
                </div>
                {node.summary && (
                  <div className="text-xs text-gray-500 truncate mt-0.5">
                    {node.summary}
                  </div>
                )}
              </div>
              <span
                className={`text-xs shrink-0 font-medium ${node.edgeType ? 'text-blue-400' : ''}`}
                style={node.edgeType ? undefined : { color: getSimilarityColor(node.similarity) }}
              >
                {node.edgeType || `${Math.round(node.similarity * 100)}%`}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
