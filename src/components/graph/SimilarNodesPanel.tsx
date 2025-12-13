import { memo, useState, useCallback } from 'react';
import type { Node } from '../../types/graph';

interface SimilarNode {
  id: string;
  title: string;
  emoji: string | null;
  summary: string | null;
  similarity: number;
}

interface SimilarNodesPanelProps {
  similarNodesMap: Map<string, SimilarNode[]>;
  nodes: Map<string, Node>;
  currentParentId: string | null;
  stackNodes: boolean;
  detailsPanelSize: { width: number; height: number };
  isResizingDetails: boolean;
  getNodeEmoji: (node: Node) => string;
  onJumpToNode: (targetNode: Node, sourceNode: Node | undefined) => void;
  onRemoveNode: (nodeId: string) => void;
  onClearAll: () => void;
  onToggleStack: () => void;
  onStartResize: () => void;
  devLog: (type: 'info' | 'warn' | 'error', message: string) => void;
}

const SIMILAR_INITIAL_COUNT = 10;

export const SimilarNodesPanel = memo(function SimilarNodesPanel({
  similarNodesMap,
  nodes,
  currentParentId,
  stackNodes,
  detailsPanelSize,
  isResizingDetails,
  getNodeEmoji,
  onJumpToNode,
  onRemoveNode,
  onClearAll,
  onToggleStack,
  onStartResize,
  devLog,
}: SimilarNodesPanelProps) {
  // Local state for expand/collapse
  const [expandedSimilar, setExpandedSimilar] = useState<Set<string>>(new Set());
  const [collapsedSimilar, setCollapsedSimilar] = useState<Set<string>>(new Set());

  // Build hierarchy path for a node
  const getHierarchyPath = useCallback((nodeId: string): { name: string; isUniverse: boolean }[] => {
    const path: { name: string; isUniverse: boolean }[] = [];
    let current = nodes.get(nodeId);
    while (current?.parentId) {
      const parent = nodes.get(current.parentId);
      if (parent) {
        path.unshift({
          name: parent.isUniverse ? '🌌' : (parent.aiTitle || parent.title || 'Untitled'),
          isUniverse: parent.isUniverse || false
        });
      }
      current = parent;
    }
    return path;
  }, [nodes]);

  if (similarNodesMap.size === 0) return null;

  return (
    <div
      className="absolute top-16 right-4 bg-gray-800/95 backdrop-blur-sm text-white rounded-lg shadow-xl border border-gray-700 z-30 overflow-hidden flex flex-col"
      style={{ width: detailsPanelSize.width, height: detailsPanelSize.height }}
    >
      {/* Panel header with stack toggle */}
      <div className="px-3 py-2 border-b border-gray-700 flex items-center justify-between flex-shrink-0">
        <span className="text-xs font-medium text-gray-400">Node Details</span>
        <div className="flex items-center gap-2">
          <button
            onClick={onToggleStack}
            className={`text-xs px-2 py-0.5 rounded transition-colors ${
              stackNodes
                ? 'bg-amber-500/30 text-amber-300'
                : 'bg-gray-700 text-gray-400 hover:text-gray-300'
            }`}
            title={stackNodes ? 'Stack mode: ON (click nodes to compare)' : 'Stack mode: OFF (single node view)'}
          >
            {stackNodes ? '📚 Stack' : '📄 Single'}
          </button>
          <button
            onClick={onClearAll}
            className="text-gray-500 hover:text-gray-300 text-xs"
          >
            ✕
          </button>
        </div>
      </div>
      <div className="overflow-y-auto flex-1">
        {Array.from(similarNodesMap.entries()).map(([sourceId, similarNodes]) => {
          const sourceNode = nodes.get(sourceId);
          const sourceParent = sourceNode?.parentId;
          const isExpanded = expandedSimilar.has(sourceId);
          const displayNodes = isExpanded ? similarNodes : similarNodes.slice(0, SIMILAR_INITIAL_COUNT);
          const hasMore = similarNodes.length > SIMILAR_INITIAL_COUNT;
          return (
            <div key={sourceId} className="border-b border-gray-700/50 last:border-0">
              {/* Source node details header */}
              <div className="p-4 bg-gray-700/40">
                <div className="flex items-start justify-between gap-2 mb-2">
                  <h3 className="text-sm font-semibold text-white leading-tight">
                    {sourceNode && <span className="mr-1.5">{getNodeEmoji(sourceNode)}</span>}
                    {sourceNode?.aiTitle || sourceNode?.title || sourceId}
                  </h3>
                  <button
                    onClick={() => onRemoveNode(sourceId)}
                    className="text-gray-500 hover:text-gray-300 p-1"
                  >
                    <span className="text-xs">✕</span>
                  </button>
                </div>
                {/* Metadata */}
                {sourceNode && (
                  <div className="text-xs text-gray-400 mb-2">
                    {new Date(sourceNode.createdAt).toLocaleDateString()} at{' '}
                    {new Date(sourceNode.createdAt).toLocaleTimeString()}
                  </div>
                )}
                {/* Summary */}
                {(sourceNode?.summary || sourceNode?.content) && (
                  <div className="text-sm text-gray-300 mb-3 p-3 bg-gray-900/50 rounded border border-gray-700/50 max-h-48 overflow-y-auto">
                    {(sourceNode.summary || sourceNode.content || '').split(', ').map((item, i) => (
                      <div key={i} className="py-0.5">• {item}</div>
                    ))}
                  </div>
                )}
                {/* URL */}
                {sourceNode?.url && (
                  <a
                    href={sourceNode.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="inline-flex items-center gap-1 text-xs text-amber-400 hover:text-amber-300 hover:underline"
                  >
                    Open in Claude →
                  </a>
                )}
              </div>

              {/* Similar nodes header - clickable to collapse */}
              <button
                onClick={() => setCollapsedSimilar(prev => {
                  const next = new Set(prev);
                  if (next.has(sourceId)) {
                    next.delete(sourceId);
                  } else {
                    next.add(sourceId);
                  }
                  return next;
                })}
                className="w-full px-3 py-1.5 bg-gray-700/20 text-xs text-gray-400 border-t border-gray-700/50 flex items-center justify-between hover:bg-gray-700/40 transition-colors"
              >
                <span className="flex items-center gap-1">
                  <span>{collapsedSimilar.has(sourceId) ? '▶' : '▼'}</span>
                  <span>↔ Similar ({similarNodes.length})</span>
                </span>
              </button>

              {/* Similar nodes list - collapsible */}
              {!collapsedSimilar.has(sourceId) && (
                <>
                  {displayNodes.map((similar) => {
                    const similarNode = nodes.get(similar.id);
                    // Check if in same view: similar node's parent matches current view's parent
                    const isInSameView = similarNode?.parentId === currentParentId ||
                      (similarNode?.parentId === sourceParent && sourceParent !== undefined);
                    const hierarchyPath = getHierarchyPath(similar.id);
                    const sourceHierarchy = getHierarchyPath(sourceId);
                    return (
                      <button
                        key={similar.id}
                        onClick={() => {
                          const targetNode = nodes.get(similar.id);
                          if (targetNode) {
                            onJumpToNode(targetNode, sourceNode);
                            devLog('info', `Jumped to similar node: ${similar.title}`);
                          }
                        }}
                        className="w-full px-3 py-2 text-left hover:bg-gray-700/50 transition-colors cursor-pointer"
                        title={similar.title}
                      >
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-sm truncate">
                            {isInSameView ? (
                              <span className="text-green-400 mr-1" title="In current view">●</span>
                            ) : (
                              <span className="mr-1 inline-block w-[1em]"></span>
                            )}
                            {similar.emoji || '📄'} {similar.title}
                          </span>
                          <span
                            className="text-xs shrink-0"
                            style={{ color: `hsl(${similar.similarity * 120}, 70%, 50%)` }}
                          >
                            {(similar.similarity * 100).toFixed(0)}%
                          </span>
                        </div>
                        {hierarchyPath.length > 0 && (
                          <div className="text-xs truncate mt-0.5 pl-5">
                            {hierarchyPath.map((segment, idx) => {
                              const isShared = idx < sourceHierarchy.length && sourceHierarchy[idx].name === segment.name;
                              // Universe gets special purple color
                              const colorClass = segment.isUniverse
                                ? 'text-purple-400'
                                : isShared ? 'text-amber-400/70' : 'text-gray-400';
                              return (
                                <span key={idx}>
                                  {idx > 0 && <span className="text-gray-600"> › </span>}
                                  <span className={colorClass}>{segment.name}</span>
                                </span>
                              );
                            })}
                          </div>
                        )}
                      </button>
                    );
                  })}
                  {hasMore && !isExpanded && (
                    <button
                      onClick={() => setExpandedSimilar(prev => new Set(prev).add(sourceId))}
                      className="w-full px-3 py-2 text-left text-blue-400 hover:bg-gray-700/50 transition-colors text-sm"
                    >
                      Show more... ({similarNodes.length - SIMILAR_INITIAL_COUNT} remaining)
                    </button>
                  )}
                  {hasMore && isExpanded && (
                    <button
                      onClick={() => setExpandedSimilar(prev => {
                        const next = new Set(prev);
                        next.delete(sourceId);
                        return next;
                      })}
                      className="w-full px-3 py-2 text-left text-blue-400 hover:bg-gray-700/50 transition-colors text-sm"
                    >
                      Show less
                    </button>
                  )}
                </>
              )}
            </div>
          );
        })}
      </div>
      {/* Resize handle - bottom-left corner */}
      <div
        onMouseDown={(e) => {
          e.preventDefault();
          onStartResize();
        }}
        className={`absolute bottom-0 left-0 w-4 h-4 cursor-sw-resize group ${isResizingDetails ? 'bg-amber-500/30' : ''}`}
        title="Drag to resize"
      >
        <svg
          className="w-3 h-3 absolute bottom-0.5 left-0.5 text-gray-500 group-hover:text-amber-400 transition-colors"
          viewBox="0 0 12 12"
          fill="currentColor"
        >
          <path d="M0 12L12 0v3L3 12H0zm0-5l7-7v3L3 10v2H0V7z" />
        </svg>
      </div>
    </div>
  );
});

// Loading indicator component (also memoized)
export const SimilarNodesLoading = memo(function SimilarNodesLoading({
  loadingSimilar
}: { loadingSimilar: Set<string> }) {
  if (loadingSimilar.size === 0) return null;

  return (
    <div className="absolute top-20 right-4 bg-gray-800/95 backdrop-blur-sm text-white rounded-lg shadow-xl border border-gray-700 z-30 px-4 py-3">
      <span className="text-xs text-gray-400">Loading similar nodes...</span>
    </div>
  );
});
