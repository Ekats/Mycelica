import React, { RefObject } from 'react';

export interface DevConsoleLog {
  type: 'info' | 'warn' | 'error';
  message: string;
  time: string;
}

const logColors: Record<string, string> = {
  info: 'text-blue-400',
  warn: 'text-yellow-400',
  error: 'text-red-400',
};

interface DevConsoleProps {
  logs: DevConsoleLog[];
  nodeCount: number;
  edgeCount: number;
  zoomLevel: number;
  autoScroll: boolean;
  setAutoScroll: (value: boolean) => void;
  consoleSize: { width: number; height: number };
  consoleRef: RefObject<HTMLDivElement>;
  onClear: () => void;
  onListNodes: () => void;
  onListPath: () => void;
  onListHierarchy: () => void;
  isResizing: boolean;
  onResizeStart: (e: React.MouseEvent) => void;
  positionAtBottom: boolean;
}

export const DevConsole = React.memo(function DevConsole({
  logs,
  nodeCount,
  edgeCount,
  zoomLevel,
  autoScroll,
  setAutoScroll,
  consoleSize,
  consoleRef,
  onClear,
  onListNodes,
  onListPath,
  onListHierarchy,
  isResizing,
  onResizeStart,
  positionAtBottom,
}: DevConsoleProps) {
  return (
    <div
      className={`absolute bg-gray-900/95 backdrop-blur-sm rounded-lg border border-gray-700 overflow-hidden flex flex-col ${
        positionAtBottom
          ? 'bottom-4 right-4'
          : 'top-14 right-4'
      }`}
      style={{ width: consoleSize.width, height: consoleSize.height }}
    >
      {/* Header */}
      <div className="px-3 py-2 bg-gray-800/80 border-b border-gray-700 flex items-center justify-between flex-shrink-0">
        <span className="text-xs font-semibold text-gray-300">Dev Console</span>
        <div className="flex items-center gap-3 text-xs">
          <span className="text-gray-500">Nodes: {nodeCount}</span>
          <span className="text-gray-500">Edges: {edgeCount}</span>
          <span className="text-amber-400">{(zoomLevel * 100).toFixed(0)}%</span>
        </div>
      </div>

      {/* Log content */}
      <div ref={consoleRef} className="flex-1 overflow-y-auto p-2 font-mono text-xs space-y-0.5 min-h-0">
        {logs.length === 0 ? (
          <div className="text-gray-600 text-center py-4">No logs yet</div>
        ) : (
          logs.map((log, i) => (
            <div key={i} className="flex gap-2">
              <span className="text-gray-600 flex-shrink-0">{log.time}</span>
              <span className={logColors[log.type]}>[{log.type.toUpperCase()}]</span>
              <span className="text-gray-300 break-all">{log.message}</span>
            </div>
          ))
        )}
      </div>

      {/* Footer controls */}
      <div className="px-3 py-2 bg-gray-800/50 border-t border-gray-700 flex items-center justify-between text-xs flex-shrink-0">
        <span className="text-gray-500">{logs.length} logs</span>
        <div className="flex items-center gap-3">
          <button
            onClick={onListNodes}
            className="text-blue-400 hover:text-blue-300 transition-colors"
            title="List all nodes in current view"
          >
            Nodes
          </button>
          <button
            onClick={onListPath}
            className="text-green-400 hover:text-green-300 transition-colors"
            title="Show current view's hierarchy path"
          >
            Path
          </button>
          <button
            onClick={onListHierarchy}
            className="text-purple-400 hover:text-purple-300 transition-colors"
            title="List full hierarchy tree (4 levels, 6 items/level)"
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
            onClick={onClear}
            className="text-gray-500 hover:text-red-400 transition-colors"
          >
            Clear
          </button>
        </div>
      </div>

      {/* Resize handle */}
      <div
        onMouseDown={onResizeStart}
        className={`absolute w-4 h-4 group ${isResizing ? 'bg-amber-500/30' : ''} ${
          positionAtBottom
            ? 'top-0 left-0 cursor-nw-resize'
            : 'bottom-0 left-0 cursor-sw-resize'
        }`}
        title="Drag to resize"
      >
        <svg
          className={`w-3 h-3 absolute text-gray-500 group-hover:text-amber-400 transition-colors ${
            positionAtBottom
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
  );
});
