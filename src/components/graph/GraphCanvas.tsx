// @ts-nocheck
import React, { useRef, useEffect, useState } from 'react';
import * as d3 from 'd3';
import type { Node } from '../../types/graph';

// =============================================================================
// TYPES
// =============================================================================

export interface GraphNode extends Node {
  x: number;
  y: number;
  renderClusterId: number;
  displayTitle: string;
  displayContent: string;
  displayEmoji: string;
}

export interface EdgeData {
  source: GraphNode;
  target: GraphNode;
  type: string;
  weight: number;
}

export interface GraphCanvasProps {
  // Data (computed in parent)
  graphNodes: GraphNode[];
  edgeData: EdgeData[];

  // Selection state
  activeNodeId: string | null;
  connectionMap: Map<string, { weight: number; distance: number }>;

  // Dimensions
  width: number;
  height: number;

  // Callbacks to parent
  onSelectNode: (id: string | null) => void;
  onNavigateToNode: (node: Node) => void;
  onOpenLeaf: (id: string) => void;
  onFetchSimilarNodes: (id: string) => void;
  onShowContextMenu: (id: string, pos: { x: number; y: number }) => void;
  onZoomChange: (zoom: number) => void;
  devLog: (type: 'info' | 'warn' | 'error', message: string) => void;

  // Helpers
  getNodeEmoji: (node: Node) => string;
  hidePrivate: boolean;
}

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

// Generate colors for clusters dynamically
const generateClusterColor = (clusterId: number): string => {
  const hue = (clusterId * 137.508) % 360;
  return `hsl(${hue}, 55%, 35%)`;
};

// Direct connection color: red→yellow→blue→cyan
const getDirectConnectionColor = (weight: number): string => {
  let hue: number;
  if (weight < 0.5) {
    hue = weight * 2 * 60;
  } else {
    hue = 210 - (weight - 0.5) * 2 * 30;
  }
  return `hsl(${hue}, 80%, 40%)`;
};

// Chain connection color: darker red tint for indirect connections
const getChainConnectionColor = (hopDistance: number): string => {
  const lightness = Math.max(25, 35 - hopDistance * 3);
  return `hsl(0, 60%, ${lightness}%)`;
};

// Calculate structural depth for shadow stacking
const getStructuralDepth = (childCount: number, isItem: boolean): number => {
  if (isItem) return 0;
  if (childCount >= 16) return 4;
  if (childCount >= 6) return 3;
  if (childCount >= 2) return 2;
  return 1;
};

// Card dimensions at 100% zoom (unified for all nodes)
const NOTE_WIDTH = 320;
const NOTE_HEIGHT = 320;
const DOT_SIZE = 24;

// Zoom limits
const MIN_ZOOM = 0.02;
const MAX_ZOOM = 2;

// Get muted cluster color (gray with hint of cluster hue)
const getMutedClusterColor = (d: GraphNode): string => {
  if (d.renderClusterId < 0) return '#374151';
  const hue = (d.renderClusterId * 137.508) % 360;
  return `hsl(${hue}, 12%, 28%)`;
};

// Get node color based on connection distance
const getNodeColor = (
  d: GraphNode,
  activeNodeId: string | null,
  connectionMap: Map<string, { weight: number; distance: number }>
): string => {
  if (!activeNodeId) return getMutedClusterColor(d);
  if (d.id === activeNodeId) {
    return d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#4b5563';
  }
  const conn = connectionMap.get(d.id);
  if (conn) {
    if (conn.distance === 1) return getDirectConnectionColor(conn.weight);
    return getChainConnectionColor(conn.distance);
  }
  return getMutedClusterColor(d);
};

// Get node opacity based on connection distance
const getNodeOpacity = (
  d: GraphNode,
  activeNodeId: string | null,
  connectionMap: Map<string, { weight: number; distance: number }>
): number => {
  if (!activeNodeId) return 1;
  if (d.id === activeNodeId) return 1;
  const conn = connectionMap.get(d.id);
  if (conn) {
    if (conn.distance === 1) return 1;
    return Math.max(0.5, 1 - conn.distance * 0.15);
  }
  return 0.7;
};

// Get date-based color (red=old, cyan=new)
const getDateColor = (timestamp: number, minDate: number, maxDate: number): string => {
  const range = maxDate - minDate || 1;
  const t = (timestamp - minDate) / range;
  let hue: number;
  if (t < 0.5) {
    hue = t * 2 * 60; // red → yellow
  } else {
    hue = 210 - (t - 0.5) * 2 * 30; // blue → cyan
  }
  return `hsl(${hue}, 75%, 65%)`;
};

// Nice font for card text
const CARD_FONT = "'Inter', 'SF Pro Display', -apple-system, BlinkMacSystemFont, sans-serif";

// Get edge color based on normalized weight
const getEdgeColor = (normalized: number): string => {
  let hue: number;
  if (normalized < 0.5) {
    hue = normalized * 2 * 60; // red → yellow
  } else {
    hue = 210 - (normalized - 0.5) * 2 * 30; // blue → cyan
  }
  return `hsl(${hue}, 80%, 50%)`;
};

