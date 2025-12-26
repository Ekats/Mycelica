import React from 'react';

interface GraphStatusBarProps {
  nodeCount: number;
  edgeCount: number;
  zoomLevel: number;
}

export const GraphStatusBar = React.memo(function GraphStatusBar({
  nodeCount,
  edgeCount,
  zoomLevel,
}: GraphStatusBarProps) {
  return (
    <>
      {/* Status bar */}
      <div className="absolute bottom-4 left-4 bg-gray-800/90 backdrop-blur-sm rounded-lg px-4 py-2 text-sm text-gray-300 flex items-center gap-4">
        <span>{nodeCount} nodes</span>
        <span className="text-gray-500">|</span>
        <span>{edgeCount} edges</span>
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

      {/* Help text */}
      <div className="absolute bottom-4 right-4 bg-gray-800/80 backdrop-blur-sm rounded-lg px-3 py-2 text-xs text-gray-400">
        Scroll to zoom - Click and drag to pan
      </div>
    </>
  );
});
