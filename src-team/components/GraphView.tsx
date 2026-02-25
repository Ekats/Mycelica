import { useEffect, useRef, useCallback } from "react";
import * as d3 from "d3";
import { useTeamStore } from "../stores/teamStore";
import type { DisplayNode, DisplayEdge } from "../types";

const TEAM_COLOR = "#60a5fa";
const CATEGORY_COLOR = "#ef4444";
const PERSONAL_COLOR = "#14b8a6";

/**
 * Heat color for normalized values (0-1).
 * Red (0%) → Yellow (50%) → Cyan (100%)
 * Matches the original Mycelica app's edge coloring.
 */
function getHeatColor(t: number, saturation = 75, lightness = 65): string {
  t = Math.max(0, Math.min(1, t));
  let hue: number;
  if (t < 0.5) {
    hue = t * 2 * 60; // red (0°) → yellow (60°)
  } else {
    hue = 60 + (t - 0.5) * 2 * 120; // yellow (60°) → cyan (180°)
  }
  return `hsl(${hue}, ${saturation}%, ${lightness}%)`;
}

interface GraphNode {
  id: string;
  title: string;
  isPersonal: boolean;
  isItem: boolean;
  childCount: number;
  author?: string;
  contentType?: string;
  emoji?: string;
  source?: string;
  body?: string; // full message content (signal)
  edgeCount: number;
  x: number;
  y: number;
  rectW?: number;
  rectH?: number;
}

// Signal message rectangle constants
const MSG_WIDTH = 500;
const MSG_PAD = 12;
const MSG_FONT_SIZE = 13;
const MSG_LINE_HEIGHT = MSG_FONT_SIZE * 1.4;
const MSG_AUTHOR_HEIGHT = 20;
const MSG_GAP = 10;
const MSG_CHARS_PER_LINE = Math.floor((MSG_WIDTH - MSG_PAD * 2) / (MSG_FONT_SIZE * 0.62));

function estimateMsgHeight(text: string): number {
  const lines = Math.max(1, Math.ceil(text.length / MSG_CHARS_PER_LINE));
  return Math.min(MSG_AUTHOR_HEIGHT + lines * MSG_LINE_HEIGHT + MSG_PAD * 2, 300);
}

// Deterministic author colors from first letter of username
function authorHue(author: string): number {
  const code = (author || "?").charCodeAt(0);
  return (code * 137) % 360; // golden-angle spread
}
function authorBg(author: string): string {
  const h = authorHue(author);
  return `hsla(${h}, 40%, 22%, 0.92)`;
}
function authorText(author: string): string {
  const h = authorHue(author);
  return `hsl(${h}, 70%, 65%)`;
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

// Edge anchor helpers — port from src/components/graph/GraphCanvas.tsx
function getSideCenters(cx: number, cy: number, w: number, h: number) {
  const hw = w / 2, hh = h / 2;
  return [
    { x: cx + hw, y: cy },  // right
    { x: cx - hw, y: cy },  // left
    { x: cx, y: cy + hh },  // bottom
    { x: cx, y: cy - hh },  // top
  ];
}

function getEdgeEndpoints(
  src: GraphNode, tgt: GraphNode, isSignal: boolean
): { sx: number; sy: number; tx: number; ty: number } {
  if (isSignal) {
    const srcW = src.rectW ?? MSG_WIDTH;
    const srcH = src.rectH ?? 50;
    const tgtW = tgt.rectW ?? MSG_WIDTH;
    const tgtH = tgt.rectH ?? 50;
    const sidesA = getSideCenters(src.x, src.y, srcW, srcH);
    const sidesB = getSideCenters(tgt.x, tgt.y, tgtW, tgtH);
    let bestA = sidesA[0], bestB = sidesB[0], minDist = Infinity;
    for (const a of sidesA) {
      for (const b of sidesB) {
        const d = Math.hypot(a.x - b.x, a.y - b.y);
        if (d < minDist) { minDist = d; bestA = a; bestB = b; }
      }
    }
    return { sx: bestA.x, sy: bestA.y, tx: bestB.x, ty: bestB.y };
  } else {
    const rS = Math.min(64 + src.edgeCount * 4, 96);
    const rT = Math.min(64 + tgt.edgeCount * 4, 96);
    const dx = tgt.x - src.x, dy = tgt.y - src.y;
    const dist = Math.hypot(dx, dy);
    if (dist === 0) return { sx: src.x, sy: src.y, tx: tgt.x, ty: tgt.y };
    const nx = dx / dist, ny = dy / dist;
    return {
      sx: src.x + nx * rS, sy: src.y + ny * rS,
      tx: tgt.x - nx * rT, ty: tgt.y - ny * rT,
    };
  }
}

interface GraphLink {
  id: string;
  sourceId: string;
  targetId: string;
  type: string;
  weight: number;
  isPersonal: boolean;
  isHuman: boolean;
}

// --- Clustered Ring Layout ---
const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5)); // ~137.5°
const MICRO_RING_SPACING = 190;
const MACRO_PAD = 60;

/** How many clusters fit on a given macro ring index */
function macroRingCapacity(ring: number): number {
  if (ring === 0) return 1;
  if (ring === 1) return 4;
  return Math.min(8, 4 + (ring - 1) * 2);
}

/** How many members fit on a given micro ring index */
function microRingCapacity(ring: number): number {
  if (ring === 0) return 1; // hub
  if (ring === 1) return 6;
  return Math.min(10, 6 + (ring - 1) * 2);
}

/** Compute footprint radius of a cluster (how far its micro rings extend) */
function clusterFootprint(size: number): number {
  if (size <= 1) return 0;
  // Count how many micro rings are needed
  let placed = 1; // hub at center
  let ring = 1;
  while (placed < size) {
    placed += microRingCapacity(ring);
    ring++;
  }
  return (ring - 1) * MICRO_RING_SPACING;
}

type ClusterResult = { clusters: string[][]; hubId: Map<string, string>; degrees: Map<string, number> };

