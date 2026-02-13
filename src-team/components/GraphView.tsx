import { useEffect, useRef, useCallback } from "react";
import * as d3 from "d3";
import { useTeamStore } from "../stores/teamStore";

const TEAM_COLOR = "#60a5fa";
const CATEGORY_COLOR = "#ef4444";
const PERSONAL_COLOR = "#14b8a6";

const EDGE_COLORS: Record<string, string> = {
  supports: "#10b981",
  contradicts: "#ef4444",
  prerequisite: "#6366f1",
  evolved_from: "#8b5cf6",
  questions: "#f59e0b",
  because: "#f59e0b",
  related: "#4b5563",
  reference: "#4b5563",
  contains: "#6b7280",
};

interface GraphNode {
  id: string;
  title: string;
  isPersonal: boolean;
  isItem: boolean;
  childCount: number;
  author?: string;
  contentType?: string;
  edgeCount: number;
  x: number;
  y: number;
}

interface GraphLink {
  id: string;
  sourceId: string;
  targetId: string;
  type: string;
  isPersonal: boolean;
  isHuman: boolean;
}

const GRID_SPACING = 220;

// Deterministic grid placement for nodes without saved positions
function gridPosition(index: number, centerX: number, centerY: number): { x: number; y: number } {
  if (index === 0) return { x: centerX, y: centerY };
  // Spiral outward in a grid
  const cols = Math.ceil(Math.sqrt(index + 1));
  const row = Math.floor(index / cols);
  const col = index % cols;
  return {
    x: centerX + (col - Math.floor(cols / 2)) * GRID_SPACING,
    y: centerY + (row - Math.floor(cols / 2)) * GRID_SPACING,
  };
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

    // Build nodes — position priority: saved (local.db) > previous render > DB > grid
    let unpositionedIndex = 0;
    const graphNodes: GraphNode[] = displayNodes.map((dn) => {
      const saved = savedPositions.get(dn.id);
      const prev = prevPositions.get(dn.id);
      let x: number, y: number;
      if (saved) {
        x = saved.x; y = saved.y;
      } else if (prev) {
        x = prev.x; y = prev.y;
      } else if (dn.x != null && dn.y != null) {
        x = dn.x; y = dn.y;
      } else {
        const pos = gridPosition(unpositionedIndex++, centerX, centerY);
        x = pos.x; y = pos.y;
      }
      return {
        id: dn.id,
        title: dn.title,
        isPersonal: dn.isPersonal,
        isItem: dn.isItem,
        childCount: dn.childCount,
        author: dn.author,
        contentType: dn.contentType,
        edgeCount: edgeCounts.get(dn.id) || 0,
        x, y,
      };
    });

    nodesRef.current = graphNodes;

    // Publish all current positions to store (in-memory only, for QuickAdd etc.)
    const posMap = new Map(graphNodes.map((n) => [n.id, { x: n.x, y: n.y }]));
    setCurrentPositions(posMap);

    const nodeMap = new Map(graphNodes.map((n) => [n.id, n]));
    const nodeIds = new Set(graphNodes.map((n) => n.id));

    const AI_SOURCES = new Set(["ai", "adaptive", "semantic"]);
    const links: GraphLink[] = displayEdges
      .filter((e) => nodeIds.has(e.source) && nodeIds.has(e.target))
      .map((e) => ({
        id: e.id,
        sourceId: e.source,
        targetId: e.target,
        type: e.type,
        isPersonal: e.isPersonal,
        isHuman: e.isPersonal || !AI_SOURCES.has(e.edgeSource || ""),
      }));

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

      const zoom = d3.zoom<SVGSVGElement, unknown>()
        .scaleExtent([0.1, 4])
        .on("zoom", (event) => g.attr("transform", event.transform));
      zoomRef.current = zoom;
      svgSel.call(zoom);
    }

    // Links
    const linkSel = g
      .selectAll<SVGLineElement, GraphLink>("line.edge")
      .data(links, (d) => d.id);
    linkSel.exit().remove();
    const linkMerge = linkSel.enter().append("line").attr("class", "edge").merge(linkSel);
    linkMerge
      .attr("stroke", (d) => d.isHuman ? "#facc15" : (EDGE_COLORS[d.type] || "#4b5563"))
      .attr("stroke-width", (d) => d.isHuman ? 6 : 1.5)
      .attr("stroke-opacity", (d) => d.isHuman ? 0.9 : (d.isPersonal ? 0.5 : 0.6))
      .attr("stroke-dasharray", (d) => (d.isPersonal ? "4,3" : "none"))
      .attr("x1", (d) => nodeMap.get(d.sourceId)?.x ?? 0)
      .attr("y1", (d) => nodeMap.get(d.sourceId)?.y ?? 0)
      .attr("x2", (d) => nodeMap.get(d.targetId)?.x ?? 0)
      .attr("y2", (d) => nodeMap.get(d.targetId)?.y ?? 0);

    // Nodes
    const nodeSel = g
      .selectAll<SVGGElement, GraphNode>("g.node")
      .data(graphNodes, (d) => d.id);
    nodeSel.exit().remove();

    const nodeEnter = nodeSel.enter().append("g").attr("class", "node").style("cursor", "pointer");
    nodeEnter.append("circle");
    nodeEnter.append("text")
      .attr("class", "label")
      .attr("dy", "0.35em")
      .attr("text-anchor", "middle")
      .style("fill", "#f9fafb")
      .style("font-size", "20px")
      .style("pointer-events", "none");

    const nodeMerge = nodeEnter.merge(nodeSel);

    nodeMerge.attr("transform", (d) => `translate(${d.x},${d.y})`);

    nodeMerge.select("circle")
      .attr("r", (d) => Math.min(64 + d.edgeCount * 4, 96))
      .attr("fill", (d) => d.isPersonal ? PERSONAL_COLOR : !d.isItem ? CATEGORY_COLOR : TEAM_COLOR)
      .attr("fill-opacity", (d) => (d.isPersonal ? 0.6 : 0.85))
      .attr("stroke", (d) => {
        if (d.id === selectedNodeId) return "#f59e0b";
        if (d.isPersonal) return PERSONAL_COLOR;
        return "#f9fafb";
      })
      .attr("stroke-width", (d) => (d.id === selectedNodeId ? 6 : 1.5))
      .attr("stroke-dasharray", (d) => (d.isPersonal ? "3,2" : "none"));

    nodeMerge.select("text.label")
      .text((d) => truncate(d.title, 14))
      .style("font-size", "20px")
      .attr("y", (d) => Math.min(64 + d.edgeCount * 4, 96) + 16);

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
          .filter((l) => l.sourceId === d.id)
          .attr("x1", d.x).attr("y1", d.y);
        linkMerge
          .filter((l) => l.targetId === d.id)
          .attr("x2", d.x).attr("y2", d.y);
      })
      .on("end", (_event, d) => {
        if (dragMoved) {
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

  }, [nodes, edges, personalNodes, personalEdges, selectedNodeId, searchResults, searchQuery, savedPositions, currentParentId, getDisplayNodes, getDisplayEdges, setSelectedNodeId, savePositions, navigateToCategory, openLeafView]);

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

  // Reset viewport when drill-down view changes
  useEffect(() => {
    if (!svgRef.current || !zoomRef.current) return;
    d3.select(svgRef.current).transition().duration(300)
      .call(zoomRef.current.transform, d3.zoomIdentity);
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
