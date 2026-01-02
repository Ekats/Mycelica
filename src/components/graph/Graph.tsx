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
import { ComponentErrorBoundary } from '../ErrorBoundary';
import { DevConsole, DevConsoleLog } from './DevConsole';
import { GraphStatusBar } from './GraphStatusBar';
import { NoteModal } from './NoteModal';
import { NodeContextMenu } from './NodeContextMenu';
import { GraphCanvas, GraphNode, EdgeData } from './GraphCanvas';

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

// Strip JATS XML tags from text (used in scientific paper abstracts)
const stripJatsTags = (text: string): string => {
  return text.replace(/<\/?jats:[^>]+>/g, '');
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

// GraphNode interface moved to GraphCanvas.tsx

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
  const consoleRef = useRef<HTMLDivElement>(null);
  const stackNodesRef = useRef(false);
  // Refs for batched logging (prevents re-render loops when devLog called in render path)
  const logQueueRef = useRef<Array<{time: string, type: 'info' | 'warn' | 'error', message: string}>>([]);
  const flushScheduledRef = useRef(false);
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
    // Privacy filtering from store
    hidePrivate,
    setHidePrivate,
    privacyThreshold,
  } = useGraphStore();
  const [zoomLevel, setZoomLevel] = useState(1);
  const [devLogs, setDevLogs] = useState<DevConsoleLog[]>([]);
  const [showDevConsole, setShowDevConsole] = useState(false);
  const [showPanels, setShowPanels] = useState(true); // Hamburger menu toggle for all panels
  const [showDetails, setShowDetails] = useState(true); // Toggle for details panel
  const [showNoteModal, setShowNoteModal] = useState(false);
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
    elapsedSecs?: number;
    stepName?: string;      // e.g., "Step 3/7: Building hierarchy"
    stepDetail?: string;    // e.g., "Organizing Mycelica into categories..."
    status: 'idle' | 'processing' | 'complete';
  }>({ current: 0, total: 0, status: 'idle' });
  const progressStartRef = useRef<number | null>(null);

  // Tick elapsed time every second while processing
  useEffect(() => {
    if (aiProgress.status !== 'processing') {
      progressStartRef.current = null;
      return;
    }

    // Set start time on first processing event
    if (progressStartRef.current === null) {
      progressStartRef.current = Date.now();
    }

    const interval = setInterval(() => {
      if (progressStartRef.current !== null) {
        const elapsed = (Date.now() - progressStartRef.current) / 1000;
        setAiProgress(prev => prev.status === 'processing' ? { ...prev, elapsedSecs: elapsed } : prev);
      }
    }, 1000);

    return () => clearInterval(interval);
  }, [aiProgress.status]);

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

    return connectionMap;
  }, [activeNodeId, edges]);

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

  // Handle splitting a node - destroy it and move its children up to parent level
  const handleSplitNode = useCallback(async (nodeId: string) => {
    const node = nodes.get(nodeId);
    const nodeName = node?.aiTitle || node?.title || nodeId.slice(0, 8);
    try {
      devLog('info', `Splitting "${nodeName}" - moving children up to parent level...`);
      const flattened = await invoke<number>('unsplit_node', { parentId: nodeId });
      devLog('info', `Split complete! Moved ${flattened} nodes up. Refreshing...`);
      // Clear the panel and refresh the graph data
      setSimilarNodesMap(new Map());
      if (onDataChanged) {
        await onDataChanged();
        devLog('info', `Graph refreshed.`);
      }
    } catch (err) {
      console.error('Failed to split node:', err);
      devLog('error', `Failed to split: ${err}`);
    }
  }, [nodes, devLog, onDataChanged]);

  // Handle grouping a node's children into sub-categories (max 5 groups)
  const handleGroupNode = useCallback(async (nodeId: string) => {
    const node = nodes.get(nodeId);
    const nodeName = node?.aiTitle || node?.title || nodeId.slice(0, 8);
    try {
      devLog('info', `Grouping children of "${nodeName}" into categories...`);
      await invoke('cluster_hierarchy_level', { parentId: nodeId, maxGroups: 5 });
      devLog('info', `Group complete! Refreshing graph...`);
      // Clear the panel and refresh the graph data
      setSimilarNodesMap(new Map());
      if (onDataChanged) {
        await onDataChanged();
        devLog('info', `Graph refreshed. Navigate into "${nodeName}" to see new structure.`);
      }
    } catch (err) {
      console.error('Failed to group node:', err);
      devLog('error', `Failed to group: ${err}`);
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

      // Update floating progress indicator with step info
      if (status === 'processing' || status === 'success') {
        setAiProgress({
          current,
          total,
          remainingSecs,
          elapsedSecs,
          stepName: nodeTitle,      // Use nodeTitle as step name (e.g., "Step 3/7: Building hierarchy")
          stepDetail: newTitle,     // Use newTitle as detail (e.g., "Creating Universe and topics...")
          status: 'processing'
        });
      } else if (status === 'complete') {
        setAiProgress({
          current: total,
          total,
          elapsedSecs,
          stepName: nodeTitle || 'Complete',
          stepDetail: newTitle,
          status: 'complete'
        });
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
    const maxDisplayDepth = 4; // Cut off after this many levels from universe
    const maxItemsPerLevel = 6; // Max items to show at each level
    devLog('info', `=== HIERARCHY TREE (max ${maxDisplayDepth} levels, ${maxItemsPerLevel} items/level) ===`);

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

        const children = await invoke<Node[]>('get_graph_children', { parentId: nodeId });
        const sortedChildren = [...children].sort((a, b) => (b.childCount || 0) - (a.childCount || 0));
        const displayChildren = sortedChildren.slice(0, maxItemsPerLevel);
        const hiddenCount = sortedChildren.length - displayChildren.length;

        for (let i = 0; i < displayChildren.length; i++) {
          const child = displayChildren[i];
          const isLastChild = i === displayChildren.length - 1 && hiddenCount === 0;
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

        // Show hidden siblings indicator
        if (hiddenCount > 0) {
          devLog('info', `${prefix}└── ... (+${hiddenCount} more siblings)`);
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
        const children = await invoke<Node[]>('get_graph_children', { parentId: currentViewParentId });
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

  // D3 rendering effects moved to GraphCanvas component
  // Main render effect and color update effect are now in GraphCanvas

  // Build breadcrumb display: Universe + navigation path + current level
  const currentLevelInfo = getLevelName(currentDepth, maxDepth);

  // ==========================================================================
  // LAYOUT COMPUTATION - Compute positioned nodes and edges for GraphCanvas
  // ==========================================================================
  const { graphNodesComputed, edgeDataComputed } = useMemo(() => {
    if (nodes.size === 0) return { graphNodesComputed: [], edgeDataComputed: [] };

    const allNodes = Array.from(nodes.values());
    const universeNode = allNodes.find(n => n.isUniverse);

    // Privacy filter helper
    const passesPrivacy = (n: Node): boolean => {
      if (n.privacy !== undefined) return n.privacy >= privacyThreshold;
      return n.isPrivate !== true;
    };

    // Recursive check: does this node have any visible descendants?
    const hasVisibleDescendants = (parentId: string): boolean => {
      const children = allNodes.filter(n => n.parentId === parentId);
      if (children.length === 0) return false; // No children = no visible descendants
      return children.some(child => {
        if (child.isItem) {
          // Items must pass privacy themselves
          return passesPrivacy(child);
        } else {
          // Categories: recurse without checking category's own privacy
          // (category privacy is derived from children, so we check the actual items)
          return hasVisibleDescendants(child.id);
        }
      });
    };

    // Filter nodes for current view
    const nodeArray = allNodes.filter(node => {
      if (hidePrivate) {
        if (node.isItem) {
          // Items must pass privacy themselves
          if (!passesPrivacy(node)) return false;
        } else {
          // Categories: visible if they have visible descendants
          // (don't check category's own privacy - it's derived from children's min)
          if (!hasVisibleDescendants(node.id)) return false;
        }
      }
      if (currentParentId) return node.parentId === currentParentId;
      if (currentDepth === 0 && universeNode) return node.parentId === universeNode.id;
      return node.depth === currentDepth && !node.parentId;
    });

    if (nodeArray.length === 0) return { graphNodesComputed: [], edgeDataComputed: [] };

    // Constants
    const noteWidth = 320, noteHeight = 320, nodeSpacing = 300;
    const ellipseAspect = Math.min((width / height) * 0.9, 2.0);
    const goldenAngle = Math.PI * (3 - Math.sqrt(5));
    const horizontalBandHalfAngle = Math.PI / 4.5;
    const restrictionStartRing = 2;

    // Ring layout helpers
    const getNodesPerRing = (ring: number): number => ring === 0 ? 1 : 4;
    const getRingRadius = (ring: number): number => {
      if (ring === 0) return 0;
      if (ring === 1) return (noteHeight + nodeSpacing * 1.5) * 1.1;
      return (noteHeight + nodeSpacing * 1.5) * (1.1 + (ring - 1) * 0.9);
    };

    // Sort by importance
    const sortedNodes = [...nodeArray].sort((a, b) => {
      if (a.id === 'container-recent-notes') return -1;
      if (b.id === 'container-recent-notes') return 1;
      return (b.childCount || 0) - (a.childCount || 0);
    });

    // Position nodes in rings
    const graphNodes: GraphNode[] = [];
    let ringIndex = 0, nodeIndex = 0;

    while (nodeIndex < sortedNodes.length) {
      const ringCapacity = getNodesPerRing(ringIndex);
      const nodesInThisRing = Math.min(ringCapacity, sortedNodes.length - nodeIndex);
      const ringRadius = getRingRadius(ringIndex);
      const ringStartAngle = ringIndex * goldenAngle;

      for (let i = 0; i < nodesInThisRing; i++) {
        const node = sortedNodes[nodeIndex];
        let angle: number;
        if (ringIndex < restrictionStartRing) {
          angle = ringStartAngle + (i / nodesInThisRing) * 2 * Math.PI;
        } else {
          const progress = i / nodesInThisRing;
          if (progress < 0.5) {
            angle = -horizontalBandHalfAngle + (progress * 2) * (2 * horizontalBandHalfAngle);
          } else {
            angle = Math.PI - horizontalBandHalfAngle + ((progress - 0.5) * 2) * (2 * horizontalBandHalfAngle);
          }
          angle += (ringIndex % 4) * 0.1;
        }

        const verticalStretch = ringIndex === 1 ? 1.3 : 1.0;
        const x = ringRadius * ellipseAspect * Math.cos(angle);
        const y = ringRadius * verticalStretch * Math.sin(angle);
        const colorId = node.clusterId ?? hashString(node.id);

        graphNodes.push({
          ...node,
          x, y,
          renderClusterId: colorId,
          displayTitle: stripJatsTags(node.aiTitle || node.title || 'Untitled'),
          displayContent: stripJatsTags(node.summary || node.content || ''),
          displayEmoji: getNodeEmoji(node),
        });
        nodeIndex++;
      }
      ringIndex++;
    }

    // Force simulation for connected nodes
    const semanticEdges: Array<{source: string, target: string, weight: number}> = [];
    edges.forEach(edge => {
      if (edge.type === 'related' && edge.weight !== undefined) {
        const sourceInView = graphNodes.some(n => n.id === edge.source);
        const targetInView = graphNodes.some(n => n.id === edge.target);
        if (sourceInView && targetInView) {
          semanticEdges.push({ source: edge.source, target: edge.target, weight: edge.weight });
        }
      }
    });

    const connectedNodeIds = new Set<string>();
    semanticEdges.forEach(e => { connectedNodeIds.add(e.source); connectedNodeIds.add(e.target); });

    if (graphNodes.length >= 2 && semanticEdges.length > 0) {
      const simNodes = graphNodes.map(n => ({
        id: n.id, x: n.x, y: n.y,
        isConnected: connectedNodeIds.has(n.id), origX: n.x, origY: n.y,
      }));
      const simLinks = semanticEdges.map(e => ({
        source: e.source, target: e.target, strength: e.weight * 0.5
      }));

      const simulation = d3.forceSimulation(simNodes)
        .force('link', d3.forceLink(simLinks)
          .id((d: any) => d.id).distance(noteWidth * 2.5).strength((d: any) => d.strength * 0.4))
        .force('charge', d3.forceManyBody().strength(-noteWidth * 6).distanceMax(noteWidth * 15))
        .force('anchorX', d3.forceX((d: any) => d.origX).strength((d: any) => d.isConnected ? 0.01 : 0.3))
        .force('anchorY', d3.forceY((d: any) => d.origY).strength((d: any) => d.isConnected ? 0.01 : 0.3))
        .force('collide', d3.forceCollide().radius(Math.max(noteWidth, noteHeight) * 1.5).strength(1.0).iterations(3))
        .stop();

      const numTicks = Math.min(300, 50 + graphNodes.length * 2);
      for (let i = 0; i < numTicks; i++) simulation.tick();

      const posMap = new Map(simNodes.map(n => [n.id, { x: n.x, y: n.y }]));
      graphNodes.forEach(node => {
        const newPos = posMap.get(node.id);
        if (newPos) { node.x = newPos.x; node.y = newPos.y; }
      });
    }

    // Build edge data
    const nodeMap = new Map(graphNodes.map(n => [n.id, n]));
    const edgeData: EdgeData[] = [];
    edges.forEach(edge => {
      const source = nodeMap.get(edge.source);
      const target = nodeMap.get(edge.target);
      if (source && target) {
        edgeData.push({ source, target, type: edge.type, weight: edge.weight ?? 0.5 });
      }
    });

    return { graphNodesComputed: graphNodes, edgeDataComputed: edgeData };
  }, [nodes, edges, width, height, currentDepth, currentParentId, hidePrivate, privacyThreshold, getNodeEmoji]);

  // Callback for showing context menu
  const handleShowContextMenu = useCallback((nodeId: string, pos: { x: number; y: number }) => {
    setNodeMenuId(nodeId);
    setNodeMenuPos(pos);
  }, []);

  // Memoized callbacks for GraphCanvas to prevent unnecessary re-renders
  const handleSelectNode = useCallback((id: string | null) => {
    setActiveNode(id);
  }, [setActiveNode]);

  const handleZoomChange = useCallback((k: number) => {
    setZoomLevel(k);
  }, []);

  const handleNavigateToNode = useCallback((node: Node) => {
    navigateToNode(node);
  }, [navigateToNode]);

  const handleOpenLeaf = useCallback((id: string, initialView?: 'abstract' | 'pdf') => {
    openLeaf(id, initialView);
  }, [openLeaf]);

  return (
    <div className="relative w-full h-full">
      {/* GraphCanvas renders the D3 graph (currently empty, D3 logic to be moved) */}
      <GraphCanvas
        graphNodes={graphNodesComputed}
        edgeData={edgeDataComputed}
        activeNodeId={activeNodeId}
        connectionMap={memoizedConnectionMap}
        width={width}
        height={height}
        onSelectNode={handleSelectNode}
        onNavigateToNode={handleNavigateToNode}
        onOpenLeaf={handleOpenLeaf}
        onFetchSimilarNodes={fetchSimilarNodes}
        onShowContextMenu={handleShowContextMenu}
        onZoomChange={handleZoomChange}
        devLog={devLog}
        getNodeEmoji={getNodeEmoji}
        hidePrivate={hidePrivate}
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

      {/* Similar nodes panel - memoized for performance */}
      {showPanels && showDetails && (
        <ComponentErrorBoundary fallbackTitle="Similar Nodes">
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
            onGroupNode={handleGroupNode}
            onClearAll={() => setSimilarNodesMap(new Map())}
            onToggleStack={() => setStackNodes(!stackNodes)}
            onStartResize={() => setIsResizingDetails(true)}
            devLog={devLog}
          />
        </ComponentErrorBoundary>
      )}

      {/* Loading indicator for similar nodes */}
      <SimilarNodesLoading loadingSimilar={loadingSimilar} />

      {/* Status bar, color legend, and help text */}
      <GraphStatusBar
        nodeCount={nodes.size}
        edgeCount={edges.size}
        zoomLevel={zoomLevel}
      />

      {/* AI Progress floating indicator */}
      {aiProgress.status !== 'idle' && (
        <div className="absolute bottom-16 right-4 bg-gray-900/95 backdrop-blur-sm rounded-lg border border-gray-700 px-4 py-3 z-40 min-w-56">
          <div className="flex items-center gap-3">
            {aiProgress.status === 'processing' && (
              <div className="w-4 h-4 border-2 border-purple-400 border-t-transparent rounded-full animate-spin" />
            )}
            {aiProgress.status === 'complete' && (
              <span className="text-green-400 text-lg">✓</span>
            )}
            <div className="flex-1 min-w-0">
              {/* Step name (e.g., "Step 3/7: Building hierarchy") */}
              <div className="text-sm text-white font-medium truncate">
                {aiProgress.stepName || (aiProgress.status === 'complete' ? 'Complete' : 'Processing')}
              </div>
              {/* Step detail (e.g., "Creating Universe and topics...") */}
              {aiProgress.stepDetail && (
                <div className="text-xs text-gray-400 truncate">
                  {aiProgress.stepDetail}
                </div>
              )}
              {/* Elapsed time */}
              {aiProgress.elapsedSecs !== undefined && (
                <div className="text-xs text-cyan-400 mt-0.5">
                  Elapsed: {formatTime(aiProgress.elapsedSecs)}
                </div>
              )}
              {/* Remaining time estimate */}
              {aiProgress.remainingSecs !== undefined && aiProgress.remainingSecs > 0 && (
                <div className="text-xs text-gray-500">
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
      <NodeContextMenu
        nodeId={nodeMenuId}
        position={nodeMenuPos}
        onClose={() => setNodeMenuId(null)}
        onDelete={(id) => {
          setNodeToDelete(id);
          setConfirmAction('deleteNode');
        }}
      />

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
        <DevConsole
          logs={devLogs}
          nodeCount={nodes.size}
          edgeCount={edges.size}
          zoomLevel={zoomLevel}
          autoScroll={autoScroll}
          setAutoScroll={setAutoScroll}
          consoleSize={consoleSize}
          consoleRef={consoleRef}
          onClear={() => setDevLogs([])}
          onListNodes={listCurrentNodes}
          onListPath={listCurrentPath}
          onListHierarchy={listHierarchy}
          onLog={(log) => setDevLogs(prev => [...prev, log])}
          isResizing={isResizing}
          onResizeStart={handleResizeStart}
          positionAtBottom={consoleAtBottom}
        />
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
      <NoteModal
        isOpen={showNoteModal}
        onClose={() => setShowNoteModal(false)}
        nodes={nodes}
        addNode={addNode}
        updateNode={updateNode}
      />
    </div>
  );
}