/** Group signal nodes by thread_id from tags JSON. Hub = most-replied-to, fallback to earliest sequenceIndex. */
function findSignalClusters(
  displayNodes: DisplayNode[],
  edges: DisplayEdge[],
): ClusterResult {
  // Only cluster signal nodes — personal/other nodes are excluded from signal view
  const signalNodes = displayNodes.filter((n) => n.source === 'signal');
  const nodeMap = new Map(signalNodes.map((n) => [n.id, n]));

  // Count replies_to edges per node (for hub selection)
  const replyCounts = new Map<string, number>();
  for (const e of edges) {
    if (e.type === 'replies_to') {
      replyCounts.set(e.target, (replyCounts.get(e.target) || 0) + 1);
    }
  }

  // Degree counts (for micro-ring member ordering)
  const degrees = new Map<string, number>();
  for (const n of signalNodes) degrees.set(n.id, 0);
  for (const e of edges) {
    if (e.source === e.target) continue;
    degrees.set(e.source, (degrees.get(e.source) || 0) + 1);
    degrees.set(e.target, (degrees.get(e.target) || 0) + 1);
  }

  // Group by thread_id
  const threadGroups = new Map<string, string[]>();
  const noThread: string[] = [];
  for (const n of signalNodes) {
    let threadId: string | undefined;
    if (n.tags) {
      try {
        const parsed = JSON.parse(n.tags);
        threadId = parsed.thread_id;
      } catch { /* ignore */ }
    }
    if (threadId) {
      let group = threadGroups.get(threadId);
      if (!group) { group = []; threadGroups.set(threadId, group); }
      group.push(n.id);
    } else {
      noThread.push(n.id);
    }
  }

  // Assign orphan nodes (e.g. link nodes without thread_id) to their parent's cluster
  // via shares_link edges: message → link node
  const nodeToThread = new Map<string, string>();
  for (const [tid, group] of threadGroups) {
    for (const id of group) nodeToThread.set(id, tid);
  }
  const stillOrphan: string[] = [];
  for (const id of noThread) {
    // Find a shares_link edge connecting this node to a threaded node
    let assigned = false;
    for (const e of edges) {
      if (e.type !== 'shares_link') continue;
      const peer = e.source === id ? e.target : e.target === id ? e.source : null;
      if (peer && nodeToThread.has(peer)) {
        const tid = nodeToThread.get(peer)!;
        threadGroups.get(tid)!.push(id);
        nodeToThread.set(id, tid);
        assigned = true;
        break;
      }
    }
    if (!assigned) stillOrphan.push(id);
  }

  // Sort members within each thread by sequenceIndex (chronological)
  for (const group of threadGroups.values()) {
    group.sort((a, b) => (nodeMap.get(a)?.sequenceIndex ?? 0) - (nodeMap.get(b)?.sequenceIndex ?? 0));
  }

  // Merge tiny threads (≤2 messages) into the chronologically nearest larger thread
  const MIN_THREAD_SIZE = 3;
  const largeThreads: string[][] = [];
  const tinyThreads: string[][] = [];
  for (const group of threadGroups.values()) {
    if (group.length >= MIN_THREAD_SIZE) largeThreads.push(group);
    else tinyThreads.push(group);
  }
  // For each tiny thread, find the large thread whose time range is closest
  const threadMidTime = (ids: string[]) => {
    const times = ids.map((id) => nodeMap.get(id)?.sequenceIndex ?? 0).filter(Boolean);
    if (!times.length) return 0;
    return times.reduce((a, b) => a + b, 0) / times.length;
  };
  for (const tiny of tinyThreads) {
    const tinyMid = threadMidTime(tiny);
    let bestThread = largeThreads[0];
    let bestDist = Infinity;
    for (const large of largeThreads) {
      const dist = Math.abs(threadMidTime(large) - tinyMid);
      if (dist < bestDist) { bestDist = dist; bestThread = large; }
    }
    if (bestThread) {
      bestThread.push(...tiny);
    } else {
      largeThreads.push(tiny); // No large threads exist — keep as-is
    }
  }
  // Re-sort merged threads by sequenceIndex
  for (const group of largeThreads) {
    group.sort((a, b) => (nodeMap.get(a)?.sequenceIndex ?? 0) - (nodeMap.get(b)?.sequenceIndex ?? 0));
  }

  // Convert to clusters array, sorted by size (largest first)
  const clusters = [...largeThreads];
  // Add remaining orphans as singletons
  for (const id of stillOrphan) clusters.push([id]);
  clusters.sort((a, b) => b.length - a.length);

  // Hub = most-replied-to in thread, fallback to first message (earliest sequenceIndex)
  const hubId = new Map<string, string>();
  for (const cluster of clusters) {
    let bestId = cluster[0]; // first message by sequenceIndex (already sorted)
    let bestReplies = replyCounts.get(bestId) || 0;
    for (const id of cluster) {
      const rc = replyCounts.get(id) || 0;
      if (rc > bestReplies) {
        bestId = id;
        bestReplies = rc;
      }
    }
    hubId.set(cluster[0], bestId);
  }

  return { clusters, hubId, degrees };
}

/** Find connected components via BFS. Used for non-signal views. */
function findClustersBFS(
  nodeIds: Set<string>,
  edges: DisplayEdge[],
): ClusterResult {
  // Build adjacency
  const adj = new Map<string, Set<string>>();
  for (const id of nodeIds) adj.set(id, new Set());
  const degrees = new Map<string, number>();
  for (const id of nodeIds) degrees.set(id, 0);

  for (const e of edges) {
    if (!nodeIds.has(e.source) || !nodeIds.has(e.target)) continue;
    if (e.source === e.target) continue;
    adj.get(e.source)!.add(e.target);
    adj.get(e.target)!.add(e.source);
    degrees.set(e.source, (degrees.get(e.source) || 0) + 1);
    degrees.set(e.target, (degrees.get(e.target) || 0) + 1);
  }

  // BFS connected components
  const visited = new Set<string>();
  const clusters: string[][] = [];
  for (const id of nodeIds) {
    if (visited.has(id)) continue;
    const component: string[] = [];
    const queue = [id];
    visited.add(id);
    while (queue.length > 0) {
      const cur = queue.shift()!;
      component.push(cur);
      for (const neighbor of adj.get(cur) || []) {
        if (!visited.has(neighbor)) {
          visited.add(neighbor);
          queue.push(neighbor);
        }
      }
    }
    clusters.push(component);
  }

  // Sort clusters by size (largest first)
  clusters.sort((a, b) => b.length - a.length);

  // Identify hub for each cluster (highest degree)
  const hubId = new Map<string, string>();
  for (const cluster of clusters) {
    let bestId = cluster[0];
    let bestDeg = degrees.get(bestId) || 0;
    for (const id of cluster) {
      const deg = degrees.get(id) || 0;
      if (deg > bestDeg) {
        bestId = id;
        bestDeg = deg;
      }
    }
    hubId.set(cluster[0], bestId);
  }

  return { clusters, hubId, degrees };
}

