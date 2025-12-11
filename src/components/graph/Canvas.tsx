import { useRef, useEffect, useCallback } from 'react';
import { useGraphStore } from '../../stores/graphStore';
import type { Node, Edge } from '../../types/graph';

interface CanvasProps {
  width: number;
  height: number;
}

export function Canvas({ width, height }: CanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { nodes, edges, viewport, activeNodeId, setViewport, setActiveNode } = useGraphStore();

  const draw = useCallback((ctx: CanvasRenderingContext2D) => {
    ctx.clearRect(0, 0, width, height);

    ctx.save();
    ctx.translate(viewport.x + width / 2, viewport.y + height / 2);
    ctx.scale(viewport.zoom, viewport.zoom);

    // Draw edges
    ctx.strokeStyle = 'rgba(0, 0, 0, 0.1)';
    ctx.lineWidth = 1 / viewport.zoom;
    edges.forEach((edge: Edge) => {
      const source = nodes.get(edge.source);
      const target = nodes.get(edge.target);
      if (source && target) {
        ctx.beginPath();
        ctx.moveTo(source.position.x, source.position.y);
        ctx.lineTo(target.position.x, target.position.y);
        ctx.stroke();
      }
    });

    // Draw nodes
    nodes.forEach((node: Node) => {
      const isActive = node.id === activeNodeId;

      // Node circle
      ctx.beginPath();
      ctx.arc(node.position.x, node.position.y, 8, 0, Math.PI * 2);
      ctx.fillStyle = isActive ? '#d97706' : '#f3f4f6';
      ctx.fill();
      ctx.strokeStyle = isActive ? '#d97706' : 'rgba(0, 0, 0, 0.1)';
      ctx.lineWidth = 1 / viewport.zoom;
      ctx.stroke();

      // Node label
      ctx.fillStyle = '#374151';
      ctx.font = `${12 / viewport.zoom}px Inter, system-ui, sans-serif`;
      ctx.textAlign = 'center';
      ctx.fillText(node.title, node.position.x, node.position.y + 20);
    });

    ctx.restore();
  }, [nodes, edges, viewport, activeNodeId, width, height]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext('2d');
    if (ctx) {
      draw(ctx);
    }
  }, [draw]);

  const handleWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault();
    const delta = e.deltaY > 0 ? 0.9 : 1.1;
    const newZoom = Math.max(0.1, Math.min(5, viewport.zoom * delta));
    setViewport({ ...viewport, zoom: newZoom });
  }, [viewport, setViewport]);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const rect = canvas.getBoundingClientRect();
    const x = (e.clientX - rect.left - width / 2 - viewport.x) / viewport.zoom;
    const y = (e.clientY - rect.top - height / 2 - viewport.y) / viewport.zoom;

    // Find clicked node
    for (const [id, node] of nodes) {
      const dx = node.position.x - x;
      const dy = node.position.y - y;
      if (dx * dx + dy * dy < 100) {
        setActiveNode(id);
        return;
      }
    }
    setActiveNode(null);
  }, [nodes, viewport, width, height, setActiveNode]);

  return (
    <canvas
      ref={canvasRef}
      width={width}
      height={height}
      onWheel={handleWheel}
      onMouseDown={handleMouseDown}
      className="bg-white"
    />
  );
}
