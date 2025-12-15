import { memo, useState, useCallback, useMemo } from 'react';
import { Pin, PinOff, Lock, LockOpen, GitBranch, GitMerge } from 'lucide-react';
import type { Node } from '../../types/graph';

interface SimilarNode {
  id: string;
  title: string;
  emoji: string | null;
  summary: string | null;
  similarity: number;
}

// Hierarchical group of similar nodes
interface SimilarGroup {
  id: string;           // Parent node id (or 'root' for top level)
  name: string;         // Display name
  emoji: string;        // Display emoji
  avgSimilarity: number; // Weighted average of top 10 items
  items: SimilarNode[]; // Direct items in this group
  children: SimilarGroup[]; // Nested groups
  depth: number;        // Hierarchy depth
}

interface SimilarNodesPanelProps {
  similarNodesMap: Map<string, SimilarNode[]>;
  nodes: Map<string, Node>;
  currentParentId: string | null;
  stackNodes: boolean;
  detailsPanelSize: { width: number; height: number };
  isResizingDetails: boolean;
  pinnedIds: Set<string>;
  getNodeEmoji: (node: Node) => string;
  onJumpToNode: (targetNode: Node, sourceNode: Node | undefined) => void;
  onFetchDetails: (nodeId: string) => void;  // Fetch and show details for a node
  onRemoveNode: (nodeId: string) => void;
  onTogglePin: (nodeId: string, currentlyPinned: boolean) => void;
  onTogglePrivacy: (nodeId: string, currentlyPrivate: boolean) => void;
  onSplitNode?: (nodeId: string) => void;  // Split node's children into sub-categories
  onUnsplitNode?: (nodeId: string) => void;  // Flatten intermediate categories back into parent
  onClearAll: () => void;
  onToggleStack: () => void;
  onStartResize: () => void;
  devLog: (type: 'info' | 'warn' | 'error', message: string) => void;
}


// Calculate weighted average of top N similarities
const calcWeightedAvg = (items: SimilarNode[], topN: number = 10): number => {
  if (items.length === 0) return 0;
  const sorted = [...items].sort((a, b) => b.similarity - a.similarity);
  const top = sorted.slice(0, topN);
  // Weight by position: first item has weight N, second N-1, etc.
  let weightedSum = 0;
  let totalWeight = 0;
  top.forEach((item, idx) => {
    const weight = topN - idx;
    weightedSum += item.similarity * weight;
    totalWeight += weight;
  });
  return totalWeight > 0 ? weightedSum / totalWeight : 0;
};