interface ClusterBoundary {
  cx: number; cy: number; r: number; size: number;
}

/** Compute clustered ring positions for all nodes */
function computeClusteredRingLayout(
  displayNodes: DisplayNode[],
  displayEdges: DisplayEdge[],
  centerX: number,
  centerY: number,
  width: number,
  height: number,
): {
  positions: Map<string, { x: number; y: number; w?: number; h?: number }>;
  boundaries: ClusterBoundary[];
  mergeMap: Map<string, string>;       // absorbed ID → representative ID
  mergedBodies: Map<string, string>;   // representative ID → combined body text
} {
  const positions = new Map<string, { x: number; y: number; w?: number; h?: number }>();
  const mergeMap = new Map<string, string>();
  const mergedBodies = new Map<string, string>();
  if (displayNodes.length === 0) return { positions, boundaries: [], mergeMap, mergedBodies };
  if (displayNodes.length === 1) {
    positions.set(displayNodes[0].id, { x: centerX, y: centerY });
    return { positions, boundaries: [], mergeMap, mergedBodies };
  }

  // Use thread_id grouping for signal containers, BFS for everything else
  const isSignal = displayNodes.some((n) => n.source === 'signal');
  const { clusters, hubId, degrees } = isSignal
    ? findSignalClusters(displayNodes, displayEdges)
    : findClustersBFS(new Set(displayNodes.map((n) => n.id)), displayEdges);

  // Separate real clusters (size > 1) from singletons
  const realClusters: string[][] = [];
  const singletons: string[] = [];
  for (const cluster of clusters) {
    if (cluster.length > 1) {
      realClusters.push(cluster);
    } else {
      singletons.push(cluster[0]);
    }
  }

  const NODE_R = 80;

  if (isSignal) {
    // --- Signal layout: vertical columns per thread, variable-height message rects ---
    const COL_SPACING = MSG_WIDTH + 60; // rect width + gap between columns

    // Build lookups for node text and author
    const nodeTextMap = new Map<string, string>();
    const nodeAuthorMap = new Map<string, string>();
    for (const dn of displayNodes) {
      nodeTextMap.set(dn.id, dn.content || dn.title);
      if (dn.author) nodeAuthorMap.set(dn.id, dn.author);
    }

    // Merge consecutive same-author messages connected by temporal edges
    const MERGE_MAX_GROUP = 8;

    // Build temporal adjacency set for fast lookup
    const temporalPairs = new Set<string>();
    for (const e of displayEdges) {
      if (e.type === 'temporal_thread') {
        temporalPairs.add(`${e.source}|${e.target}`);
        temporalPairs.add(`${e.target}|${e.source}`);
      }
    }

    function mergeTemporalBurst(cluster: string[]): string[] {
      if (cluster.length <= 1) return cluster;
      const result: string[] = [];
      let group: string[] = [cluster[0]];

      const flush = () => {
        if (group.length >= 2) {
          const repId = group[0];
          const bodies = group.map((id) => nodeTextMap.get(id) || '');
          const combined = bodies.join('\n');
          mergedBodies.set(repId, combined);
          for (let k = 1; k < group.length; k++) {
            mergeMap.set(group[k], repId);
          }
          nodeTextMap.set(repId, combined);
          result.push(repId);
        } else {
          result.push(group[0]);
        }
      };

      for (let i = 1; i < cluster.length; i++) {
        const curId = cluster[i];
        const prevId = group[group.length - 1];
        const curAuthor = nodeAuthorMap.get(curId);
        const groupAuthor = nodeAuthorMap.get(group[0]);
        const isTemporallyConnected = temporalPairs.has(`${prevId}|${curId}`);
        if (
          curAuthor === groupAuthor &&
          isTemporallyConnected &&
          group.length < MERGE_MAX_GROUP
        ) {
          group.push(curId);
        } else {
          flush();
          group = [curId];
        }
      }
      flush();
      return result;
    }

    // Apply temporal burst merging to all clusters
    for (let i = 0; i < realClusters.length; i++) {
      realClusters[i] = mergeTemporalBurst(realClusters[i]);
    }

    // All columns: real clusters first, then singletons grouped together
    const allColumns: string[][] = [...realClusters];
    if (singletons.length > 0) allColumns.push(singletons);

    // Order columns by cross-edge affinity: clusters sharing the most edges go next to each other.
    // Build node→column index lookup
    const nodeToCol = new Map<string, number>();
    for (let i = 0; i < allColumns.length; i++) {
      for (const id of allColumns[i]) nodeToCol.set(id, i);
    }
    // Count cross-edges between each pair of columns
    const crossEdges = new Map<string, number>(); // "i,j" → count
    for (const e of displayEdges) {
      const ci = nodeToCol.get(e.source);
      const cj = nodeToCol.get(e.target);
      if (ci === undefined || cj === undefined || ci === cj) continue;
      const key = ci < cj ? `${ci},${cj}` : `${cj},${ci}`;
      crossEdges.set(key, (crossEdges.get(key) || 0) + 1);
    }
    // Greedy ordering: start with most-connected cluster, grow sequence from both ends
    const totalCross = new Map<number, number>();
    for (let i = 0; i < allColumns.length; i++) totalCross.set(i, 0);
    for (const [key, count] of crossEdges) {
      const [a, b] = key.split(",").map(Number);
      totalCross.set(a, (totalCross.get(a) || 0) + count);
      totalCross.set(b, (totalCross.get(b) || 0) + count);
    }
    const placed = new Set<number>();
    const sequence: number[] = [];
    // Start with the cluster that has the most cross-edges overall
    let startIdx = 0;
    let bestTotal = -1;
    for (const [idx, total] of totalCross) {
      if (total > bestTotal) { bestTotal = total; startIdx = idx; }
    }
    sequence.push(startIdx);
    placed.add(startIdx);
    while (placed.size < allColumns.length) {
      const leftEnd = sequence[0];
      const rightEnd = sequence[sequence.length - 1];
      let bestIdx = -1, bestScore = -1, bestSide: "left" | "right" = "right";
      for (let i = 0; i < allColumns.length; i++) {
        if (placed.has(i)) continue;
        const lKey = leftEnd < i ? `${leftEnd},${i}` : `${i},${leftEnd}`;
        const rKey = rightEnd < i ? `${rightEnd},${i}` : `${i},${rightEnd}`;
        const lScore = crossEdges.get(lKey) || 0;
        const rScore = crossEdges.get(rKey) || 0;
        if (lScore > bestScore) { bestScore = lScore; bestIdx = i; bestSide = "left"; }
        if (rScore > bestScore) { bestScore = rScore; bestIdx = i; bestSide = "right"; }
      }
      if (bestIdx === -1) {
        // No cross-edges left — just append remaining
        for (let i = 0; i < allColumns.length; i++) {
          if (!placed.has(i)) { sequence.push(i); placed.add(i); }
        }
        break;
      }
      if (bestSide === "left") sequence.unshift(bestIdx);
      else sequence.push(bestIdx);
      placed.add(bestIdx);
    }

    const colCount = sequence.length;
    const totalWidth = (colCount - 1) * COL_SPACING;
    const startX = centerX - totalWidth / 2;

    for (let pos = 0; pos < sequence.length; pos++) {
      const col = allColumns[sequence[pos]];
      const colX = startX + pos * COL_SPACING;

      // Compute per-node heights and total column height
      const heights: number[] = col.map((id) => estimateMsgHeight(nodeTextMap.get(id) || ''));
      const totalH = heights.reduce((s, h) => s + h, 0) + (col.length - 1) * MSG_GAP;

      // Place nodes top-to-bottom, centered vertically
      let y = centerY - totalH / 2;
      for (let j = 0; j < col.length; j++) {
        const h = heights[j];
        positions.set(col[j], { x: colX, y: y + h / 2, w: MSG_WIDTH, h });
        y += h + MSG_GAP;
      }
    }

    // Skip cluster boundaries for signal (columns are visually distinct)
    return { positions, boundaries: [], mergeMap, mergedBodies };
  }

  // --- Non-signal: ring layout (unchanged) ---
  // Elliptical stretch for wider screens
  const ellipseAspect = Math.min((width / height) * 1.2, 2.5);

  // --- Macro ring: place cluster centers ---
  type MacroItem = { id: string; footprint: number; cluster?: string[] };
  const macroItems: MacroItem[] = [];
  for (const cluster of realClusters) {
    const hub = hubId.get(cluster[0])!;
    macroItems.push({ id: hub, footprint: clusterFootprint(cluster.length), cluster });
  }
  for (const id of singletons) {
    macroItems.push({ id, footprint: 0 });
  }

  let macroRing = 0;
  let prevRadius = 0;

  for (let i = 0; i < macroItems.length; ) {
    const cap = macroRingCapacity(macroRing);
    const batch = macroItems.slice(i, i + cap);

    let radius: number;
    if (macroRing === 0) {
      radius = 0;
    } else {
      const maxFootprint = Math.max(...batch.map((b) => b.footprint), 0);
      const prevBatchStart = Math.max(0, i - macroRingCapacity(macroRing - 1));
      const prevBatch = macroItems.slice(prevBatchStart, i);
      const prevMaxFootprint = Math.max(...prevBatch.map((b) => b.footprint), 0);
      radius = prevRadius + prevMaxFootprint + maxFootprint + MACRO_PAD;
    }

    const ringOffset = GOLDEN_ANGLE * macroRing;

    for (let j = 0; j < batch.length; j++) {
      const item = batch[j];
      const angle = (2 * Math.PI * j) / batch.length + ringOffset;
      let x: number, y: number;
      if (radius === 0) {
        x = centerX;
        y = centerY;
      } else {
        x = centerX + Math.cos(angle) * radius * ellipseAspect;
        y = centerY + Math.sin(angle) * radius;
      }

      positions.set(item.id, { x, y });

      if (item.cluster) {
        const hub = item.id;
        const members = item.cluster
          .filter((id) => id !== hub)
          .sort((a, b) => (degrees.get(b) || 0) - (degrees.get(a) || 0));

        let microRing = 1;
        for (let m = 0; m < members.length; ) {
          const mCap = microRingCapacity(microRing);
          const mBatch = members.slice(m, m + mCap);
          const microRadius = microRing * MICRO_RING_SPACING;
          const microOffset = GOLDEN_ANGLE * microRing;

          for (let k = 0; k < mBatch.length; k++) {
            const mAngle = (2 * Math.PI * k) / mBatch.length + microOffset;
            const mx = x + Math.cos(mAngle) * microRadius;
            const my = y + Math.sin(mAngle) * microRadius;
            positions.set(mBatch[k], { x: mx, y: my });
          }

          m += mBatch.length;
          microRing++;
        }
      }
    }

    i += batch.length;
    prevRadius = radius;
    macroRing++;
  }

  // Compute bounding circles for clusters with 2+ nodes
  const boundaries: ClusterBoundary[] = [];
  for (const cluster of realClusters) {
    const pts = cluster.map((id) => positions.get(id)!).filter(Boolean);
    if (pts.length < 2) continue;
    const cx = pts.reduce((s, p) => s + p.x, 0) / pts.length;
    const cy = pts.reduce((s, p) => s + p.y, 0) / pts.length;
    const r = Math.max(...pts.map((p) => Math.hypot(p.x - cx, p.y - cy))) + NODE_R + 20;
    boundaries.push({ cx, cy, r, size: cluster.length });
  }

  return { positions, boundaries, mergeMap, mergedBodies };
}

