// @ts-nocheck
import { useRef, useEffect, useState, useCallback, useMemo } from 'react';
import * as d3 from 'd3';
import { invoke } from '@tauri-apps/api/core';
import { listen, emit } from '@tauri-apps/api/event';
import { useGraphStore } from '../../stores/graphStore';
import type { Node } from '../../types/graph';
import { getEmojiForNode, initLearnedMappings } from '../../utils/emojiMatcher';
import { ChevronRight, AlertTriangle, X, Lock, LockOpen } from 'lucide-react';
import { SimilarNodesPanel, SimilarNodesLoading } from './SimilarNodesPanel';

// Confirmation dialog types
type ConfirmAction = 'deleteNode' | null;

interface ConfirmConfig {
  title: string;
  message: string;
  confirmText: string;
  variant: 'danger' | 'warning' | 'info';
}

// Dynamic level naming based on distance from items
const getLevelName = (depth: number, maxDepth: number): { name: string; emoji: string } => {
  if (depth === 0) return { name: 'Universe', emoji: '🌌' };

  const distanceFromItems = maxDepth - depth;
  switch (distanceFromItems) {
    case 0: return { name: 'Items', emoji: '🍃' };
    case 1: return { name: 'Topics', emoji: '🗂️' };
    case 2: return { name: 'Domains', emoji: '🌍' };
    case 3: return { name: 'Galaxies', emoji: '🌀' };
    default: return { name: `Level ${depth}`, emoji: '📁' };
  }
};

// Convert number to Unicode superscript
const toSuperscript = (n: number): string => {
  const superscripts = '⁰¹²³⁴⁵⁶⁷⁸⁹';
  return String(n).split('').map(d => superscripts[parseInt(d)]).join('');
};

interface AiProgressEvent {
  current: number;
  total: number;
  nodeTitle: string;
  newTitle: string;
  status: 'processing' | 'success' | 'error' | 'complete';
  errorMessage?: string;
  elapsedSecs?: number;
  estimateSecs?: number;
  remainingSecs?: number;
}

// Format seconds to human readable string
const formatTime = (secs: number): string => {
  if (secs < 60) return `${Math.round(secs)}s`;
  const mins = Math.floor(secs / 60);
  const remainSecs = Math.round(secs % 60);
  if (mins < 60) return `${mins}m ${remainSecs}s`;
  const hours = Math.floor(mins / 60);
  const remainMins = mins % 60;
  return `${hours}h ${remainMins}m`;
};

interface GraphProps {
  width: number;
  height: number;
  onDataChanged?: () => void;
}

interface GraphNode extends Node {
  x: number;
  y: number;
  renderClusterId: number; // The cluster this node belongs to for layout
  displayTitle: string;    // AI title or fallback to raw title
  displayContent: string;  // Summary or fallback to raw content
  displayEmoji: string;    // Topic emoji
}

interface DevConsoleLog {
  time: string;
  type: 'info' | 'warn' | 'error';
  message: string;
}

interface SimilarNode {
  id: string;
  title: string;
  emoji: string | null;
  summary: string | null;
  similarity: number;
}

// Generate colors for clusters dynamically
const generateClusterColor = (clusterId: number): string => {
  const hue = (clusterId * 137.508) % 360; // Golden angle for good color distribution
  return `hsl(${hue}, 55%, 35%)`;
};

// Direct connection color: red→yellow→blue→cyan matching edge colors
// Skips green for colorblind accessibility
const getDirectConnectionColor = (weight: number): string => {
  let hue: number;
  if (weight < 0.5) {
    // First half: red (0°) → yellow (60°)
    hue = weight * 2 * 60;
  } else {
    // Second half: blue (210°) → cyan (180°)
    hue = 210 - (weight - 0.5) * 2 * 30;
  }
  return `hsl(${hue}, 80%, 40%)`; // Match edge saturation (80%), slightly darker
};

// Chain connection color: darker red tint for indirect connections
const getChainConnectionColor = (hopDistance: number): string => {
  // Further = darker/more faded red
  const lightness = Math.max(25, 35 - hopDistance * 3); // 35% → 25% as distance increases
  return `hsl(0, 60%, ${lightness}%)`; // Red hue, moderate saturation
};

// getUnconnectedColor removed - using getMutedClusterColor for unconnected nodes

// Calculate structural depth for shadow stacking
// Items: 0 (no stack, just subtle shadow)
// Topics: 1-4 (violet base + 0-3 cluster shadows)
const getStructuralDepth = (childCount: number, isItem: boolean): number => {
  if (isItem) return 0;
  if (childCount >= 16) return 4;  // violet + 3 cluster
  if (childCount >= 6) return 3;   // violet + 2 cluster
  if (childCount >= 2) return 2;   // violet + 1 cluster
  return 1;  // just violet (all topics)
};

// Simple hash function to generate consistent number from string
const hashString = (str: string): number => {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    const char = str.charCodeAt(i);
    hash = ((hash << 5) - hash) + char;
    hash = hash & hash; // Convert to 32-bit integer
  }
  return Math.abs(hash);
};

