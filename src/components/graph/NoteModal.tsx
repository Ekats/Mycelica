import React, { useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Node } from '../../types/graph';

interface NoteModalProps {
  isOpen: boolean;
  onClose: () => void;
  nodes: Map<string, Node>;
  addNode: (node: Partial<Node>) => void;
  updateNode: (id: string, updates: Partial<Node>) => void;
}

export const NoteModal = React.memo(function NoteModal({
  isOpen,
  onClose,
  nodes,
  addNode,
  updateNode,
}: NoteModalProps) {
  const [noteTitle, setNoteTitle] = useState('');
  const [noteContent, setNoteContent] = useState('');

  const handleClose = useCallback(() => {
    onClose();
    setNoteTitle('');
    setNoteContent('');
  }, [onClose]);

  const handleSave = useCallback(async () => {
    if (!noteContent.trim()) return;

    const title = noteTitle.trim() || 'Untitled Note';
    const content = noteContent.trim();
    const containerId = 'container-recent-notes';

    // Save to database and get the ID
    const noteId = await invoke<string>('add_note', { title, content });

    const now = Date.now();

    // Check if Recent Notes container exists in local state
    const container = nodes.get(containerId);
    if (!container) {
      // Container was just created in backend - add to local state
      const universe = Array.from(nodes.values()).find(n => n.isUniverse);
      addNode({
        id: containerId,
        type: 'cluster',
        title: 'Recent Notes',
        emoji: '📝',
        depth: 1,
        isItem: false,
        isUniverse: false,
        parentId: universe?.id,
        childCount: 1,
        position: { x: 0, y: 0 },
        createdAt: now,
        updatedAt: now,
        latestChildDate: now,
        isProcessed: true,
        isPinned: false,
      });
    } else {
      // Update existing container's childCount and latestChildDate
      updateNode(containerId, {
        childCount: container.childCount + 1,
        latestChildDate: now,
      });
    }

    // Add note to local state
    addNode({
      id: noteId,
      type: 'thought',
      title,
      content,
      depth: 2,
      isItem: true,
      isUniverse: false,
      parentId: containerId,
      childCount: 0,
      position: { x: 0, y: 0 },
      createdAt: now,
      updatedAt: now,
      isProcessed: false,
      isPinned: false,
    });

    handleClose();
  }, [noteTitle, noteContent, nodes, addNode, updateNode, handleClose]);

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center">
      <div
        className="absolute inset-0 bg-black/60 backdrop-blur-sm"
        onClick={handleClose}
      />
      <div className="relative bg-gray-800 rounded-lg border border-gray-700 shadow-xl max-w-lg w-full mx-4 p-6">
        <h2 className="text-lg font-medium text-white mb-4">Add Note</h2>
        <input
          type="text"
          placeholder="Title (optional)"
          value={noteTitle}
          onChange={(e) => setNoteTitle(e.target.value)}
          className="w-full bg-gray-700 text-white rounded px-3 py-2 mb-3 border border-gray-600 focus:border-amber-500 focus:outline-none"
        />
        <textarea
          placeholder="What's on your mind?"
          value={noteContent}
          onChange={(e) => setNoteContent(e.target.value)}
          rows={6}
          className="w-full bg-gray-700 text-white rounded px-3 py-2 mb-4 resize-none border border-gray-600 focus:border-amber-500 focus:outline-none"
          autoFocus
        />
        <div className="flex gap-2 justify-end">
          <button
            onClick={handleClose}
            className="px-4 py-2 text-gray-400 hover:text-white"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={!noteContent.trim()}
            className="px-4 py-2 bg-amber-600 hover:bg-amber-500 disabled:opacity-50 disabled:cursor-not-allowed text-white rounded"
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
});
