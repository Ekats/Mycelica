import { useEffect, useRef, useCallback } from "react";
import * as d3 from "d3";
import { useTeamStore } from "../stores/teamStore";

const AUTHOR_COLORS = ["#f59e0b", "#10b981", "#6366f1", "#ef4444", "#8b5cf6", "#06b6d4", "#ec4899", "#84cc16"];
const PERSONAL_COLOR = "#14b8a6"; // teal

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

interface SimNode extends d3.SimulationNodeDatum {
  id: string;
  title: string;
  isPersonal: boolean;
  author?: string;
  contentType?: string;
  edgeCount: number;
}

interface SimLink extends d3.SimulationLinkDatum<SimNode> {
  id: string;
  type: string;
  isPersonal: boolean;
}

export default function GraphView() {
  const svgRef = useRef<SVGSVGElement>(null);
  const simulationRef = useRef<d3.Simulation<SimNode, SimLink> | null>(null);
  const nodesRef = useRef<SimNode[]>([]);
  const linksRef = useRef<SimLink[]>([]);

  const {
    getDisplayNodes, getDisplayEdges, selectedNodeId, searchResults, searchQuery,
    setSelectedNodeId, savePositions,
  } = useTeamStore();

  const authorColorMap = useRef(new Map<string, string>());

  const getAuthorColor = useCallback((author?: string) => {
    if (!author) return "#6b7280";
    if (!authorColorMap.current.has(author)) {
      authorColorMap.current.set(author, AUTHOR_COLORS[authorColorMap.current.size % AUTHOR_COLORS.length]);
    }
    return authorColorMap.current.get(author)!;
  }, []);

  // Build/update simulation when data changes
  useEffect(() => {
    const svg = svgRef.current;
    if (!svg) return;

    const displayNodes = getDisplayNodes();
    const displayEdges = getDisplayEdges();

    // Build lookup for edge counts
    const edgeCounts = new Map<string, number>();
    for (const e of displayEdges) {
      edgeCounts.set(e.source, (edgeCounts.get(e.source) || 0) + 1);
      edgeCounts.set(e.target, (edgeCounts.get(e.target) || 0) + 1);
    }

    // Build node set for filtering edges
    const nodeIds = new Set(displayNodes.map((n) => n.id));

    // Preserve positions from existing sim nodes
    const oldPositions = new Map<string, { x: number; y: number; vx: number; vy: number }>();
    for (const n of nodesRef.current) {
      if (n.x != null && n.y != null) {
        oldPositions.set(n.id, { x: n.x, y: n.y, vx: n.vx || 0, vy: n.vy || 0 });
      }
    }

    const simNodes: SimNode[] = displayNodes.map((dn) => {
      const old = oldPositions.get(dn.id);
      return {
        id: dn.id,
        title: dn.title,
        isPersonal: dn.isPersonal,
        author: dn.author,
        contentType: dn.contentType,
        edgeCount: edgeCounts.get(dn.id) || 0,
        x: dn.x ?? old?.x,
        y: dn.y ?? old?.y,
        vx: old?.vx,
        vy: old?.vy,
      };
    });

    const nodeMap = new Map(simNodes.map((n) => [n.id, n]));

    const simLinks: SimLink[] = displayEdges
      .filter((e) => nodeIds.has(e.source) && nodeIds.has(e.target))
      .map((e) => ({
        id: e.id,
        source: nodeMap.get(e.source) || e.source,
        target: nodeMap.get(e.target) || e.target,
        type: e.type,
        isPersonal: e.isPersonal,
      }));

    nodesRef.current = simNodes;
    linksRef.current = simLinks;

    // Stop old simulation
    if (simulationRef.current) {
      simulationRef.current.stop();
    }

    const width = svg.clientWidth;
    const height = svg.clientHeight;

    const simulation = d3
      .forceSimulation<SimNode>(simNodes)
      .force("link", d3.forceLink<SimNode, SimLink>(simLinks).id((d) => d.id).distance(100))
      .force("charge", d3.forceManyBody().strength(-200))
      .force("center", d3.forceCenter(width / 2, height / 2))
      .force("collide", d3.forceCollide().radius(24))
      .alphaDecay(0.02);

    simulationRef.current = simulation;

    // D3 rendering
    const svgSel = d3.select(svg);
    let g = svgSel.select<SVGGElement>("g.graph-root");
    if (g.empty()) {
      g = svgSel.append("g").attr("class", "graph-root");

      // Zoom
      const zoom = d3.zoom<SVGSVGElement, unknown>()
        .scaleExtent([0.1, 4])
        .on("zoom", (event) => {
          g.attr("transform", event.transform);
        });
      svgSel.call(zoom);
    }

    // Links
    const linkSel = g
      .selectAll<SVGLineElement, SimLink>("line.edge")
      .data(simLinks, (d) => d.id);

    linkSel.exit().remove();

    const linkEnter = linkSel
      .enter()
      .append("line")
      .attr("class", "edge");

    const linkMerge = linkEnter.merge(linkSel);
    linkMerge
      .attr("stroke", (d) => EDGE_COLORS[d.type] || "#4b5563")
      .attr("stroke-width", 1.5)
      .attr("stroke-opacity", (d) => (d.isPersonal ? 0.5 : 0.6))
      .attr("stroke-dasharray", (d) => (d.isPersonal ? "4,3" : "none"));

    // Nodes
    const nodeSel = g
      .selectAll<SVGGElement, SimNode>("g.node")
      .data(simNodes, (d) => d.id);

    nodeSel.exit().remove();

    const nodeEnter = nodeSel
      .enter()
      .append("g")
      .attr("class", "node")
      .style("cursor", "pointer");

    nodeEnter.append("circle");
    nodeEnter.append("text")
      .attr("dy", "0.35em")
      .attr("text-anchor", "middle")
      .style("fill", "#f9fafb")
      .style("font-size", "10px")
      .style("pointer-events", "none");

    const nodeMerge = nodeEnter.merge(nodeSel);

    nodeMerge
      .select("circle")
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

    nodeMerge
      .select("text")
      .text((d) => truncate(d.title, 14))
      .attr("y", (d) => Math.min(8 + d.edgeCount * 1.5, 20) + 12);

    // Highlight search results
    if (searchQuery && searchResults.length > 0) {
      const resultIds = new Set(searchResults.map((r) => r.id));
      nodeMerge
        .select("circle")
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

    // Click handler
    nodeMerge.on("click", (_event, d) => {
      setSelectedNodeId(d.id === selectedNodeId ? null : d.id);
    });

    // Drag
    const drag = d3.drag<SVGGElement, SimNode>()
      .on("start", (event, d) => {
        if (!event.active) simulation.alphaTarget(0.1).restart();
        d.fx = d.x;
        d.fy = d.y;
      })
      .on("drag", (event, d) => {
        d.fx = event.x;
        d.fy = event.y;
      })
      .on("end", (event, d) => {
        if (!event.active) simulation.alphaTarget(0);
        d.fx = null;
        d.fy = null;
        // Save position
        if (d.x != null && d.y != null) {
          savePositions([{ node_id: d.id, x: d.x, y: d.y }]);
        }
      });

    nodeMerge.call(drag);

    // Tick
    simulation.on("tick", () => {
      linkMerge
        .attr("x1", (d) => ((d.source as SimNode).x ?? 0))
        .attr("y1", (d) => ((d.source as SimNode).y ?? 0))
        .attr("x2", (d) => ((d.target as SimNode).x ?? 0))
        .attr("y2", (d) => ((d.target as SimNode).y ?? 0));

      nodeMerge.attr("transform", (d) => `translate(${d.x ?? 0},${d.y ?? 0})`);
    });

    return () => {
      simulation.stop();
    };
  }, [getDisplayNodes, getDisplayEdges, selectedNodeId, searchResults, searchQuery, setSelectedNodeId, savePositions, getAuthorColor]);

  // Click on empty space to deselect
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