// Get all 4 side centers of a node
const getSideCenters = (center: { x: number; y: number }, width: number, height: number) => {
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
const getEdgePoints = (centerA: { x: number; y: number }, centerB: { x: number; y: number }, width: number, height: number) => {
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

// =============================================================================
// GRAPHCANVAS COMPONENT
// =============================================================================

export const GraphCanvas = React.memo(function GraphCanvas(props: GraphCanvasProps) {
  const {
    graphNodes,
    edgeData,
    activeNodeId,
    connectionMap,
    width,
    height,
    onSelectNode,
    onNavigateToNode,
    onOpenLeaf,
    onFetchSimilarNodes,
    onShowContextMenu,
    onZoomChange,
    devLog,
    getNodeEmoji,
    hidePrivate,
  } = props;

  // ==========================================================================
  // REFS
  // ==========================================================================
  const svgRef = useRef<SVGSVGElement>(null);
  const activeNodeIdRef = useRef<string | null>(null);
  const connectionMapRef = useRef<Map<string, { weight: number; distance: number }>>(new Map());
  const lastClickTimeRef = useRef(0);
  const pendingFetchRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingDeselectRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const zoomTransformRef = useRef<{ k: number; x: number; y: number }>({ k: 1, x: 0, y: 0 });
  const rafPendingRef = useRef<number | null>(null);
  const lastProcessedZoomRef = useRef<number>(1);
  const zoomSettleTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const dotPositionsRef = useRef<{ x: number; y: number }[]>([]);

  // ==========================================================================
  // STATE
  // ==========================================================================
  const [hoveredNode, setHoveredNode] = useState<GraphNode | null>(null);

  // ==========================================================================
  // SYNC REFS WITH PROPS
  // ==========================================================================
  useEffect(() => {
    activeNodeIdRef.current = activeNodeId;
    connectionMapRef.current = connectionMap;
  }, [activeNodeId, connectionMap]);

  // ==========================================================================
  // MAIN D3 RENDER EFFECT
  // ==========================================================================
  useEffect(() => {
    devLog('info', 'GraphCanvas render effect running');
    if (!svgRef.current) return;

    const svg = d3.select(svgRef.current);
    svg.selectAll('*').remove(); // Always clear first

    if (graphNodes.length === 0) {
      devLog('info', 'No nodes to render - cleared canvas');
      return;
    }

    svg.attr('width', width).attr('height', height);

    const container = svg.append('g').attr('class', 'graph-container');

    devLog('info', `GraphCanvas: ${graphNodes.length} nodes, ${edgeData.length} edges to render`);

    // Create node lookup
    const nodeMap = new Map(graphNodes.map(n => [n.id, n]));

    // Calculate graph bounds for pan limiting
    const boundsPadding = NOTE_WIDTH * 40;
    const graphBounds = {
      minX: graphNodes.length > 0 ? Math.min(...graphNodes.map(n => n.x)) - boundsPadding : -1000,
      maxX: graphNodes.length > 0 ? Math.max(...graphNodes.map(n => n.x)) + boundsPadding : 1000,
      minY: graphNodes.length > 0 ? Math.min(...graphNodes.map(n => n.y)) - boundsPadding : -1000,
      maxY: graphNodes.length > 0 ? Math.max(...graphNodes.map(n => n.y)) + boundsPadding : 1000,
    };

    // ==========================================================================
    // EDGE RENDERING
    // ==========================================================================
    const linksGroup = container.append('g').attr('class', 'links');

    // Calculate min/max weights for normalization
    const relatedWeights = edgeData.filter(e => e.type !== 'contains').map(e => e.weight);
    const minWeight = relatedWeights.length > 0 ? Math.min(...relatedWeights) : 0;
    const maxWeight = relatedWeights.length > 0 ? Math.max(...relatedWeights) : 1;
    const weightRange = maxWeight - minWeight || 0.1;

    // Define arrow markers
    const defs = svg.append('defs');
    edgeData.forEach((d, i) => {
      const normalized = (d.weight - minWeight) / weightRange;
      const color = d.type === 'contains' ? '#6b7280' : getEdgeColor(normalized);
      const arrowSize = d.type === 'contains' ? 4 : 5 - normalized * 2;

      defs.append('marker')
        .attr('id', `arrow-${i}`)
        .attr('viewBox', '0 -5 10 10')
        .attr('refX', 8)
        .attr('refY', 0)
        .attr('markerWidth', arrowSize)
        .attr('markerHeight', arrowSize)
        .attr('orient', 'auto')
        .append('path')
        .attr('d', 'M0,-5L10,0L0,5')
        .attr('fill', color);
    });

    // Draw edge paths
    linksGroup.selectAll('path')
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
        if (d.type === 'contains') return 6;
        const normalized = (d.weight - minWeight) / weightRange;
        return 6 + normalized * 18;
      })
      .attr('opacity', (e: EdgeData) => {
        if (!activeNodeId) return 1;
        if (e.source.id === activeNodeId || e.target.id === activeNodeId) return 0.9;
        const srcConn = connectionMap.has(e.source.id);
        const tgtConn = connectionMap.has(e.target.id);
        if (srcConn && tgtConn) return 0.7;
        return 0.15;
      })
      .attr('d', d => {
        const points = getEdgePoints(d.source, d.target, NOTE_WIDTH, NOTE_HEIGHT);
        const dx = points.target.x - points.source.x;
        const dy = points.target.y - points.source.y;
        const dr = Math.sqrt(dx * dx + dy * dy) * 1.5;
        return `M${points.source.x},${points.source.y} A${dr},${dr} 0 0,1 ${points.target.x},${points.target.y}`;
      });

    // Connection point dots at endpoints
    linksGroup.selectAll('circle.connection-dot')
      .data(edgeData.flatMap(d => {
        const points = getEdgePoints(d.source, d.target, NOTE_WIDTH, NOTE_HEIGHT);
        const normalized = (d.weight - minWeight) / weightRange;
        const color = d.type === 'contains' ? '#6b7280' : getEdgeColor(normalized);
        return [
          { x: points.source.x, y: points.source.y, color },
          { x: points.target.x, y: points.target.y, color }
        ];
      }))
      .join('circle')
      .attr('class', 'connection-dot')
      .attr('cx', d => d.x)
      .attr('cy', d => d.y)
      .attr('r', 4)
      .attr('fill', d => d.color);

    // ==========================================================================
    // NODE RENDERING
    // ==========================================================================

    // Calculate min/max dates for date coloring
    const dates = graphNodes.map(n => n.createdAt);
    const minDate = Math.min(...dates);
    const maxDate = Math.max(...dates);

    // Create card and dot groups
    const cardsGroup = container.append('g').attr('class', 'cards');
    const dotsGroup = container.append('g').attr('class', 'dots');

    // Unified card rendering for ALL nodes
    const cardGroups = cardsGroup.selectAll('g.node-card')
      .data(graphNodes)
      .join('g')
      .attr('class', 'card')
      .attr('cursor', 'pointer')
      .attr('transform', d => `translate(${d.x - NOTE_WIDTH/2}, ${d.y - NOTE_HEIGHT/2})`);

    // === TOPIC SHADOWS (render order = z-order, first = bottom) ===

    // Violet shadow FIRST (ALL topics) - offset = 7 * depth
    cardGroups.filter(d => getStructuralDepth(d.childCount, d.isItem) >= 1)
      .append('rect')
      .attr('class', 'shadow-violet')
      .attr('x', d => 7 * getStructuralDepth(d.childCount, d.isItem))
      .attr('y', d => 7 * getStructuralDepth(d.childCount, d.isItem))
      .attr('width', NOTE_WIDTH)
      .attr('height', NOTE_HEIGHT)
      .attr('rx', 6)
      .attr('fill', '#5b21b6')
      .attr('stroke', d => d.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.2)')
      .attr('stroke-width', d => d.id === activeNodeId ? 2 : 1);

    // Cluster shadow 3 - for 16+ children
    cardGroups.filter(d => getStructuralDepth(d.childCount, d.isItem) >= 4)
      .append('rect')
      .attr('class', 'shadow-cluster')
      .attr('x', 21).attr('y', 21)
      .attr('width', NOTE_WIDTH).attr('height', NOTE_HEIGHT)
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
      .attr('x', 14).attr('y', 14)
      .attr('width', NOTE_WIDTH).attr('height', NOTE_HEIGHT)
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
      .attr('x', 7).attr('y', 7)
      .attr('width', NOTE_WIDTH).attr('height', NOTE_HEIGHT)
      .attr('rx', 6)
      .attr('fill', d => {
        const base = d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#374151';
        return d3.color(base)?.darker(1.0)?.toString() || '#3d3d3d';
      })
      .attr('stroke', d => d.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.15)')
      .attr('stroke-width', d => d.id === activeNodeId ? 2 : 1);

    // === ITEM SHADOW ===
    cardGroups.filter(d => getStructuralDepth(d.childCount, d.isItem) === 0)
      .append('rect')
      .attr('x', 2).attr('y', 2)
      .attr('width', NOTE_WIDTH).attr('height', NOTE_HEIGHT)
      .attr('rx', 6)
      .attr('fill', 'rgba(0,0,0,0.3)');

    // Card background
    cardGroups.append('rect')
      .attr('class', 'card-bg')
      .attr('width', NOTE_WIDTH).attr('height', NOTE_HEIGHT)
      .attr('rx', 6)
      .attr('fill', d => getNodeColor(d, activeNodeId, connectionMap))
      .attr('stroke', d => d.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.15)')
      .attr('stroke-width', d => d.id === activeNodeId ? 2 : 1);

    // Apply opacity to entire card group
    cardGroups.style('opacity', d => getNodeOpacity(d, activeNodeId, connectionMap));

    // Titlebar - darker area at top
    cardGroups.append('rect')
      .attr('width', NOTE_WIDTH).attr('height', 80)
      .attr('rx', 6)
      .attr('fill', 'rgba(0,0,0,0.4)');
    cardGroups.append('rect')
      .attr('y', 70).attr('width', NOTE_WIDTH).attr('height', 10)
      .attr('fill', 'rgba(0,0,0,0.4)');

    // Virtual DOM: Track node indices for limiting detailed content
    const nodeIndexMap = new Map(graphNodes.map((n, i) => [n.id, i]));
    const MAX_DETAILED_NODES = 150; // Only create full content for first N nodes

    // Large emoji
    cardGroups.append('text')
      .attr('x', 14).attr('y', 52)
      .attr('font-size', '42px')
      .text(d => d.displayEmoji);

    // Title - SVG text (replaces slow foreignObject)
    // Helper to truncate and wrap title into 2 lines
    const wrapTitle = (title: string, maxCharsPerLine: number = 18): string[] => {
      if (!title) return [''];
      if (title.length <= maxCharsPerLine) return [title];
      // Try to break at word boundary
      const words = title.split(' ');
      const lines: string[] = [];
      let currentLine = '';
      for (const word of words) {
        if (currentLine.length + word.length + 1 <= maxCharsPerLine) {
          currentLine += (currentLine ? ' ' : '') + word;
        } else if (lines.length === 0) {
          if (currentLine) lines.push(currentLine);
          currentLine = word;
        } else {
          break; // Already have 2 lines
        }
      }
      if (currentLine) lines.push(currentLine.slice(0, maxCharsPerLine));
      // Truncate second line if needed
      if (lines.length > 1 && lines[1].length > maxCharsPerLine - 3) {
        lines[1] = lines[1].slice(0, maxCharsPerLine - 3) + '...';
      }
      return lines.slice(0, 2);
    };

    // Create title text group for each card
    const titleGroups = cardGroups.append('g')
      .attr('class', 'title-group')
      .attr('transform', `translate(${58 + (NOTE_WIDTH - 68) / 2}, 40)`);

    titleGroups.each(function(d) {
      const g = d3.select(this);
      const lines = wrapTitle(d.displayTitle);
      const lineHeight = 26;
      const startY = lines.length === 1 ? 0 : -lineHeight / 2;

      lines.forEach((line, i) => {
        g.append('text')
          .attr('class', 'title-text')
          .attr('x', 0)
          .attr('y', startY + i * lineHeight)
          .attr('text-anchor', 'middle')
          .attr('font-family', CARD_FONT)
          .attr('font-size', '22px')
          .attr('font-weight', '600')
          .attr('fill', '#ffffff')
          .style('pointer-events', 'none')
          .text(line);
      });
    });

    // Synopsis area background
    const synopsisHeight = NOTE_HEIGHT - 148;
    cardGroups.append('rect')
      .attr('class', 'synopsis-bg')
      .attr('x', 0).attr('y', 80)
      .attr('width', NOTE_WIDTH).attr('height', synopsisHeight)
      .attr('fill', 'rgba(0,0,0,0.2)');

    // Synopsis - SVG text (only for first N nodes - virtual DOM optimization)
    const synopsisGroups = cardGroups
      .filter(d => (nodeIndexMap.get(d.id) ?? 0) < MAX_DETAILED_NODES)
      .append('g')
      .attr('class', 'synopsis-group')
      .attr('transform', 'translate(14, 100)');

    synopsisGroups.each(function(d) {
      if (!d.displayContent) return;
      const g = d3.select(this);
      const items = d.displayContent.split(', ').filter(s => s.trim()).slice(0, 5);
      const maxChars = 24;
      const lineHeight = 28;

      items.forEach((item, i) => {
        const truncated = item.length > maxChars ? item.slice(0, maxChars - 3) + '...' : item;
        g.append('text')
          .attr('x', 0)
          .attr('y', i * lineHeight)
          .attr('font-family', CARD_FONT)
          .attr('font-size', '20px')
          .attr('font-weight', '500')
          .attr('fill', '#ffffff')
          .style('pointer-events', 'none')
          .text(`• ${truncated}`);
      });
    });

    // Footer background
    cardGroups.append('rect')
      .attr('class', 'footer-bg')
      .attr('x', 8).attr('y', NOTE_HEIGHT - 34)
      .attr('width', NOTE_WIDTH - 16).attr('height', 26)
      .attr('rx', 4).attr('ry', 4)
      .attr('fill', 'rgba(0, 0, 0, 0.5)');

    // Footer left text
    cardGroups.append('text')
      .attr('class', 'footer-left')
      .attr('x', 14).attr('y', NOTE_HEIGHT - 16)
      .attr('font-family', CARD_FONT)
      .attr('font-size', '17px')
      .attr('fill', d => d.childCount > 0 ? 'rgba(255,255,255,0.7)' : getDateColor(d.createdAt, minDate, maxDate))
      .text(d => d.childCount > 0 ? `${d.childCount} items` : new Date(d.createdAt).toLocaleDateString());

    // Footer right text (latest date for groups)
    cardGroups.append('text')
      .attr('class', 'footer-right')
      .attr('x', NOTE_WIDTH - 14).attr('y', NOTE_HEIGHT - 16)
      .attr('text-anchor', 'end')
      .attr('font-family', CARD_FONT)
      .attr('font-size', '17px')
      .attr('fill', d => (d.childCount > 0 && d.latestChildDate) ? getDateColor(d.latestChildDate, minDate, maxDate) : 'transparent')
      .text(d => (d.childCount > 0 && d.latestChildDate) ? `Latest: ${new Date(d.latestChildDate).toLocaleDateString()}` : '');

    // "NOTE" badge for items
    const noteBadges = cardGroups.filter(d => d.isItem && d.childCount === 0);
    noteBadges.append('rect')
      .attr('x', NOTE_WIDTH / 2 - 36).attr('y', NOTE_HEIGHT - 34)
      .attr('width', 72).attr('height', 26)
      .attr('rx', 4)
      .attr('fill', '#5b21b6');
    noteBadges.append('text')
      .attr('x', NOTE_WIDTH / 2).attr('y', NOTE_HEIGHT - 15)
      .attr('text-anchor', 'middle')
      .attr('font-family', CARD_FONT)
      .attr('font-size', '18px')
      .attr('font-weight', '700')
      .attr('letter-spacing', '1.5px')
      .attr('fill', '#ffffff')
      .text('NOTE');

    // Menu button for items
    const menuBtnGroups = cardGroups.filter(d => d.childCount === 0)
      .append('g')
      .attr('class', 'node-menu-btn')
      .attr('cursor', 'pointer')
      .attr('transform', `translate(${NOTE_WIDTH - 36}, ${NOTE_HEIGHT - 32})`);
    menuBtnGroups.append('rect')
      .attr('width', 28).attr('height', 22)
      .attr('rx', 4)
      .attr('fill', 'rgba(255,255,255,0.1)');
    menuBtnGroups.append('text')
      .attr('x', 14).attr('y', 16)
      .attr('text-anchor', 'middle')
      .attr('font-size', '16px')
      .attr('font-weight', 'bold')
      .attr('fill', 'rgba(255,255,255,0.6)')
      .text('•••');

    // === DOT RENDERING (zoomed out view) ===
    const dotGroups = dotsGroup.selectAll('g.node-dot')
      .data(graphNodes)
      .join('g')
      .attr('class', 'dot')
      .attr('cursor', 'pointer')
      .attr('transform', d => `translate(${d.x}, ${d.y})`);

    // Violet ring for topics
    dotGroups.filter(d => getStructuralDepth(d.childCount, d.isItem) >= 1)
      .append('circle')
      .attr('class', 'dot-violet')
      .attr('cx', d => 2 * getStructuralDepth(d.childCount, d.isItem))
      .attr('cy', d => 2 * getStructuralDepth(d.childCount, d.isItem))
      .attr('r', DOT_SIZE + 2)
      .attr('fill', 'none')
      .attr('stroke', '#5b21b6')
      .attr('stroke-width', 4);

    // Stack circles for deep topics
    dotGroups.filter(d => getStructuralDepth(d.childCount, d.isItem) >= 3)
      .append('circle')
      .attr('class', 'dot-stack-2')
      .attr('cx', 6).attr('cy', 6)
      .attr('r', DOT_SIZE - 2)
      .attr('fill', d => {
        const base = d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#374151';
        return d3.color(base)?.darker(1.5)?.toString() || '#1a1a1a';
      })
      .attr('stroke', 'rgba(255,255,255,0.2)')
      .attr('stroke-width', 1);

    dotGroups.filter(d => getStructuralDepth(d.childCount, d.isItem) >= 2)
      .append('circle')
      .attr('class', 'dot-stack-1')
      .attr('cx', 3).attr('cy', 3)
      .attr('r', DOT_SIZE - 1)
      .attr('fill', d => {
        const base = d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#374151';
        return d3.color(base)?.darker(0.8)?.toString() || '#252525';
      })
      .attr('stroke', 'rgba(255,255,255,0.2)')
      .attr('stroke-width', 1);

    // Main dot with glow
    dotGroups.append('circle')
      .attr('class', 'dot-glow')
      .attr('r', DOT_SIZE + 4)
      .attr('fill', 'none')
      .attr('stroke', d => getNodeColor(d, activeNodeId, connectionMap))
      .attr('stroke-width', 3)
      .attr('stroke-opacity', 0.3);

    dotGroups.append('circle')
      .attr('class', 'dot-main')
      .attr('r', DOT_SIZE)
      .attr('fill', d => getNodeColor(d, activeNodeId, connectionMap))
      .attr('stroke', d => d.id === activeNodeId ? '#fbbf24' : 'rgba(255,255,255,0.6)')
      .attr('stroke-width', d => d.id === activeNodeId ? 3 : 1.5);

    dotGroups.style('opacity', d => getNodeOpacity(d, activeNodeId, connectionMap));

    dotGroups.append('text')
      .attr('text-anchor', 'middle')
      .attr('dy', '0.35em')
      .attr('font-size', '18px')
      .attr('fill', '#fff')
      .text(d => d.displayEmoji);

    // Start with cards shown, dots hidden
    dotsGroup.style('display', 'none');

    // ==========================================================================
    // CLICK HANDLERS
    // ==========================================================================

    // Prevent text selection on mousedown (must happen before click/dblclick)
    cardGroups.on('mousedown', function(event) {
      event.preventDefault(); // Prevent text selection on all card clicks
    });

    // Card click - select node
    cardGroups.on('click', function(event, d) {
      event.stopPropagation();

      // If clicking already-selected node, defer deselection to allow double-click
      if (activeNodeIdRef.current === d.id) {
        // Clear any pending deselect and set a new one
        if (pendingDeselectRef.current) clearTimeout(pendingDeselectRef.current);
        pendingDeselectRef.current = setTimeout(() => {
          // Only deselect if still selected (double-click handler may have navigated away)
          if (activeNodeIdRef.current === d.id) {
            onSelectNode(null);
          }
          pendingDeselectRef.current = null;
        }, 250); // Wait for potential double-click
        return;
      }

      // Select node
      onSelectNode(d.id);

      // Defer similar nodes fetch
      if (pendingFetchRef.current) clearTimeout(pendingFetchRef.current);
      pendingFetchRef.current = setTimeout(() => onFetchSimilarNodes(d.id), 50);
    });

    // Card double-click - navigate/open
    cardGroups.on('dblclick', function(event, d) {
      event.stopPropagation();
      event.preventDefault(); // Prevent text selection
      // Cancel pending deselection and fetch
      if (pendingDeselectRef.current) {
        clearTimeout(pendingDeselectRef.current);
        pendingDeselectRef.current = null;
      }
      if (pendingFetchRef.current) {
        clearTimeout(pendingFetchRef.current);
        pendingFetchRef.current = null;
      }
      if (d.isItem) {
        devLog('info', `Opening item "${d.displayTitle}" in Leaf mode`);
        onOpenLeaf(d.id);
      } else if (d.childCount > 0) {
        devLog('info', `Drilling into "${d.displayTitle}"`);
        onSelectNode(null);
        onNavigateToNode(d);
      }
    });

    // Prevent text selection on dots too
    dotGroups.on('mousedown', function(event) {
      event.preventDefault();
    });

    // Dot click - same as card
    dotGroups.on('click', function(event, d) {
      event.stopPropagation();

      // If clicking already-selected node, defer deselection to allow double-click
      if (activeNodeIdRef.current === d.id) {
        if (pendingDeselectRef.current) clearTimeout(pendingDeselectRef.current);
        pendingDeselectRef.current = setTimeout(() => {
          if (activeNodeIdRef.current === d.id) {
            onSelectNode(null);
          }
          pendingDeselectRef.current = null;
        }, 250);
        return;
      }

      onSelectNode(d.id);
      if (pendingFetchRef.current) clearTimeout(pendingFetchRef.current);
      pendingFetchRef.current = setTimeout(() => onFetchSimilarNodes(d.id), 50);
    });

    // Dot double-click - same as card
    dotGroups.on('dblclick', function(event, d) {
      event.stopPropagation();
      event.preventDefault(); // Prevent text selection
      // Cancel pending deselection and fetch
      if (pendingDeselectRef.current) {
        clearTimeout(pendingDeselectRef.current);
        pendingDeselectRef.current = null;
      }
      if (pendingFetchRef.current) {
        clearTimeout(pendingFetchRef.current);
        pendingFetchRef.current = null;
      }
      if (d.isItem) {
        onOpenLeaf(d.id);
      } else if (d.childCount > 0) {
        onSelectNode(null);
        onNavigateToNode(d);
      }
    });

    // Menu button handlers
    menuBtnGroups
      .on('click', function(event, d) {
        event.stopPropagation();
        const rect = (event.target as SVGElement).getBoundingClientRect();
        onShowContextMenu(d.id, { x: rect.right + 10, y: rect.top });
      })
      .on('mouseenter', function() {
        d3.select(this).select('rect').attr('fill', 'rgba(255,255,255,0.25)');
        d3.select(this).select('text').attr('fill', 'rgba(255,255,255,0.9)');
      })
      .on('mouseleave', function() {
        d3.select(this).select('rect').attr('fill', 'rgba(255,255,255,0.1)');
        d3.select(this).select('text').attr('fill', 'rgba(255,255,255,0.6)');
      });

    // ==========================================================================
    // HOVER HANDLERS
    // ==========================================================================

    // Card hover - show tooltip and highlight
    cardGroups
      .on('mouseenter', function(_, d) {
        setHoveredNode(d);
        // Highlight hovered card
        d3.select(this).select('.card-bg')
          .attr('fill', d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#4b5563');
      })
      .on('mouseleave', function(_, d) {
        setHoveredNode(null);
        // Restore color based on selection state
        const isSelected = d.id === activeNodeIdRef.current;
        if (isSelected) {
          d3.select(this).select('.card-bg')
            .attr('fill', d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#4b5563');
        } else {
          const conn = connectionMapRef.current.get(d.id);
          let color: string;
          if (conn) {
            color = conn.distance === 1 ? getDirectConnectionColor(conn.weight) : getChainConnectionColor(conn.distance);
          } else {
            color = getMutedClusterColor(d);
          }
          d3.select(this).select('.card-bg').attr('fill', color);
        }
      });

    // Dot hover - same as card
    dotGroups
      .on('mouseenter', function(_, d) {
        setHoveredNode(d);
        d3.select(this).select('.dot-main')
          .attr('fill', d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#4b5563');
      })
      .on('mouseleave', function(_, d) {
        setHoveredNode(null);
        const isSelected = d.id === activeNodeIdRef.current;
        if (isSelected) {
          d3.select(this).select('.dot-main')
            .attr('fill', d.renderClusterId >= 0 ? generateClusterColor(d.renderClusterId) : '#4b5563');
        } else {
          const conn = connectionMapRef.current.get(d.id);
          let color: string;
          if (conn) {
            color = conn.distance === 1 ? getDirectConnectionColor(conn.weight) : getChainConnectionColor(conn.distance);
          } else {
            color = getMutedClusterColor(d);
          }
          d3.select(this).select('.dot-main').attr('fill', color);
        }
      });

    // Store edge paths selection for zoom updates
    const edgePaths = linksGroup.selectAll('path');
    const connectionDots = linksGroup.selectAll('circle.connection-dot');

    // ==========================================================================
    // ZOOM AND PAN
    // ==========================================================================

    // Level-of-detail thresholds
    const bubbleThreshold = 0.12;
    const curveStart = 0.14;
    const curveEnd = 0.35;
    const minApparentSize = 0.75;
    const sparseMinApparentSize = 0.52;

    const zoom = d3.zoom<SVGSVGElement, unknown>()
      .scaleExtent([MIN_ZOOM, MAX_ZOOM])
      .translateExtent([[graphBounds.minX, graphBounds.minY], [graphBounds.maxX, graphBounds.maxY]])
      .filter((event) => {
        if (event.type === 'wheel') return true;
        const target = event.target as Element;
        if (target.closest('.card') || target.closest('.dot')) return false;
        return true;
      })
      .on('zoom', (event) => {
        // Store transform in ref - actual DOM update happens in RAF
        zoomTransformRef.current = { k: event.transform.k, x: event.transform.x, y: event.transform.y };

        // Batch ALL DOM updates into requestAnimationFrame (max 60fps)
        if (rafPendingRef.current !== null) {
          cancelAnimationFrame(rafPendingRef.current);
        }

        rafPendingRef.current = requestAnimationFrame(() => {
          rafPendingRef.current = null;
          const { k, x, y } = zoomTransformRef.current;

          // Apply container transform (now batched with other updates)
          container.attr('transform', `translate(${x},${y}) scale(${k})`);

          // === OPTIMIZATION 1: Skip ALL work if zoom delta < 2% ===
          const zoomDelta = Math.abs(k - lastProcessedZoomRef.current) / lastProcessedZoomRef.current;
          if (zoomDelta < 0.02) {
            return; // Skip everything for tiny zoom changes - no React re-renders
          }
          lastProcessedZoomRef.current = k;

          // === OPTIMIZATION 2: Merge viewport culling into single pass ===
          const viewportBuffer = 400;
          const visibleIds = new Set<string>();
          graphNodes.forEach(node => {
            const screenX = node.x * k + x;
            const screenY = node.y * k + y;
            if (screenX > -viewportBuffer && screenX < width + viewportBuffer &&
                screenY > -viewportBuffer && screenY < height + viewportBuffer) {
              visibleIds.add(node.id);
            }
          });

          // Determine mode once, then only update the active mode's elements
          const isCardMode = k >= bubbleThreshold;

          if (isCardMode) {
            // Card mode - only update cards, not dots
            dotsGroup.style('display', 'none');
            cardsGroup.style('display', null);

            // Viewport culling for cards only
            cardGroups.style('display', d => visibleIds.has(d.id) ? null : 'none');

            const smoothstep = (edge0: number, edge1: number, val: number) => {
              const t = Math.max(0, Math.min(1, (val - edge0) / (edge1 - edge0)));
              return t * t * (3 - 2 * t);
            };
            const blendFactor = smoothstep(curveStart, curveEnd, k);
            const effectiveMinSize = sparseMinApparentSize + blendFactor * (minApparentSize - sparseMinApparentSize);
            const positionScale = k < curveEnd ? Math.sqrt(curveEnd / k) : 1;

            let cardScale = 1;
            if (k < 1) {
              const targetApparent = Math.max(effectiveMinSize, Math.sqrt(k));
              cardScale = targetApparent / k;
            }

            cardGroups.attr('transform', d =>
              `translate(${d.x * positionScale}, ${d.y * positionScale}) scale(${cardScale}) translate(${-NOTE_WIDTH/2}, ${-NOTE_HEIGHT/2})`
            );

            // === OPTIMIZATION 3: Fade edges during active zoom, recalc on settle ===
            linksGroup.style('opacity', 0.15);
            connectionDots.style('opacity', 0.15);

            // Clear existing settle timeout
            if (zoomSettleTimeoutRef.current) {
              clearTimeout(zoomSettleTimeoutRef.current);
            }

            // Schedule edge update for when zoom settles
            zoomSettleTimeoutRef.current = setTimeout(() => {
              const scaledWidth = NOTE_WIDTH * cardScale;
              const scaledHeight = NOTE_HEIGHT * cardScale;

              // Pre-calculate all edge points once
              const edgePointsCache = edgeData.map(d => {
                const scaledSource = { x: d.source.x * positionScale, y: d.source.y * positionScale };
                const scaledTarget = { x: d.target.x * positionScale, y: d.target.y * positionScale };
                return getEdgePoints(scaledSource, scaledTarget, scaledWidth, scaledHeight);
              });

              // Use cached points for edge paths
              edgePaths.attr('d', (_d: EdgeData, i: number) => {
                const points = edgePointsCache[i];
                const dx = points.target.x - points.source.x;
                const dy = points.target.y - points.source.y;
                const dr = Math.sqrt(dx * dx + dy * dy) * 1.5;
                return `M${points.source.x},${points.source.y} A${dr},${dr} 0 0,1 ${points.target.x},${points.target.y}`;
              });

              // === OPTIMIZATION 4: Reuse dot positions array ===
              const neededLength = edgeData.length * 2;
              if (dotPositionsRef.current.length !== neededLength) {
                dotPositionsRef.current = new Array(neededLength).fill(null).map(() => ({ x: 0, y: 0 }));
              }
              edgePointsCache.forEach((points, i) => {
                dotPositionsRef.current[i * 2].x = points.source.x;
                dotPositionsRef.current[i * 2].y = points.source.y;
                dotPositionsRef.current[i * 2 + 1].x = points.target.x;
                dotPositionsRef.current[i * 2 + 1].y = points.target.y;
              });

              connectionDots
                .attr('r', 4 * cardScale)
                .attr('cx', (_d: unknown, i: number) => dotPositionsRef.current[i]?.x ?? 0)
                .attr('cy', (_d: unknown, i: number) => dotPositionsRef.current[i]?.y ?? 0);

              // Fade edges back in
              linksGroup.transition().duration(100).style('opacity', 1);
              connectionDots.transition().duration(100).style('opacity', 1);
            }, 120); // Recalculate edges after 120ms of no zoom

            // Show links group but faded
            linksGroup.style('display', null);

          } else {
            // Bubble mode - only update dots, not cards
            cardsGroup.style('display', 'none');
            dotsGroup.style('display', null);
            linksGroup.style('display', 'none');
            connectionDots.style('display', 'none');

            // Viewport culling for dots only
            dotGroups.style('display', d => visibleIds.has(d.id) ? null : 'none');

            const positionScale = Math.sqrt(curveEnd / k);
            const targetScreenSize = 40;
            const bubbleScale = targetScreenSize / (DOT_SIZE * 2 * k);

            dotGroups.attr('transform', d =>
              `translate(${d.x * positionScale}, ${d.y * positionScale}) scale(${bubbleScale})`
            );
          }

          onZoomChange(k);
        });
      });

    svg.call(zoom as any);

    // Handle background click to deselect
    svg.on('click', function(event) {
      if (event.target === svgRef.current) {
        onSelectNode(null);
        devLog('info', 'Background click - deselected');
      }
    });

    // ==========================================================================
    // FIT TO VIEW
    // ==========================================================================
    if (graphNodes.length > 0) {
      const xs = graphNodes.map(n => n.x);
      const ys = graphNodes.map(n => n.y);
      const padding = NOTE_WIDTH;

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

      svg.call(zoom.transform as any, initialTransform);
    } else {
      devLog('warn', 'No nodes to display');
    }

  // Note: activeNodeId and connectionMap removed from dependencies - color updates handled by separate effect
  // This prevents clicking a node from resetting the viewport (connectionMap changes on selection)
  }, [graphNodes, edgeData, width, height, devLog, onSelectNode, onZoomChange, onNavigateToNode, onOpenLeaf, onFetchSimilarNodes, onShowContextMenu]);

  // ==========================================================================
  // COLOR UPDATE EFFECT - Updates colors when selection changes
  // ==========================================================================
  useEffect(() => {
    if (!svgRef.current || graphNodes.length === 0) return;

    const svg = d3.select(svgRef.current);

    // Helper to get color from node data
    const getColorFromData = (data: GraphNode | null): string => {
      if (!data) return '#374151';
      return getNodeColor(data, activeNodeId, connectionMap);
    };

    // Helper to get opacity from node data
    const getOpacityFromData = (data: GraphNode | null): number => {
      if (!data) return 1;
      return getNodeOpacity(data, activeNodeId, connectionMap);
    };

    // Update card background colors with transition
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

    // Update shadow-violet strokes (main violet layer)
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

    // Update shadow-cluster strokes (depth layers)
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

    // Update card group opacity
    svg.selectAll<SVGGElement, GraphNode>('g.card')
      .transition().duration(150)
      .style('opacity', (d: GraphNode) => getOpacityFromData(d));

    // Update dot glow and main colors
    svg.selectAll<SVGCircleElement, GraphNode>('.dot-glow')
      .transition().duration(150)
      .attr('stroke', function(this: SVGCircleElement) {
        const parentEl = this.parentNode as Element;
        if (!parentEl) return '#374151';
        const data = d3.select<Element, GraphNode>(parentEl).datum();
        return getColorFromData(data);
      });

    svg.selectAll<SVGCircleElement, GraphNode>('.dot-main')
      .transition().duration(150)
      .attr('fill', function(this: SVGCircleElement) {
        const parentEl = this.parentNode as Element;
        if (!parentEl) return '#374151';
        const data = d3.select<Element, GraphNode>(parentEl).datum();
        return getColorFromData(data);
      });

    // Update dot group opacity
    svg.selectAll<SVGGElement, GraphNode>('g.dot')
      .transition().duration(150)
      .style('opacity', (d: GraphNode) => getOpacityFromData(d));

  }, [activeNodeId, connectionMap, graphNodes]);

  // ==========================================================================
  // RENDER
  // ==========================================================================
  return (
    <>
      <svg
        ref={svgRef}
        className="bg-gray-900 w-full h-full"
        style={{ cursor: 'grab', willChange: 'transform' }}
      />

      {/* Hovered node tooltip */}
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
    </>
  );
});
