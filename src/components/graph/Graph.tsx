import { useRef, useEffect, useState, useCallback } from 'react';
import * as d3 from 'd3';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useGraphStore } from '../../stores/graphStore';
import type { Node } from '../../types/graph';
import { getEmojiForNode, initLearnedMappings } from '../../utils/emojiMatcher';
import { ChevronRight, AlertTriangle, X } from 'lucide-react';

// Confirmation dialog types
type ConfirmAction = 'processAi' | 'cluster' | 'buildHierarchy' | null;

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

export function Graph({ width, height }: GraphProps) {
  const svgRef = useRef<SVGSVGElement>(null);
  const consoleRef = useRef<HTMLDivElement>(null);
  const stackNodesRef = useRef(false);
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
  } = useGraphStore();
  const [hoveredNode, setHoveredNode] = useState<GraphNode | null>(null);
  const [zoomLevel, setZoomLevel] = useState(1);
  const [devLogs, setDevLogs] = useState<DevConsoleLog[]>([]);
  const [showDevConsole, setShowDevConsole] = useState(true);
  const [showPanels, setShowPanels] = useState(true); // Hamburger menu toggle for all panels
  const [showDetails, setShowDetails] = useState(true); // Toggle for details panel
  const [isClustering, setIsClustering] = useState(false);
  const [isProcessing, setIsProcessing] = useState(false);
  const [isBuildingHierarchy, setIsBuildingHierarchy] = useState(false);
  const [rebuildQueued, setRebuildQueued] = useState(false);
  const rebuildQueuedRef = useRef(false);
  const [cancelRequested, setCancelRequested] = useState<'ai' | 'rebuild' | null>(null);
  const fullRebuildRef = useRef<() => Promise<void>>();
  const processingStartTimeRef = useRef<number>(0);
  const rebuildStartTimeRef = useRef<number>(0);
  const [autoScroll, setAutoScroll] = useState(true);
  const [hierarchyBuilt, setHierarchyBuilt] = useState(false);
  const [consoleSize, setConsoleSize] = useState({ width: 450, height: 400 }); // Larger for hierarchy output
  const [isResizing, setIsResizing] = useState(false);

  // Similar nodes state
  const [similarNodesMap, setSimilarNodesMap] = useState<Map<string, SimilarNode[]>>(new Map());
  const [loadingSimilar, setLoadingSimilar] = useState<Set<string>>(new Set());
  const [expandedSimilar, setExpandedSimilar] = useState<Set<string>>(new Set());
  const [collapsedSimilar, setCollapsedSimilar] = useState<Set<string>>(new Set());
  const [stackNodes, setStackNodesState] = useState(false); // Toggle to stack multiple node panels
  const [confirmAction, setConfirmAction] = useState<ConfirmAction>(null); // Confirmation dialog state
  const setStackNodes = (value: boolean) => {
    stackNodesRef.current = value;
    setStackNodesState(value);
  };
  const SIMILAR_INITIAL_COUNT = 10;
  const [detailsPanelSize, setDetailsPanelSize] = useState({ width: 400, height: 400 });
  const [isResizingDetails, setIsResizingDetails] = useState(false);
  const [zenModeNodeId, setZenModeNodeId] = useState<string | null>(null); // Node ID for zen mode focus
  const zenModeNodeIdRef = useRef<string | null>(null);
  const setZenMode = (nodeId: string | null) => {
    zenModeNodeIdRef.current = nodeId;
    setZenModeNodeId(nodeId);
  };

  // AI Progress indicator state
  const [aiProgress, setAiProgress] = useState<{
    current: number;
    total: number;
    remainingSecs?: number;
    status: 'idle' | 'processing' | 'complete';
  }>({ current: 0, total: 0, status: 'idle' });

  // Confirmation dialog configs
  const confirmConfigs: Record<NonNullable<ConfirmAction>, ConfirmConfig> = {
    processAi: {
      title: 'Process AI',
      message: 'This will use AI to generate titles, summaries, and embeddings for all unprocessed items. This can take a long time for large databases and uses API credits.',
      confirmText: 'Start Processing',
      variant: 'warning',
    },
    cluster: {
      title: 'Cluster Items',
      message: 'This will assign all items to topic clusters using AI. Existing cluster assignments will be replaced. This uses API credits.',
      confirmText: 'Run Clustering',
      variant: 'warning',
    },
    buildHierarchy: {
      title: 'Build Hierarchy',
      message: 'This will rebuild the navigation structure (Universe → Topics → Items). All intermediate levels will be recreated. This uses API credits for grouping.',
      confirmText: 'Build Hierarchy',
      variant: 'warning',
    },
  };

  const handleConfirmAction = useCallback(async () => {
    const action = confirmAction;
    setConfirmAction(null);

    if (action === 'processAi') {
      runProcessing();
    } else if (action === 'cluster') {
      runClustering();
    } else if (action === 'buildHierarchy') {
      buildHierarchy();
    }
  }, [confirmAction]);

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
    setDevLogs(prev => [...prev.slice(-2000), { time, type, message }]);
    if (type === 'error') console.error(`[Graph] ${message}`);
    else if (type === 'warn') console.warn(`[Graph] ${message}`);
    else console.log(`[Graph] ${message}`);
  }, []);

  // Fetch similar nodes for a node
  const fetchSimilarNodes = useCallback(async (nodeId: string) => {
    const isStacking = stackNodesRef.current;

    // Toggle off if already showing
    if (similarNodesMap.has(nodeId)) {
      if (isStacking) {
        setSimilarNodesMap(prev => {
          const next = new Map(prev);
          next.delete(nodeId);
          return next;
        });
      } else {
        setSimilarNodesMap(new Map());
      }
      return;
    }

    // If stacking enabled, add to existing; otherwise clear and show only this one
    if (!isStacking) {
      setSimilarNodesMap(new Map());
    }

    setLoadingSimilar(prev => new Set(prev).add(nodeId));
    try {
      const similar = await invoke<SimilarNode[]>('get_similar_nodes', { nodeId, topN: 1000, minSimilarity: 0.25 });
      if (isStacking) {
        setSimilarNodesMap(prev => new Map(prev).set(nodeId, similar));
      } else {
        setSimilarNodesMap(new Map([[nodeId, similar]]));
      }
      devLog('info', `Found ${similar.length} similar nodes for ${nodeId}`);
    } catch (err) {
      devLog('warn', `No similar nodes: ${err}`);
    } finally {
      setLoadingSimilar(prev => {
        const next = new Set(prev);
        next.delete(nodeId);
        return next;
      });
    }
  }, [similarNodesMap, devLog]);

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
  const [isFullRebuilding, setIsFullRebuilding] = useState(false);
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

  useEffect(() => {
    if (!svgRef.current || nodes.size === 0) return;

    const svg = d3.select(svgRef.current);
    svg.selectAll('*').remove();
    svg.attr('width', width).attr('height', height);

    const container = svg.append('g').attr('class', 'graph-container');

    // Filter nodes by current depth and parent
    const allNodes = Array.from(nodes.values());
    const universeNode = allNodes.find(n => n.isUniverse);

    const nodeArray = allNodes.filter(node => {
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

    // Ring capacity: Ring 0 = 1, Ring 1 = 4, Ring N = 6 * N
    const getNodesPerRing = (ring: number): number => {
      if (ring === 0) return 1;
      if (ring === 1) return 4;
      return 6 * ring;
    };

    // Ring radius: based on noteHeight + spacing
    const getRingRadius = (ring: number): number => {
      return ring * (noteHeight + nodeSpacing * 1.5);
    };

    // Golden angle for ring offset (prevents spoke alignment)
    const goldenAngle = Math.PI * (3 - Math.sqrt(5)); // 137.5°

    // Ellipse aspect ratio for wide monitors
    const ellipseAspect = Math.min((width / height) * 0.9, 2.0);

    // Center of unified layout
    const centerX = 0;
    const centerY = 0;

    const graphNodes: GraphNode[] = [];

    // Combine all nodes for unified layout
    const allLayoutNodes: Node[] = [
      ...sortedClusters.flatMap(([, nodes]) => nodes),
      ...unclustered,
    ];

    // Sort by childCount descending (importance)
    const sortedByImportance = [...allLayoutNodes].sort((a, b) => (b.childCount || 0) - (a.childCount || 0));

    devLog('info', `Sorted ${sortedByImportance.length} nodes by importance (childCount)`);

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
          const angle = ringStartAngle + (i / nodesInThisRing) * 2 * Math.PI;

          // Apply ellipse aspect ratio
          const x = centerX + ringRadius * ellipseAspect * Math.cos(angle);
          const y = centerY + ringRadius * Math.sin(angle);

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

    // Define arrow markers for edge endpoints
    const defs = svg.append('defs');
    edgeData.forEach((d, i) => {
      const normalized = (d.weight - minWeight) / weightRange;
      const hue = d.type === 'contains' ? 220 : normalized * 120;
      const color = d.type === 'contains' ? '#6b7280' : `hsl(${hue}, 80%, 50%)`;
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
        // Normalize weight to 0-1 range based on actual min/max in view
        const normalized = (d.weight - minWeight) / weightRange;
        // Color from red (low) to green (high similarity)
        const hue = normalized * 120; // 0=red, 60=yellow, 120=green
        return `hsl(${hue}, 80%, 50%)`;
      })
      .attr('stroke-opacity', d => d.type === 'contains' ? 0.5 : 0.7)
      .attr('stroke-width', d => {
        // Base thickness + weight-based scaling
        if (d.type === 'contains') return 3;
        // Normalize and use exponential scaling
        const normalized = (d.weight - minWeight) / weightRange;
        // 0% (min) → 2px, 100% (max) → 16px
        return 2 + normalized * 14;
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

    // Shadow
    cardGroups.append('rect')
      .attr('x', 3)
      .attr('y', 3)
      .attr('width', noteWidth)
      .attr('height', noteHeight)
      .attr('rx', 6)
      .attr('fill', 'rgba(0,0,0,0.35)');

    // Card background - color by cluster
    cardGroups.append('rect')
      .attr('class', 'card-bg')
      .attr('width', noteWidth)
      .attr('height', noteHeight)
      .attr('rx', 6)
      .attr('fill', d => d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#374151')
      .attr('stroke', d => d.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.15)')
      .attr('stroke-width', d => d.id === activeNodeId ? 2 : 1);

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
    cardGroups.append('text')
      .attr('x', 14)
      .attr('y', noteHeight - 16)
      .attr('font-family', cardFont)
      .attr('font-size', '16px')
      .attr('fill', 'rgba(255,255,255,0.5)')
      .text(d => d.childCount > 0 ? `${d.childCount} items` : new Date(d.createdAt).toLocaleDateString());

    // Helper to reset zen mode
    const resetZenMode = () => {
      setZenMode(null);
      cardGroups.style('opacity', 1);
      dotGroups.style('opacity', 1);
      edgePaths.style('opacity', null);
    };

    // Zen mode button (bottom right of card)
    // Position: 32px from right and bottom edges
    const zenButtons = cardGroups.append('g')
      .attr('class', 'zen-button')
      .attr('transform', `translate(${noteWidth - 32}, ${noteHeight - 32})`)
      .attr('cursor', 'pointer')
      .on('mouseenter', function() {
        d3.select(this).select('rect')
          .attr('fill', 'rgba(139,92,246,0.9)'); // Purple highlight
      })
      .on('mouseleave', function() {
        d3.select(this).select('rect')
          .attr('fill', 'rgba(0,0,0,0.6)');
      })
      .on('click', function(event, d) {
        event.stopPropagation();
        const currentZen = zenModeNodeIdRef.current;
        if (currentZen === d.id) {
          // Toggle off
          resetZenMode();
        } else {
          // Enable zen mode for this node
          applyZenMode(d.id);
        }
      });

    zenButtons.append('rect')
      .attr('x', -24)
      .attr('y', -24)
      .attr('width', 48)
      .attr('height', 48)
      .attr('rx', 10)
      .attr('fill', 'rgba(0,0,0,0.6)')
      .attr('stroke', 'rgba(255,255,255,0.5)')
      .attr('stroke-width', 2);

    zenButtons.append('text')
      .attr('text-anchor', 'middle')
      .attr('dy', '0.35em')
      .attr('font-size', '38px')
      .attr('fill', '#fff')
      .text('☯');

    // Unified bubble rendering for ALL nodes (low zoom)
    const dotGroups = dotsGroup.selectAll('g.node-dot')
      .data(graphNodes)
      .join('g')
      .attr('class', 'node-dot')
      .attr('cursor', 'pointer')
      .attr('transform', d => `translate(${d.x}, ${d.y})`);

    dotGroups.append('circle')
      .attr('class', 'dot-glow')
      .attr('r', dotSize + 4)
      .attr('fill', 'none')
      .attr('stroke', d => d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#374151')
      .attr('stroke-width', 3)
      .attr('stroke-opacity', 0.3);

    dotGroups.append('circle')
      .attr('class', 'dot-main')
      .attr('r', dotSize)
      .attr('fill', d => d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#374151')
      .attr('stroke', d => d.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.6)')
      .attr('stroke-width', d => d.id === activeNodeId ? 3 : 1.5);

    dotGroups.append('text')
      .attr('text-anchor', 'middle')
      .attr('dy', '0.35em')
      .attr('font-size', '18px')
      .attr('fill', '#fff')
      .text(d => d.displayEmoji);

    // Start with cards shown, dots hidden
    dotsGroup.style('display', 'none');

    // Helper to apply zen mode for a node
    const applyZenMode = (nodeId: string) => {
      setZenMode(nodeId);
      // Build relevance map from edges
      const relevanceMap = new Map<string, number>();
      relevanceMap.set(nodeId, 1.0); // Self is always 1.0
      edgeData.forEach(edge => {
        if (edge.source.id === nodeId) {
          const existing = relevanceMap.get(edge.target.id) || 0;
          relevanceMap.set(edge.target.id, Math.max(existing, edge.weight));
        } else if (edge.target.id === nodeId) {
          const existing = relevanceMap.get(edge.source.id) || 0;
          relevanceMap.set(edge.source.id, Math.max(existing, edge.weight));
        }
      });
      // Fade nodes by relevance
      cardGroups.style('opacity', (n: GraphNode) => {
        const relevance = relevanceMap.get(n.id);
        if (relevance === undefined) return 0.15; // Unconnected
        return 0.3 + relevance * 0.7; // 0.3 to 1.0 based on weight
      });
      dotGroups.style('opacity', (n: GraphNode) => {
        const relevance = relevanceMap.get(n.id);
        if (relevance === undefined) return 0.15;
        return 0.3 + relevance * 0.7;
      });
      // Fade edges by relevance to selected node
      edgePaths.style('opacity', (e: {source: GraphNode, target: GraphNode, weight: number}) => {
        if (e.source.id === nodeId || e.target.id === nodeId) {
          return 0.5 + e.weight * 0.5; // Connected edges: 0.5 to 1.0
        }
        return 0.05; // Unconnected edges nearly invisible
      });
    };

    // Unified interactions
    cardGroups
      .on('click', function(event, d) {
        event.stopPropagation();
        setActiveNode(d.id);
        fetchSimilarNodes(d.id);
        cardsGroup.selectAll('.card-bg')
          .attr('stroke', 'rgba(255,255,255,0.15)')
          .attr('stroke-width', 1);
        d3.select(this).select('.card-bg')
          .attr('stroke', '#fbbf24')
          .attr('stroke-width', 2);
        // If zen mode is active, switch to the new node
        if (zenModeNodeIdRef.current && zenModeNodeIdRef.current !== d.id) {
          applyZenMode(d.id);
        }
      })
      .on('dblclick', function(event, d) {
        event.stopPropagation();
        if (d.isItem) {
          devLog('info', `Opening item "${d.displayTitle}" in Leaf mode`);
          openLeaf(d.id);
        } else if (d.childCount > 0) {
          devLog('info', `Drilling into "${d.displayTitle}" (depth ${d.depth} → ${d.depth + 1})`);
          navigateToNode(d);
        }
      })
      .on('mouseenter', (_, d) => setHoveredNode(d))
      .on('mouseleave', () => setHoveredNode(null));

    // Interactions for dots (same as cards but for bubble mode)
    dotGroups
      .on('click', function(event, d) {
        event.stopPropagation();
        setActiveNode(d.id);
        fetchSimilarNodes(d.id);
        // If zen mode is active, switch to the new node
        if (zenModeNodeIdRef.current && zenModeNodeIdRef.current !== d.id) {
          applyZenMode(d.id);
        }
      })
      .on('dblclick', function(event, d) {
        event.stopPropagation();
        if (d.isItem) {
          devLog('info', `Opening item "${d.displayTitle}" in Leaf mode`);
          openLeaf(d.id);
        } else if (d.childCount > 0) {
          devLog('info', `Drilling into "${d.displayTitle}" (depth ${d.depth} → ${d.depth + 1})`);
          navigateToNode(d);
        }
      })
      .on('mouseenter', (_, d) => setHoveredNode(d))
      .on('mouseleave', () => setHoveredNode(null));

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
      .on('zoom', (event) => {
        const { k } = event.transform;
        currentScale = k;

        container.attr('transform', event.transform);

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

    // Click to deselect
    svg.on('click', () => {
      setActiveNode(null);
      setSimilarNodesMap(new Map());
      cardsGroup.selectAll('.card-bg')
        .attr('stroke', 'rgba(255,255,255,0.15)')
        .attr('stroke-width', 1);
      // Reset zen mode
      if (zenModeNodeIdRef.current) {
        resetZenMode();
      }
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

  // Note: activeNodeId removed from deps to prevent re-render/zoom on click
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nodes, edges, width, height, setActiveNode, devLog, getNodeEmoji, currentDepth, maxDepth, currentParentId, navigateToNode]);

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
            const jumpFromNode = crumb.jumpFromId ? nodes.get(crumb.jumpFromId) : null;
            const jumpFromEmoji = jumpFromNode ? getNodeEmoji(jumpFromNode) : crumb.jumpFromEmoji;

            return (
              <div key={`${crumb.id}-${index}`} className="flex items-center">
                {isJump ? (
                  // Show source node + dashed arrow for jump navigation
                  <>
                    <ChevronRight size={16} className="text-gray-600 mx-0.5" />
                    <span className="text-gray-400 text-sm px-1" title={`Jumped from: ${crumb.jumpFromTitle}`}>
                      {jumpFromEmoji && <span className="mr-1">{jumpFromEmoji}</span>}
                      {crumb.jumpFromTitle}
                    </span>
                    <span className="text-blue-400 mx-1 text-sm" title="Jumped via Similar Nodes">⤳</span>
                  </>
                ) : (
                  <ChevronRight size={16} className="text-gray-600 mx-0.5" />
                )}
                {isLast ? (
                  // Current location: highlighted, not clickable
                  <span
                    className={`flex items-center gap-1 px-2 py-1 text-sm font-medium ${isJump ? 'text-blue-300' : 'text-amber-300'}`}
                    title={lastNode?.summary || lastNode?.content || crumb.title}
                  >
                    {crumbEmoji && <span>{crumbEmoji}</span>}
                    {crumb.title}{toSuperscript(crumb.depth)}
                  </span>
                ) : (
                  // Ancestor: clickable
                  <button
                    onClick={() => {
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

      {/* Similar nodes panel - resizable */}
      {showPanels && showDetails && similarNodesMap.size > 0 && (
        <div
          className="absolute top-16 right-4 bg-gray-800/95 backdrop-blur-sm text-white rounded-lg shadow-xl border border-gray-700 z-30 overflow-hidden flex flex-col"
          style={{ width: detailsPanelSize.width, height: detailsPanelSize.height }}
        >
          {/* Panel header with stack toggle */}
          <div className="px-3 py-2 border-b border-gray-700 flex items-center justify-between flex-shrink-0">
            <span className="text-xs font-medium text-gray-400">Node Details</span>
            <div className="flex items-center gap-2">
              <button
                onClick={() => setStackNodes(!stackNodes)}
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
                onClick={() => setSimilarNodesMap(new Map())}
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
                        onClick={() => setSimilarNodesMap(prev => {
                          const next = new Map(prev);
                          next.delete(sourceId);
                          return next;
                        })}
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
                        // Build hierarchy path (including Universe)
                        const getHierarchyPath = (nodeId: string): { name: string; isUniverse: boolean }[] => {
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
                        };
                        const hierarchyPath = getHierarchyPath(similar.id);
                        return (
                          <button
                            key={similar.id}
                            onClick={() => {
                              const targetNode = nodes.get(similar.id);
                              if (targetNode) {
                                jumpToNode(targetNode, sourceNode);
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
                                {(() => {
                                  // Get source node's hierarchy to compare
                                  const sourceHierarchy = getHierarchyPath(sourceId);
                                  return hierarchyPath.map((segment, idx) => {
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
                                  });
                                })()}
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
              setIsResizingDetails(true);
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
      )}

      {/* Loading indicator for similar nodes */}
      {loadingSimilar.size > 0 && (
        <div className="absolute top-20 right-4 bg-gray-800/95 backdrop-blur-sm text-white rounded-lg shadow-xl border border-gray-700 z-30 px-4 py-3">
          <span className="text-xs text-gray-400">Loading similar nodes...</span>
        </div>
      )}

      {/* Status bar */}
      <div className="absolute bottom-4 left-4 bg-gray-800/90 backdrop-blur-sm rounded-lg px-4 py-2 text-sm text-gray-300 flex items-center gap-4">
        <span>{nodes.size} nodes</span>
        <span className="text-gray-500">|</span>
        <span>{edges.size} edges</span>
        <span className="text-gray-500">|</span>
        <span className="text-amber-400">Zoom: {(zoomLevel * 100).toFixed(0)}%</span>
        <span className="text-gray-500">|</span>
        {/* Workflow buttons: Process AI | Cluster | Build Hierarchy */}
        <div className="flex items-center gap-1">
          <button
            onClick={() => setConfirmAction('processAi')}
            disabled={isProcessing}
            className="bg-purple-600 hover:bg-purple-500 disabled:bg-purple-800 disabled:cursor-wait px-3 py-1 rounded-l text-white text-xs font-medium transition-colors"
            title="AI generates titles, summaries & embeddings for items"
          >
            {isProcessing ? 'Processing...' : 'Process AI'}
          </button>
          {isProcessing && (
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
              className="bg-red-600 hover:bg-red-500 px-2 py-1 rounded-r text-white text-xs font-medium transition-colors"
              title="Cancel AI processing"
            >
              ✕
            </button>
          )}
        </div>
        <div className="flex items-center gap-1">
          <button
            onClick={() => setConfirmAction('cluster')}
            disabled={isClustering || isFullRebuilding}
            className="bg-blue-600 hover:bg-blue-500 disabled:bg-blue-800 disabled:cursor-wait px-3 py-1 rounded text-white text-xs font-medium transition-colors"
            title="Assign items to topic clusters using AI"
          >
            {isClustering ? 'Clustering...' : 'Cluster'}
          </button>
        </div>
        <div className="flex items-center gap-1">
          <button
            onClick={() => setConfirmAction('buildHierarchy')}
            disabled={isBuildingHierarchy || isFullRebuilding}
            className="bg-green-600 hover:bg-green-500 disabled:bg-green-800 disabled:cursor-wait px-3 py-1 rounded-l text-white text-xs font-medium transition-colors"
            title="Build navigation hierarchy from clusters"
          >
            {isBuildingHierarchy ? 'Building...' : 'Hierarchy'}
          </button>
          {isBuildingHierarchy && (
            <button
              onClick={async () => {
                await invoke('cancel_rebuild');
                setIsBuildingHierarchy(false);
                console.log('[Cancel] Hierarchy build cancelled');
                devLog('info', 'Hierarchy build cancelled');
              }}
              className="bg-red-600 hover:bg-red-500 px-2 py-1 rounded-r text-white text-xs font-medium transition-colors"
              title="Cancel hierarchy build"
            >
              ✕
            </button>
          )}
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
            onClick={() => setConfirmAction(null)}
          />
          <div className="relative bg-gray-800 rounded-lg border border-gray-700 shadow-xl max-w-md w-full mx-4 p-6">
            <button
              onClick={() => setConfirmAction(null)}
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
                onClick={() => setConfirmAction(null)}
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
    </div>
  );
}
