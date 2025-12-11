import { useRef, useEffect, useState, useCallback } from 'react';
import * as d3 from 'd3';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useGraphStore } from '../../stores/graphStore';
import type { Node } from '../../types/graph';
import { getEmojiForNode, initLearnedMappings } from '../../utils/emojiMatcher';
import { ChevronRight } from 'lucide-react';

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
}

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
    breadcrumbs,
    setCurrentDepth,
    setMaxDepth,
  } = useGraphStore();
  const [hoveredNode, setHoveredNode] = useState<GraphNode | null>(null);
  const [zoomLevel, setZoomLevel] = useState(1);
  const [devLogs, setDevLogs] = useState<DevConsoleLog[]>([]);
  const [showDevConsole, setShowDevConsole] = useState(true);
  const [isClustering, setIsClustering] = useState(false);
  const [isProcessing, setIsProcessing] = useState(false);
  const [isBuildingHierarchy, setIsBuildingHierarchy] = useState(false);
  const [autoScroll, setAutoScroll] = useState(true);
  const [hierarchyBuilt, setHierarchyBuilt] = useState(false);
  const [consoleSize, setConsoleSize] = useState({ width: 384, height: 320 }); // w-96 = 384px
  const [isResizing, setIsResizing] = useState(false);

  // Similar nodes state
  const [similarNodesMap, setSimilarNodesMap] = useState<Map<string, SimilarNode[]>>(new Map());
  const [loadingSimilar, setLoadingSimilar] = useState<Set<string>>(new Set());

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
      setDevLogs(prev => [...prev.slice(-50), { time, type, message: `[Hierarchy] ${message}` }]);
    });
    return () => { unlisten.then(f => f()); };
  }, []);

  const devLog = useCallback((type: 'info' | 'warn' | 'error', message: string) => {
    const time = new Date().toLocaleTimeString();
    setDevLogs(prev => [...prev.slice(-50), { time, type, message }]);
    if (type === 'error') console.error(`[Graph] ${message}`);
    else if (type === 'warn') console.warn(`[Graph] ${message}`);
    else console.log(`[Graph] ${message}`);
  }, []);

  // Fetch similar nodes for a node
  const fetchSimilarNodes = useCallback(async (nodeId: string) => {
    // Toggle off if already showing
    if (similarNodesMap.has(nodeId)) {
      setSimilarNodesMap(prev => {
        const next = new Map(prev);
        next.delete(nodeId);
        return next;
      });
      return;
    }

    setLoadingSimilar(prev => new Set(prev).add(nodeId));
    try {
      const similar = await invoke<SimilarNode[]>('get_similar_nodes', { nodeId, topN: 5 });
      setSimilarNodesMap(prev => new Map(prev).set(nodeId, similar));
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

  useEffect(() => {
    if (!isResizing) return;

    const handleMouseMove = (e: MouseEvent) => {
      // Console is positioned from right edge, so width increases as mouse moves left
      const newWidth = Math.max(280, Math.min(800, window.innerWidth - e.clientX - 16));
      // Height increases as mouse moves down
      const newHeight = Math.max(200, Math.min(600, e.clientY - 56)); // 56 = top offset
      setConsoleSize({ width: newWidth, height: newHeight });
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
  }, [isResizing]);

  // Listen for AI progress events
  useEffect(() => {
    const unlisten = listen<AiProgressEvent>('ai-progress', (event) => {
      const { current, total, nodeTitle, newTitle, status, errorMessage } = event.payload;

      if (status === 'processing') {
        devLog('info', `[${current}/${total}] Processing: ${nodeTitle}`);
      } else if (status === 'success') {
        devLog('info', `[${current}/${total}] Done: "${nodeTitle}" → "${newTitle}"`);
      } else if (status === 'error') {
        devLog('error', `[${current}/${total}] Failed: ${nodeTitle}${errorMessage ? ` - ${errorMessage}` : ''}`);
      } else if (status === 'complete') {
        devLog('info', `AI processing complete (${total} nodes)`);
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
    devLog('info', 'Starting AI processing...');
    try {
      const result = await invoke<{ processed: number; failed: number; errors: string[] }>('process_nodes');
      devLog('info', `Processing complete: ${result.processed} processed, ${result.failed} failed`);
      if (result.errors.length > 0) {
        result.errors.forEach(e => devLog('error', e));
      }
      // Reload to get updated AI fields
      window.location.reload();
    } catch (error) {
      devLog('error', `Processing failed: ${error}`);
    } finally {
      setIsProcessing(false);
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
    setIsFullRebuilding(true);
    devLog('info', '=== FULL REBUILD: Clustering + Hierarchy + AI Grouping ===');
    try {
      const result = await invoke<{
        clusteringResult: { itemsProcessed: number; clustersCreated: number; itemsAssigned: number } | null;
        hierarchyResult: { levelsCreated: number; intermediateNodesCreated: number; itemsOrganized: number; maxDepth: number };
        levelsCreated: number;
        groupingIterations: number;
      }>('build_full_hierarchy', { runClustering: true });

      if (result.clusteringResult) {
        devLog('info', `Clustering: ${result.clusteringResult.itemsAssigned} items → ${result.clusteringResult.clustersCreated} clusters`);
      }
      devLog('info', `Hierarchy: ${result.levelsCreated} levels, ${result.groupingIterations} AI grouping iterations`);

      setHierarchyBuilt(true);
      setMaxDepth(result.hierarchyResult.maxDepth);

      devLog('info', 'Reloading in 3 seconds...');
      await new Promise(resolve => setTimeout(resolve, 3000));
      window.location.reload();
    } catch (error) {
      devLog('error', `Full rebuild FAILED: ${JSON.stringify(error)}`);
      setIsFullRebuilding(false);
    }
  }, [devLog, setMaxDepth]);

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
    const noteHeight = 240;

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

    // Ring capacity: Ring 0 = 1, Ring N = 6 * N
    const getNodesPerRing = (ring: number): number => {
      return ring === 0 ? 1 : 6 * ring;
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

    // Draw edges - store source/target data for zoom updates
    const linksGroup = container.append('g').attr('class', 'links');

    // Build edge data with resolved coordinates
    const edgeData: Array<{source: GraphNode, target: GraphNode, type: string}> = [];
    edges.forEach(edge => {
      const source = nodeMap.get(edge.source);
      const target = nodeMap.get(edge.target);
      if (source && target) {
        edgeData.push({ source, target, type: edge.type });
      }
    });

    const edgePaths = linksGroup.selectAll('path')
      .data(edgeData)
      .join('path')
      .attr('fill', 'none')
      .attr('stroke', d => d.type === 'contains' ? '#6b7280' : '#ef4444')
      .attr('stroke-opacity', d => d.type === 'contains' ? 0.3 : 0.5)
      .attr('stroke-width', d => d.type === 'contains' ? 1.5 : 2)
      .attr('d', d => {
        const dx = d.target.x - d.source.x;
        const dy = d.target.y - d.source.y;
        const dr = Math.sqrt(dx * dx + dy * dy) * 0.4;
        return `M${d.source.x},${d.source.y} A${dr},${dr} 0 0,1 ${d.target.x},${d.target.y}`;
      });

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

    // Titlebar - darker area at top (72px for 2 lines of text + emoji)
    // First rect: full width, rounded top corners match card
    cardGroups.append('rect')
      .attr('width', noteWidth)
      .attr('height', 72)
      .attr('rx', 6)
      .attr('fill', 'rgba(0,0,0,0.4)');
    // Second rect: fills in bottom corners of titlebar
    cardGroups.append('rect')
      .attr('y', 62)
      .attr('width', noteWidth)
      .attr('height', 10)
      .attr('fill', 'rgba(0,0,0,0.4)');

    // Large emoji spanning titlebar height
    cardGroups.append('text')
      .attr('x', 14)
      .attr('y', 48)
      .attr('font-size', '42px')
      .text(d => d.displayEmoji);

    // Title - foreignObject for natural wrapping (2 lines), centered horizontally
    cardGroups.append('foreignObject')
      .attr('x', 62)
      .attr('y', 10)
      .attr('width', noteWidth - 76)
      .attr('height', 56)
      .append('xhtml:div')
      .style('font-size', '20px')
      .style('font-weight', '600')
      .style('color', '#ffffff')
      .style('line-height', '1.3')
      .style('overflow', 'hidden')
      .style('word-wrap', 'break-word')
      .style('text-align', 'center')
      .text(d => d.displayTitle);

    // Synopsis - foreignObject for natural wrapping
    cardGroups.append('foreignObject')
      .attr('x', 14)
      .attr('y', 78)
      .attr('width', noteWidth - 28)
      .attr('height', noteHeight - 112)
      .append('xhtml:div')
      .style('font-size', '18px')
      .style('color', '#ffffff')
      .style('line-height', '1.4')
      .style('overflow', 'hidden')
      .style('word-wrap', 'break-word')
      .text(d => d.displayContent || '');

    // Footer info
    cardGroups.append('text')
      .attr('x', 14)
      .attr('y', noteHeight - 12)
      .attr('font-size', '11px')
      .attr('fill', 'rgba(255,255,255,0.5)')
      .text(d => d.childCount > 0 ? `${d.childCount} items` : new Date(d.createdAt).toLocaleDateString());

    // Similar nodes button (↔) - for ALL nodes (items and categories)
    cardGroups
      .append('text')
      .attr('class', 'similar-btn')
      .attr('x', noteWidth - 28)
      .attr('y', noteHeight - 10)
      .attr('font-size', '14px')
      .attr('fill', 'rgba(255,255,255,0.4)')
      .attr('cursor', 'pointer')
      .text('↔')
      .on('click', function(event, d) {
        event.stopPropagation();
        fetchSimilarNodes(d.id);
      })
      .on('mouseenter', function() {
        d3.select(this).attr('fill', '#fbbf24');
      })
      .on('mouseleave', function() {
        d3.select(this).attr('fill', 'rgba(255,255,255,0.4)');
      });

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

    // Unified interactions
    cardGroups
      .on('click', function(event, d) {
        event.stopPropagation();
        setActiveNode(d.id);
        cardsGroup.selectAll('.card-bg')
          .attr('stroke', 'rgba(255,255,255,0.15)')
          .attr('stroke-width', 1);
        d3.select(this).select('.card-bg')
          .attr('stroke', '#fbbf24')
          .attr('stroke-width', 2);
      })
      .on('dblclick', function(event, d) {
        event.stopPropagation();
        if (d.childCount > 0 && !d.isItem) {
          devLog('info', `Drilling into "${d.displayTitle}" (depth ${d.depth} → ${d.depth + 1})`);
          navigateToNode(d);
        } else if (d.isItem) {
          devLog('info', `Opening item "${d.displayTitle}" (would open in Leaf mode)`);
        }
      })
      .on('mouseenter', (_, d) => setHoveredNode(d))
      .on('mouseleave', () => setHoveredNode(null));

    // Interactions for dots (same as cards but for bubble mode)
    dotGroups
      .on('click', function(event, d) {
        event.stopPropagation();
        setActiveNode(d.id);
      })
      .on('dblclick', function(event, d) {
        event.stopPropagation();
        if (d.childCount > 0 && !d.isItem) {
          devLog('info', `Drilling into "${d.displayTitle}" (depth ${d.depth} → ${d.depth + 1})`);
          navigateToNode(d);
        } else if (d.isItem) {
          devLog('info', `Opening item "${d.displayTitle}" (would open in Leaf mode)`);
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
          edgePaths.attr('d', d => {
            const sx = d.source.x * positionScale;
            const sy = d.source.y * positionScale;
            const tx = d.target.x * positionScale;
            const ty = d.target.y * positionScale;
            const dx = tx - sx;
            const dy = ty - sy;
            const dr = Math.sqrt(dx * dx + dy * dy) * 0.4;
            return `M${sx},${sy} A${dr},${dr} 0 0,1 ${tx},${ty}`;
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
          edgePaths.attr('d', d => {
            const sx = d.source.x * positionScale;
            const sy = d.source.y * positionScale;
            const tx = d.target.x * positionScale;
            const ty = d.target.y * positionScale;
            const dx = tx - sx;
            const dy = ty - sy;
            const dr = Math.sqrt(dx * dx + dy * dy) * 0.4;
            return `M${sx},${sy} A${dr},${dr} 0 0,1 ${tx},${ty}`;
          });
        }

        setZoomLevel(k);
      });

    svg.call(zoom);

    // Click to deselect
    svg.on('click', () => {
      setActiveNode(null);
      cardsGroup.selectAll('.card-bg')
        .attr('stroke', 'rgba(255,255,255,0.15)')
        .attr('stroke-width', 1);
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
            <span className="text-sm">🌌</span>
            <span className="text-xs font-medium">Universe{toSuperscript(0)}</span>
          </button>

          {/* Navigation path - show each node we've drilled into */}
          {breadcrumbs.map((crumb, index) => {
            const isLast = index === breadcrumbs.length - 1;
            const lastNode = isLast ? nodes.get(crumb.id) : null;

            return (
              <div key={crumb.id} className="flex items-center">
                <ChevronRight size={14} className="text-gray-600 mx-0.5" />
                {isLast ? (
                  // Current location: highlighted, not clickable
                  <span
                    className="flex items-center gap-1 px-2 py-1 text-amber-300 text-xs font-medium"
                    title={lastNode?.summary || lastNode?.content || crumb.title}
                  >
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
                    className="flex items-center gap-1 px-2 py-1 rounded transition-colors hover:bg-gray-700 text-gray-300 cursor-pointer"
                  >
                    <span className="text-xs font-medium">{crumb.title}{toSuperscript(crumb.depth)}</span>
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

      {/* Similar nodes panel */}
      {similarNodesMap.size > 0 && (
        <div className="absolute top-20 right-4 bg-gray-800/95 backdrop-blur-sm text-white rounded-lg shadow-xl border border-gray-700 z-30 max-w-sm">
          <div className="px-3 py-2 border-b border-gray-700 flex items-center justify-between">
            <span className="text-xs font-medium text-gray-400">Similar Nodes</span>
            <button
              onClick={() => setSimilarNodesMap(new Map())}
              className="text-gray-500 hover:text-gray-300 text-xs"
            >
              ✕
            </button>
          </div>
          <div className="max-h-80 overflow-y-auto">
            {Array.from(similarNodesMap.entries()).map(([sourceId, similarNodes]) => {
              const sourceNode = nodes.get(sourceId);
              return (
                <div key={sourceId} className="border-b border-gray-700/50 last:border-0">
                  <div className="px-3 py-1.5 bg-gray-700/30 text-xs text-gray-400 truncate">
                    ↔ {sourceNode?.aiTitle || sourceNode?.title || sourceId}
                  </div>
                  {similarNodes.map((similar) => (
                    <button
                      key={similar.id}
                      onClick={() => {
                        setActiveNode(similar.id);
                        devLog('info', `Navigating to similar node: ${similar.title}`);
                      }}
                      className="w-full px-3 py-2 text-left hover:bg-gray-700/50 transition-colors"
                    >
                      <div className="flex items-center justify-between gap-2">
                        <span className="text-sm truncate">
                          {similar.emoji || '📄'} {similar.title}
                        </span>
                        <span className="text-xs text-amber-400 shrink-0">
                          {(similar.similarity * 100).toFixed(0)}%
                        </span>
                      </div>
                    </button>
                  ))}
                </div>
              );
            })}
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
        {/* Workflow buttons: Process AI (per-item) | Full Rebuild (clustering + hierarchy) */}
        <button
          onClick={runProcessing}
          disabled={isProcessing}
          className="bg-purple-600 hover:bg-purple-500 disabled:bg-gray-600 disabled:cursor-wait px-3 py-1 rounded text-white text-xs font-medium transition-colors"
          title="AI generates titles, summaries & embeddings for items"
        >
          {isProcessing ? 'Processing...' : 'Process AI'}
        </button>
        <button
          onClick={fullRebuild}
          disabled={isFullRebuilding || isBuildingHierarchy}
          className="bg-green-600 hover:bg-green-500 disabled:bg-gray-600 disabled:cursor-wait px-3 py-1 rounded text-white text-xs font-medium transition-colors"
          title="Cluster items + Build hierarchy with recursive AI grouping"
        >
          {isFullRebuilding ? 'Rebuilding...' : 'Full Rebuild'}
        </button>
      </div>

      <div className="absolute bottom-4 right-4 bg-gray-800/80 backdrop-blur-sm rounded-lg px-3 py-2 text-xs text-gray-400">
        Scroll to zoom - Click and drag to pan
      </div>

      {/* Dev Console Toggle */}
      <button
        onClick={() => setShowDevConsole(!showDevConsole)}
        className="absolute top-4 right-4 bg-gray-800/90 hover:bg-gray-700/90 backdrop-blur-sm rounded-lg px-3 py-2 text-xs text-gray-400 hover:text-white transition-colors"
      >
        {showDevConsole ? 'Hide' : 'Show'} Console
      </button>

      {/* Dev Console */}
      {showDevConsole && (
        <div
          className="absolute top-14 right-4 bg-gray-900/95 backdrop-blur-sm rounded-lg border border-gray-700 overflow-hidden flex flex-col"
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
                List Nodes
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
          {/* Resize handle - bottom-left corner */}
          <div
            onMouseDown={handleResizeStart}
            className={`absolute bottom-0 left-0 w-4 h-4 cursor-sw-resize group ${isResizing ? 'bg-amber-500/30' : ''}`}
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
    </div>
  );
}
