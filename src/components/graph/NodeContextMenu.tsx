import React from 'react';

interface NodeContextMenuProps {
  nodeId: string | null;
  position: { x: number; y: number } | null;
  onClose: () => void;
  onDelete: (nodeId: string) => void;
}

export const NodeContextMenu = React.memo(function NodeContextMenu({
  nodeId,
  position,
  onClose,
  onDelete,
}: NodeContextMenuProps) {
  if (!nodeId || !position) return null;

  return (
    <>
      <div
        className="fixed inset-0 z-[55]"
        onClick={onClose}
      />
      <div
        className="fixed z-[56] bg-gray-800 border border-gray-700 rounded-lg shadow-xl py-1 min-w-[120px]"
        style={{ left: position.x, top: position.y }}
      >
        <button
          onClick={() => {
            onDelete(nodeId);
            onClose();
          }}
          className="w-full px-4 py-2 text-left text-sm text-red-400 hover:bg-gray-700 flex items-center gap-2"
        >
          <span>🗑️</span>
          <span>Delete</span>
        </button>
      </div>
    </>
  );
});