export const SimilarNodesPanel = memo(function SimilarNodesPanel({
  similarNodesMap,
  nodes,
  currentParentId,
  stackNodes,
  detailsPanelSize,
  isResizingDetails,
  pinnedIds,
  getNodeEmoji,
  onJumpToNode,
  onFetchDetails: _onFetchDetails,
  onRemoveNode,
  onTogglePin,
  onTogglePrivacy,
  onSplitNode,
  onUnsplitNode,
  onClearAll,
  onToggleStack,
  onStartResize,
  devLog,
}: SimilarNodesPanelProps) {
  // Local state for expand/collapse - tracks expanded group paths
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());
  const [collapsedSimilar, setCollapsedSimilar] = useState<Set<string>>(() =>
    new Set(Array.from(similarNodesMap.keys())) // Collapsed by default
  );
  const [collapsedHierarchy, setCollapsedHierarchy] = useState<Set<string>>(new Set());
  const [expandedHierarchyNodes, setExpandedHierarchyNodes] = useState<Set<string>>(new Set());

  // Toggle a group's expanded state
  const toggleGroup = useCallback((groupPath: string) => {
    setExpandedGroups(prev => {
      const next = new Set(prev);
      if (next.has(groupPath)) {
        next.delete(groupPath);
      } else {
        next.add(groupPath);
      }
      return next;
    });
  }, []);

  // Helper to get all descendant items from a group (recursive)
  const getAllDescendantItems = useCallback((group: SimilarGroup): SimilarNode[] => {
    const items = [...group.items];
    group.children.forEach(child => {
      items.push(...getAllDescendantItems(child));
    });
    return items;
  }, []);

  // Helper to count all descendants of a node in the graph (recursive)
  const countNodeDescendants = useCallback((nodeId: string): number => {
    let count = 0;
    // Find all direct children
    nodes.forEach((node) => {
      if (node.parentId === nodeId) {
        count += 1 + countNodeDescendants(node.id);
      }
    });
    return count;
  }, [nodes]);

  // Get all descendant items of a node (recursive)
  const getDescendantItems = useCallback((nodeId: string): Node[] => {
    const items: Node[] = [];
    nodes.forEach((node) => {
      if (node.parentId === nodeId) {
        if (node.childCount === 0) {
          items.push(node);
        } else {
          items.push(...getDescendantItems(node.id));
        }
      }
    });
    return items;
  }, [nodes]);

  // Get direct children of a node
  const getDirectChildren = useCallback((nodeId: string): Node[] => {
    const children: Node[] = [];
    nodes.forEach((node) => {
      if (node.parentId === nodeId) {
        children.push(node);
      }
    });
    // Sort by latestChildDate/createdAt descending (newest first)
    return children.sort((a, b) => {
      const aDate = a.childCount > 0 && a.latestChildDate ? a.latestChildDate : a.createdAt;
      const bDate = b.childCount > 0 && b.latestChildDate ? b.latestChildDate : b.createdAt;
      return bDate - aDate;
    });
  }, [nodes]);

  // Date range from current VIEW (siblings with same parent)
  // Uses displayed "Latest" date from each node
  const { viewMinDate, viewMaxDate } = useMemo(() => {
    const toMs = (ts: number) => ts > 9999999999 ? ts : ts * 1000;
    // Get siblings (nodes with same parent as current view)
    const viewNodes = currentParentId
      ? Array.from(nodes.values()).filter(n => n.parentId === currentParentId)
      : Array.from(nodes.values());
    const viewDates = viewNodes.map(n => {
      const ts = n.childCount > 0 && n.latestChildDate ? n.latestChildDate : n.createdAt;
      return toMs(ts);
    });
    return {
      viewMinDate: viewDates.length > 0 ? Math.min(...viewDates) : Date.now(),
      viewMaxDate: viewDates.length > 0 ? Math.max(...viewDates) : Date.now(),
    };
  }, [nodes, currentParentId]);

  // Date to color: red (oldest in view) → yellow → blue → cyan (newest in view)
  const getDateColor = useCallback((dateValue: number): string => {
    const toMs = (ts: number) => ts > 9999999999 ? ts : ts * 1000;
    const timestamp = toMs(dateValue);
    const dateRange = viewMaxDate - viewMinDate || 1;
    const t = Math.max(0, Math.min(1, (timestamp - viewMinDate) / dateRange));

    let hue: number;
    if (t < 0.5) {
      hue = t * 2 * 60; // red → yellow
    } else {
      hue = 210 - (t - 0.5) * 2 * 30; // blue → cyan
    }
    return `hsl(${hue}, 75%, 65%)`;
  }, [viewMinDate, viewMaxDate]);

  // Helper to count all descendants
  const countAllDescendants = useCallback((group: SimilarGroup): number => {
    let count = group.items.length;
    group.children.forEach(child => {
      count += countAllDescendants(child);
    });
    return count;
  }, []);

  // Helper to check if a node is a descendant of an ancestor
  const isDescendant = useCallback((nodeId: string, ancestorId: string): boolean => {
    let current = nodes.get(nodeId);
    while (current?.parentId) {
      if (current.parentId === ancestorId) return true;
      current = nodes.get(current.parentId);
    }
    return false;
  }, [nodes]);

  // Build hierarchical groups from flat similar nodes
  const buildHierarchy = useCallback((similarNodes: SimilarNode[]): SimilarGroup[] => {
    // Minimum depth to create groups (skip Universe/near-root)
    const MIN_GROUP_DEPTH = 2;

    // Group items by their parent (immediate container)
    const groupMap = new Map<string, SimilarNode[]>();

    similarNodes.forEach(item => {
      const itemNode = nodes.get(item.id);
      const parentId = itemNode?.parentId || 'orphan';
      if (!groupMap.has(parentId)) {
        groupMap.set(parentId, []);
      }
      groupMap.get(parentId)!.push(item);
    });

    // Build groups with metadata
    const groups: SimilarGroup[] = [];
    groupMap.forEach((items, parentId) => {
      const parentNode = nodes.get(parentId);
      groups.push({
        id: parentId,
        name: parentNode?.aiTitle || parentNode?.title || 'Other',
        emoji: parentNode?.emoji || (parentNode ? getNodeEmoji(parentNode) : '📁'),
        avgSimilarity: calcWeightedAvg(items), // Will be recalculated with all descendants
        items: items.sort((a, b) => b.similarity - a.similarity),
        children: [],
        depth: parentNode?.depth ?? 0,
      });
    });

    // Sort groups by weighted average (descending)
    groups.sort((a, b) => b.avgSimilarity - a.avgSimilarity);

    // Now group the groups by THEIR parents (for nested hierarchy)
    const buildNestedGroups = (currentGroups: SimilarGroup[], iteration: number): SimilarGroup[] => {
      if (currentGroups.length <= 1 || iteration > 10) return currentGroups;

      // Check if we should stop nesting (all groups at or above MIN_GROUP_DEPTH)
      const shouldStopNesting = currentGroups.every(g => {
        const node = nodes.get(g.id);
        return !node || node.depth <= MIN_GROUP_DEPTH || node.isUniverse;
      });
      if (shouldStopNesting) return currentGroups;

      // Group by grandparent
      const grandparentMap = new Map<string, SimilarGroup[]>();
      currentGroups.forEach(group => {
        const parentNode = nodes.get(group.id);
        // Don't group under Universe or very shallow nodes
        if (!parentNode || parentNode.depth < MIN_GROUP_DEPTH || parentNode.isUniverse) {
          // Keep this group as-is at top level
          grandparentMap.set(group.id, [group]);
        } else {
          const grandparentId = parentNode.parentId || 'root';
          const grandparentNode = nodes.get(grandparentId);
          // Skip grouping if grandparent is Universe or too shallow
          if (!grandparentNode || grandparentNode.isUniverse || grandparentNode.depth < MIN_GROUP_DEPTH) {
            grandparentMap.set(group.id, [group]);
          } else {
            if (!grandparentMap.has(grandparentId)) {
              grandparentMap.set(grandparentId, []);
            }
            grandparentMap.get(grandparentId)!.push(group);
          }
        }
      });

      // If no actual grouping happened, return as-is
      if (grandparentMap.size === currentGroups.length) return currentGroups;

      // Build grandparent groups
      const nestedGroups: SimilarGroup[] = [];
      grandparentMap.forEach((childGroups, grandparentId) => {
        // If only one child and it's the same as grandparentId (self-reference), just pass through
        if (childGroups.length === 1 && childGroups[0].id === grandparentId) {
          nestedGroups.push(childGroups[0]);
          return;
        }

        const grandparentNode = nodes.get(grandparentId);

        // Collect ALL descendant items for avg calculation
        const allDescendantItems: SimilarNode[] = [];
        childGroups.forEach(g => {
          allDescendantItems.push(...g.items);
          g.children.forEach(c => allDescendantItems.push(...getAllDescendantItems(c)));
        });

        nestedGroups.push({
          id: grandparentId,
          name: grandparentNode?.aiTitle || grandparentNode?.title || 'Group',
          emoji: grandparentNode?.emoji || (grandparentNode ? getNodeEmoji(grandparentNode) : '📁'),
          avgSimilarity: calcWeightedAvg(allDescendantItems), // Use ALL descendants
          items: [], // No direct items at this level
          children: childGroups.sort((a, b) => b.avgSimilarity - a.avgSimilarity),
          depth: grandparentNode?.depth ?? 0,
        });
      });

      nestedGroups.sort((a, b) => b.avgSimilarity - a.avgSimilarity);

      // Recursively nest
      return buildNestedGroups(nestedGroups, iteration + 1);
    };

    let result = buildNestedGroups(groups, 0);

    // Flatten redundant levels: if a group has only 1 child and 0 direct items, skip it
    const flattenRedundant = (groups: SimilarGroup[]): SimilarGroup[] => {
      return groups.map(group => {
        // Recursively flatten children first
        group.children = flattenRedundant(group.children);

        // If this group has exactly 1 child and no direct items, replace with child
        while (group.children.length === 1 && group.items.length === 0) {
          const child = group.children[0];
          group = {
            ...child,
            children: flattenRedundant(child.children),
          };
        }
        return group;
      });
    };

    result = flattenRedundant(result);

    // Recalculate avgSimilarity for all groups based on ALL descendants
    const recalcAvg = (group: SimilarGroup): SimilarGroup => {
      group.children = group.children.map(recalcAvg);
      const allItems = getAllDescendantItems(group);
      group.avgSimilarity = calcWeightedAvg(allItems);
      return group;
    };
    result = result.map(recalcAvg);

    // Final sort
    result.sort((a, b) => b.avgSimilarity - a.avgSimilarity);

    return result;
  }, [nodes, getNodeEmoji, getAllDescendantItems]);

  // Recursive component to render a group
  const renderGroup = useCallback((group: SimilarGroup, sourceNode: Node | undefined, sourceId: string, path: string, indentLevel: number) => {
    const isExpanded = expandedGroups.has(path);
    const hasChildren = group.children.length > 0;
    const hasItems = group.items.length > 0;
    const totalItems = countAllDescendants(group);
    // Check if this group is a descendant of the selected node
    const groupIsInside = isDescendant(group.id, sourceId);
    const displaySimilarity = groupIsInside ? 1.0 : group.avgSimilarity;

    return (
      <div key={path} style={{ marginLeft: indentLevel * 12 }}>
        {/* Group header */}
        <button
          onClick={() => toggleGroup(path)}
          className="w-full px-3 py-2 text-left hover:bg-gray-700/50 transition-colors flex items-center justify-between gap-2"
        >
          <span className="flex items-center gap-1.5 text-sm">
            <span className="text-gray-500 w-4">{(hasChildren || hasItems) ? (isExpanded ? '▼' : '▶') : '•'}</span>
            {groupIsInside && <span className="text-green-400" title="Inside this node">📍</span>}
            <span>{group.emoji}</span>
            <span className="truncate">{group.name}</span>
            <span className="text-xs text-gray-500">({totalItems})</span>
          </span>
          <span
            className="text-xs shrink-0 font-medium"
            style={{ color: groupIsInside ? '#4ade80' : `hsl(${displaySimilarity * 120}, 70%, 50%)` }}
          >
            {(displaySimilarity * 100).toFixed(0)}%
          </span>
        </button>

        {/* Expanded content */}
        {isExpanded && (
          <div className="border-l border-gray-700/50 ml-4">
            {/* Render child groups first */}
            {group.children.map((child) =>
              renderGroup(child, sourceNode, sourceId, `${path}/${child.id}`, indentLevel + 1)
            )}
            {/* Then render direct items */}
            {group.items.map(item => {
              const itemNode = nodes.get(item.id);
              const isAlreadyShown = similarNodesMap.has(item.id);
              const isInside = isDescendant(item.id, sourceId);
              const displaySimilarity = isInside ? 1.0 : item.similarity;
              return (
                <button
                  key={item.id}
                  onClick={() => {
                    // Navigate to show this node in the graph
                    if (itemNode) {
                      onJumpToNode(itemNode, sourceNode);
                      devLog('info', `Navigating to similar node: ${item.title}`);
                    }
                  }}
                  className={`w-full px-3 py-1.5 text-left hover:bg-gray-700/50 transition-colors flex items-center justify-between gap-2 ${
                    isAlreadyShown ? 'bg-amber-500/10' : ''
                  }`}
                  style={{ marginLeft: (indentLevel + 1) * 12 }}
                  title={isInside ? "Inside this node - click to navigate" : "Click to navigate to this node"}
                >
                  <span className="text-sm truncate flex items-center gap-1.5">
                    <span className={`w-4 ${isAlreadyShown ? 'text-amber-400' : 'text-gray-600'}`}>
                      {isAlreadyShown ? '●' : '•'}
                    </span>
                    {isInside && <span className="text-green-400" title="Inside this node">📍</span>}
                    <span>{item.emoji || '📄'}</span>
                    <span className="truncate">{item.title}</span>
                  </span>
                  <span
                    className="text-xs shrink-0"
                    style={{ color: isInside ? '#4ade80' : `hsl(${displaySimilarity * 120}, 70%, 50%)` }}
                  >
                    {(displaySimilarity * 100).toFixed(0)}%
                  </span>
                </button>
              );
            })}
          </div>
        )}
      </div>
    );
  }, [expandedGroups, toggleGroup, nodes, onJumpToNode, devLog, countAllDescendants, similarNodesMap, isDescendant]);

  // Recursive render for hierarchy inside node - shows clear groupings
  const renderHierarchyNode = useCallback((node: Node, sourceId: string, depth: number) => {
    const children = getDirectChildren(node.id);
    const hasChildren = children.length > 0;
    const path = `hierarchy-${sourceId}-${node.id}`;
    const isExpanded = expandedHierarchyNodes.has(path);
    const displayDate = node.childCount > 0 && node.latestChildDate ? node.latestChildDate : node.createdAt;

    // Count total items in this subtree
    const totalItems = countNodeDescendants(node.id);
    const isGroup = hasChildren && !node.isItem;

    return (
      <div key={node.id} className={depth === 0 && isGroup ? 'mb-1' : ''}>
        <button
          onClick={() => {
            if (hasChildren) {
              setExpandedHierarchyNodes(prev => {
                const next = new Set(prev);
                if (next.has(path)) {
                  next.delete(path);
                } else {
                  next.add(path);
                }
                return next;
              });
            } else {
              // Navigate to item
              onJumpToNode(node, nodes.get(sourceId));
            }
          }}
          className={`w-full px-2 py-1 text-left hover:bg-gray-700/50 transition-colors flex items-center justify-between gap-2 ${
            depth === 0 && isGroup ? 'bg-gray-700/30 rounded' : ''
          }`}
          style={{ marginLeft: depth * 12 }}
          title={node.aiTitle || node.title}
        >
          <span className="flex items-center gap-1.5 text-sm min-w-0">
            <span className="text-gray-500 w-4 shrink-0">
              {hasChildren ? (isExpanded ? '▼' : '▶') : '•'}
            </span>
            <span className="shrink-0">{node.emoji || getNodeEmoji(node)}</span>
            <span className={`truncate max-w-[180px] ${depth === 0 && isGroup ? 'font-medium' : ''}`}>
              {node.aiTitle || node.title}
            </span>
            {isGroup && (
              <span className="text-xs text-gray-500 shrink-0">
                ({totalItems > 0 ? `${totalItems} items` : node.childCount})
              </span>
            )}
          </span>
          <span className="text-xs shrink-0" style={{ color: getDateColor(displayDate) }}>
            {new Date(displayDate).toLocaleDateString()}
          </span>
        </button>
        {isExpanded && hasChildren && (
          <div className="border-l-2 border-gray-600/50 ml-4 pl-1">
            {children.map(child => renderHierarchyNode(child, sourceId, depth + 1))}
          </div>
        )}
      </div>
    );
  }, [getDirectChildren, expandedHierarchyNodes, getNodeEmoji, onJumpToNode, nodes, getDateColor, countNodeDescendants]);

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
          return (
            <div key={sourceId} className="border-b border-gray-700/50 last:border-0">
              {/* Source node details header */}
              <div className="p-4 bg-gray-700/40">
                <div className="flex items-start justify-between gap-2 mb-2">
                  <h3 className="text-sm font-semibold text-white leading-tight">
                    {sourceNode && <span className="mr-1.5">{getNodeEmoji(sourceNode)}</span>}
                    {sourceNode?.aiTitle || sourceNode?.title || sourceId}
                  </h3>
                  <div className="flex items-center gap-1.5">
                    {sourceNode && (
                      <button
                        onClick={() => onJumpToNode(sourceNode, undefined)}
                        className="p-1.5 rounded-md bg-gray-600/50 hover:bg-gray-600 text-gray-300 hover:text-amber-400 transition-colors"
                        title="Go to this node"
                      >
                        <span className="text-sm font-medium">↩</span>
                      </button>
                    )}
                    <button
                      onClick={() => onTogglePin(sourceId, pinnedIds.has(sourceId))}
                      className={`p-1.5 rounded-md transition-colors ${
                        pinnedIds.has(sourceId)
                          ? 'bg-amber-500/30 text-amber-400 hover:bg-amber-500/40 hover:text-amber-300'
                          : 'bg-gray-600/50 hover:bg-gray-600 text-gray-300 hover:text-gray-100'
                      }`}
                      title={pinnedIds.has(sourceId) ? 'Unpin' : 'Pin'}
                    >
                      {pinnedIds.has(sourceId) ? <PinOff className="w-4 h-4" /> : <Pin className="w-4 h-4" />}
                    </button>
                    <button
                      onClick={() => onTogglePrivacy(sourceId, sourceNode?.isPrivate === true)}
                      className={`p-1.5 rounded-md transition-colors ${
                        sourceNode?.isPrivate === true
                          ? 'bg-rose-500/30 text-rose-400 hover:bg-rose-500/40 hover:text-rose-300'
                          : 'bg-gray-600/50 hover:bg-gray-600 text-gray-300 hover:text-gray-100'
                      }`}
                      title={sourceNode?.isPrivate === true ? 'Mark as safe' : 'Mark as private'}
                    >
                      {sourceNode?.isPrivate === true ? <Lock className="w-4 h-4" /> : <LockOpen className="w-4 h-4" />}
                    </button>
                    {sourceNode && sourceNode.childCount > 0 && (
                      <>
                        {onSplitNode && (
                          <button
                            onClick={() => onSplitNode(sourceId)}
                            className={`p-1.5 rounded-md transition-colors ${
                              sourceNode.childCount > 5
                                ? 'bg-blue-500/20 text-blue-400 hover:bg-blue-500/30 hover:text-blue-300'
                                : 'bg-gray-600/50 text-gray-400 hover:bg-gray-600 hover:text-gray-300'
                            }`}
                            title={sourceNode.childCount > 5
                              ? `Split ${sourceNode.childCount} children into sub-categories`
                              : `${sourceNode.childCount} children (split works best with >5)`
                            }
                          >
                            <GitBranch className="w-4 h-4" />
                          </button>
                        )}
                        {onUnsplitNode && (
                          <button
                            onClick={() => onUnsplitNode(sourceId)}
                            className="p-1.5 rounded-md bg-gray-600/50 text-gray-400 hover:bg-orange-500/30 hover:text-orange-400 transition-colors"
                            title="Unsplit - flatten intermediate categories back into this node"
                          >
                            <GitMerge className="w-4 h-4" />
                          </button>
                        )}
                      </>
                    )}
                    {stackNodes && (
                      <button
                        onClick={() => onRemoveNode(sourceId)}
                        className="p-1.5 rounded-md bg-gray-600/50 hover:bg-red-500/30 text-gray-300 hover:text-red-400 transition-colors"
                        title="Remove from stack"
                      >
                        <span className="text-sm">✕</span>
                      </button>
                    )}
                  </div>
                </div>
                {/* Metadata */}
                {sourceNode && (
                  <div className="text-xs text-gray-400 mb-2 flex items-center gap-2 flex-wrap">
                    {(() => {
                      const descendantCount = countNodeDescendants(sourceId);
                      return descendantCount > 0 ? (
                        <span className="px-2 py-0.5 bg-amber-500/20 text-amber-300 rounded-full font-medium">
                          {descendantCount} item{descendantCount !== 1 ? 's' : ''}
                        </span>
                      ) : null;
                    })()}
                    {sourceNode.childCount > 0 && sourceNode.latestChildDate ? (
                      <>
                        <span style={{ color: getDateColor(sourceNode.latestChildDate) }}>
                          Latest: {new Date(sourceNode.latestChildDate).toLocaleDateString()}
                        </span>
                        <span className="text-gray-500">
                          Created: {new Date(sourceNode.createdAt).toLocaleDateString()}
                        </span>
                      </>
                    ) : (
                      <span style={{ color: getDateColor(sourceNode.createdAt) }}>
                        {new Date(sourceNode.createdAt).toLocaleDateString()}
                      </span>
                    )}
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

              {/* Hierarchy section - only for nodes with children */}
              {sourceNode && sourceNode.childCount > 0 && (
                <>
                  <button
                    onClick={() => setCollapsedHierarchy(prev => {
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
                      <span>{collapsedHierarchy.has(sourceId) ? '▶' : '▼'}</span>
                      <span>📂 Hierarchy ({sourceNode.childCount})</span>
                    </span>
                  </button>
                  {!collapsedHierarchy.has(sourceId) && (
                    <div className="py-1 max-h-64 overflow-y-auto">
                      {getDirectChildren(sourceId).map(child =>
                        renderHierarchyNode(child, sourceId, 0)
                      )}
                    </div>
                  )}
                </>
              )}

              {/* Similar nodes header - clickable to collapse (collapsed by default) */}
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

              {/* Hierarchical similar nodes - collapsible */}
              {!collapsedSimilar.has(sourceId) && (
                <div className="py-1">
                  {buildHierarchy(similarNodes)
                    .sort((a, b) => {
                      // Sort descendants (100%) to top, then by avgSimilarity
                      const aIsInside = isDescendant(a.id, sourceId);
                      const bIsInside = isDescendant(b.id, sourceId);
                      if (aIsInside && !bIsInside) return -1;
                      if (!aIsInside && bIsInside) return 1;
                      return b.avgSimilarity - a.avgSimilarity;
                    })
                    .map((group) =>
                      renderGroup(group, sourceNode, sourceId, `${sourceId}/${group.id}`, 0)
                    )}
                </div>
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