export default function GraphView() {
  const svgRef = useRef<SVGSVGElement>(null);
  const nodesRef = useRef<GraphNode[]>([]);
  const zoomRef = useRef<d3.ZoomBehavior<SVGSVGElement, unknown> | null>(null);
  const lastClickRef = useRef<{ time: number; id: string }>({ time: 0, id: "" });

  // Subscribe to actual data so effect re-runs when nodes/edges change
  const nodes = useTeamStore((s) => s.nodes);
  const edges = useTeamStore((s) => s.edges);
  const personalNodes = useTeamStore((s) => s.personalNodes);
  const personalEdges = useTeamStore((s) => s.personalEdges);
  const selectedNodeId = useTeamStore((s) => s.selectedNodeId);
  const searchResults = useTeamStore((s) => s.searchResults);
  const searchQuery = useTeamStore((s) => s.searchQuery);
  const savedPositions = useTeamStore((s) => s.savedPositions);
  const currentParentId = useTeamStore((s) => s.currentParentId);
  const setSelectedNodeId = useTeamStore((s) => s.setSelectedNodeId);
  const savePositions = useTeamStore((s) => s.savePositions);
  const setCurrentPositions = useTeamStore((s) => s.setCurrentPositions);
  const setMergedBodies = useTeamStore((s) => s.setMergedBodies);
  const setMergeGroupIds = useTeamStore((s) => s.setMergeGroupIds);
  const getDisplayNodes = useTeamStore((s) => s.getDisplayNodes);
  const getDisplayEdges = useTeamStore((s) => s.getDisplayEdges);
  const navigateToCategory = useTeamStore((s) => s.navigateToCategory);
  const openLeafView = useTeamStore((s) => s.openLeafView);


  useEffect(() => {
    const svg = svgRef.current;
    if (!svg) return;

    const width = svg.clientWidth;
    const height = svg.clientHeight;
    const centerX = width / 2;
    const centerY = height / 2;

    const displayNodes = getDisplayNodes();
    const displayEdges = getDisplayEdges();

    // Edge counts for sizing
    const edgeCounts = new Map<string, number>();
    for (const e of displayEdges) {
      edgeCounts.set(e.source, (edgeCounts.get(e.source) || 0) + 1);
      edgeCounts.set(e.target, (edgeCounts.get(e.target) || 0) + 1);
    }

    // Preserve positions from previous render
    const prevPositions = new Map<string, { x: number; y: number }>();
    for (const n of nodesRef.current) {
      prevPositions.set(n.id, { x: n.x, y: n.y });
    }

    // Compute ring layout for all nodes
    const { positions: ringPositions, boundaries: clusterBoundaries, mergeMap, mergedBodies } = computeClusteredRingLayout(
      displayNodes, displayEdges, centerX, centerY, width, height
    );

    // In signal views, exclude non-signal nodes and merged (absorbed) nodes
    const isSignalView = displayNodes.some((n) => n.source === 'signal');
    const layoutNodes = isSignalView
      ? displayNodes.filter((n) => n.source === 'signal' && !mergeMap.has(n.id))
      : displayNodes;

    // Build nodes — position priority:
    //   Signal view: layout only (columns are authoritative, no drift)
    //   Non-signal:  saved (local.db) > previous render > ring layout
    const graphNodes: GraphNode[] = layoutNodes.map((dn) => {
      const saved = savedPositions.get(dn.id);
      const prev = prevPositions.get(dn.id);
      const layoutPos = ringPositions.get(dn.id);
      let x: number, y: number;
      if (isSignalView) {
        // Signal columns are computed layout — always use them
        x = layoutPos?.x ?? centerX;
        y = layoutPos?.y ?? centerY;
      } else if (saved) {
        x = saved.x; y = saved.y;
      } else if (prev) {
        x = prev.x; y = prev.y;
      } else {
        x = layoutPos?.x ?? centerX;
        y = layoutPos?.y ?? centerY;
      }
      return {
        id: dn.id,
        title: dn.title,
        isPersonal: dn.isPersonal,
        isItem: dn.isItem,
        childCount: dn.childCount,
        author: dn.author,
        contentType: dn.contentType,
        emoji: dn.emoji,
        source: dn.source,
        body: mergedBodies.get(dn.id) || dn.content || dn.title,
        edgeCount: edgeCounts.get(dn.id) || 0,
        x, y,
        rectW: layoutPos?.w,
        rectH: layoutPos?.h,
      };
    });

    nodesRef.current = graphNodes;

    // Publish all current positions to store (in-memory only, for QuickAdd etc.)
    const posMap = new Map(graphNodes.map((n) => [n.id, { x: n.x, y: n.y }]));
    setCurrentPositions(posMap);
    setMergedBodies(mergedBodies);
    // Build reverse map: representative ID → all IDs in the merge group
    const groupIds = new Map<string, string[]>();
    for (const [absorbedId, repId] of mergeMap) {
      let group = groupIds.get(repId);
      if (!group) { group = [repId]; groupIds.set(repId, group); }
      group.push(absorbedId);
    }
    setMergeGroupIds(groupIds);

    const nodeMap = new Map(graphNodes.map((n) => [n.id, n]));
    const nodeIds = new Set(graphNodes.map((n) => n.id));

    // Link nodes (🔗 emoji) — show as purple
    const linkNodeIds = new Set<string>();
    for (const dn of displayNodes) {
      if (dn.emoji === '🔗') linkNodeIds.add(dn.id);
    }

    const AI_SOURCES = new Set(["ai", "adaptive", "semantic"]);
    // Structural edge types always shown regardless of weight
    const ALWAYS_SHOW = new Set(["contains", "replies_to", "shares_link", "temporal_thread", "belongs_to", "defined_in", "calls", "documents"]);
    const EDGE_WEIGHT_THRESHOLD = 0.5;
    const links: GraphLink[] = displayEdges
      .filter((e) => ALWAYS_SHOW.has(e.type) || (e.weight ?? 0.5) >= EDGE_WEIGHT_THRESHOLD)
      .map((e) => {
        // Remap absorbed node IDs to their representative
        const src = mergeMap.get(e.source) ?? e.source;
        const tgt = mergeMap.get(e.target) ?? e.target;
        return {
          id: e.id,
          sourceId: src,
          targetId: tgt,
          type: e.type,
          weight: e.weight ?? 0.5,
          isPersonal: e.isPersonal,
          isHuman: e.isPersonal || !AI_SOURCES.has(e.edgeSource || ""),
        };
      })
      .filter((l) => nodeIds.has(l.sourceId) && nodeIds.has(l.targetId) && l.sourceId !== l.targetId);

    // D3 rendering
    const svgSel = d3.select(svg);
    let g = svgSel.select<SVGGElement>("g.graph-root");
    if (g.empty()) {
      // Grid pattern defs
      const defs = svgSel.append("defs");

      const smallGrid = defs.append("pattern")
        .attr("id", "graph-grid-small")
        .attr("width", 40)
        .attr("height", 40)
        .attr("patternUnits", "userSpaceOnUse");
      smallGrid.append("path")
        .attr("d", "M 40 0 L 0 0 0 40")
        .attr("fill", "none")
        .attr("stroke", "rgba(255,255,255,0.04)")
        .attr("stroke-width", 0.5);

      const largeGrid = defs.append("pattern")
        .attr("id", "graph-grid-large")
        .attr("width", 200)
        .attr("height", 200)
        .attr("patternUnits", "userSpaceOnUse");
      largeGrid.append("rect")
        .attr("width", 200)
        .attr("height", 200)
        .attr("fill", "url(#graph-grid-small)");
      largeGrid.append("path")
        .attr("d", "M 200 0 L 0 0 0 200")
        .attr("fill", "none")
        .attr("stroke", "rgba(255,255,255,0.08)")
        .attr("stroke-width", 1);

      g = svgSel.append("g").attr("class", "graph-root");

      // Grid background (inside g so it transforms with zoom/pan)
      g.append("rect")
        .attr("class", "grid-background")
        .attr("x", -50000)
        .attr("y", -50000)
        .attr("width", 100000)
        .attr("height", 100000)
        .attr("fill", "url(#graph-grid-large)");

      // Layer groups ensure edges always render behind nodes
      g.append("g").attr("class", "layer-boundaries");
      g.append("g").attr("class", "layer-edges");
      g.append("g").attr("class", "layer-nodes");

      const zoom = d3.zoom<SVGSVGElement, unknown>()
        .scaleExtent([0.1, 4])
        .on("zoom", (event) => g.attr("transform", event.transform));
      zoomRef.current = zoom;
      svgSel.call(zoom);
    }

    // Cluster boundary circles
    const boundaryLayer = g.select<SVGGElement>("g.layer-boundaries");
    const boundarySel = boundaryLayer
      .selectAll<SVGCircleElement, ClusterBoundary>("circle.cluster-boundary")
      .data(clusterBoundaries, (_d, i) => `cluster-${i}`);
    boundarySel.exit().remove();
    const boundaryMerge = boundarySel.enter().append("circle").attr("class", "cluster-boundary").merge(boundarySel);
    boundaryMerge
      .attr("cx", (d) => d.cx)
      .attr("cy", (d) => d.cy)
      .attr("r", (d) => d.r)
      .attr("fill", "rgba(239,68,68,0.05)")
      .attr("stroke", "#ef4444")
      .attr("stroke-width", 6)
      .attr("stroke-opacity", 1)
      .attr("stroke-dasharray", "12,6");

    // Build set of nodes connected to selected node (for fade logic)
    const connectedNodeIds = new Set<string>();
    if (selectedNodeId) {
      connectedNodeIds.add(selectedNodeId);
      for (const l of links) {
        if (l.sourceId === selectedNodeId) connectedNodeIds.add(l.targetId);
        if (l.targetId === selectedNodeId) connectedNodeIds.add(l.sourceId);
      }
    }

    // Links — weight-normalized heat colors (red→yellow→cyan)
    const nonContainsWeights = links.filter((l) => l.type !== "contains").map((l) => l.weight);
    const minW = nonContainsWeights.length > 0 ? Math.min(...nonContainsWeights) : 0;
    const maxW = nonContainsWeights.length > 0 ? Math.max(...nonContainsWeights) : 1;
    const wRange = maxW - minW || 0.1;

    const edgeLayer = g.select<SVGGElement>("g.layer-edges");
    const linkSel = edgeLayer
      .selectAll<SVGLineElement, GraphLink>("line.edge")
      .data(links, (d) => d.id);
    linkSel.exit().remove();
    const linkEnter = linkSel.enter().append("line").attr("class", "edge")
      .style("transition", "stroke-opacity 150ms ease");
    const linkMerge = linkEnter.merge(linkSel);
    linkMerge
      .attr("stroke", (d) => {
        if (d.type === "contains") return "#6b7280";
        const norm = (d.weight - minW) / wRange;
        return getHeatColor(norm);
      })
      .attr("stroke-width", (d) => {
        if (d.type === "contains") return 4;
        const norm = (d.weight - minW) / wRange;
        const base = 4 + norm * 14;
        const isConnected = selectedNodeId && (d.sourceId === selectedNodeId || d.targetId === selectedNodeId);
        return isConnected ? base * 1.8 : base;
      })
      .style("stroke-opacity", (d) => {
        if (d.type === "contains") return 0.5;
        if (selectedNodeId) {
          return (d.sourceId === selectedNodeId || d.targetId === selectedNodeId) ? 0.8 : 0.05;
        }
        return 0.7;
      })
      .attr("stroke-dasharray", (d) => (d.isPersonal ? "4,3" : "none"))
      .each(function (d) {
        const src = nodeMap.get(d.sourceId);
        const tgt = nodeMap.get(d.targetId);
        if (!src || !tgt) return;
        const pts = getEdgeEndpoints(src, tgt, isSignalView);
        d3.select(this)
          .attr("x1", pts.sx).attr("y1", pts.sy)
          .attr("x2", pts.tx).attr("y2", pts.ty);
      });

    // Nodes
    const nodeLayer = g.select<SVGGElement>("g.layer-nodes");
    const nodeSel = nodeLayer
      .selectAll<SVGGElement, GraphNode>("g.node")
      .data(graphNodes, (d) => d.id);
    nodeSel.exit().remove();

    const nodeEnter = nodeSel.enter().append("g").attr("class", "node").style("cursor", "pointer")
      .style("transition", "opacity 150ms ease");

    if (isSignalView) {
      // Signal: message rectangles with word-wrapped text
      nodeEnter.append("rect").attr("class", "msg-bg");
      nodeEnter.append("foreignObject").attr("class", "msg-fo");
    } else {
      // Non-signal: circles with label
      nodeEnter.append("circle");
      nodeEnter.append("text")
        .attr("class", "label")
        .attr("dy", "0.35em")
        .attr("text-anchor", "middle")
        .style("fill", "#f9fafb")
        .style("font-size", "20px")
        .style("pointer-events", "none");
      nodeEnter.append("foreignObject")
        .attr("class", "url-label");
    }

    const nodeMerge = nodeEnter.merge(nodeSel);

    nodeMerge.attr("transform", (d) => `translate(${d.x},${d.y})`);

    // Fade unconnected nodes when a node is selected
    nodeMerge.style("opacity", (d) => {
      if (!selectedNodeId) return 1;
      return connectedNodeIds.has(d.id) ? 1 : 0.8;
    });

    if (isSignalView) {
      // --- Signal message rectangle rendering ---
      nodeMerge.select("rect.msg-bg")
        .attr("width", (d) => d.rectW ?? MSG_WIDTH)
        .attr("height", (d) => d.rectH ?? 50)
        .attr("x", (d) => -(d.rectW ?? MSG_WIDTH) / 2)
        .attr("y", (d) => -(d.rectH ?? 50) / 2)
        .attr("rx", 8)
        .attr("ry", 8)
        .attr("fill", (d) => {
          if (linkNodeIds.has(d.id)) return "rgba(88, 28, 135, 0.8)";
          return d.author ? authorBg(d.author) : "rgba(51, 65, 85, 0.9)";
        })
        .attr("stroke", (d) => {
          if (d.id === selectedNodeId) return "#f59e0b";
          return "rgba(148, 163, 184, 0.2)";
        })
        .attr("stroke-width", (d) => (d.id === selectedNodeId ? 3 : 1));

      // foreignObject for word-wrapped HTML text
      nodeMerge.select("foreignObject.msg-fo")
        .attr("width", (d) => d.rectW ?? MSG_WIDTH)
        .attr("height", (d) => d.rectH ?? 50)
        .attr("x", (d) => -(d.rectW ?? MSG_WIDTH) / 2)
        .attr("y", (d) => -(d.rectH ?? 50) / 2);

      // Render text content inside foreignObject
      nodeMerge.each(function (d) {
        const fo = d3.select(this).select("foreignObject.msg-fo");
        let div = fo.select("div");
        if (div.empty()) {
          div = fo.append("xhtml:div");
        }
        const authorColor = d.author ? authorText(d.author) : "#9ca3af";
        const authorHtml = d.author
          ? `<span style="color:${authorColor};font-weight:700;font-size:12px;text-transform:uppercase;letter-spacing:0.05em">${escapeHtml(d.author)}</span><br/>`
          : "";
        div
          .style("padding", `${MSG_PAD}px`)
          .style("box-sizing", "border-box")
          .style("width", "100%")
          .style("height", "100%")
          .style("color", "#e2e8f0")
          .style("font-size", `${MSG_FONT_SIZE}px`)
          .style("line-height", `${MSG_LINE_HEIGHT}px`)
          .style("word-wrap", "break-word")
          .style("overflow-wrap", "anywhere")
          .style("word-break", "break-all")
          .style("overflow", "hidden")
          .style("pointer-events", "none")
          .style("font-family", "'Inter', 'Segoe UI', system-ui, sans-serif")
          .html(authorHtml + escapeHtml(d.body || d.title).replace(/\n/g, '<br>'));
      });

      // Search highlight for signal
      if (searchQuery && searchResults.length > 0) {
        const resultIds = new Set(searchResults.map((r) => r.id));
        nodeMerge.select("rect.msg-bg")
          .attr("stroke", (d) => {
            if (d.id === selectedNodeId) return "#f59e0b";
            if (resultIds.has(d.id)) return "#f59e0b";
            return "rgba(148, 163, 184, 0.2)";
          })
          .attr("stroke-width", (d) => {
            if (d.id === selectedNodeId) return 3;
            if (resultIds.has(d.id)) return 3;
            return 1;
          });
      }
    } else {
      // --- Non-signal circle rendering (unchanged) ---
      nodeMerge.select("circle")
        .attr("r", (d) => Math.min(64 + d.edgeCount * 4, 96))
        .attr("fill", (d) => d.isPersonal ? PERSONAL_COLOR : !d.isItem ? CATEGORY_COLOR : linkNodeIds.has(d.id) ? "#a855f7" : TEAM_COLOR)
        .attr("fill-opacity", (d) => (d.isPersonal ? 0.6 : 0.85))
        .attr("stroke", (d) => {
          if (d.id === selectedNodeId) return "#f59e0b";
          if (d.isPersonal) return PERSONAL_COLOR;
          return "#f9fafb";
        })
        .attr("stroke-width", (d) => (d.id === selectedNodeId ? 6 : 1.5))
        .attr("stroke-dasharray", (d) => (d.isPersonal ? "3,2" : "none"));

      nodeMerge.select("text.label")
        .text((d) => isUrl(d.title) ? "" : truncate(d.title, 14))
        .style("display", (d) => isUrl(d.title) ? "none" : null)
        .style("font-size", "20px")
        .attr("y", (d) => Math.min(64 + d.edgeCount * 4, 96) + 16);

      const urlW = 220;
      nodeMerge.select("foreignObject.url-label")
        .attr("width", urlW)
        .attr("x", -urlW / 2)
        .attr("y", (d) => Math.min(64 + d.edgeCount * 4, 96) + 4)
        .attr("height", 44)
        .style("display", (d) => isUrl(d.title) ? null : "none")
        .style("pointer-events", "none")
        .each(function (_d) {
          const el = this as unknown as SVGForeignObjectElement;
          const d = _d;
          if (!isUrl(d.title)) { el.innerHTML = ""; return; }
          el.innerHTML = `<div xmlns="http://www.w3.org/1999/xhtml" style="
            color: #f9fafb;
            font-size: 14px;
            line-height: 1.3;
            text-align: center;
            overflow: hidden;
            display: -webkit-box;
            -webkit-line-clamp: 2;
            -webkit-box-orient: vertical;
            word-break: break-all;
            pointer-events: none;
          ">${formatUrlTitle(d.title)}</div>`;
        });

      // Child count indicator inside category nodes
      nodeMerge.selectAll("text.child-count").remove();
      nodeMerge
        .filter((d) => !d.isItem && d.childCount > 0)
        .append("text")
        .attr("class", "child-count")
        .attr("dy", "0.35em")
        .attr("text-anchor", "middle")
        .style("fill", "#f9fafb")
        .style("font-size", "13px")
        .style("pointer-events", "none")
        .style("opacity", 0.7)
        .text((d) => `${d.childCount}`);

      // Emoji inside node circle
      nodeMerge.selectAll("text.emoji").remove();
      nodeMerge
        .filter((d) => !!d.emoji)
        .append("text")
        .attr("class", "emoji")
        .attr("dy", "0.35em")
        .attr("text-anchor", "middle")
        .style("font-size", "28px")
        .style("pointer-events", "none")
        .text((d) => d.emoji!);

      // Search highlight
      if (searchQuery && searchResults.length > 0) {
        const resultIds = new Set(searchResults.map((r) => r.id));
        nodeMerge.select("circle")
          .attr("stroke", (d) => {
            if (d.id === selectedNodeId) return "#f59e0b";
            if (resultIds.has(d.id)) return "#f59e0b";
            if (d.isPersonal) return PERSONAL_COLOR;
            return "#f9fafb";
          })
          .attr("stroke-width", (d) => {
            if (d.id === selectedNodeId) return 6;
            if (resultIds.has(d.id)) return 4;
            return 1.5;
          });
      }
    }

    // Drag + click + double-click
    // No timer — first click selects immediately, second click within 400ms drills in.
    // lastClickRef survives effect re-runs (useRef, not local variable).
    let dragMoved = false;
    const drag = d3.drag<SVGGElement, GraphNode>()
      .on("start", function () {
        dragMoved = false;
        d3.select(this).raise();
      })
      .on("drag", function (event, d) {
        dragMoved = true;
        d.x = event.x;
        d.y = event.y;
        d3.select(this).attr("transform", `translate(${d.x},${d.y})`);
        linkMerge
          .filter((l) => l.sourceId === d.id || l.targetId === d.id)
          .each(function (l) {
            const srcNode = l.sourceId === d.id ? d : nodeMap.get(l.sourceId);
            const tgtNode = l.targetId === d.id ? d : nodeMap.get(l.targetId);
            if (!srcNode || !tgtNode) return;
            const pts = getEdgeEndpoints(srcNode, tgtNode, isSignalView);
            d3.select(this)
              .attr("x1", pts.sx).attr("y1", pts.sy)
              .attr("x2", pts.tx).attr("y2", pts.ty);
          });
      })
      .on("end", (_event, d) => {
        if (dragMoved && !isSignalView) {
          savePositions([{ node_id: d.id, x: d.x, y: d.y }]);
        } else {
          const now = Date.now();
          const last = lastClickRef.current;
          if (last.id === d.id && now - last.time < 400) {
            // Double-click — drill into category
            lastClickRef.current = { time: 0, id: "" };
            if (!d.isItem) {
              navigateToCategory(d.id);
            } else {
              openLeafView(d.id);
            }
          } else {
            // Single click — select immediately, record for double-click detection
            lastClickRef.current = { time: now, id: d.id };
            setSelectedNodeId(d.id === selectedNodeId ? null : d.id);
          }
        }
      });

    nodeMerge.call(drag);

  }, [nodes, edges, personalNodes, personalEdges, selectedNodeId, searchResults, searchQuery, savedPositions, currentParentId, getDisplayNodes, getDisplayEdges, setSelectedNodeId, savePositions, navigateToCategory, openLeafView, setMergedBodies, setMergeGroupIds]);

  // Pan to node when requested
  const panToNodeId = useTeamStore((s) => s.panToNodeId);
  const setPanToNodeId = useTeamStore((s) => s.setPanToNodeId);
  useEffect(() => {
    if (!panToNodeId || !svgRef.current || !zoomRef.current) return;
    const node = nodesRef.current.find((n) => n.id === panToNodeId);
    if (!node) return;
    const svg = svgRef.current;
    const width = svg.clientWidth;
    const height = svg.clientHeight;
    const svgSel = d3.select(svg);
    const transform = d3.zoomIdentity.translate(width / 2 - node.x, height / 2 - node.y);
    svgSel.transition().duration(500).call(zoomRef.current.transform, transform);
    setPanToNodeId(null);
  }, [panToNodeId, setPanToNodeId]);

  // Auto-fit viewport when drill-down view changes
  useEffect(() => {
    if (!svgRef.current || !zoomRef.current) return;
    const svg = svgRef.current;
    const graphNodes = nodesRef.current;
    if (graphNodes.length === 0) {
      d3.select(svg).transition().duration(300)
        .call(zoomRef.current.transform, d3.zoomIdentity);
      return;
    }

    // Compute bounding box
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const n of graphNodes) {
      const r = Math.min(64 + n.edgeCount * 4, 96);
      minX = Math.min(minX, n.x - r);
      minY = Math.min(minY, n.y - r);
      maxX = Math.max(maxX, n.x + r);
      maxY = Math.max(maxY, n.y + r);
    }
    const bw = maxX - minX;
    const bh = maxY - minY;
    const sw = svg.clientWidth;
    const sh = svg.clientHeight;
    const pad = 100;
    const scale = Math.max(0.1, Math.min(
      sw / (bw + pad * 2),
      sh / (bh + pad * 2),
      1, // don't zoom in past 1x
    ));
    const cx = (minX + maxX) / 2;
    const cy = (minY + maxY) / 2;
    const transform = d3.zoomIdentity
      .translate(sw / 2 - cx * scale, sh / 2 - cy * scale)
      .scale(scale);
    d3.select(svg).transition().duration(500)
      .call(zoomRef.current.transform, transform);
  }, [currentParentId]);

  const handleSvgClick = useCallback((e: React.MouseEvent) => {
    if ((e.target as Element).tagName === "svg") {
      setSelectedNodeId(null);
      if (useTeamStore.getState().leafViewNodeId) {
        useTeamStore.setState({ leafViewNodeId: null });
      }
    }
  }, [setSelectedNodeId]);

  return (
    <svg
      ref={svgRef}
      className="flex-1"
      style={{ background: "var(--bg-primary)" }}
      onClick={handleSvgClick}
    />
  );
}

function truncate(s: string, n: number): string {
  return s.length > n ? s.slice(0, n - 1) + "\u2026" : s;
}

function isUrl(s: string): boolean {
  return /^https?:\/\//i.test(s);
}

function formatUrlTitle(url: string): string {
  return url.replace(/^https?:\/\/(www\.)?/, "");
}
