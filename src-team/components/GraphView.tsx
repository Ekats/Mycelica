import { useEffect, useRef, useCallback } from "react";
import * as d3 from "d3";
import { useTeamStore } from "../stores/teamStore";

const AUTHOR_COLORS = ["#f59e0b", "#10b981", "#6366f1", "#ef4444", "#8b5cf6", "#06b6d4", "#ec4899", "#84cc16"];
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
}

const GRID_SPACING = 120;

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

  const {
    getDisplayNodes, getDisplayEdges, selectedNodeId, searchResults, searchQuery,
    setSelectedNodeId, savePositions, savedPositions,
  } = useTeamStore();

  const authorColorMap = useRef(new Map<string, string>());

  const getAuthorColor = useCallback((author?: string) => {
    if (!author) return "#6b7280";
    if (!authorColorMap.current.has(author)) {
      authorColorMap.current.set(author, AUTHOR_COLORS[authorColorMap.current.size % AUTHOR_COLORS.length]);
    }
    return authorColorMap.current.get(author)!;
  }, []);

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

    // Build nodes with deterministic positions
    let unpositionedIndex = 0;
    const nodes: GraphNode[] = displayNodes.map((dn) => {
      // Priority: saved position > previous render > DB position > grid
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
        author: dn.author,
        contentType: dn.contentType,
        edgeCount: edgeCounts.get(dn.id) || 0,
        x, y,
      };
    });

    nodesRef.current = nodes;
    const nodeMap = new Map(nodes.map((n) => [n.id, n]));
    const nodeIds = new Set(nodes.map((n) => n.id));

    const links: GraphLink[] = displayEdges
      .filter((e) => nodeIds.has(e.source) && nodeIds.has(e.target))
      .map((e) => ({
        id: e.id,
        sourceId: e.source,
        targetId: e.target,
        type: e.type,
        isPersonal: e.isPersonal,
      }));

    // D3 rendering
    const svgSel = d3.select(svg);
    let g = svgSel.select<SVGGElement>("g.graph-root");
    if (g.empty()) {
      g = svgSel.append("g").attr("class", "graph-root");
      const zoom = d3.zoom<SVGSVGElement, unknown>()
        .scaleExtent([0.1, 4])
        .on("zoom", (event) => g.attr("transform", event.transform));
      svgSel.call(zoom);
    }

    // Links
    const linkSel = g
      .selectAll<SVGLineElement, GraphLink>("line.edge")
      .data(links, (d) => d.id);
    linkSel.exit().remove();
    const linkMerge = linkSel.enter().append("line").attr("class", "edge").merge(linkSel);
    linkMerge
      .attr("stroke", (d) => EDGE_COLORS[d.type] || "#4b5563")
      .attr("stroke-width", 1.5)
      .attr("stroke-opacity", (d) => (d.isPersonal ? 0.5 : 0.6))
      .attr("stroke-dasharray", (d) => (d.isPersonal ? "4,3" : "none"))
      .attr("x1", (d) => nodeMap.get(d.sourceId)?.x ?? 0)
      .attr("y1", (d) => nodeMap.get(d.sourceId)?.y ?? 0)
      .attr("x2", (d) => nodeMap.get(d.targetId)?.x ?? 0)
      .attr("y2", (d) => nodeMap.get(d.targetId)?.y ?? 0);

    // Nodes
    const nodeSel = g
      .selectAll<SVGGElement, GraphNode>("g.node")
      .data(nodes, (d) => d.id);
    nodeSel.exit().remove();

    const nodeEnter = nodeSel.enter().append("g").attr("class", "node").style("cursor", "pointer");
    nodeEnter.append("circle");
    nodeEnter.append("text")
      .attr("dy", "0.35em")
      .attr("text-anchor", "middle")
      .style("fill", "#f9fafb")
      .style("font-size", "10px")
      .style("pointer-events", "none");

    const nodeMerge = nodeEnter.merge(nodeSel);

    nodeMerge.attr("transform", (d) => `translate(${d.x},${d.y})`);

    nodeMerge.select("circle")
      .attr("r", (d) => Math.min(8 + d.edgeCount * 1.5, 20))
      .attr("fill", (d) => (d.isPersonal ? PERSONAL_COLOR : getAuthorColor(d.author)))
      .attr("fill-opacity", (d) => (d.isPersonal ? 0.6 : 0.85))
      .attr("stroke", (d) => {
        if (d.id === selectedNodeId) return "#f59e0b";
        if (d.isPersonal) return PERSONAL_COLOR;
        return "#f9fafb";
      })
      .attr("stroke-width", (d) => (d.id === selectedNodeId ? 3 : 1.5))
      .attr("stroke-dasharray", (d) => (d.isPersonal ? "3,2" : "none"));

    nodeMerge.select("text")
      .text((d) => truncate(d.title, 14))
      .attr("y", (d) => Math.min(8 + d.edgeCount * 1.5, 20) + 12);

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
          if (d.id === selectedNodeId) return 3;
          if (resultIds.has(d.id)) return 2.5;
          return 1.5;
        });
    }

    // Click
    nodeMerge.on("click", (_event, d) => {
      setSelectedNodeId(d.id === selectedNodeId ? null : d.id);
    });

    // Drag — moves node directly, no physics
    const drag = d3.drag<SVGGElement, GraphNode>()
      .on("start", function () {
        d3.select(this).raise();
      })
      .on("drag", function (event, d) {
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
        savePositions([{ node_id: d.id, x: d.x, y: d.y }]);
      });

    nodeMerge.call(drag);

  }, [getDisplayNodes, getDisplayEdges, selectedNodeId, searchResults, searchQuery, savedPositions, setSelectedNodeId, savePositions, getAuthorColor]);

  const handleSvgClick = useCallback((e: React.MouseEvent) => {
    if ((e.target as Element).tagName === "svg") {
      setSelectedNodeId(null);
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