export function Graph({ width, height, onDataChanged }: GraphProps) {
  const svgRef = useRef<SVGSVGElement>(null);
  const consoleRef = useRef<HTMLDivElement>(null);
  const stackNodesRef = useRef(false);
  // Refs for selection state - accessible by event handlers
  const activeNodeIdRef = useRef<string | null>(null);
  const connectionMapRef = useRef<Map<string, {weight: number, distance: number}>>(new Map());
  // Refs for batched logging (prevents re-render loops when devLog called in render path)
  const logQueueRef = useRef<Array<{time: string, type: 'info' | 'warn' | 'error', message: string}>>([]);
  const flushScheduledRef = useRef(false);
  // Ref to skip activeNodeId useEffect when click handler already did D3 update
  const clickHandledRef = useRef(false);
  // Ref to track last click time (to avoid deselecting during double-click)
  const lastClickTimeRef = useRef(0);
  // Ref to track pending fetch timeout (to cancel on double-click navigation)
  const pendingFetchRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Ref to track current zoom transform for viewport culling
  const zoomTransformRef = useRef<{ k: number; x: number; y: number }>({ k: 1, x: 0, y: 0 });
  const {
    nodes,
    edges,
    activeNodeId,
    setActiveNode,
    currentDepth,
    maxDepth,
    currentParentId,
    navigateToNode,
    navigateBack,
    navigateToRoot,
    jumpToNode,
    breadcrumbs,
    setCurrentDepth,
    setMaxDepth,
    openLeaf,
    addNode,
    updateNode,
    removeNode,
  } = useGraphStore();
  const [hoveredNode, setHoveredNode] = useState<GraphNode | null>(null);
  const [zoomLevel, setZoomLevel] = useState(1);
  const [devLogs, setDevLogs] = useState<DevConsoleLog[]>([]);
  const [showDevConsole, setShowDevConsole] = useState(false);
  const [showPanels, setShowPanels] = useState(true); // Hamburger menu toggle for all panels
  const [showDetails, setShowDetails] = useState(true); // Toggle for details panel
  const [hidePrivate, setHidePrivate] = useState(false); // Privacy filter toggle
  const [showNoteModal, setShowNoteModal] = useState(false);
  const [noteTitle, setNoteTitle] = useState('');
  const [noteContent, setNoteContent] = useState('');
  const [nodeMenuId, setNodeMenuId] = useState<string | null>(null); // Which node's menu is open
  const [nodeMenuPos, setNodeMenuPos] = useState<{ x: number; y: number } | null>(null);
  const [nodeToDelete, setNodeToDelete] = useState<string | null>(null); // Node pending deletion
  // Workflow states (used for auto-build and internal functions)
  const [isProcessing, setIsProcessing] = useState(false);
  const [isClustering, setIsClustering] = useState(false);
  const [isBuildingHierarchy, setIsBuildingHierarchy] = useState(false);
  const [isUpdatingDates, setIsUpdatingDates] = useState(false);
  const [isFullRebuilding, setIsFullRebuilding] = useState(false);
  const [rebuildQueued, setRebuildQueued] = useState(false);
  const [cancelRequested, setCancelRequested] = useState<'ai' | 'rebuild' | null>(null);
  const rebuildQueuedRef = useRef(false);
  const fullRebuildRef = useRef<(() => Promise<void>) | undefined>(undefined);
  const processingStartTimeRef = useRef<number>(0);
  const rebuildStartTimeRef = useRef<number>(0);
  const [autoScroll, setAutoScroll] = useState(true);
  const [hierarchyBuilt, setHierarchyBuilt] = useState(false);
  const [consoleSize, setConsoleSize] = useState({ width: 450, height: 400 }); // Larger for hierarchy output
  const [isResizing, setIsResizing] = useState(false);

  // Similar nodes state
  const [similarNodesMap, setSimilarNodesMap] = useState<Map<string, SimilarNode[]>>(new Map());
  const [loadingSimilar, setLoadingSimilar] = useState<Set<string>>(new Set());
  const [stackNodes, setStackNodesState] = useState(false); // Toggle to stack multiple node panels
  const [confirmAction, setConfirmAction] = useState<ConfirmAction>(null); // Confirmation dialog state
  const setStackNodes = (value: boolean) => {
    stackNodesRef.current = value;
    setStackNodesState(value);
  };
  const [detailsPanelSize, setDetailsPanelSize] = useState({ width: 400, height: 800 });
  const [isResizingDetails, setIsResizingDetails] = useState(false);
  const [pinnedIds, setPinnedIds] = useState<Set<string>>(new Set());
  // Zen mode removed - hover/selection now handles connection highlighting
  // const [zenModeNodeId, setZenModeNodeId] = useState<string | null>(null);
  // const zenModeNodeIdRef = useRef<string | null>(null);
  // const setZenMode = (nodeId: string | null) => {
  //   zenModeNodeIdRef.current = nodeId;
  //   setZenModeNodeId(nodeId);
  // };

  // AI Progress indicator state
  const [aiProgress, setAiProgress] = useState<{
    current: number;
    total: number;
    remainingSecs?: number;
    status: 'idle' | 'processing' | 'complete';
  }>({ current: 0, total: 0, status: 'idle' });

  // Confirmation dialog configs
  const confirmConfigs: Record<NonNullable<ConfirmAction>, ConfirmConfig> = {
    deleteNode: {
      title: 'Delete Item',
      message: 'Are you sure you want to delete this item? This cannot be undone.',
      confirmText: 'Delete',
      variant: 'danger',
    },
  };

  const handleConfirmAction = useCallback(async () => {
    const action = confirmAction;
    setConfirmAction(null);

    if (action === 'deleteNode' && nodeToDelete) {
      // Get the node to find its parent BEFORE removing
      const nodeData = nodes.get(nodeToDelete);
      const parentId = nodeData?.parentId;

      try {
        // Delete from database FIRST
        await invoke('delete_node', { id: nodeToDelete });

        // Then update local state
        removeNode(nodeToDelete);

        // Update parent's childCount and latestChildDate
        if (parentId) {
          const parent = nodes.get(parentId);
          if (parent) {
            // Find remaining children to recalculate latest date
            const remainingChildren = Array.from(nodes.values())
              .filter(n => n.parentId === parentId && n.id !== nodeToDelete);
            const latestChildDate = remainingChildren.length > 0
              ? Math.max(...remainingChildren.map(n => n.createdAt))
              : undefined;

            updateNode(parentId, {
              childCount: Math.max(0, parent.childCount - 1),
              latestChildDate,
            });
          }
        }
      } catch (err) {
        console.error('Failed to delete node:', err);
      }

      setNodeToDelete(null);
    }
  }, [confirmAction, nodeToDelete, nodes, removeNode, updateNode]);

  // ==========================================================================
  // MEMOIZED CONNECTION MAP - Computed once per activeNodeId/edges change
  // Replaces 4 redundant BFS computations throughout the component
  // ==========================================================================
  const memoizedConnectionMap = useMemo(() => {
    const connectionMap = new Map<string, { weight: number; distance: number }>();

    if (!activeNodeId || edges.size === 0) {
      return connectionMap;
    }

    // BFS from activeNodeId to find all connected nodes with distances
    connectionMap.set(activeNodeId, { weight: 1.0, distance: 0 });
    const edgeArr = Array.from(edges.values());
    const queue: { nodeId: string; dist: number }[] = [{ nodeId: activeNodeId, dist: 0 }];
    const maxDist = 3;

    while (queue.length > 0) {
      const { nodeId, dist } = queue.shift()!;
      if (dist >= maxDist) continue;

      for (const edge of edgeArr) {
        let neighborId: string | null = null;
        let weight = edge.weight || 0.5;

        if (edge.sourceId === nodeId && !connectionMap.has(edge.targetId)) {
          neighborId = edge.targetId;
        } else if (edge.targetId === nodeId && !connectionMap.has(edge.sourceId)) {
          neighborId = edge.sourceId;
        }

        if (neighborId) {
          connectionMap.set(neighborId, { weight, distance: dist + 1 });
          queue.push({ nodeId: neighborId, dist: dist + 1 });
        }
      }
    }

    // Update the ref for event handlers that can't use the memoized value
    connectionMapRef.current = connectionMap;

    return connectionMap;
  }, [activeNodeId, edges]);

  // ==========================================================================
  // VIEWPORT CULLING HELPER - Check if a node is visible in current viewport
  // ==========================================================================
  const isNodeVisible = useCallback((nodeX: number, nodeY: number, nodeSize: number = 320): boolean => {
    const { k, x, y } = zoomTransformRef.current;
    const buffer = nodeSize; // Buffer around viewport

    // Transform node position to screen coordinates
    const screenX = nodeX * k + x;
    const screenY = nodeY * k + y;
    const scaledSize = nodeSize * k;

    // Check if node (with buffer) intersects viewport
    return (
      screenX + scaledSize + buffer > 0 &&
      screenX - buffer < width &&
      screenY + scaledSize + buffer > 0 &&
      screenY - buffer < height
    );
  }, [width, height]);

  // Load learned emoji mappings on mount
  useEffect(() => {
    const loadLearnedEmojis = async () => {
      try {
        const mappings = await invoke<Record<string, string>>('get_learned_emojis');
        initLearnedMappings(mappings);
      } catch (err) {
        console.error('Failed to load learned emojis:', err);
      }
    };
    loadLearnedEmojis();
  }, []);

  // Load max depth on mount
  useEffect(() => {
    const loadMaxDepth = async () => {
      try {
        const depth = await invoke<number>('get_max_depth');
        setMaxDepth(depth);
        devLog('info', `Hierarchy max depth: ${depth}`);
      } catch (err) {
        console.error('Failed to load max depth:', err);
      }
    };
    loadMaxDepth();
  }, [setMaxDepth]);

  // Load pinned node IDs on mount
  useEffect(() => {
    const loadPinnedIds = async () => {
      try {
        const pinned = await invoke<{ id: string }[]>('get_pinned_nodes');
        setPinnedIds(new Set(pinned.map(n => n.id)));
      } catch (err) {
        console.error('Failed to load pinned nodes:', err);
      }
    };
    loadPinnedIds();
  }, []);

  // Handle toggling pin status
  const handleTogglePin = useCallback(async (nodeId: string, currentlyPinned: boolean) => {
    try {
      await invoke('set_node_pinned', { nodeId, pinned: !currentlyPinned });
      setPinnedIds(prev => {
        const next = new Set(prev);
        if (currentlyPinned) {
          next.delete(nodeId);
        } else {
          next.add(nodeId);
        }
        return next;
      });
      // Notify sidebar to refresh pinned list
      emit('pins-changed');
    } catch (err) {
      console.error('Failed to toggle pin:', err);
    }
  }, []);

  // Handle toggling privacy status
  const handleTogglePrivacy = useCallback(async (nodeId: string, currentlyPrivate: boolean) => {
    try {
      const result = await invoke<{ affectedIds: string[] }>('set_node_privacy', { nodeId, isPrivate: !currentlyPrivate });
      // Update all affected nodes in-place (no reload to preserve viewport)
      const newPrivacy = !currentlyPrivate;
      for (const id of result.affectedIds) {
        updateNode(id, { isPrivate: newPrivacy });
      }
      console.log(`Privacy updated for ${result.affectedIds.length} nodes`);
    } catch (err) {
      console.error('Failed to toggle privacy:', err);
    }
  }, [updateNode]);

  // Listen for hierarchy log events from Rust backend
  useEffect(() => {
    const unlisten = listen<{ message: string; level: string }>('hierarchy-log', (event) => {
      const { message, level } = event.payload;
      const type = level === 'error' ? 'error' : level === 'warn' ? 'warn' : 'info';
      const time = new Date().toLocaleTimeString();
      setDevLogs(prev => [...prev.slice(-2000), { time, type, message: `[Hierarchy] ${message}` }]);
    });
    return () => { unlisten.then(f => f()); };
  }, []);

  const devLog = useCallback((type: 'info' | 'warn' | 'error', message: string) => {
    const time = new Date().toLocaleTimeString();

    // Add to queue (no state update = no re-render)
    logQueueRef.current.push({ time, type, message });

    // Schedule a flush if not already scheduled
    if (!flushScheduledRef.current) {
      flushScheduledRef.current = true;
      // Flush after current execution stack clears
      setTimeout(() => {
        flushScheduledRef.current = false;
        const logs = logQueueRef.current;
        logQueueRef.current = [];
        if (logs.length > 0) {
          setDevLogs(prev => [...prev.slice(-2000), ...logs]); // Single batch update
        }
      }, 0);
    }

    // Still log to browser console immediately for debugging
    if (type === 'error') console.error(`[Graph] ${message}`);
    else if (type === 'warn') console.warn(`[Graph] ${message}`);
    else console.log(`[Graph] ${message}`);
  }, []);

  // Handle splitting a node's children into sub-categories (max 5 groups)
  const handleSplitNode = useCallback(async (nodeId: string) => {
    const node = nodes.get(nodeId);
    const nodeName = node?.aiTitle || node?.title || nodeId.slice(0, 8);
    try {
      devLog('info', `Splitting children of "${nodeName}" into max 5 sub-categories...`);
      await invoke('cluster_hierarchy_level', { parentId: nodeId, maxGroups: 5 });
      devLog('info', `Split complete! Refreshing graph...`);
      // Clear the panel and refresh the graph data
      setSimilarNodesMap(new Map());
      if (onDataChanged) {
        await onDataChanged();
        devLog('info', `Graph refreshed. Navigate into "${nodeName}" to see new structure.`);
      }
    } catch (err) {
      console.error('Failed to split node:', err);
      devLog('error', `Failed to split: ${err}`);
    }
  }, [nodes, devLog, onDataChanged]);

  // Handle unsplitting a node - flatten intermediate categories back into parent
  const handleUnsplitNode = useCallback(async (nodeId: string) => {
    const node = nodes.get(nodeId);
    const nodeName = node?.aiTitle || node?.title || nodeId.slice(0, 8);
    try {
      devLog('info', `Unsplitting "${nodeName}" - flattening intermediate categories...`);
      const flattened = await invoke<number>('unsplit_node', { parentId: nodeId });
      devLog('info', `Unsplit complete! Moved ${flattened} nodes up. Refreshing...`);
      // Clear the panel and refresh the graph data
      setSimilarNodesMap(new Map());
      if (onDataChanged) {
        await onDataChanged();
        devLog('info', `Graph refreshed.`);
      }
    } catch (err) {
      console.error('Failed to unsplit node:', err);
      devLog('error', `Failed to unsplit: ${err}`);
    }
  }, [nodes, devLog, onDataChanged]);

  // Track the latest requested node to prevent race conditions
  const latestFetchRef = useRef<string | null>(null);

  // Fetch similar nodes for a node - prioritize nodes in current view
  const fetchSimilarNodes = useCallback(async (nodeId: string) => {
    const isStacking = stackNodesRef.current;

    // In stacking mode, toggle off if already showing
    if (isStacking) {
      // Use functional update to avoid stale closure
      let alreadyShowing = false;
      setSimilarNodesMap(prev => {
        if (prev.has(nodeId)) {
          alreadyShowing = true;
          const next = new Map(prev);
          next.delete(nodeId);
          return next;
        }
        return prev;
      });
      if (alreadyShowing) return;
    } else {
      // In single mode, always show the clicked node (no toggle)
      latestFetchRef.current = nodeId;
    }

    setLoadingSimilar(prev => new Set(prev).add(nodeId));
    try {
      // Fetch top 200 similar nodes (enough for practical use)
      const similar = await invoke<SimilarNode[]>('get_similar_nodes', { nodeId, topN: 200, minSimilarity: 0.25 });

      // Race condition guard: only update if this is still the latest request (when not stacking)
      if (!isStacking && latestFetchRef.current !== nodeId) {
        devLog('info', `Ignoring stale fetch for ${nodeId}, latest is ${latestFetchRef.current}`);
        return;
      }

      // Prioritize nodes in current view (ones you can see and click)
      const currentViewNodeIds = new Set(nodes.keys());
      const inView = similar.filter(s => currentViewNodeIds.has(s.id));
      const elsewhere = similar.filter(s => !currentViewNodeIds.has(s.id));
      const prioritized = [...inView, ...elsewhere];

      if (isStacking) {
        setSimilarNodesMap(prev => new Map(prev).set(nodeId, prioritized));
      } else {
        setSimilarNodesMap(new Map([[nodeId, prioritized]]));
      }
      devLog('info', `Found ${similar.length} similar nodes (${inView.length} in view)`);
    } catch (err) {
      devLog('warn', `No similar nodes: ${err}`);
      // Still show source node details even if no similar nodes found
      if (isStacking) {
        setSimilarNodesMap(prev => new Map(prev).set(nodeId, []));
      } else {
        setSimilarNodesMap(new Map([[nodeId, []]]));
      }
    } finally {
      setLoadingSimilar(prev => {
        const next = new Set(prev);
        next.delete(nodeId);
        return next;
      });
    }
  }, [devLog, nodes]); // Removed similarNodesMap - using functional updates to avoid stale closure

  // Auto-build hierarchy if we have items but no Universe
  useEffect(() => {
    const autoBuild = async () => {
      if (hierarchyBuilt) return;

      const allNodes = Array.from(nodes.values());
      const items = allNodes.filter(n => n.isItem);
      const itemsWithClusters = items.filter(n => n.clusterId !== undefined && n.clusterId !== null);
      const universe = allNodes.find(n => n.isUniverse);

      // Log what we have
      const depthCounts: Record<number, number> = {};
      allNodes.forEach(n => {
        depthCounts[n.depth] = (depthCounts[n.depth] || 0) + 1;
      });
      const depthSummary = Object.entries(depthCounts)
        .map(([d, c]) => `D${d}:${c}`)
        .join(', ');
      devLog('info', `Database: ${allNodes.length} total (${depthSummary}), ${items.length} items, ${itemsWithClusters.length} clustered`);

      // Auto-build hierarchy if we have clustered items but no Universe
      if (itemsWithClusters.length > 0 && !universe) {
        devLog('info', `Auto-build: Found ${itemsWithClusters.length} clustered items, no Universe - building hierarchy...`);
        setIsBuildingHierarchy(true);
        try {
          // Use build_full_hierarchy with runClustering=false (already clustered)
          const result = await invoke<{
            clusteringResult: { itemsProcessed: number; clustersCreated: number; itemsAssigned: number } | null;
            hierarchyResult: { levelsCreated: number; intermediateNodesCreated: number; itemsOrganized: number; maxDepth: number };
            levelsCreated: number;
            groupingIterations: number;
          }>('build_full_hierarchy', { runClustering: false });
          devLog('info', `Auto-build SUCCESS: ${result.levelsCreated} levels, ${result.groupingIterations} AI grouping iterations`);
          setHierarchyBuilt(true);
          setMaxDepth(result.hierarchyResult.maxDepth);

          // Wait 3 seconds so user can see the log, then reload
          devLog('info', 'Reloading in 3 seconds...');
          await new Promise(resolve => setTimeout(resolve, 3000));
          window.location.reload();
        } catch (error) {
          devLog('error', `Auto-build FAILED: ${JSON.stringify(error)}`);
          setIsBuildingHierarchy(false);
        }
      }
    };

    autoBuild();
  }, [nodes, hierarchyBuilt, devLog, setMaxDepth]);

  // Helper to get emoji for a node - uses stored emoji or falls back to matcher
  const getNodeEmoji = useCallback((node: { emoji?: string; title?: string; aiTitle?: string; tags?: string[]; content?: string }) => {
    if (node.emoji) return node.emoji;
    return getEmojiForNode({
      title: node.aiTitle || node.title,
      tags: node.tags,
      content: node.content
    });
  }, []);

  // Auto-scroll console to bottom when new logs appear
  useEffect(() => {
    if (consoleRef.current && autoScroll) {
      consoleRef.current.scrollTop = consoleRef.current.scrollHeight;
    }
  }, [devLogs, autoScroll]);

  // Console resize handling
  const handleResizeStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setIsResizing(true);
  }, []);

  // Track if console is at bottom (when details panel is active)
  const consoleAtBottom = showDetails && similarNodesMap.size > 0;

  useEffect(() => {
    if (!isResizing) return;

    const handleMouseMove = (e: MouseEvent) => {
      // Console is positioned from right edge, so width increases as mouse moves left
      const newWidth = Math.max(280, Math.min(800, window.innerWidth - e.clientX - 16));

      if (consoleAtBottom) {
        // At bottom: resize from top-left, height increases as mouse moves up
        const newHeight = Math.max(200, Math.min(600, window.innerHeight - e.clientY - 16));
        setConsoleSize({ width: newWidth, height: newHeight });
      } else {
        // At top: resize from bottom-left, height increases as mouse moves down
        const newHeight = Math.max(200, Math.min(600, e.clientY - 56)); // 56 = top offset
        setConsoleSize({ width: newWidth, height: newHeight });
      }
    };

    const handleMouseUp = () => {
      setIsResizing(false);
    };

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isResizing, consoleAtBottom]);

  // Details panel resize handling
  useEffect(() => {
    if (!isResizingDetails) return;

    const handleMouseMove = (e: MouseEvent) => {
      // Panel is positioned from right edge, so width increases as mouse moves left
      const newWidth = Math.max(300, Math.min(800, window.innerWidth - e.clientX - 16));
      // Height increases as mouse moves down
      const newHeight = Math.max(300, Math.min(window.innerHeight - 100, e.clientY - 64)); // 64 = top offset
      setDetailsPanelSize({ width: newWidth, height: newHeight });
    };

    const handleMouseUp = () => {
      setIsResizingDetails(false);
    };

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isResizingDetails]);

  // Listen for AI progress events
  useEffect(() => {
    const unlisten = listen<AiProgressEvent>('ai-progress', (event) => {
      const { current, total, nodeTitle, newTitle, status, errorMessage, elapsedSecs, remainingSecs } = event.payload;

      // Update floating progress indicator
      if (status === 'processing' || status === 'success') {
        setAiProgress({ current, total, remainingSecs, status: 'processing' });
      } else if (status === 'complete') {
        setAiProgress({ current: total, total, status: 'complete' });
        // Hide after 3 seconds
        setTimeout(() => setAiProgress(prev => prev.status === 'complete' ? { ...prev, status: 'idle' } : prev), 3000);
      }

      // Log to console (simplified)
      if (status === 'processing') {
        devLog('info', `[${current}/${total}] Processing: ${nodeTitle.substring(0, 50)}...`);
      } else if (status === 'success') {
        devLog('info', `[${current}/${total}] ✓ "${newTitle.substring(0, 50)}..."`);
      } else if (status === 'error') {
        devLog('error', `[${current}/${total}] ✗ ${nodeTitle}${errorMessage ? ` - ${errorMessage}` : ''}`);
      } else if (status === 'complete') {
        const totalTimeStr = elapsedSecs !== undefined ? formatTime(elapsedSecs) : '';
        devLog('info', `✓ AI complete: ${total} nodes${totalTimeStr ? ` in ${totalTimeStr}` : ''}`);
      }
    });

    return () => {
      unlisten.then(fn => fn());
    };
  }, [devLog]);

  // Run clustering
  const runClustering = useCallback(async () => {
    setIsClustering(true);
    devLog('info', 'Starting clustering...');
    try {
      const result = await invoke<{ clusters_created: number; nodes_clustered: number }>('run_clustering');
      devLog('info', `Clustering complete: ${result.clusters_created} clusters, ${result.nodes_clustered} nodes`);
      // Reload the page to get updated cluster assignments
      window.location.reload();
    } catch (error) {
      devLog('error', `Clustering failed: ${error}`);
    } finally {
      setIsClustering(false);
    }
  }, [devLog]);

  // Run AI processing on nodes
  const runProcessing = useCallback(async () => {
    setIsProcessing(true);
    setCancelRequested(null);
    processingStartTimeRef.current = Date.now();
    devLog('info', 'Starting AI processing...');
    try {
      const result = await invoke<{ processed: number; failed: number; errors: string[]; cancelled: boolean }>('process_nodes');
      const elapsedSecs = (Date.now() - processingStartTimeRef.current) / 1000;

      if (result.cancelled) {
        devLog('warn', `Processing cancelled after ${result.processed} nodes (took ${formatTime(elapsedSecs)})`);
      } else {
        devLog('info', `Processing complete: ${result.processed} processed, ${result.failed} failed (took ${formatTime(elapsedSecs)})`);
      }

      // Always save processing time - even cancelled runs save work to DB
      if (result.processed > 0) {
        try {
          await invoke('add_ai_processing_time', { elapsedSecs });
        } catch (e) {
          devLog('warn', `Failed to save processing time: ${e}`);
        }
      }

      if (result.errors.length > 0) {
        result.errors.forEach(e => devLog('error', e));
      }
      setIsProcessing(false);

      // Check if rebuild was queued (use ref to get current value)
      if (rebuildQueuedRef.current) {
        console.log('[QUEUE] AI processing finished - starting queued Full Rebuild');
        devLog('info', 'Queued Full Rebuild starting...');
        setRebuildQueued(false);
        rebuildQueuedRef.current = false;
        // Don't reload yet - let fullRebuild handle it
        setTimeout(() => fullRebuildRef.current?.(), 500);
      } else {
        // Reload to get updated AI fields
        window.location.reload();
      }
    } catch (error) {
      devLog('error', `Processing failed: ${error}`);
      setIsProcessing(false);
      setRebuildQueued(false);
      rebuildQueuedRef.current = false;
    }
  }, [devLog]);

  // Build hierarchy: create dynamic levels from clustered items with recursive AI grouping
  const buildHierarchy = useCallback(async () => {
    setIsBuildingHierarchy(true);

    // First show what we have
    const allNodes = Array.from(nodes.values());
    const depthCounts: Record<number, number> = {};
    const typeCounts: Record<string, number> = {};
    let clusteredCount = 0;
    let itemCount = 0;

    allNodes.forEach(n => {
      depthCounts[n.depth] = (depthCounts[n.depth] || 0) + 1;
      typeCounts[n.type] = (typeCounts[n.type] || 0) + 1;
      if (n.clusterId !== undefined && n.clusterId !== null) {
        clusteredCount++;
      }
      if (n.isItem) {
        itemCount++;
      }
    });

    devLog('info', `=== BEFORE BUILD ===`);
    devLog('info', `Total: ${allNodes.length} nodes, ${itemCount} items`);
    devLog('info', `Depths: ${Object.entries(depthCounts).map(([d, c]) => `D${d}:${c}`).join(', ')}`);
    devLog('info', `Types: ${Object.entries(typeCounts).map(([t, c]) => `${t}:${c}`).join(', ')}`);
    devLog('info', `With cluster_id: ${clusteredCount}`);

    devLog('info', 'Building full hierarchy with recursive AI grouping...');
    try {
      // Use new build_full_hierarchy command - does NOT re-run clustering
      const result = await invoke<{
        clusteringResult: { itemsProcessed: number; clustersCreated: number; itemsAssigned: number } | null;
        hierarchyResult: { levelsCreated: number; intermediateNodesCreated: number; itemsOrganized: number; maxDepth: number };
        levelsCreated: number;
        groupingIterations: number;
      }>('build_full_hierarchy', { runClustering: false });

      devLog('info', `Build SUCCESS: ${result.levelsCreated} levels, ${result.groupingIterations} AI grouping iterations`);
      devLog('info', `Intermediate nodes: ${result.hierarchyResult.intermediateNodesCreated}, Items organized: ${result.hierarchyResult.itemsOrganized}`);

      if (result.hierarchyResult.intermediateNodesCreated === 0 && itemCount > 0) {
        devLog('warn', 'No intermediate nodes created. Items may be unclustered.');
      }

      setHierarchyBuilt(true);
      setMaxDepth(result.hierarchyResult.maxDepth);

      // Wait 3 seconds so user can see the log, then reload
      devLog('info', 'Reloading in 3 seconds...');
      await new Promise(resolve => setTimeout(resolve, 3000));
      window.location.reload();
    } catch (error) {
      devLog('error', `Manual build FAILED: ${JSON.stringify(error)}`);
      setIsBuildingHierarchy(false);
    }
  }, [devLog, nodes, setMaxDepth]);

  // Full rebuild: cluster + hierarchy in one shot
  const fullRebuild = useCallback(async () => {
    // If AI processing is running, queue the rebuild instead
    if (isProcessing) {
      setRebuildQueued(true);
      rebuildQueuedRef.current = true;
      console.log('[QUEUE] Full Rebuild queued - waiting for AI processing to complete');
      devLog('info', 'Full Rebuild queued (will run after AI processing completes)');
      return;
    }

    setIsFullRebuilding(true);
    setCancelRequested(null);
    rebuildStartTimeRef.current = Date.now();
    devLog('info', '=== FULL REBUILD: Clustering + Hierarchy + AI Grouping ===');
    try {
      const result = await invoke<{
        clusteringResult: { itemsProcessed: number; clustersCreated: number; itemsAssigned: number } | null;
        hierarchyResult: { levelsCreated: number; intermediateNodesCreated: number; itemsOrganized: number; maxDepth: number };
        levelsCreated: number;
        groupingIterations: number;
      }>('build_full_hierarchy', { runClustering: true });

      const elapsedSecs = (Date.now() - rebuildStartTimeRef.current) / 1000;

      if (result.clusteringResult) {
        devLog('info', `Clustering: ${result.clusteringResult.itemsAssigned} items → ${result.clusteringResult.clustersCreated} clusters`);
      }
      devLog('info', `Hierarchy: ${result.levelsCreated} levels, ${result.groupingIterations} AI grouping iterations (took ${formatTime(elapsedSecs)})`);

      // Save rebuild time to persistent stats
      try {
        await invoke('add_rebuild_time', { elapsedSecs });
      } catch (e) {
        devLog('warn', `Failed to save rebuild time: ${e}`);
      }

      setHierarchyBuilt(true);
      setMaxDepth(result.hierarchyResult.maxDepth);

      devLog('info', 'Reloading in 3 seconds...');
      await new Promise(resolve => setTimeout(resolve, 3000));
      window.location.reload();
    } catch (error) {
      devLog('error', `Full rebuild FAILED: ${JSON.stringify(error)}`);
      setIsFullRebuilding(false);
    }
  }, [devLog, setMaxDepth, isProcessing]);

  // Keep ref updated
  fullRebuildRef.current = fullRebuild;

  // Update latest dates for all groups (fast, no AI)
  const handleUpdateDates = useCallback(async () => {
    setIsUpdatingDates(true);
    devLog('info', 'Propagating latest dates from leaves to groups...');
    try {
      await invoke('propagate_latest_dates');
      devLog('info', '✓ Latest dates propagated to all nodes');
      devLog('info', 'Reloading in 1 second...');
      await new Promise(resolve => setTimeout(resolve, 1000));
      window.location.reload();
    } catch (err) {
      devLog('error', `Failed to update dates: ${err}`);
      setIsUpdatingDates(false);
    }
  }, [devLog]);

  // List all nodes in current view
  const listCurrentNodes = useCallback(() => {
    const allNodes = Array.from(nodes.values());
    const universeNode = allNodes.find(n => n.isUniverse);

    const nodeArray = allNodes.filter(node => {
      if (currentParentId) {
        return node.parentId === currentParentId;
      }
      if (universeNode) {
        return node.parentId === universeNode.id;
      }
      return node.depth === currentDepth;
    });

    // Sort by childCount descending (same as layout)
    const sortedNodes = [...nodeArray].sort((a, b) => (b.childCount || 0) - (a.childCount || 0));

    devLog('info', `=== CURRENT VIEW: ${sortedNodes.length} nodes (depth=${currentDepth}, parent=${currentParentId?.slice(0, 8) || 'universe'}) ===`);
    sortedNodes.forEach((node, i) => {
      const emoji = node.emoji || '';
      const title = node.aiTitle || node.title || 'Untitled';
      const synopsis = node.summary || node.content || '';
      const children = node.childCount || 0;
      const isItem = node.isItem ? ' [ITEM]' : '';
      const clusterId = node.clusterId !== undefined ? ` cluster=${node.clusterId}` : '';
      devLog('info', `  ${i + 1}. ${emoji} "${title}" (children=${children}${clusterId}${isItem})`);
      if (synopsis) {
        const truncated = synopsis.length > 80 ? synopsis.slice(0, 77) + '...' : synopsis;
        devLog('info', `      → ${truncated}`);
      }
    });
    devLog('info', `=== END NODE LIST ===`);
  }, [nodes, currentParentId, currentDepth, devLog]);

  // List full hierarchy tree (up to maxDisplayDepth levels)
  const listHierarchy = useCallback(async () => {
    const maxDisplayDepth = 2; // Cut off after this many levels from universe
    devLog('info', `=== HIERARCHY TREE (max ${maxDisplayDepth} levels) ===`);

    try {
      // Get universe node
      const universe = await invoke<Node | null>('get_universe');
      if (!universe) {
        devLog('warn', 'No universe node found');
        return;
      }

      // Recursive function to print tree
      const printNode = async (nodeId: string, depth: number, prefix: string, _isLast: boolean) => {
        if (depth > maxDisplayDepth) return;

        const children = await invoke<Node[]>('get_children', { parentId: nodeId });
        const sortedChildren = [...children].sort((a, b) => (b.childCount || 0) - (a.childCount || 0));

        for (let i = 0; i < sortedChildren.length; i++) {
          const child = sortedChildren[i];
          const isLastChild = i === sortedChildren.length - 1;
          const connector = isLastChild ? '└── ' : '├── ';
          const emoji = child.emoji || (child.isItem ? '📄' : '📁');
          const title = child.clusterLabel || child.aiTitle || child.title || 'Untitled';
          const childCount = child.childCount || 0;
          const itemTag = child.isItem ? ' [ITEM]' : '';
          const countTag = childCount > 0 ? ` (${childCount})` : '';

          // Truncate title if too long
          const maxTitleLen = 40;
          const displayTitle = title.length > maxTitleLen ? title.slice(0, maxTitleLen - 3) + '...' : title;

          devLog('info', `${prefix}${connector}${emoji} ${displayTitle}${countTag}${itemTag}`);

          // Recurse if not at max depth and has children
          if (depth < maxDisplayDepth && childCount > 0 && !child.isItem) {
            const newPrefix = prefix + (isLastChild ? '    ' : '│   ');
            await printNode(child.id, depth + 1, newPrefix, isLastChild);
          } else if (depth === maxDisplayDepth && childCount > 0) {
            // Show truncation indicator
            const truncPrefix = prefix + (isLastChild ? '    ' : '│   ');
            devLog('info', `${truncPrefix}└── ... (${childCount} more)`);
          }
        }
      };

      // Print universe as root
      const universeTitle = universe.clusterLabel || universe.aiTitle || universe.title || 'Universe';
      devLog('info', `🌌 ${universeTitle} (root)`);

      // Print children
      await printNode(universe.id, 1, '', true);

      devLog('info', `=== END HIERARCHY ===`);
    } catch (error) {
      devLog('error', `Failed to list hierarchy: ${error}`);
    }
  }, [devLog]);

  // List current view's hierarchy path with children
  const listCurrentPath = useCallback(async () => {
    devLog('info', `=== CURRENT PATH ===`);

    try {
      // Always start with Universe
      const universe = await invoke<Node | null>('get_universe');
      if (!universe) {
        devLog('warn', 'No universe node found');
        return;
      }

      const universeTitle = universe.clusterLabel || universe.aiTitle || universe.title || 'Universe';
      devLog('info', `🌌 ${universeTitle} (depth 0)`);

      // Determine current parent to show children of
      let currentViewParentId: string | null = null;
      let baseIndent = 1;

      // Show breadcrumb path
      if (breadcrumbs.length === 0) {
        currentViewParentId = universe.id;
      } else {
        for (let i = 0; i < breadcrumbs.length; i++) {
          const crumb = breadcrumbs[i];
          const isLast = i === breadcrumbs.length - 1;
          const indent = '    '.repeat(i + 1);
          const connector = '└── ';
          const arrow = crumb.isJump ? ' ⟿ ' : '';
          const emoji = crumb.emoji || '📁';
          const title = crumb.title || 'Untitled';
          const depthTag = ` (depth ${crumb.depth})`;

          devLog('info', `${indent}${connector}${arrow}${emoji} ${title}${depthTag}`);

          if (isLast) {
            currentViewParentId = crumb.id;
            baseIndent = i + 2;
          }
        }
      }

      // List children of current view
      if (currentViewParentId) {
        const children = await invoke<Node[]>('get_children', { parentId: currentViewParentId });
        const sortedChildren = [...children].sort((a, b) => (b.childCount || 0) - (a.childCount || 0));

        const childIndent = '    '.repeat(baseIndent);
        devLog('info', `${childIndent}┌── Children (${sortedChildren.length}):`);

        for (let i = 0; i < sortedChildren.length; i++) {
          const child = sortedChildren[i];
          const isLastChild = i === sortedChildren.length - 1;
          const connector = isLastChild ? '└── ' : '├── ';
          const emoji = child.emoji || (child.isItem ? '📄' : '📁');
          const title = child.clusterLabel || child.aiTitle || child.title || 'Untitled';
          const childCount = child.childCount || 0;
          const itemTag = child.isItem ? ' [item]' : '';
          const countTag = childCount > 0 ? ` (${childCount})` : '';

          // Truncate title
          const maxLen = 35;
          const displayTitle = title.length > maxLen ? title.slice(0, maxLen - 3) + '...' : title;

          devLog('info', `${childIndent}${connector}${emoji} ${displayTitle}${countTag}${itemTag}`);
        }
      }

      devLog('info', `=== END PATH ===`);
    } catch (error) {
      devLog('error', `Failed to list path: ${error}`);
    }
  }, [devLog, breadcrumbs, nodes]);

  useEffect(() => {
    devLog('info', 'Main render useEffect running');
    if (!svgRef.current || nodes.size === 0) return;

    // Reset click handled flag so activeNodeId useEffect runs after rebuild
    clickHandledRef.current = false;

    const svg = d3.select(svgRef.current);
    svg.selectAll('*').remove();
    svg.attr('width', width).attr('height', height);

    const container = svg.append('g').attr('class', 'graph-container');

    // Filter nodes by current depth and parent
    const allNodes = Array.from(nodes.values());
    const universeNode = allNodes.find(n => n.isUniverse);

    const nodeArray = allNodes.filter(node => {
      // Privacy filter: hide private nodes AND categories with no visible children
      if (hidePrivate) {
        // Hide explicitly private nodes
        if (node.isPrivate === true) return false;

        // For categories (non-items with children), check if they have any non-private children
        // If ALL children are private, hide the category too
        if (!node.isItem && node.childCount > 0) {
          const children = allNodes.filter(n => n.parentId === node.id);
          const hasVisibleChild = children.some(c => c.isPrivate !== true);
          if (!hasVisibleChild && children.length > 0) return false;
        }
      }

      // If we have a specific parent, show its children
      if (currentParentId) {
        return node.parentId === currentParentId;
      }

      // At root (no parent selected), show Universe's direct children (depth 1)
      // This gives immediate access to top-level categories instead of just the Universe node
      if (currentDepth === 0 && universeNode) {
        return node.parentId === universeNode.id;
      }

      // Fallback: show nodes at current depth without parents
      return node.depth === currentDepth && !node.parentId;
    });

    const levelInfo = getLevelName(currentDepth, maxDepth);
    devLog('info', `Depth ${currentDepth} (${levelInfo.name})${currentParentId ? ` parent: ${currentParentId.slice(0,8)}...` : ''}: showing ${nodeArray.length} of ${allNodes.length} nodes`);

    // Card dimensions at 100% zoom (unified for all nodes)
    const noteWidth = 320;
    const noteHeight = 320;

    // Spacing between nodes (in graph units)
    const nodeSpacing = 300;

    // Zoom limits
    const minZoom = 0.02;
    const maxZoom = 2;

    devLog('info', `Rendering ${nodeArray.length} nodes, ${edges.size} edges`);

    // Layout strategy depends on context:
    // - At root level (viewing Universe children): each node may have unique cluster_id, treat all as one group
    // - When viewing children of a specific parent: treat all children as one group (ring layout)
    // - Only group by cluster_id when nodes naturally share the same cluster_id (e.g., items in same topic)

    const clusterMap = new Map<number, Node[]>();
    const unclustered: Node[] = [];

    // Check if we should use single-group layout
    // Use single group when: viewing a parent's children OR when each node has unique cluster_id
    const uniqueClusterIds = new Set(nodeArray.map(n => n.clusterId).filter(id => id !== undefined && id !== null));
    const allUniqueClusterIds = uniqueClusterIds.size === nodeArray.length && nodeArray.length > 1;
    const useSingleGroupLayout = currentParentId !== null || allUniqueClusterIds;

    if (useSingleGroupLayout) {
      // All nodes in one group - ring layout
      devLog('info', `Using single-group layout (parent: ${currentParentId ? 'yes' : 'no'}, unique ids: ${allUniqueClusterIds})`);
      unclustered.push(...nodeArray);
    } else {
      // Group by cluster_id
      nodeArray.forEach(node => {
        if (node.clusterId !== undefined && node.clusterId !== null) {
          if (!clusterMap.has(node.clusterId)) {
            clusterMap.set(node.clusterId, []);
          }
          clusterMap.get(node.clusterId)!.push(node);
        } else {
          unclustered.push(node);
        }
      });
    }

    // Sort clusters by size (largest first)
    const sortedClusters = Array.from(clusterMap.entries())
      .sort((a, b) => b[1].length - a[1].length);

    devLog('info', `Clusters: ${sortedClusters.length}, unclustered: ${unclustered.length}`);

    // ==========================================================================
    // IMPORTANCE-BASED TIERED LAYOUT (per pyspiral.md spec)
    // ==========================================================================

    // Ring capacity: Ring 0 = 1 (center), all others = 4 max
    const getNodesPerRing = (ring: number): number => {
      if (ring === 0) return 1; // Center node (Recent Notes or most important)
      return 4; // Max 4 nodes per ring
    };

    // Ring radius: based on noteHeight + spacing
    const getRingRadius = (ring: number): number => {
      if (ring === 0) return 0; // Center node at exact center
      if (ring === 1) return (noteHeight + nodeSpacing * 1.5) * 1.1; // First ring - bigger
      // Outer rings: consistent spacing
      return (noteHeight + nodeSpacing * 1.5) * (1.1 + (ring - 1) * 0.9);
    };

    // Golden angle for ring offset (prevents spoke alignment)
    const goldenAngle = Math.PI * (3 - Math.sqrt(5)); // 137.5°

    // Ellipse aspect ratio for wide monitors
    const ellipseAspect = Math.min((width / height) * 0.9, 2.0);

    // Angle restriction for outer rings - avoid top/bottom, favor left/right
    const restrictionStartRing = 2; // Start restricting angles at this ring
    const horizontalBandHalfAngle = Math.PI / 4.5; // ~40° - nodes placed within ±40° of horizontal (more vertical spread)

    // Center of unified layout
    const centerX = 0;
    const centerY = 0;

    const graphNodes: GraphNode[] = [];

    // Combine all nodes for unified layout
    const allLayoutNodes: Node[] = [
      ...sortedClusters.flatMap(([, nodes]) => nodes),
      ...unclustered,
    ];

    // Sort: Recent Notes first (center), then by childCount descending (importance)
    const sortedByImportance = [...allLayoutNodes].sort((a, b) => {
      // Recent Notes always goes to center
      if (a.id === 'container-recent-notes') return -1;
      if (b.id === 'container-recent-notes') return 1;
      // Then sort by childCount descending
      return (b.childCount || 0) - (a.childCount || 0);
    });

    devLog('info', `Sorted ${sortedByImportance.length} nodes by importance (Recent Notes first, then childCount)`);

    // Helper to count total nodes that fit in N rings
    const totalNodesInRings = (numRings: number): number => {
      let total = 0;
      for (let r = 0; r < numRings; r++) total += getNodesPerRing(r);
      return total;
    };

    // Helper to calculate how many rings a tier needs
    const ringsNeededFor = (nodeCount: number, startRing: number): number => {
      if (nodeCount === 0) return 0;
      let rings = 0;
      let placed = 0;
      let ringIdx = startRing;
      while (placed < nodeCount) {
        placed += getNodesPerRing(ringIdx);
        rings++;
        ringIdx++;
      }
      return rings;
    };

    // Position nodes within a tier, starting from a given ring
    const positionTierNodes = (tierNodes: Node[], tierIndex: number, startRing: number): number => {
      if (tierNodes.length === 0) return startRing;

      let ringIndex = startRing;
      let nodeIndex = 0;
      let nodesPlacedInCurrentRing = 0;

      while (nodeIndex < tierNodes.length) {
        const ringCapacity = getNodesPerRing(ringIndex);
        const nodesInThisRing = Math.min(ringCapacity, tierNodes.length - nodeIndex);
        const ringRadius = getRingRadius(ringIndex);
        const ringStartAngle = ringIndex * goldenAngle; // Golden angle offset per ring

        for (let i = 0; i < nodesInThisRing; i++) {
          const node = tierNodes[nodeIndex];

          let angle: number;
          if (ringIndex < restrictionStartRing) {
            // Inner rings: full 360° distribution
            angle = ringStartAngle + (i / nodesInThisRing) * 2 * Math.PI;
          } else {
            // Outer rings: restrict to horizontal bands (left and right sides)
            // Map nodes to two bands: right side and left side, avoiding top/bottom
            const progress = i / nodesInThisRing; // 0 to 1
            if (progress < 0.5) {
              // Right side: map 0-0.5 to -halfAngle to +halfAngle around 0°
              angle = -horizontalBandHalfAngle + (progress * 2) * (2 * horizontalBandHalfAngle);
            } else {
              // Left side: map 0.5-1 to -halfAngle to +halfAngle around π
              angle = Math.PI - horizontalBandHalfAngle + ((progress - 0.5) * 2) * (2 * horizontalBandHalfAngle);
            }
            // Small offset per ring to prevent perfect alignment (but don't rotate bands)
            angle += (ringIndex % 4) * 0.1;
          }

          // Apply ellipse aspect ratio (ring 1 gets extra vertical stretch)
          const verticalStretch = ringIndex === 1 ? 1.3 : 1.0;
          const x = centerX + ringRadius * ellipseAspect * Math.cos(angle);
          const y = centerY + ringRadius * verticalStretch * Math.sin(angle);

          const colorId = node.clusterId ?? hashString(node.id);
          graphNodes.push({
            ...node,
            x,
            y,
            renderClusterId: colorId,
            displayTitle: node.aiTitle || node.title || 'Untitled',
            displayContent: node.summary || node.content || '',
            displayEmoji: getNodeEmoji(node),
          });

          nodeIndex++;
        }

        ringIndex++;
      }

      devLog('info', `  Tier ${tierIndex}: ${tierNodes.length} nodes in rings ${startRing}-${ringIndex - 1}`);
      return ringIndex; // Return next available ring
    };

    // Position all nodes sequentially (already sorted by importance)
    const finalRing = positionTierNodes(sortedByImportance, 0, 0);

    devLog('info', `Layout complete: ${graphNodes.length} nodes in ${finalRing} rings`);

    // Calculate graph bounds for pan limiting (with generous padding)
    const boundsPadding = noteWidth * 40; // 40x node width padding
    const graphBounds = {
      minX: graphNodes.length > 0 ? Math.min(...graphNodes.map(n => n.x)) - boundsPadding : -1000,
      maxX: graphNodes.length > 0 ? Math.max(...graphNodes.map(n => n.x)) + boundsPadding : 1000,
      minY: graphNodes.length > 0 ? Math.min(...graphNodes.map(n => n.y)) - boundsPadding : -1000,
      maxY: graphNodes.length > 0 ? Math.max(...graphNodes.map(n => n.y)) + boundsPadding : 1000,
    };

    // Create node lookup
    const nodeMap = new Map(graphNodes.map(n => [n.id, n]));

    // Helper to get all 4 side centers of a node
    const getSideCenters = (center: {x: number, y: number}, width: number, height: number) => {
      const halfW = width / 2;
      const halfH = height / 2;
      return [
        { x: center.x + halfW, y: center.y },  // right
        { x: center.x - halfW, y: center.y },  // left
        { x: center.x, y: center.y + halfH },  // bottom
        { x: center.x, y: center.y - halfH },  // top
      ];
    };

    // Find the pair of side centers (one from each node) that are closest to each other
    const getEdgePoints = (centerA: {x: number, y: number}, centerB: {x: number, y: number}, width: number, height: number) => {
      const sidesA = getSideCenters(centerA, width, height);
      const sidesB = getSideCenters(centerB, width, height);

      let bestA = sidesA[0];
      let bestB = sidesB[0];
      let minDist = Infinity;

      for (const sideA of sidesA) {
        for (const sideB of sidesB) {
          const dist = Math.hypot(sideA.x - sideB.x, sideA.y - sideB.y);
          if (dist < minDist) {
            minDist = dist;
            bestA = sideA;
            bestB = sideB;
          }
        }
      }

      return { source: bestA, target: bestB };
    };

    // Draw edges - store source/target data for zoom updates
    const linksGroup = container.append('g').attr('class', 'links');

    // Build edge data with resolved coordinates
    const edgeData: Array<{source: GraphNode, target: GraphNode, type: string, weight: number}> = [];
    let containsCount = 0, relatedCount = 0;
    edges.forEach(edge => {
      const source = nodeMap.get(edge.source);
      const target = nodeMap.get(edge.target);
      if (source && target) {
        edgeData.push({ source, target, type: edge.type, weight: edge.weight ?? 0.5 });
        if (edge.type === 'contains') containsCount++;
        else relatedCount++;
      }
    });

    // Calculate min/max weights for normalization (only for related edges)
    const relatedWeights = edgeData.filter(e => e.type !== 'contains').map(e => e.weight);
    const minWeight = relatedWeights.length > 0 ? Math.min(...relatedWeights) : 0;
    const maxWeight = relatedWeights.length > 0 ? Math.max(...relatedWeights) : 1;
    const weightRange = maxWeight - minWeight || 0.1; // Avoid division by zero
    devLog('info', `Edges in view: ${containsCount} contains, ${relatedCount} related (weights: ${(minWeight*100).toFixed(0)}%-${(maxWeight*100).toFixed(0)}%)`);

    // Use memoized connection map (computed once per activeNodeId/edges change)
    const connectionMap = memoizedConnectionMap;

    // Helper to get muted cluster color (gray with hint of cluster hue)
    const getMutedClusterColor = (d: GraphNode): string => {
      if (d.renderClusterId < 0) return '#374151';
      const hue = (d.renderClusterId * 137.508) % 360;
      return `hsl(${hue}, 12%, 28%)`; // Very low saturation, dark
    };

    // Helper to get node color based on connection distance
    const getNodeColor = (d: GraphNode): string => {
      if (!activeNodeId) return getMutedClusterColor(d); // Muted cluster hint when nothing selected
      if (d.id === activeNodeId) {
        // Selected node shows its cluster color
        return d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#4b5563';
      }
      const conn = connectionMap.get(d.id);
      if (conn) {
        if (conn.distance === 1) {
          // Direct connection: red→green based on weight
          return getDirectConnectionColor(conn.weight);
        } else {
          // Chain connection (2+ hops): darker red
          return getChainConnectionColor(conn.distance);
        }
      }
      return getMutedClusterColor(d); // Unconnected = muted cluster hint
    };

    // Helper to get node opacity based on connection distance
    const getNodeOpacity = (d: GraphNode): number => {
      if (!activeNodeId) return 1; // Full opacity when nothing selected
      if (d.id === activeNodeId) return 1; // Selected = full
      const conn = connectionMap.get(d.id);
      if (conn) {
        if (conn.distance === 1) return 1; // Direct = full
        // Chain: slight fade based on distance (0.85 at 2 hops, 0.70 at 3, etc.)
        return Math.max(0.5, 1 - conn.distance * 0.15);
      }
      return 0.7; // Unconnected = subtle fade
    };

    // Define arrow markers for edge endpoints
    // Edge color: red (low weight) → yellow → blue → cyan (high weight)
    // Skips green for colorblind accessibility
    const getEdgeColor = (normalized: number): string => {
      let hue: number;
      if (normalized < 0.5) {
        // First half: red (0°) → yellow (60°)
        hue = normalized * 2 * 60;
      } else {
        // Second half: blue (210°) → cyan (180°)
        hue = 210 - (normalized - 0.5) * 2 * 30;
      }
      return `hsl(${hue}, 80%, 50%)`;
    };

    const defs = svg.append('defs');
    edgeData.forEach((d, i) => {
      const normalized = (d.weight - minWeight) / weightRange;
      const color = d.type === 'contains' ? '#6b7280' : getEdgeColor(normalized);
      // Scale down arrow size for thicker connections (inverse of weight)
      // Low weight (thin edge) → 5px arrow, High weight (thick edge) → 3px arrow
      const arrowSize = d.type === 'contains' ? 4 : 5 - normalized * 2;

      defs.append('marker')
        .attr('id', `arrow-${i}`)
        .attr('viewBox', '0 -5 10 10')
        .attr('refX', 8)  // Offset back from endpoint to appear just before the dot
        .attr('refY', 0)
        .attr('markerWidth', arrowSize)
        .attr('markerHeight', arrowSize)
        .attr('orient', 'auto')
        .append('path')
        .attr('d', 'M0,-5L10,0L0,5')
        .attr('fill', color);
    });

    const edgePaths = linksGroup.selectAll('path')
      .data(edgeData)
      .join('path')
      .attr('fill', 'none')
      .attr('stroke', d => {
        if (d.type === 'contains') return '#6b7280';
        const normalized = (d.weight - minWeight) / weightRange;
        return getEdgeColor(normalized);
      })
      .attr('stroke-opacity', d => d.type === 'contains' ? 0.5 : 0.7)
      .attr('stroke-width', d => {
        // Base thickness + weight-based scaling
        if (d.type === 'contains') return 6;
        // Normalize weight to thickness
        const normalized = (d.weight - minWeight) / weightRange;
        // 0% (min) → 6px, 100% (max) → 24px
        return 6 + normalized * 18;
      })
      .attr('opacity', (e: {source: GraphNode, target: GraphNode}) => {
        if (!activeNodeId) return 1;
        if (e.source.id === activeNodeId || e.target.id === activeNodeId) return 0.9;
        const srcConn = connectionMap.has(e.source.id);
        const tgtConn = connectionMap.has(e.target.id);
        if (srcConn && tgtConn) return 0.7;
        return 0.15;
      })
      .attr('d', d => {
        // Find closest pair of side centers between the two nodes
        const points = getEdgePoints(d.source, d.target, noteWidth, noteHeight);

        const dx = points.target.x - points.source.x;
        const dy = points.target.y - points.source.y;
        const dr = Math.sqrt(dx * dx + dy * dy) * 1.5; // Larger radius = less curve
        return `M${points.source.x},${points.source.y} A${dr},${dr} 0 0,1 ${points.target.x},${points.target.y}`;
      })
      .each(function(d) {
        // Store endpoint coordinates for dot rendering
        const points = getEdgePoints(d.source, d.target, noteWidth, noteHeight);
        (d as any).sourcePoint = points.source;
        (d as any).targetPoint = points.target;
      });

    // Add connection point dots at both endpoints
    const connectionDots = linksGroup.selectAll('circle.connection-dot')
      .data(edgeData.flatMap(d => {
        const points = getEdgePoints(d.source, d.target, noteWidth, noteHeight);
        const normalized = (d.weight - minWeight) / weightRange;
        const hue = d.type === 'contains' ? 220 : normalized * 120;
        const color = d.type === 'contains' ? '#6b7280' : `hsl(${hue}, 80%, 50%)`;
        return [
          { x: points.source.x, y: points.source.y, color, edge: d, isSource: true },
          { x: points.target.x, y: points.target.y, color, edge: d, isSource: false }
        ];
      }))
      .join('circle')
      .attr('class', 'connection-dot')
      .attr('cx', d => d.x)
      .attr('cy', d => d.y)
      .attr('r', 12)
      .attr('fill', d => d.color);

    // ==================== UNIFIED NODE RENDERING ====================
    // All nodes use the same card style - clean "cute api"

    // Date range from VISIBLE nodes in current view
    const viewDates = graphNodes.map(n => {
      const ts = n.childCount > 0 && n.latestChildDate ? n.latestChildDate : n.createdAt;
      return typeof ts === 'number' ? ts : new Date(ts).getTime();
    });
    const viewMinDate = viewDates.length > 0 ? Math.min(...viewDates) : Date.now();
    const viewMaxDate = viewDates.length > 0 ? Math.max(...viewDates) : Date.now();
    const viewDateRange = viewMaxDate - viewMinDate || 1;

    // Date to color: red (oldest in view) → yellow → blue → cyan (newest in view)
    // Skips green for colorblind accessibility
    const getDateColor = (dateValue: string | number): string => {
      const timestamp = typeof dateValue === 'number' ? dateValue : new Date(dateValue).getTime();
      const t = Math.max(0, Math.min(1, (timestamp - viewMinDate) / viewDateRange));

      let hue: number;
      if (t < 0.5) {
        hue = t * 2 * 60; // red → yellow
      } else {
        hue = 210 - (t - 0.5) * 2 * 30; // blue → cyan
      }
      return `hsl(${hue}, 75%, 65%)`;
    };

    const cardsGroup = container.append('g').attr('class', 'cards');
    const dotsGroup = container.append('g').attr('class', 'dots');
    const dotSize = 24;

    // Unified card rendering for ALL nodes
    const cardGroups = cardsGroup.selectAll('g.node-card')
      .data(graphNodes)
      .join('g')
      .attr('class', 'node-card')
      .attr('cursor', 'pointer')
      .attr('transform', d => `translate(${d.x - noteWidth/2}, ${d.y - noteHeight/2})`);

    // === TOPIC SHADOWS (render order = z-order, first = bottom) ===

    // Violet shadow FIRST (ALL topics) - offset = 7 * depth, peeks out at bottom
    cardGroups.filter(d => getStructuralDepth(d.childCount, d.isItem) >= 1)
      .append('rect')
      .attr('class', 'shadow-violet')
      .attr('x', d => 7 * getStructuralDepth(d.childCount, d.isItem))
      .attr('y', d => 7 * getStructuralDepth(d.childCount, d.isItem))
      .attr('width', noteWidth)
      .attr('height', noteHeight)
      .attr('rx', 6)
      .attr('fill', '#5b21b6')
      .attr('stroke', d => d.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.2)')
      .attr('stroke-width', d => d.id === activeNodeId ? 2 : 1);

    // Cluster shadow 3 - for 16+ children
    cardGroups.filter(d => getStructuralDepth(d.childCount, d.isItem) >= 4)
      .append('rect')
      .attr('class', 'shadow-cluster')
      .attr('x', 21)
      .attr('y', 21)
      .attr('width', noteWidth)
      .attr('height', noteHeight)
      .attr('rx', 6)
      .attr('fill', d => {
        const base = d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#374151';
        return d3.color(base)?.darker(1.8)?.toString() || '#2a2a2a';
      })
      .attr('stroke', d => d.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.15)')
      .attr('stroke-width', d => d.id === activeNodeId ? 2 : 1);

    // Cluster shadow 2 - for 6+ children
    cardGroups.filter(d => getStructuralDepth(d.childCount, d.isItem) >= 3)
      .append('rect')
      .attr('class', 'shadow-cluster')
      .attr('x', 14)
      .attr('y', 14)
      .attr('width', noteWidth)
      .attr('height', noteHeight)
      .attr('rx', 6)
      .attr('fill', d => {
        const base = d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#374151';
        return d3.color(base)?.darker(1.4)?.toString() || '#333333';
      })
      .attr('stroke', d => d.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.15)')
      .attr('stroke-width', d => d.id === activeNodeId ? 2 : 1);

    // Cluster shadow 1 - for 2+ children
    cardGroups.filter(d => getStructuralDepth(d.childCount, d.isItem) >= 2)
      .append('rect')
      .attr('class', 'shadow-cluster')
      .attr('x', 7)
      .attr('y', 7)
      .attr('width', noteWidth)
      .attr('height', noteHeight)
      .attr('rx', 6)
      .attr('fill', d => {
        const base = d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#374151';
        return d3.color(base)?.darker(1.0)?.toString() || '#3d3d3d';
      })
      .attr('stroke', d => d.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.15)')
      .attr('stroke-width', d => d.id === activeNodeId ? 2 : 1);

    // === ITEM SHADOW ===

    // Subtle drop shadow for items only
    cardGroups.filter(d => getStructuralDepth(d.childCount, d.isItem) === 0)
      .append('rect')
      .attr('x', 2)
      .attr('y', 2)
      .attr('width', noteWidth)
      .attr('height', noteHeight)
      .attr('rx', 6)
      .attr('fill', 'rgba(0,0,0,0.3)');

    // Card background - color by distance (if active) or cluster
    cardGroups.append('rect')
      .attr('class', 'card-bg')
      .attr('width', noteWidth)
      .attr('height', noteHeight)
      .attr('rx', 6)
      .attr('fill', d => getNodeColor(d))
      .attr('stroke', d => d.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.15)')
      .attr('stroke-width', d => d.id === activeNodeId ? 2 : 1);

    // Apply opacity to entire card group (so shadows fade with card)
    cardGroups.style('opacity', d => getNodeOpacity(d));

    // Titlebar - darker area at top (80px for 2 lines of text + emoji + padding)
    // First rect: full width, rounded top corners match card
    cardGroups.append('rect')
      .attr('width', noteWidth)
      .attr('height', 80)
      .attr('rx', 6)
      .attr('fill', 'rgba(0,0,0,0.4)');
    // Second rect: fills in bottom corners of titlebar
    cardGroups.append('rect')
      .attr('y', 70)
      .attr('width', noteWidth)
      .attr('height', 10)
      .attr('fill', 'rgba(0,0,0,0.4)');

    // Large emoji spanning titlebar height
    cardGroups.append('text')
      .attr('x', 14)
      .attr('y', 52)
      .attr('font-size', '42px')
      .text(d => d.displayEmoji);

    // Nice font for all card text
    const cardFont = "'Inter', 'SF Pro Display', -apple-system, BlinkMacSystemFont, sans-serif";

    // Title - foreignObject for natural wrapping (2 lines), centered horizontally and vertically
    cardGroups.append('foreignObject')
      .attr('x', 58)
      .attr('y', 0)
      .attr('width', noteWidth - 68)
      .attr('height', 80)
      .append('xhtml:div')
      .style('font-family', cardFont)
      .style('font-size', '22px')
      .style('font-weight', '600')
      .style('color', '#ffffff')
      .style('line-height', '1.3')
      .style('text-align', 'center')
      .style('height', '80px')
      .style('display', 'flex')
      .style('align-items', 'center')
      .style('justify-content', 'center')
      .html(d => {
        const title = d.displayTitle || '';
        return `<div style="display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;">${title}</div>`;
      });

    // Synopsis area - foreignObject with background built into the div (no separate rect)
    const synopsisHeight = noteHeight - 148; // From titlebar (80px) to near zen button
    cardGroups.append('foreignObject')
      .attr('x', 0)
      .attr('y', 80)
      .attr('width', noteWidth)
      .attr('height', synopsisHeight)
      .append('xhtml:div')
      .style('background', 'rgba(0,0,0,0.2)')
      .style('width', '100%')
      .style('height', '100%')
      .style('padding', '4px 14px')
      .style('box-sizing', 'border-box')
      .style('font-family', cardFont)
      .style('font-size', '20px')
      .style('font-weight', '500')
      .style('color', '#ffffff')
      .style('line-height', '1.5')
      .style('overflow', 'hidden')
      .html(d => {
        if (!d.displayContent) return '';
        const items = d.displayContent.split(', ').filter(s => s.trim()).slice(0, 5);
        const bullets = items.map(item => `<div>• ${item}</div>`).join('');
        return `<div style="max-height: 150px; overflow: hidden; line-height: 1.5;">${bullets}</div>`;
      });

    // Footer info
    // "NOTE" badge for items (background + text)
    const noteBadges = cardGroups.filter(d => d.isItem && d.childCount === 0);

    noteBadges.append('rect')
      .attr('x', noteWidth / 2 - 36)
      .attr('y', noteHeight - 34)
      .attr('width', 72)
      .attr('height', 26)
      .attr('rx', 4)
      .attr('fill', '#5b21b6');

    noteBadges.append('text')
      .attr('x', noteWidth / 2)
      .attr('y', noteHeight - 15)
      .attr('text-anchor', 'middle')
      .attr('font-family', cardFont)
      .attr('font-size', '18px')
      .attr('font-weight', '700')
      .attr('letter-spacing', '1.5px')
      .attr('fill', '#ffffff')
      .text('NOTE');

    // Footer info (line 2 for items, only line for topics)
    // Items get date-colored text (red=oldest, cyan=newest)
    // Groups get latestChildDate coloring if available

    // Background rect for footer text (added before text so it renders behind)
    cardGroups.append('rect')
      .attr('class', 'footer-bg')
      .attr('x', 8)
      .attr('y', noteHeight - 34)
      .attr('width', noteWidth - 16)
      .attr('height', 26)
      .attr('rx', 4)
      .attr('ry', 4)
      .attr('fill', 'rgba(0, 0, 0, 0.5)');

    // Left side: item count (for groups) or date (for items)
    cardGroups.append('text')
      .attr('class', 'footer-left')
      .attr('x', 14)
      .attr('y', noteHeight - 16)
      .attr('font-family', cardFont)
      .attr('font-size', '17px')
      .attr('fill', d => {
        if (d.childCount > 0) {
          return 'rgba(255,255,255,0.7)';
        }
        return getDateColor(d.createdAt);
      })
      .text(d => {
        if (d.childCount > 0) {
          return `${d.childCount} items`;
        }
        return new Date(d.createdAt).toLocaleDateString();
      });

    // Right side: latest date (for groups only)
    cardGroups.append('text')
      .attr('class', 'footer-right')
      .attr('x', noteWidth - 14)
      .attr('y', noteHeight - 16)
      .attr('text-anchor', 'end')
      .attr('font-family', cardFont)
      .attr('font-size', '17px')
      .attr('fill', d => {
        if (d.childCount > 0 && d.latestChildDate) {
          return getDateColor(d.latestChildDate);
        }
        return 'transparent';
      })
      .text(d => {
        if (d.childCount > 0 && d.latestChildDate) {
          return `Latest: ${new Date(d.latestChildDate).toLocaleDateString()}`;
        }
        return '';
      });

    // 3-dots menu button for end nodes (items) - inside footer area
    const menuBtnGroups = cardGroups.filter(d => d.childCount === 0)
      .append('g')
      .attr('class', 'node-menu-btn')
      .attr('cursor', 'pointer')
      .attr('transform', `translate(${noteWidth - 36}, ${noteHeight - 32})`);

    // Button background
    menuBtnGroups.append('rect')
      .attr('width', 28)
      .attr('height', 22)
      .attr('rx', 4)
      .attr('fill', 'rgba(255,255,255,0.1)');

    // Three dots
    menuBtnGroups.append('text')
      .attr('x', 14)
      .attr('y', 16)
      .attr('text-anchor', 'middle')
      .attr('font-size', '16px')
      .attr('font-weight', 'bold')
      .attr('fill', 'rgba(255,255,255,0.6)')
      .text('•••');

    menuBtnGroups
      .on('click', function(event, d) {
        event.stopPropagation();
        const rect = (event.target as SVGElement).getBoundingClientRect();
        setNodeMenuId(prev => prev === d.id ? null : d.id);
        setNodeMenuPos({ x: rect.right + 10, y: rect.top });
      })
      .on('mouseenter', function() {
        d3.select(this).select('rect').attr('fill', 'rgba(255,255,255,0.25)');
        d3.select(this).select('text').attr('fill', 'rgba(255,255,255,0.9)');
      })
      .on('mouseleave', function() {
        d3.select(this).select('rect').attr('fill', 'rgba(255,255,255,0.1)');
        d3.select(this).select('text').attr('fill', 'rgba(255,255,255,0.6)');
      });

    // Zen mode removed - hover/selection now handles connection highlighting

    // Unified bubble rendering for ALL nodes (low zoom)
    const dotGroups = dotsGroup.selectAll('g.node-dot')
      .data(graphNodes)
      .join('g')
      .attr('class', 'node-dot')
      .attr('cursor', 'pointer')
      .attr('transform', d => `translate(${d.x}, ${d.y})`);

    // === TOPIC BUBBLE SHADOWS (render order = z-order, first = bottom) ===

    // Violet ring FIRST (ALL topics) - offset = 2 * depth, peeks out at bottom
    dotGroups.filter(d => getStructuralDepth(d.childCount, d.isItem) >= 1)
      .append('circle')
      .attr('class', 'dot-violet')
      .attr('cx', d => 2 * getStructuralDepth(d.childCount, d.isItem))
      .attr('cy', d => 2 * getStructuralDepth(d.childCount, d.isItem))
      .attr('r', dotSize + 2)
      .attr('fill', 'none')
      .attr('stroke', '#5b21b6')
      .attr('stroke-width', 4);

    // Cluster stack 2 - for 6+ children
    dotGroups.filter(d => getStructuralDepth(d.childCount, d.isItem) >= 3)
      .append('circle')
      .attr('class', 'dot-stack-2')
      .attr('cx', 6)
      .attr('cy', 6)
      .attr('r', dotSize - 2)
      .attr('fill', d => {
        const base = d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#374151';
        return d3.color(base)?.darker(1.5)?.toString() || '#1a1a1a';
      })
      .attr('stroke', 'rgba(255,255,255,0.2)')
      .attr('stroke-width', 1);

    // Cluster stack 1 - for 2+ children
    dotGroups.filter(d => getStructuralDepth(d.childCount, d.isItem) >= 2)
      .append('circle')
      .attr('class', 'dot-stack-1')
      .attr('cx', 3)
      .attr('cy', 3)
      .attr('r', dotSize - 1)
      .attr('fill', d => {
        const base = d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#374151';
        return d3.color(base)?.darker(0.8)?.toString() || '#252525';
      })
      .attr('stroke', 'rgba(255,255,255,0.2)')
      .attr('stroke-width', 1);

    dotGroups.append('circle')
      .attr('class', 'dot-glow')
      .attr('r', dotSize + 4)
      .attr('fill', 'none')
      .attr('stroke', d => getNodeColor(d))
      .attr('stroke-width', 3)
      .attr('stroke-opacity', 0.3);

    dotGroups.append('circle')
      .attr('class', 'dot-main')
      .attr('r', dotSize)
      .attr('fill', d => getNodeColor(d))
      .attr('stroke', d => d.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.6)')
      .attr('stroke-width', d => d.id === activeNodeId ? 3 : 1.5);

    // Apply opacity to entire dot group (so violet ring fades with dot)
    dotGroups.style('opacity', d => getNodeOpacity(d));

    dotGroups.append('text')
      .attr('text-anchor', 'middle')
      .attr('dy', '0.35em')
      .attr('font-size', '18px')
      .attr('fill', '#fff')
      .text(d => d.displayEmoji);

    // Start with cards shown, dots hidden
    dotsGroup.style('display', 'none');

    // Zen mode applyZenMode removed - hover/selection handles this now

    // Unified interactions
    cardGroups
      .on('click', function(event, d) {
        event.stopPropagation();
        clickHandledRef.current = true; // Skip duplicate D3 work in activeNodeId useEffect

        // If clicking the already-selected node, deselect (reset to default)
        // But NOT if it's a quick reclick (likely part of double-click)
        const now = Date.now();
        const timeSinceLastClick = now - lastClickTimeRef.current;
        lastClickTimeRef.current = now;

        if (activeNodeIdRef.current === d.id) {
          // Quick reclick (< 300ms) = likely double-click, don't deselect
          if (timeSinceLastClick < 300) {
            return;
          }

          activeNodeIdRef.current = null;
          connectionMapRef.current = new Map();

          // Reset everything to default state
          cardGroups.style('opacity', 1);
          dotGroups.style('opacity', 1);

          cardsGroup.selectAll('.card-bg')
            .attr('fill', (n: GraphNode) => getMutedClusterColor(n))
            .attr('stroke', 'rgba(255,255,255,0.15)')
            .attr('stroke-width', 1);

          cardsGroup.selectAll('.shadow-violet')
            .attr('stroke', 'rgba(255,255,255,0.2)')
            .attr('stroke-width', 1);
          cardsGroup.selectAll('.shadow-cluster')
            .attr('stroke', 'rgba(255,255,255,0.15)')
            .attr('stroke-width', 1);

          dotsGroup.selectAll('.dot-main')
            .attr('fill', (n: GraphNode) => getMutedClusterColor(n))
            .attr('stroke', 'rgba(255,255,255,0.6)')
            .attr('stroke-width', 1.5);

          edgePaths.attr('opacity', 1);
          edgePaths.each(function(e: {source: GraphNode, target: GraphNode, weight: number, type: string}) {
            const el = d3.select(this);
            const normalized = (e.weight - minWeight) / weightRange;
            const originalWidth = e.type === 'contains' ? 6 : 6 + normalized * 18;
            el.attr('stroke-width', originalWidth);
          });

          // Keep details panel open on deselect (user can close manually)
          return;
        }

        // Build connection map directly in click handler (same as hover pattern)
        const clickConnectionMap = new Map<string, {weight: number, distance: number}>();
        const adjacency = new Map<string, Array<{nodeId: string, weight: number}>>();
        edgeData.forEach(edge => {
          const srcId = edge.source.id;
          const tgtId = edge.target.id;
          if (!adjacency.has(srcId)) adjacency.set(srcId, []);
          if (!adjacency.has(tgtId)) adjacency.set(tgtId, []);
          adjacency.get(srcId)!.push({nodeId: tgtId, weight: edge.weight});
          adjacency.get(tgtId)!.push({nodeId: srcId, weight: edge.weight});
        });

        clickConnectionMap.set(d.id, {weight: 1.0, distance: 0});
        const queue: Array<{id: string, dist: number}> = [{id: d.id, dist: 0}];
        while (queue.length > 0) {
          const {id, dist} = queue.shift()!;
          const neighbors = adjacency.get(id) || [];
          for (const {nodeId, weight} of neighbors) {
            if (!clickConnectionMap.has(nodeId)) {
              clickConnectionMap.set(nodeId, {weight, distance: dist + 1});
              queue.push({id: nodeId, dist: dist + 1});
            }
          }
        }

        // Update refs immediately for other handlers to use
        activeNodeIdRef.current = d.id;
        connectionMapRef.current = clickConnectionMap;

        // Helper functions for this click (same logic as useEffect)
        const getClickColor = (node: GraphNode): string => {
          if (node.id === d.id) {
            return node.renderClusterId >= 0 ? generateClusterColor(node.renderClusterId) : '#4b5563';
          }
          const conn = clickConnectionMap.get(node.id);
          if (conn) {
            if (conn.distance === 1) return getDirectConnectionColor(conn.weight);
            return getChainConnectionColor(conn.distance);
          }
          // Muted cluster color for unconnected
          if (node.renderClusterId < 0) return '#374151';
          const hue = (node.renderClusterId * 137.508) % 360;
          return `hsl(${hue}, 12%, 28%)`;
        };

        const getClickOpacity = (node: GraphNode): number => {
          if (node.id === d.id) return 1;
          const conn = clickConnectionMap.get(node.id);
          if (conn) {
            if (conn.distance === 1) return 1;
            return Math.max(0.5, 1 - conn.distance * 0.15);
          }
          return 0.7; // Unconnected = subtle fade
        };

        // D3 updates directly in click handler (instant, no transitions)
        cardGroups.style('opacity', (n: GraphNode) => getClickOpacity(n));

        cardsGroup.selectAll('.card-bg')
          .attr('fill', function(this: SVGRectElement) {
            const parentEl = this.parentNode as Element;
            if (!parentEl) return '#374151';
            const data = d3.select<Element, GraphNode>(parentEl).datum();
            return getClickColor(data);
          })
          .attr('stroke', function(this: SVGRectElement) {
            const parentEl = this.parentNode as Element;
            if (!parentEl) return 'rgba(255,255,255,0.15)';
            const data = d3.select<Element, GraphNode>(parentEl).datum();
            return data.id === d.id ? '#fbbf24' : 'rgba(255,255,255,0.15)';
          })
          .attr('stroke-width', function(this: SVGRectElement) {
            const parentEl = this.parentNode as Element;
            if (!parentEl) return 1;
            const data = d3.select<Element, GraphNode>(parentEl).datum();
            return data.id === d.id ? 2 : 1;
          });

        // Update shadow strokes for selection (instant)
        cardsGroup.selectAll('.shadow-violet')
          .attr('stroke', function(this: SVGRectElement) {
            const parentEl = this.parentNode as Element;
            if (!parentEl) return 'rgba(255,255,255,0.2)';
            const data = d3.select<Element, GraphNode>(parentEl).datum();
            return data.id === d.id ? '#fbbf24' : 'rgba(255,255,255,0.2)';
          })
          .attr('stroke-width', function(this: SVGRectElement) {
            const parentEl = this.parentNode as Element;
            if (!parentEl) return 1;
            const data = d3.select<Element, GraphNode>(parentEl).datum();
            return data.id === d.id ? 2 : 1;
          });

        cardsGroup.selectAll('.shadow-cluster')
          .attr('stroke', function(this: SVGRectElement) {
            const parentEl = this.parentNode as Element;
            if (!parentEl) return 'rgba(255,255,255,0.15)';
            const data = d3.select<Element, GraphNode>(parentEl).datum();
            return data.id === d.id ? '#fbbf24' : 'rgba(255,255,255,0.15)';
          })
          .attr('stroke-width', function(this: SVGRectElement) {
            const parentEl = this.parentNode as Element;
            if (!parentEl) return 1;
            const data = d3.select<Element, GraphNode>(parentEl).datum();
            return data.id === d.id ? 2 : 1;
          });

        // Update dots too (instant)
        dotGroups.style('opacity', (n: GraphNode) => getClickOpacity(n));

        dotsGroup.selectAll('.dot-main')
          .attr('fill', function(this: SVGCircleElement) {
            const parentEl = this.parentNode as Element;
            if (!parentEl) return '#374151';
            const data = d3.select<Element, GraphNode>(parentEl).datum();
            return getClickColor(data);
          });

        // Update edges - direct connections bright, chain dimmer, unconnected very dim
        edgePaths.attr('opacity', (e: {source: GraphNode, target: GraphNode}) => {
          if (e.source.id === d.id || e.target.id === d.id) return 0.9; // Direct connection
          // Chain edge: both endpoints in connection map
          const srcConn = clickConnectionMap.has(e.source.id);
          const tgtConn = clickConnectionMap.has(e.target.id);
          if (srcConn && tgtConn) return 0.3; // Chain - dimmer
          return 0.08; // Unconnected - very dim
        });

        // Double width for direct connections
        edgePaths.each(function(e: {source: GraphNode, target: GraphNode, weight: number, type: string}) {
          const el = d3.select(this);
          const normalized = (e.weight - minWeight) / weightRange;
          const originalWidth = e.type === 'contains' ? 6 : 6 + normalized * 18;
          if (e.source.id === d.id || e.target.id === d.id) {
            el.attr('stroke-width', originalWidth * 2);
          } else {
            el.attr('stroke-width', originalWidth);
          }
        });

        // THEN update React state for sidebar/other consumers
        // setActiveNode(d.id); // TEMP: testing if React re-render is bottleneck
        // Defer similar nodes fetch - don't block visual feedback
        // Store timeout ID so we can cancel on double-click
        pendingFetchRef.current = setTimeout(() => fetchSimilarNodes(d.id), 50);
      })
      .on('dblclick', function(event, d) {
        event.stopPropagation();
        // Cancel pending fetch to preserve existing details pane
        if (pendingFetchRef.current) {
          clearTimeout(pendingFetchRef.current);
          pendingFetchRef.current = null;
        }
        if (d.isItem) {
          devLog('info', `Opening item "${d.displayTitle}" in Leaf mode`);
          openLeaf(d.id);
        } else if (d.childCount > 0) {
          devLog('info', `Drilling into "${d.displayTitle}" (depth ${d.depth} → ${d.depth + 1})`);
          setActiveNode(null); // Clear selection when navigating
          navigateToNode(d);
        }
      })
      .on('mouseenter', function(_, d) {
        setHoveredNode(d);
        // Color change (instant)
        d3.select(this).select('.card-bg')
          .attr('fill', d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#4b5563');
        // Build connection map with BFS for multi-hop distances
        const hoverConnectionMap = new Map<string, {weight: number, distance: number}>();
        const adjacency = new Map<string, Array<{nodeId: string, weight: number}>>();
        edgeData.forEach(edge => {
          const srcId = edge.source.id;
          const tgtId = edge.target.id;
          if (!adjacency.has(srcId)) adjacency.set(srcId, []);
          if (!adjacency.has(tgtId)) adjacency.set(tgtId, []);
          adjacency.get(srcId)!.push({nodeId: tgtId, weight: edge.weight});
          adjacency.get(tgtId)!.push({nodeId: srcId, weight: edge.weight});
        });

        hoverConnectionMap.set(d.id, {weight: 1.0, distance: 0});
        const queue: Array<{id: string, dist: number}> = [{id: d.id, dist: 0}];
        while (queue.length > 0) {
          const {id, dist} = queue.shift()!;
          const neighbors = adjacency.get(id) || [];
          for (const {nodeId, weight} of neighbors) {
            if (!hoverConnectionMap.has(nodeId)) {
              hoverConnectionMap.set(nodeId, {weight, distance: dist + 1});
              queue.push({id: nodeId, dist: dist + 1});
            }
          }
        }

        // Opacity: direct=bright, chain=visible, unconnected=dim
        const getHoverOpacity = (n: GraphNode): number => {
          const conn = hoverConnectionMap.get(n.id);
          if (!conn) return 0.7; // Unconnected = subtle fade
          if (conn.distance === 0) return 1; // Hovered node
          if (conn.distance === 1) return 0.6 + conn.weight * 0.4; // Direct: 0.6-1.0
          return 0.8; // Chain: consistently visible
        };

        // Instant updates (no transitions for testing)
        cardGroups.style('opacity', (n: GraphNode) => getHoverOpacity(n));
        dotGroups.style('opacity', (n: GraphNode) => getHoverOpacity(n));
        // Reset ALL edges to original width
        edgePaths.each(function(e: {source: GraphNode, target: GraphNode, weight: number, type: string}) {
          const el = d3.select(this);
          const normalized = (e.weight - minWeight) / weightRange;
          const originalWidth = e.type === 'contains' ? 6 : 6 + normalized * 18;
          el.attr('stroke-width', originalWidth);
        });
        // Set edge opacity
        edgePaths.attr('opacity', (e: {source: GraphNode, target: GraphNode}) => {
          if (e.source.id === d.id || e.target.id === d.id) return 0.9;
          const srcConn = hoverConnectionMap.has(e.source.id);
          const tgtConn = hoverConnectionMap.has(e.target.id);
          if (srcConn && tgtConn) return 0.7;
          return 0.15;
        });
        // Double connected edges for this card
        edgePaths.filter((e: {source: GraphNode, target: GraphNode}) =>
          e.source.id === d.id || e.target.id === d.id
        ).each(function(e: {source: GraphNode, target: GraphNode, weight: number, type: string}) {
          const el = d3.select(this);
          const normalized = (e.weight - minWeight) / weightRange;
          const originalWidth = e.type === 'contains' ? 6 : 6 + normalized * 18;
          el.attr('stroke-width', originalWidth * 2);
        });
      })
      .on('mouseleave', function(_, d) {
        setHoveredNode(null);
        // Color change back (instant)
        d3.select(this).select('.card-bg')
          .attr('fill', getNodeColor(d));

        // Get current selection state from refs
        const currentActiveId = activeNodeIdRef.current;
        const currentConnMap = connectionMapRef.current;

        // Reset node opacities based on selection state (instant)
        if (currentActiveId) {
          // Selection active: apply selection-based opacity
          cardGroups.style('opacity', (n: GraphNode) => {
            if (n.id === currentActiveId) return 1;
            const conn = currentConnMap.get(n.id);
            if (conn) {
              if (conn.distance === 1) return 1;
              return Math.max(0.5, 1 - conn.distance * 0.15);
            }
            return 0.3;
          });
          dotGroups.style('opacity', (n: GraphNode) => {
            if (n.id === currentActiveId) return 1;
            const conn = currentConnMap.get(n.id);
            if (conn) {
              if (conn.distance === 1) return 1;
              return Math.max(0.5, 1 - conn.distance * 0.15);
            }
            return 0.3;
          });
          // Selection active: apply selection-based edge opacity
          edgePaths.attr('opacity', (e: {source: GraphNode, target: GraphNode}) => {
            if (e.source.id === currentActiveId || e.target.id === currentActiveId) return 0.9;
            const srcConn = currentConnMap.has(e.source.id);
            const tgtConn = currentConnMap.has(e.target.id);
            if (srcConn && tgtConn) return 0.7;
            return 0.15;
          });
        } else {
          // No selection: reset to full opacity
          cardGroups.style('opacity', 1);
          dotGroups.style('opacity', 1);
          edgePaths.attr('opacity', 1);
        }

        // Reset stroke-width to original (instant)
        edgePaths.each(function(e: {source: GraphNode, target: GraphNode, weight: number, type: string}) {
          const el = d3.select(this);
          const normalized = (e.weight - minWeight) / weightRange;
          const originalWidth = e.type === 'contains' ? 6 : 6 + normalized * 18;
          el.attr('stroke-width', originalWidth);
        });
      });

    // Interactions for dots (same as cards but for bubble mode)
    dotGroups
      .on('click', function(event, d) {
        event.stopPropagation();
        clickHandledRef.current = true; // Skip duplicate D3 work in activeNodeId useEffect

        // Build connection map directly in click handler (same pattern as cards)
        const clickConnectionMap = new Map<string, {weight: number, distance: number}>();
        const adjacency = new Map<string, Array<{nodeId: string, weight: number}>>();
        edgeData.forEach(edge => {
          const srcId = edge.source.id;
          const tgtId = edge.target.id;
          if (!adjacency.has(srcId)) adjacency.set(srcId, []);
          if (!adjacency.has(tgtId)) adjacency.set(tgtId, []);
          adjacency.get(srcId)!.push({nodeId: tgtId, weight: edge.weight});
          adjacency.get(tgtId)!.push({nodeId: srcId, weight: edge.weight});
        });
        clickConnectionMap.set(d.id, {weight: 1.0, distance: 0});
        const queue: Array<{id: string, dist: number}> = [{id: d.id, dist: 0}];
        while (queue.length > 0) {
          const {id, dist} = queue.shift()!;
          const neighbors = adjacency.get(id) || [];
          for (const {nodeId, weight} of neighbors) {
            if (!clickConnectionMap.has(nodeId)) {
              clickConnectionMap.set(nodeId, {weight, distance: dist + 1});
              queue.push({id: nodeId, dist: dist + 1});
            }
          }
        }

        // Update refs immediately
        activeNodeIdRef.current = d.id;
        connectionMapRef.current = clickConnectionMap;

        // Helper functions
        const getClickColor = (node: GraphNode): string => {
          if (node.id === d.id) {
            return node.renderClusterId >= 0 ? generateClusterColor(node.renderClusterId) : '#4b5563';
          }
          const conn = clickConnectionMap.get(node.id);
          if (conn) {
            if (conn.distance === 1) return getDirectConnectionColor(conn.weight);
            return getChainConnectionColor(conn.distance);
          }
          if (node.renderClusterId < 0) return '#374151';
          const hue = (node.renderClusterId * 137.508) % 360;
          return `hsl(${hue}, 12%, 28%)`;
        };

        const getClickOpacity = (node: GraphNode): number => {
          if (node.id === d.id) return 1;
          const conn = clickConnectionMap.get(node.id);
          if (conn) {
            if (conn.distance === 1) return 1;
            return Math.max(0.5, 1 - conn.distance * 0.15);
          }
          return 0.7; // Unconnected = subtle fade
        };

        // D3 updates directly (instant, no transitions)
        dotGroups.style('opacity', (n: GraphNode) => getClickOpacity(n));

        dotsGroup.selectAll('.dot-main')
          .attr('fill', function(this: SVGCircleElement) {
            const parentEl = this.parentNode as Element;
            if (!parentEl) return '#374151';
            const data = d3.select<Element, GraphNode>(parentEl).datum();
            return getClickColor(data);
          })
          .attr('stroke', function(this: SVGCircleElement) {
            const parentEl = this.parentNode as Element;
            if (!parentEl) return 'transparent';
            const data = d3.select<Element, GraphNode>(parentEl).datum();
            return data.id === d.id ? '#fbbf24' : 'transparent';
          })
          .attr('stroke-width', function(this: SVGCircleElement) {
            const parentEl = this.parentNode as Element;
            if (!parentEl) return 0;
            const data = d3.select<Element, GraphNode>(parentEl).datum();
            return data.id === d.id ? 3 : 0;
          });

        cardGroups.style('opacity', (n: GraphNode) => getClickOpacity(n));

        // THEN update React state
        // setActiveNode(d.id); // TEMP: testing if React re-render is bottleneck
        // Defer similar nodes fetch - don't block visual feedback
        // Store timeout ID so we can cancel on double-click
        pendingFetchRef.current = setTimeout(() => fetchSimilarNodes(d.id), 50);
      })
      .on('dblclick', function(event, d) {
        event.stopPropagation();
        // Cancel pending fetch to preserve existing details pane
        if (pendingFetchRef.current) {
          clearTimeout(pendingFetchRef.current);
          pendingFetchRef.current = null;
        }
        if (d.isItem) {
          devLog('info', `Opening item "${d.displayTitle}" in Leaf mode`);
          openLeaf(d.id);
        } else if (d.childCount > 0) {
          devLog('info', `Drilling into "${d.displayTitle}" (depth ${d.depth} → ${d.depth + 1})`);
          setActiveNode(null); // Clear selection when navigating (keep this one)
          navigateToNode(d);
        }
      })
      .on('mouseenter', function(_, d) {
        setHoveredNode(d);
        // Color transition
        d3.select(this).select('.dot-main')
          .transition().duration(200)
          .attr('fill', d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#4b5563');
        d3.select(this).select('.dot-glow')
          .transition().duration(200)
          .attr('stroke', d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#4b5563');
        // Build connection map with BFS for multi-hop distances
        const hoverConnectionMap = new Map<string, {weight: number, distance: number}>();
        const adjacency = new Map<string, Array<{nodeId: string, weight: number}>>();
        edgeData.forEach(edge => {
          const srcId = edge.source.id;
          const tgtId = edge.target.id;
          if (!adjacency.has(srcId)) adjacency.set(srcId, []);
          if (!adjacency.has(tgtId)) adjacency.set(tgtId, []);
          adjacency.get(srcId)!.push({nodeId: tgtId, weight: edge.weight});
          adjacency.get(tgtId)!.push({nodeId: srcId, weight: edge.weight});
        });
        hoverConnectionMap.set(d.id, {weight: 1.0, distance: 0});
        const queue: Array<{id: string, dist: number}> = [{id: d.id, dist: 0}];
        while (queue.length > 0) {
          const {id, dist} = queue.shift()!;
          const neighbors = adjacency.get(id) || [];
          for (const {nodeId, weight} of neighbors) {
            if (!hoverConnectionMap.has(nodeId)) {
              hoverConnectionMap.set(nodeId, {weight, distance: dist + 1});
              queue.push({id: nodeId, dist: dist + 1});
            }
          }
        }
        // Opacity: direct=bright, chain=visible, unconnected=dim
        const getHoverOpacity = (n: GraphNode): number => {
          const conn = hoverConnectionMap.get(n.id);
          if (!conn) return 0.7; // Unconnected = subtle fade
          if (conn.distance === 0) return 1; // Hovered node
          if (conn.distance === 1) return 0.6 + conn.weight * 0.4; // Direct: 0.6-1.0
          return 0.8; // Chain: consistently visible
        };
        cardGroups.transition().duration(1500).style('opacity', (n: GraphNode) => getHoverOpacity(n));
        dotGroups.transition().duration(1500).style('opacity', (n: GraphNode) => getHoverOpacity(n));
        // First: reset ALL edges to original width (interrupt any transitions)
        edgePaths.interrupt().each(function(e: {source: GraphNode, target: GraphNode, weight: number, type: string}) {
          const el = d3.select(this);
          const normalized = (e.weight - minWeight) / weightRange;
          const originalWidth = e.type === 'contains' ? 6 : 6 + normalized * 18;
          el.attr('stroke-width', originalWidth);
        });
        // Then set edge opacity and width
        edgePaths.transition().duration(1500)
          .attr('opacity', (e: {source: GraphNode, target: GraphNode}) => {
            if (e.source.id === d.id || e.target.id === d.id) return 0.9; // Direct
            // Chain edge: both endpoints are in connection map
            const srcConn = hoverConnectionMap.has(e.source.id);
            const tgtConn = hoverConnectionMap.has(e.target.id);
            if (srcConn && tgtConn) return 0.7; // Chain edge stays visible
            return 0.15; // Unconnected edge fades
          });
        // Double connected edges for this dot
        edgePaths.filter((e: {source: GraphNode, target: GraphNode}) =>
          e.source.id === d.id || e.target.id === d.id
        ).each(function(e: {source: GraphNode, target: GraphNode, weight: number, type: string}) {
          const el = d3.select(this);
          const normalized = (e.weight - minWeight) / weightRange;
          const originalWidth = e.type === 'contains' ? 6 : 6 + normalized * 18;
          el.transition().duration(150).attr('stroke-width', originalWidth * 2);
        });
      })
      .on('mouseleave', function(_, d) {
        setHoveredNode(null);
        // Color transition back
        d3.select(this).select('.dot-main')
          .transition().duration(200)
          .attr('fill', getNodeColor(d));
        d3.select(this).select('.dot-glow')
          .transition().duration(200)
          .attr('stroke', getNodeColor(d));

        // Get current selection state from refs
        const currentActiveId = activeNodeIdRef.current;
        const currentConnMap = connectionMapRef.current;

        // Reset node opacities based on selection state
        if (currentActiveId) {
          // Selection active: apply selection-based opacity
          cardGroups.transition().duration(300).style('opacity', (n: GraphNode) => {
            if (n.id === currentActiveId) return 1;
            const conn = currentConnMap.get(n.id);
            if (conn) {
              if (conn.distance === 1) return 1;
              return Math.max(0.5, 1 - conn.distance * 0.15);
            }
            return 0.3;
          });
          dotGroups.transition().duration(300).style('opacity', (n: GraphNode) => {
            if (n.id === currentActiveId) return 1;
            const conn = currentConnMap.get(n.id);
            if (conn) {
              if (conn.distance === 1) return 1;
              return Math.max(0.5, 1 - conn.distance * 0.15);
            }
            return 0.3;
          });
          // Selection active: apply selection-based edge opacity
          edgePaths.transition().duration(150).attr('opacity', (e: {source: GraphNode, target: GraphNode}) => {
            if (e.source.id === currentActiveId || e.target.id === currentActiveId) return 0.9;
            const srcConn = currentConnMap.has(e.source.id);
            const tgtConn = currentConnMap.has(e.target.id);
            if (srcConn && tgtConn) return 0.7;
            return 0.15;
          });
        } else {
          // No selection: reset to full opacity
          cardGroups.transition().duration(300).style('opacity', 1);
          dotGroups.transition().duration(300).style('opacity', 1);
          edgePaths.transition().duration(150).attr('opacity', 1);
        }

        // Reset stroke-width to original
        edgePaths.each(function(e: {source: GraphNode, target: GraphNode, weight: number, type: string}) {
          const el = d3.select(this);
          const normalized = (e.weight - minWeight) / weightRange;
          const originalWidth = e.type === 'contains' ? 6 : 6 + normalized * 18;
          el.transition().duration(150).attr('stroke-width', originalWidth);
        });
      });

    // Track current zoom
    let currentScale = 1;

    // Zoom level-of-detail
    const minApparentSize = 0.75;       // Size at 35%+ zoom
    const sparseMinApparentSize = 0.52; // Size at 14% zoom
    const curveStart = 0.14;            // Start of smooth curve
    const curveEnd = 0.35;              // End of smooth curve
    // Switch to bubble view at 12% zoom
    const bubbleThreshold = 0.12;

    const zoom = d3.zoom<SVGSVGElement, unknown>()
      .scaleExtent([minZoom, maxZoom])
      .translateExtent([[graphBounds.minX, graphBounds.minY], [graphBounds.maxX, graphBounds.maxY]])
      .filter((event) => {
        // Allow scroll wheel for zooming
        if (event.type === 'wheel') return true;
        // Prevent pan from starting on card/dot elements (so double-click works)
        const target = event.target as Element;
        if (target.closest('.card-group') || target.closest('.dot-group')) {
          return false;
        }
        return true;
      })
      .on('zoom', (event) => {
        const { k, x, y } = event.transform;
        currentScale = k;

        // Store transform for viewport culling
        zoomTransformRef.current = { k, x, y };

        container.attr('transform', event.transform);

        // Viewport culling: hide off-screen nodes for performance
        const viewportBuffer = 400; // pixels
        cardGroups.style('visibility', (d: GraphNode) => {
          const screenX = d.x * k + x;
          const screenY = d.y * k + y;
          const isVisible =
            screenX > -viewportBuffer && screenX < width + viewportBuffer &&
            screenY > -viewportBuffer && screenY < height + viewportBuffer;
          return isVisible ? 'visible' : 'hidden';
        });
        dotGroups.style('visibility', (d: GraphNode) => {
          const screenX = d.x * k + x;
          const screenY = d.y * k + y;
          const isVisible =
            screenX > -viewportBuffer && screenX < width + viewportBuffer &&
            screenY > -viewportBuffer && screenY < height + viewportBuffer;
          return isVisible ? 'visible' : 'hidden';
        });

        if (k >= bubbleThreshold) {
          // Card mode - show cards, hide bubbles
          dotsGroup.style('display', 'none');
          cardsGroup.style('display', 'block');

          // Smooth curve for minimum size from 14% to 35% zoom
          const smoothstep = (edge0: number, edge1: number, x: number) => {
            const t = Math.max(0, Math.min(1, (x - edge0) / (edge1 - edge0)));
            return t * t * (3 - 2 * t);
          };
          const blendFactor = smoothstep(curveStart, curveEnd, k);
          const effectiveMinSize = sparseMinApparentSize + blendFactor * (minApparentSize - sparseMinApparentSize);

          // Below 35%, gently expand positions to slow down how fast things get closer
          // sqrt makes the compression more gradual
          const positionScale = k < curveEnd ? Math.sqrt(curveEnd / k) : 1;

          let cardScale = 1;
          if (k < 1) {
            const targetApparent = Math.max(effectiveMinSize, Math.sqrt(k));
            cardScale = targetApparent / k;
          }
          cardGroups.attr('transform', d =>
            `translate(${d.x * positionScale}, ${d.y * positionScale}) scale(${cardScale}) translate(${-noteWidth/2}, ${-noteHeight/2})`
          );

          // Update edge positions to match node scaling
          const scaledWidth = noteWidth * cardScale;
          const scaledHeight = noteHeight * cardScale;
          edgePaths.attr('d', d => {
            // Find closest pair of side centers between the two nodes
            const scaledSource = { x: d.source.x * positionScale, y: d.source.y * positionScale };
            const scaledTarget = { x: d.target.x * positionScale, y: d.target.y * positionScale };
            const points = getEdgePoints(scaledSource, scaledTarget, scaledWidth, scaledHeight);

            const dx = points.target.x - points.source.x;
            const dy = points.target.y - points.source.y;
            const dr = Math.sqrt(dx * dx + dy * dy) * 1.5;
            return `M${points.source.x},${points.source.y} A${dr},${dr} 0 0,1 ${points.target.x},${points.target.y}`;
          });

          // Update connection dots (both endpoints)
          connectionDots.each(function(d: any) {
            const edge = d.edge;
            if (!edge) return;

            const scaledSource = { x: edge.source.x * positionScale, y: edge.source.y * positionScale };
            const scaledTarget = { x: edge.target.x * positionScale, y: edge.target.y * positionScale };
            const points = getEdgePoints(scaledSource, scaledTarget, scaledWidth, scaledHeight);
            const point = d.isSource ? points.source : points.target;

            d3.select(this).attr('cx', point.x).attr('cy', point.y);
          });

        } else {
          // Bubble mode - show bubbles, hide cards
          cardsGroup.style('display', 'none');
          dotsGroup.style('display', 'block');

          // Gentle position expansion for bubbles too
          const positionScale = Math.sqrt(curveEnd / k);

          const targetScreenSize = 40;
          const bubbleScale = targetScreenSize / (dotSize * 2 * k);

          dotGroups.attr('transform', d =>
            `translate(${d.x * positionScale}, ${d.y * positionScale}) scale(${bubbleScale})`
          );

          // Update edge positions for bubble mode too
          const scaledDotSize = dotSize * bubbleScale;
          const bubbleDiameter = scaledDotSize * 2;
          edgePaths.attr('d', d => {
            // Find closest pair of side centers between the two nodes
            const scaledSource = { x: d.source.x * positionScale, y: d.source.y * positionScale };
            const scaledTarget = { x: d.target.x * positionScale, y: d.target.y * positionScale };
            const points = getEdgePoints(scaledSource, scaledTarget, bubbleDiameter, bubbleDiameter);

            const dx = points.target.x - points.source.x;
            const dy = points.target.y - points.source.y;
            const dr = Math.sqrt(dx * dx + dy * dy) * 1.5;
            return `M${points.source.x},${points.source.y} A${dr},${dr} 0 0,1 ${points.target.x},${points.target.y}`;
          });

          // Update connection dots for bubble mode (both endpoints)
          connectionDots.each(function(d: any) {
            const edge = d.edge;
            if (!edge) return;

            const scaledSource = { x: edge.source.x * positionScale, y: edge.source.y * positionScale };
            const scaledTarget = { x: edge.target.x * positionScale, y: edge.target.y * positionScale };
            const points = getEdgePoints(scaledSource, scaledTarget, bubbleDiameter, bubbleDiameter);
            const point = d.isSource ? points.source : points.target;

            d3.select(this).attr('cx', point.x).attr('cy', point.y);
          });
        }

        setZoomLevel(k);
      });

    svg.call(zoom);

    // Click to deselect - reset everything to default state
    svg.on('click', () => {
      // Clear refs
      activeNodeIdRef.current = null;
      connectionMapRef.current = new Map();

      // Reset card opacities
      cardGroups.style('opacity', 1);
      dotGroups.style('opacity', 1);

      // Reset card-bg fills to muted cluster colors and strokes to default
      cardsGroup.selectAll('.card-bg')
        .attr('fill', function(this: SVGRectElement) {
          const parentEl = this.parentNode as Element;
          if (!parentEl) return '#374151';
          const data = d3.select<Element, GraphNode>(parentEl).datum();
          return getMutedClusterColor(data);
        })
        .attr('stroke', 'rgba(255,255,255,0.15)')
        .attr('stroke-width', 1);

      // Reset shadow strokes
      cardsGroup.selectAll('.shadow-violet')
        .attr('stroke', 'rgba(255,255,255,0.2)')
        .attr('stroke-width', 1);
      cardsGroup.selectAll('.shadow-cluster')
        .attr('stroke', 'rgba(255,255,255,0.15)')
        .attr('stroke-width', 1);

      // Reset dots to muted colors
      dotsGroup.selectAll('.dot-main')
        .attr('fill', function(this: SVGCircleElement) {
          const parentEl = this.parentNode as Element;
          if (!parentEl) return '#374151';
          const data = d3.select<Element, GraphNode>(parentEl).datum();
          return getMutedClusterColor(data);
        })
        .attr('stroke', 'rgba(255,255,255,0.6)')
        .attr('stroke-width', 1.5);

      // Reset edge opacities and widths
      edgePaths.attr('opacity', 1);
      edgePaths.each(function(e: {source: GraphNode, target: GraphNode, weight: number, type: string}) {
        const el = d3.select(this);
        const normalized = (e.weight - minWeight) / weightRange;
        const originalWidth = e.type === 'contains' ? 6 : 6 + normalized * 18;
        el.attr('stroke-width', originalWidth);
      });

      // Clear selection but keep details panel open (user can close manually)
      setActiveNode(null);
    });

    // Fit to view
    if (graphNodes.length > 0) {
      const xs = graphNodes.map(n => n.x);
      const ys = graphNodes.map(n => n.y);
      const padding = noteWidth;

      const minX = Math.min(...xs) - padding;
      const maxX = Math.max(...xs) + padding;
      const minY = Math.min(...ys) - padding;
      const maxY = Math.max(...ys) + padding;

      const graphWidth = maxX - minX;
      const graphHeight = maxY - minY;

      const fitScale = Math.min(width / graphWidth, height / graphHeight) * 0.85;
      const scale = Math.max(0.1, Math.min(fitScale, 1));

      const graphCenterX = (minX + maxX) / 2;
      const graphCenterY = (minY + maxY) / 2;

      devLog('info', `Fit: scale=${(scale * 100).toFixed(0)}%, bounds=${graphWidth.toFixed(0)}x${graphHeight.toFixed(0)}`);

      const initialTransform = d3.zoomIdentity
        .translate(width / 2, height / 2)
        .scale(scale)
        .translate(-graphCenterX, -graphCenterY);

      svg.call(zoom.transform, initialTransform);
    } else {
      devLog('warn', 'No nodes to display at this depth');
    }

  // Note: activeNodeId removed from deps - color updates handled by separate useEffect below
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nodes, edges, width, height, setActiveNode, devLog, getNodeEmoji, currentDepth, maxDepth, currentParentId, navigateToNode, hidePrivate]);

  // Update connection colors when activeNodeId changes (without full re-render)
  useEffect(() => {
    devLog('info', 'activeNodeId useEffect running');
    if (!svgRef.current || nodes.size === 0) return;

    // Skip if click handler already did the D3 update
    if (clickHandledRef.current) {
      clickHandledRef.current = false;
      devLog('info', 'activeNodeId useEffect skipped (click handler did update)');
      return;
    }

    const svg = d3.select(svgRef.current);

    // Use memoized connection map (computed once per activeNodeId/edges change)
    const connectionMap = memoizedConnectionMap;

    // Update refs for access by event handlers
    activeNodeIdRef.current = activeNodeId;
    connectionMapRef.current = connectionMap;

    // Helper to get muted cluster color (gray with hint of cluster hue)
    const getMutedColor = (data: GraphNode): string => {
      if (data.renderClusterId < 0) return '#374151';
      const hue = (data.renderClusterId * 137.508) % 360;
      return `hsl(${hue}, 12%, 28%)`; // Very low saturation, dark
    };

    // Helper to get color from node data based on connection distance
    const getColorFromData = (data: GraphNode | null): string => {
      if (!data) return '#374151';
      if (!activeNodeId) return getMutedColor(data); // Muted cluster hint when nothing selected
      if (data.id === activeNodeId) {
        // Selected node shows its cluster color
        return data.renderClusterId >= 0 ? generateClusterColor(data.renderClusterId) : '#4b5563';
      }
      const conn = connectionMap.get(data.id);
      if (conn) {
        if (conn.distance === 1) {
          // Direct connection: red→green based on weight
          return getDirectConnectionColor(conn.weight);
        } else {
          // Chain connection (2+ hops): darker red
          return getChainConnectionColor(conn.distance);
        }
      }
      return getMutedColor(data); // Unconnected = muted cluster hint
    };

    // Helper to get opacity from node data based on connection distance
    const getOpacityFromData = (data: GraphNode | null): number => {
      if (!data) return 1;
      if (!activeNodeId) return 1; // Full opacity when nothing selected
      if (data.id === activeNodeId) return 1; // Selected = full
      const conn = connectionMap.get(data.id);
      if (conn) {
        if (conn.distance === 1) return 1; // Direct = full
        // Chain: slight fade based on distance
        return Math.max(0.5, 1 - conn.distance * 0.15);
      }
      return 0.7; // Unconnected = subtle fade
    };

    // Update card background colors and selection stroke (with transition for smooth feel)
    svg.selectAll<SVGRectElement, GraphNode>('.card-bg')
      .transition().duration(150)
      .attr('fill', function(this: SVGRectElement) {
        const parentEl = this.parentNode as Element;
        if (!parentEl) return '#374151';
        const data = d3.select<Element, GraphNode>(parentEl).datum();
        return getColorFromData(data);
      })
      .attr('stroke', function(this: SVGRectElement) {
        const parentEl = this.parentNode as Element;
        if (!parentEl) return 'rgba(255,255,255,0.15)';
        const data = d3.select<Element, GraphNode>(parentEl).datum();
        return data.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.15)';
      })
      .attr('stroke-width', function(this: SVGRectElement) {
        const parentEl = this.parentNode as Element;
        if (!parentEl) return 1;
        const data = d3.select<Element, GraphNode>(parentEl).datum();
        return data.id === activeNodeId ? 2 : 1;
      });

    // Update shadow strokes for selection
    svg.selectAll<SVGRectElement, GraphNode>('.shadow-violet')
      .transition().duration(150)
      .attr('stroke', function(this: SVGRectElement) {
        const parentEl = this.parentNode as Element;
        if (!parentEl) return 'rgba(255,255,255,0.2)';
        const data = d3.select<Element, GraphNode>(parentEl).datum();
        return data.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.2)';
      })
      .attr('stroke-width', function(this: SVGRectElement) {
        const parentEl = this.parentNode as Element;
        if (!parentEl) return 1;
        const data = d3.select<Element, GraphNode>(parentEl).datum();
        return data.id === activeNodeId ? 2 : 1;
      });

    svg.selectAll<SVGRectElement, GraphNode>('.shadow-cluster')
      .transition().duration(150)
      .attr('stroke', function(this: SVGRectElement) {
        const parentEl = this.parentNode as Element;
        if (!parentEl) return 'rgba(255,255,255,0.15)';
        const data = d3.select<Element, GraphNode>(parentEl).datum();
        return data.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.15)';
      })
      .attr('stroke-width', function(this: SVGRectElement) {
        const parentEl = this.parentNode as Element;
        if (!parentEl) return 1;
        const data = d3.select<Element, GraphNode>(parentEl).datum();
        return data.id === activeNodeId ? 2 : 1;
      });

    // Update card group opacity (fades entire card including shadows)
    svg.selectAll<SVGGElement, GraphNode>('g.card')
      .transition().duration(150)
      .style('opacity', (d: GraphNode) => getOpacityFromData(d));

    // Update bubble glow colors
    svg.selectAll<SVGCircleElement, GraphNode>('.dot-glow')
      .transition().duration(150)
      .attr('stroke', function(this: SVGCircleElement) {
        const parentEl = this.parentNode as Element;
        if (!parentEl) return '#374151';
        const data = d3.select<Element, GraphNode>(parentEl).datum();
        return getColorFromData(data);
      });

    // Update bubble main colors
    svg.selectAll<SVGCircleElement, GraphNode>('.dot-main')
      .transition().duration(150)
      .attr('fill', function(this: SVGCircleElement) {
        const parentEl = this.parentNode as Element;
        if (!parentEl) return '#374151';
        const data = d3.select<Element, GraphNode>(parentEl).datum();
        return getColorFromData(data);
      });

    // Update dot group opacity (fades entire dot including violet ring)
    svg.selectAll<SVGGElement, GraphNode>('g.dot')
      .transition().duration(150)
      .style('opacity', (d: GraphNode) => getOpacityFromData(d));

    // Edge opacity is now applied at render time in main useEffect

  }, [activeNodeId, nodes, edges]);

  const logColors = {
    info: 'text-blue-400',
    warn: 'text-yellow-400',
    error: 'text-red-400',
  };

  // Build breadcrumb display: Universe + navigation path + current level
  const currentLevelInfo = getLevelName(currentDepth, maxDepth);

  return (
    <div className="relative w-full h-full">
      <svg
        ref={svgRef}
        className="bg-gray-900 w-full h-full"
        style={{ cursor: 'grab' }}
      />

      {/* Hierarchy Breadcrumbs - Shows actual navigation path with node titles */}
      <div className="absolute top-4 left-4 bg-gray-800/90 backdrop-blur-sm rounded-lg px-3 py-2 z-10">
        <div className="flex items-center gap-1">
          {/* Universe root - always shown */}
          <button
            onClick={() => {
              setActiveNode(null); // Clear selection when navigating
              navigateToRoot();
              devLog('info', 'Navigated to Universe (root)');
            }}
            className={`flex items-center gap-1 px-2 py-1 rounded transition-colors ${
              currentDepth === 0 && breadcrumbs.length === 0
                ? 'bg-amber-500/20 text-amber-300'
                : 'hover:bg-gray-700 text-gray-300 cursor-pointer'
            }`}
            title="Go to Universe (root)"
          >
            <span className="text-base">🌌</span>
            <span className="text-sm font-medium">Universe{toSuperscript(0)}</span>
          </button>

          {/* Navigation path - show each node we've drilled into */}
          {breadcrumbs.map((crumb, index) => {
            const isLast = index === breadcrumbs.length - 1;
            const crumbNode = nodes.get(crumb.id);
            const lastNode = isLast ? crumbNode : null;
            const isJump = crumb.isJump;
            const crumbEmoji = crumbNode ? getNodeEmoji(crumbNode) : crumb.emoji;
            // Check if jumpFrom is same as previous breadcrumb (avoid duplicate)
            const prevCrumb = index > 0 ? breadcrumbs[index - 1] : null;
            const jumpFromIsDuplicate = isJump && prevCrumb && crumb.jumpFromId === prevCrumb.id;
            // Build full hierarchy path for tooltip
            const getHierarchyPath = (nodeId: string): string => {
              const path: string[] = [];
              let current = nodes.get(nodeId);
              while (current) {
                path.unshift(`${current.emoji || getNodeEmoji(current)} ${current.aiTitle || current.title}`);
                current = current.parentId ? nodes.get(current.parentId) : undefined;
              }
              return path.join(' → ');
            };
            const hierarchyTooltip = crumbNode ? getHierarchyPath(crumb.id) : crumb.title;

            return (
              <div key={`${crumb.id}-${index}`} className="flex items-center">
                {isJump ? (
                  // Show jump arrow (skip duplicate "from" if same as previous breadcrumb)
                  <>
                    {!jumpFromIsDuplicate && (
                      <>
                        <ChevronRight size={16} className="text-gray-600 mx-0.5" />
                        <button
                          onClick={() => {
                            // Navigate to the jumpFrom node
                            const jumpFromNode = crumb.jumpFromId ? nodes.get(crumb.jumpFromId) : null;
                            if (jumpFromNode) {
                              setActiveNode(null);
                              jumpToNode(jumpFromNode, undefined);
                            }
                          }}
                          className="text-gray-400 hover:text-gray-200 text-sm px-1 rounded hover:bg-gray-700 transition-colors"
                          title={`Jumped from: ${crumb.jumpFromTitle}`}
                        >
                          {(() => {
                            const jumpFromNode = crumb.jumpFromId ? nodes.get(crumb.jumpFromId) : null;
                            const jumpFromEmoji = jumpFromNode ? getNodeEmoji(jumpFromNode) : crumb.jumpFromEmoji;
                            return jumpFromEmoji && <span className="mr-1">{jumpFromEmoji}</span>;
                          })()}
                          {crumb.jumpFromTitle}
                        </button>
                      </>
                    )}
                    <span className="text-blue-400 mx-1 text-sm" title={`Jumped to: ${hierarchyTooltip}`}>⤳</span>
                  </>
                ) : (
                  <ChevronRight size={16} className="text-gray-600 mx-0.5" />
                )}
                {isLast ? (
                  // Current location: highlighted, not clickable
                  <span
                    className={`flex items-center gap-1 px-2 py-1 text-sm font-medium ${isJump ? 'text-blue-300' : 'text-amber-300'}`}
                    title={hierarchyTooltip}
                  >
                    {crumbEmoji && <span>{crumbEmoji}</span>}
                    {crumb.title}{toSuperscript(crumb.depth)}
                  </span>
                ) : (
                  // Ancestor: clickable
                  <button
                    onClick={() => {
                      setActiveNode(null); // Clear selection when navigating
                      const stepsBack = breadcrumbs.length - index - 1;
                      for (let i = 0; i < stepsBack; i++) {
                        navigateBack();
                      }
                      devLog('info', `Navigated to "${crumb.title}" (depth ${crumb.depth})`);
                    }}
                    className={`flex items-center gap-1 px-2 py-1 rounded transition-colors hover:bg-gray-700 cursor-pointer ${isJump ? 'text-blue-300' : 'text-gray-300'}`}
                  >
                    <span className="text-sm font-medium">
                      {crumbEmoji && <span className="mr-1">{crumbEmoji}</span>}
                      {crumb.title}{toSuperscript(crumb.depth)}
                    </span>
                  </button>
                )}
              </div>
            );
          })}
        </div>

        {/* Subtitle with current node's summary */}
        {breadcrumbs.length > 0 && (() => {
          const lastCrumb = breadcrumbs[breadcrumbs.length - 1];
          const lastNode = nodes.get(lastCrumb.id);
          const subtitle = lastNode?.summary || lastNode?.aiTitle;
          if (!subtitle) return null;
          return (
            <div className="mt-1 text-xs text-gray-500 truncate max-w-md">
              {subtitle.length > 60 ? subtitle.slice(0, 57) + '...' : subtitle}
            </div>
          );
        })()}
      </div>

      {hoveredNode && (
        <div
          className="absolute bottom-16 left-1/2 -translate-x-1/2 pointer-events-none bg-gray-800/95 text-white px-4 py-3 rounded-lg shadow-xl text-sm max-w-lg border border-gray-700 z-20"
        >
          <div className="font-semibold mb-2">{hoveredNode.displayEmoji} {hoveredNode.displayTitle}</div>
          {hoveredNode.displayContent && (
            <div className="text-gray-300 text-xs leading-relaxed whitespace-pre-wrap">
              {hoveredNode.displayContent.slice(0, 300)}
              {hoveredNode.displayContent.length > 300 && '...'}
            </div>
          )}
          {hoveredNode.tags && hoveredNode.tags.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-1">
              {hoveredNode.tags.map((tag, i) => (
                <span key={i} className="px-2 py-0.5 bg-gray-700 rounded text-xs text-gray-300">{tag}</span>
              ))}
            </div>
          )}
          <div className="mt-2 text-xs text-amber-400 text-right">
            {hoveredNode.type}
            {hoveredNode.isProcessed && <span className="ml-2 text-green-400">AI</span>}
            {hoveredNode.isItem && <span className="ml-2 text-blue-400">Item</span>}
          </div>
        </div>
      )}

      {/* Similar nodes panel - memoized for performance */}
      {showPanels && showDetails && (
        <SimilarNodesPanel
          similarNodesMap={similarNodesMap}
          nodes={nodes}
          currentParentId={currentParentId}
          stackNodes={stackNodes}
          detailsPanelSize={detailsPanelSize}
          isResizingDetails={isResizingDetails}
          pinnedIds={pinnedIds}
          getNodeEmoji={getNodeEmoji}
          onJumpToNode={jumpToNode}
          onFetchDetails={fetchSimilarNodes}
          onRemoveNode={(nodeId) => setSimilarNodesMap(prev => {
            const next = new Map(prev);
            next.delete(nodeId);
            return next;
          })}
          onTogglePin={handleTogglePin}
          onTogglePrivacy={handleTogglePrivacy}
          onSplitNode={handleSplitNode}
          onUnsplitNode={handleUnsplitNode}
          onClearAll={() => setSimilarNodesMap(new Map())}
          onToggleStack={() => setStackNodes(!stackNodes)}
          onStartResize={() => setIsResizingDetails(true)}
          devLog={devLog}
        />
      )}

      {/* Loading indicator for similar nodes */}
      <SimilarNodesLoading loadingSimilar={loadingSimilar} />

      {/* Status bar */}
      <div className="absolute bottom-4 left-4 bg-gray-800/90 backdrop-blur-sm rounded-lg px-4 py-2 text-sm text-gray-300 flex items-center gap-4">
        <span>{nodes.size} nodes</span>
        <span className="text-gray-500">|</span>
        <span>{edges.size} edges</span>
        <span className="text-gray-500">|</span>
        <span className="text-amber-400">Zoom: {(zoomLevel * 100).toFixed(0)}%</span>
      </div>

      {/* Color legend */}
      <div className="absolute bottom-16 right-4 bg-gray-800/90 backdrop-blur-sm rounded-lg px-3 py-2 text-xs text-gray-300">
        <div className="flex items-center gap-2 mb-1">
          <span className="text-gray-400">Age / Similarity:</span>
        </div>
        <div className="flex items-center gap-1">
          <span style={{ color: 'hsl(0, 75%, 65%)' }}>Old/Weak</span>
          <div
            className="h-4 w-24 rounded"
            style={{
              background: 'linear-gradient(to right, hsl(0, 75%, 65%), hsl(60, 75%, 65%), hsl(210, 75%, 65%), hsl(180, 75%, 65%))'
            }}
          />
          <span style={{ color: 'hsl(180, 75%, 65%)' }}>New/Strong</span>
        </div>
      </div>

      <div className="absolute bottom-4 right-4 bg-gray-800/80 backdrop-blur-sm rounded-lg px-3 py-2 text-xs text-gray-400">
        Scroll to zoom - Click and drag to pan
      </div>

      {/* AI Progress floating indicator */}
      {aiProgress.status !== 'idle' && (
        <div className="absolute bottom-16 right-4 bg-gray-900/95 backdrop-blur-sm rounded-lg border border-gray-700 px-4 py-3 z-40 min-w-48">
          <div className="flex items-center gap-3">
            {aiProgress.status === 'processing' && (
              <div className="w-4 h-4 border-2 border-purple-400 border-t-transparent rounded-full animate-spin" />
            )}
            {aiProgress.status === 'complete' && (
              <span className="text-green-400">✓</span>
            )}
            <div className="flex-1">
              <div className="text-sm text-white font-medium">
                {aiProgress.status === 'complete' ? 'Complete' : 'Processing AI'}
              </div>
              <div className="text-xs text-gray-400">
                {aiProgress.current}/{aiProgress.total} nodes
                {aiProgress.total > 0 && (
                  <span className="ml-2 text-amber-400">
                    {((aiProgress.current / aiProgress.total) * 100).toFixed(0)}%
                  </span>
                )}
              </div>
              {aiProgress.remainingSecs !== undefined && aiProgress.remainingSecs > 0 && (
                <div className="text-xs text-gray-500 mt-0.5">
                  ~{formatTime(aiProgress.remainingSecs)} remaining
                </div>
              )}
            </div>
            {aiProgress.status === 'processing' && (
              <button
                onClick={async () => {
                  // Clear queue first so the callback doesn't trigger rebuild
                  setRebuildQueued(false);
                  rebuildQueuedRef.current = false;
                  await invoke('cancel_processing');
                  setIsProcessing(false);
                  setAiProgress({ current: 0, total: 0, status: 'idle' });
                  console.log('[Cancel] AI processing cancelled (queue cleared)');
                  devLog('info', 'AI processing cancelled');
                }}
                className="text-gray-400 hover:text-red-400 transition-colors p-1"
                title="Cancel AI processing"
              >
                ✕
              </button>
            )}
          </div>
          {/* Progress bar */}
          {aiProgress.total > 0 && (
            <div className="mt-2 h-1.5 bg-gray-700 rounded-full overflow-hidden">
              <div
                className={`h-full transition-all duration-300 ${
                  aiProgress.status === 'complete' ? 'bg-green-500' : 'bg-purple-500'
                }`}
                style={{ width: `${(aiProgress.current / aiProgress.total) * 100}%` }}
              />
            </div>
          )}
        </div>
      )}

      {/* Hamburger Menu */}
      <div className="absolute top-4 right-4 z-40 flex items-center gap-2">
        {showPanels && (
          <div className="flex items-center gap-2 bg-gray-800/90 backdrop-blur-sm rounded-lg px-2 py-1">
            <button
              onClick={async () => {
                if (showDetails && similarNodesMap.size > 0) {
                  // Hide details
                  setShowDetails(false);
                } else {
                  // Show details - fetch most recent node from sidebar
                  try {
                    const recentNodes = await invoke<Node[]>('get_recent_nodes', { limit: 1 });
                    if (recentNodes.length > 0) {
                      const recentNode = recentNodes[0];
                      // Fetch similar nodes for the most recent node
                      fetchSimilarNodes(recentNode.id);
                      setShowDetails(true);
                    }
                  } catch (err) {
                    console.error('Failed to load recent node:', err);
                  }
                }
              }}
              className={`px-3 py-1.5 text-xs rounded transition-colors ${
                showDetails && similarNodesMap.size > 0
                  ? 'bg-amber-500/30 text-amber-300'
                  : 'text-gray-400 hover:text-white hover:bg-gray-700'
              }`}
            >
              Details
            </button>
            <button
              onClick={() => setShowDevConsole(!showDevConsole)}
              className={`px-3 py-1.5 text-xs rounded transition-colors ${
                showDevConsole
                  ? 'bg-amber-500/30 text-amber-300'
                  : 'text-gray-400 hover:text-white hover:bg-gray-700'
              }`}
            >
              Console
            </button>
            <button
              onClick={() => setHidePrivate(!hidePrivate)}
              className={`w-7 h-7 flex items-center justify-center rounded transition-colors ${
                hidePrivate
                  ? 'bg-rose-500/30 text-rose-400'
                  : 'text-gray-400 hover:text-white hover:bg-gray-700'
              }`}
              title={hidePrivate ? 'Show private nodes' : 'Hide private nodes'}
            >
              {hidePrivate ? <Lock className="w-4 h-4" /> : <LockOpen className="w-4 h-4" />}
            </button>
          </div>
        )}
        <button
          onClick={() => setShowPanels(!showPanels)}
          className="bg-gray-800/90 hover:bg-gray-700/90 backdrop-blur-sm rounded-lg p-2 text-gray-400 hover:text-white transition-colors"
          title={showPanels ? 'Hide menu' : 'Show menu'}
        >
          <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            {showPanels ? (
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            ) : (
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
            )}
          </svg>
        </button>
      </div>

      {/* Node context menu */}
      {nodeMenuId && nodeMenuPos && (
        <>
          <div
            className="fixed inset-0 z-[55]"
            onClick={() => setNodeMenuId(null)}
          />
          <div
            className="fixed z-[56] bg-gray-800 border border-gray-700 rounded-lg shadow-xl py-1 min-w-[120px]"
            style={{ left: nodeMenuPos.x, top: nodeMenuPos.y }}
          >
            <button
              onClick={() => {
                setNodeToDelete(nodeMenuId);
                setNodeMenuId(null);
                setConfirmAction('deleteNode');
              }}
              className="w-full px-4 py-2 text-left text-sm text-red-400 hover:bg-gray-700 flex items-center gap-2"
            >
              <span>🗑️</span>
              <span>Delete</span>
            </button>
          </div>
        </>
      )}

      {/* Add Note Button - Bottom Center */}
      <button
        onClick={() => setShowNoteModal(true)}
        className="absolute bottom-6 left-1/2 -translate-x-1/2 bg-gray-800/90 hover:bg-gray-700/90 backdrop-blur-sm rounded-full p-3 text-gray-400 hover:text-white transition-all hover:scale-110 shadow-lg"
        title="Add a note"
      >
        <span className="text-xl">📝</span>
      </button>

      {/* Dev Console */}
      {showPanels && showDevConsole && (
        <div
          className={`absolute bg-gray-900/95 backdrop-blur-sm rounded-lg border border-gray-700 overflow-hidden flex flex-col ${
            showDetails && similarNodesMap.size > 0
              ? 'bottom-4 right-4'
              : 'top-14 right-4'
          }`}
          style={{ width: consoleSize.width, height: consoleSize.height }}
        >
          <div className="px-3 py-2 bg-gray-800/80 border-b border-gray-700 flex items-center justify-between flex-shrink-0">
            <span className="text-xs font-semibold text-gray-300">Dev Console</span>
            <div className="flex items-center gap-3 text-xs">
              <span className="text-gray-500">Nodes: {nodes.size}</span>
              <span className="text-gray-500">Edges: {edges.size}</span>
              <span className="text-amber-400">{(zoomLevel * 100).toFixed(0)}%</span>
            </div>
          </div>
          <div ref={consoleRef} className="flex-1 overflow-y-auto p-2 font-mono text-xs space-y-0.5 min-h-0">
            {devLogs.length === 0 ? (
              <div className="text-gray-600 text-center py-4">No logs yet</div>
            ) : (
              devLogs.map((log, i) => (
                <div key={i} className="flex gap-2">
                  <span className="text-gray-600 flex-shrink-0">{log.time}</span>
                  <span className={logColors[log.type]}>[{log.type.toUpperCase()}]</span>
                  <span className="text-gray-300 break-all">{log.message}</span>
                </div>
              ))
            )}
          </div>
          <div className="px-3 py-2 bg-gray-800/50 border-t border-gray-700 flex items-center justify-between text-xs flex-shrink-0">
            <span className="text-gray-500">{devLogs.length} logs</span>
            <div className="flex items-center gap-3">
              <button
                onClick={listCurrentNodes}
                className="text-blue-400 hover:text-blue-300 transition-colors"
                title="List all nodes in current view"
              >
                Nodes
              </button>
              <button
                onClick={listCurrentPath}
                className="text-green-400 hover:text-green-300 transition-colors"
                title="Show current view's hierarchy path"
              >
                Path
              </button>
              <button
                onClick={listHierarchy}
                className="text-purple-400 hover:text-purple-300 transition-colors"
                title="List full hierarchy tree (up to 4 levels)"
              >
                Tree
              </button>
              <button
                onClick={() => setAutoScroll(!autoScroll)}
                className={`transition-colors ${autoScroll ? 'text-green-400' : 'text-gray-500 hover:text-gray-300'}`}
                title={autoScroll ? 'Auto-scroll ON' : 'Auto-scroll OFF'}
              >
                {autoScroll ? 'Auto' : 'Off'}
              </button>
              <button
                onClick={() => setDevLogs([])}
                className="text-gray-500 hover:text-red-400 transition-colors"
              >
                Clear
              </button>
            </div>
          </div>
          {/* Resize handle - top-left when at bottom, bottom-left when at top */}
          <div
            onMouseDown={handleResizeStart}
            className={`absolute w-4 h-4 group ${isResizing ? 'bg-amber-500/30' : ''} ${
              consoleAtBottom
                ? 'top-0 left-0 cursor-nw-resize'
                : 'bottom-0 left-0 cursor-sw-resize'
            }`}
            title="Drag to resize"
          >
            <svg
              className={`w-3 h-3 absolute text-gray-500 group-hover:text-amber-400 transition-colors ${
                consoleAtBottom
                  ? 'top-0.5 left-0.5 rotate-90'
                  : 'bottom-0.5 left-0.5'
              }`}
              viewBox="0 0 12 12"
              fill="currentColor"
            >
              <path d="M0 12L12 0v3L3 12H0zm0-5l7-7v3L3 10v2H0V7z" />
            </svg>
          </div>
        </div>
      )}

      {/* Confirmation Dialog */}
      {confirmAction && (
        <div className="fixed inset-0 z-[60] flex items-center justify-center">
          <div
            className="absolute inset-0 bg-black/60 backdrop-blur-sm"
            onClick={() => { setConfirmAction(null); setNodeToDelete(null); }}
          />
          <div className="relative bg-gray-800 rounded-lg border border-gray-700 shadow-xl max-w-md w-full mx-4 p-6">
            <button
              onClick={() => { setConfirmAction(null); setNodeToDelete(null); }}
              className="absolute top-4 right-4 p-1 text-gray-400 hover:text-white rounded transition-colors"
            >
              <X className="w-5 h-5" />
            </button>
            <div className="flex items-start gap-4 mb-4">
              <div className={`p-2 rounded-full bg-gray-700/50 ${
                confirmConfigs[confirmAction].variant === 'danger' ? 'text-red-400' :
                confirmConfigs[confirmAction].variant === 'warning' ? 'text-amber-400' : 'text-blue-400'
              }`}>
                <AlertTriangle className="w-6 h-6" />
              </div>
              <div>
                <h3 className="text-lg font-semibold text-white">{confirmConfigs[confirmAction].title}</h3>
                <p className="mt-2 text-sm text-gray-300">{confirmConfigs[confirmAction].message}</p>
              </div>
            </div>
            <div className="flex justify-end gap-3 mt-6">
              <button
                onClick={() => { setConfirmAction(null); setNodeToDelete(null); }}
                className="px-4 py-2 text-sm font-medium text-gray-300 bg-gray-700 hover:bg-gray-600 rounded-lg transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleConfirmAction}
                className={`px-4 py-2 text-sm font-medium rounded-lg transition-colors ${
                  confirmConfigs[confirmAction].variant === 'danger' ? 'bg-red-600 hover:bg-red-700' :
                  confirmConfigs[confirmAction].variant === 'warning' ? 'bg-amber-600 hover:bg-amber-700' : 'bg-blue-600 hover:bg-blue-700'
                } text-white`}
              >
                {confirmConfigs[confirmAction].confirmText}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Note Modal */}
      {showNoteModal && (
        <div className="fixed inset-0 z-[60] flex items-center justify-center">
          <div
            className="absolute inset-0 bg-black/60 backdrop-blur-sm"
            onClick={() => setShowNoteModal(false)}
          />
          <div className="relative bg-gray-800 rounded-lg border border-gray-700 shadow-xl max-w-lg w-full mx-4 p-6">
            <h2 className="text-lg font-medium text-white mb-4">Add Note</h2>
            <input
              type="text"
              placeholder="Title (optional)"
              value={noteTitle}
              onChange={(e) => setNoteTitle(e.target.value)}
              className="w-full bg-gray-700 text-white rounded px-3 py-2 mb-3 border border-gray-600 focus:border-amber-500 focus:outline-none"
            />
            <textarea
              placeholder="What's on your mind?"
              value={noteContent}
              onChange={(e) => setNoteContent(e.target.value)}
              rows={6}
              className="w-full bg-gray-700 text-white rounded px-3 py-2 mb-4 resize-none border border-gray-600 focus:border-amber-500 focus:outline-none"
              autoFocus
            />
            <div className="flex gap-2 justify-end">
              <button
                onClick={() => {
                  setShowNoteModal(false);
                  setNoteTitle('');
                  setNoteContent('');
                }}
                className="px-4 py-2 text-gray-400 hover:text-white"
              >
                Cancel
              </button>
              <button
                onClick={async () => {
                  if (noteContent.trim()) {
                    const title = noteTitle.trim() || 'Untitled Note';
                    const content = noteContent.trim();
                    const containerId = 'container-recent-notes';

                    // Save to database and get the ID
                    const noteId = await invoke<string>('add_note', { title, content });

                    const now = Date.now();

                    // Check if Recent Notes container exists in local state
                    let container = nodes.get(containerId);
                    if (!container) {
                      // Container was just created in backend - add to local state
                      const universe = Array.from(nodes.values()).find(n => n.isUniverse);
                      addNode({
                        id: containerId,
                        type: 'cluster',
                        title: 'Recent Notes',
                        emoji: '📝',
                        depth: 1,
                        isItem: false,
                        isUniverse: false,
                        parentId: universe?.id,
                        childCount: 1,
                        position: { x: 0, y: 0 },
                        createdAt: now,
                        updatedAt: now,
                        latestChildDate: now,
                        isProcessed: true,
                        isPinned: false,
                      });
                    } else {
                      // Update existing container's childCount and latestChildDate
                      updateNode(containerId, {
                        childCount: container.childCount + 1,
                        latestChildDate: now,
                      });
                    }

                    // Add note to local state
                    addNode({
                      id: noteId,
                      type: 'thought',
                      title,
                      content,
                      depth: 2,
                      isItem: true,
                      isUniverse: false,
                      parentId: containerId,
                      childCount: 0,
                      position: { x: 0, y: 0 },
                      createdAt: now,
                      updatedAt: now,
                      isProcessed: false,
                      isPinned: false,
                    });

                    setShowNoteModal(false);
                    setNoteTitle('');
                    setNoteContent('');
                  }
                }}
                disabled={!noteContent.trim()}
                className="px-4 py-2 bg-amber-600 hover:bg-amber-500 disabled:opacity-50 disabled:cursor-not-allowed text-white rounded"
              >
                Save
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
