import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import ReactMarkdown from 'react-markdown';
import { ChevronLeft, ChevronRight, Pencil, Save, X } from 'lucide-react';
import { useGraphStore } from '../../stores/graphStore';
import { getEmojiForNode } from '../../utils/emojiMatcher';
import { ConversationRenderer, isConversationContent } from './ConversationRenderer';
import type { Node } from '../../types/graph';

interface LeafViewProps {
  nodeId: string;
  onBack: () => void;
}

export function LeafView({ nodeId, onBack }: LeafViewProps) {
  const { nodes, navigateToBreadcrumb, closeLeaf, updateNode } = useGraphStore();
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [isEditing, setIsEditing] = useState(false);
  const [editContent, setEditContent] = useState<string>('');
  const [saving, setSaving] = useState(false);

  const node = nodes.get(nodeId);

  // Fetch full content from backend
  useEffect(() => {
    const fetchContent = async () => {
      setLoading(true);
      setError(null);
      try {
        const result = await invoke<string>('get_leaf_content', { nodeId });
        setContent(result);
      } catch (err) {
        console.error('Failed to fetch leaf content:', err);
        setError(err instanceof Error ? err.message : 'Failed to load content');
        // Fall back to node.content if available
        if (node?.content) {
          setContent(node.content);
        }
      } finally {
        setLoading(false);
      }
    };

    fetchContent();
  }, [nodeId, node?.content]);

  // Build hierarchy path for breadcrumbs
  const getHierarchyPath = (): Node[] => {
    const path: Node[] = [];
    let current = node;
    while (current?.parentId) {
      const parent = nodes.get(current.parentId);
      if (parent) {
        path.unshift(parent);
      }
      current = parent;
    }
    return path;
  };

  const hierarchyPath = getHierarchyPath();

  // Start editing
  const handleEdit = () => {
    setEditContent(content || '');
    setIsEditing(true);
  };

  // Cancel editing
  const handleCancel = () => {
    setIsEditing(false);
    setEditContent('');
  };

  // Save changes to database
  const handleSave = async () => {
    if (!node) return;
    setSaving(true);
    try {
      // Update content in database using simpler API
      await invoke('update_node_content', {
        nodeId,
        content: editContent,
      });
      // Update local state
      setContent(editContent);
      updateNode(nodeId, { content: editContent });
      setIsEditing(false);
    } catch (err) {
      console.error('Failed to save:', err);
      alert('Failed to save: ' + (err instanceof Error ? err.message : String(err)));
    } finally {
      setSaving(false);
    }
  };

  if (!node) {
    return (
      <div className="h-full flex items-center justify-center bg-gray-900 text-gray-400">
        <div className="text-center">
          <div className="text-4xl mb-4">🔍</div>
          <p>Node not found</p>
          <button
            onClick={onBack}
            className="mt-4 px-4 py-2 bg-gray-700 hover:bg-gray-600 rounded text-white transition-colors"
          >
            Go Back
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col bg-gray-900">
      {/* Header with breadcrumbs and back button */}
      <header className="flex-shrink-0 border-b border-gray-700 bg-gray-800/50">
        {/* Breadcrumb trail */}
        <div className="px-4 py-2 flex items-center gap-1 text-sm overflow-x-auto">
          <button
            onClick={onBack}
            className="flex items-center gap-1 px-2 py-1 rounded hover:bg-gray-700 text-gray-400 hover:text-white transition-colors"
            title="Back to Graph"
          >
            <ChevronLeft className="w-4 h-4" />
            <span>Graph</span>
          </button>

          {hierarchyPath.map((pathNode) => (
            <span key={pathNode.id} className="flex items-center">
              <ChevronRight className="w-4 h-4 text-gray-600" />
              <button
                onClick={() => {
                  navigateToBreadcrumb(pathNode.id);
                  closeLeaf();
                }}
                className="px-2 py-1 text-gray-400 hover:text-white hover:bg-gray-700 rounded transition-colors"
              >
                {pathNode.isUniverse ? '🌌' : (pathNode.emoji || getEmojiForNode(pathNode))}
                <span className="ml-1">{pathNode.isUniverse ? 'Universe' : (pathNode.aiTitle || pathNode.title)}</span>
              </button>
            </span>
          ))}

          <span className="flex items-center">
            <ChevronRight className="w-4 h-4 text-gray-600" />
            <span className="px-2 py-1 text-amber-300 font-medium">
              {node.emoji || getEmojiForNode(node)}
              <span className="ml-1">{node.aiTitle || node.title}</span>
            </span>
          </span>
        </div>

        {/* Title bar */}
        <div className="px-6 py-4">
          <div className="flex items-start justify-between gap-4">
            <h1 className="text-2xl font-bold text-white flex items-center gap-3">
              <span className="text-3xl">{node.emoji || getEmojiForNode(node)}</span>
              {node.aiTitle || node.title}
            </h1>
            {/* Edit/Save/Cancel buttons */}
            <div className="flex items-center gap-2 shrink-0">
              {isEditing ? (
                <>
                  <button
                    onClick={handleCancel}
                    disabled={saving}
                    className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-gray-700 hover:bg-gray-600 text-gray-300 hover:text-white transition-colors disabled:opacity-50"
                  >
                    <X className="w-4 h-4" />
                    Cancel
                  </button>
                  <button
                    onClick={handleSave}
                    disabled={saving}
                    className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-amber-600 hover:bg-amber-500 text-white transition-colors disabled:opacity-50"
                  >
                    <Save className="w-4 h-4" />
                    {saving ? 'Saving...' : 'Save'}
                  </button>
                </>
              ) : (
                <button
                  onClick={handleEdit}
                  className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-gray-700 hover:bg-gray-600 text-gray-300 hover:text-white transition-colors"
                >
                  <Pencil className="w-4 h-4" />
                  Edit
                </button>
              )}
            </div>
          </div>
          {node.summary && (
            <p className="mt-2 text-gray-400 text-sm">{node.summary}</p>
          )}
          <div className="mt-2 flex items-center gap-4 text-xs text-gray-500">
            <span>Created: {new Date(node.createdAt).toLocaleDateString()}</span>
            {node.url && (
              <a
                href={node.url}
                target="_blank"
                rel="noopener noreferrer"
                className="text-amber-400 hover:text-amber-300 hover:underline"
              >
                Open in Claude →
              </a>
            )}
          </div>
        </div>
      </header>

      {/* Content area */}
      <main className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="flex items-center justify-center h-64">
            <div className="text-center text-gray-400">
              <div className="text-3xl mb-2 animate-pulse">📄</div>
              <p>Loading content...</p>
            </div>
          </div>
        ) : error && !content ? (
          <div className="flex items-center justify-center h-64">
            <div className="text-center text-red-400">
              <div className="text-3xl mb-2">⚠️</div>
              <p>{error}</p>
            </div>
          </div>
        ) : isEditing ? (
          // Edit mode - full-height textarea
          <div className="h-full p-4">
            <textarea
              value={editContent}
              onChange={(e) => setEditContent(e.target.value)}
              className="w-full h-full bg-gray-800 text-gray-200 border border-gray-600 rounded-lg p-4 font-mono text-sm resize-none focus:outline-none focus:border-amber-500 focus:ring-1 focus:ring-amber-500"
              placeholder="Enter content..."
              autoFocus
            />
          </div>
        ) : content && isConversationContent(content) ? (
          // Conversation format - document style
          <article className="max-w-4xl px-8 py-8">
            <ConversationRenderer content={content} />
          </article>
        ) : (
          // Markdown format - prose article
          <article className="max-w-4xl mx-auto px-6 py-8">
            <div className="prose prose-invert prose-lg max-w-none
              prose-headings:text-white prose-headings:font-semibold
              prose-p:text-gray-300 prose-p:leading-relaxed
              prose-a:text-amber-400 prose-a:no-underline hover:prose-a:underline
              prose-strong:text-white
              prose-code:text-amber-300 prose-code:bg-gray-800 prose-code:px-1 prose-code:py-0.5 prose-code:rounded
              prose-pre:bg-gray-800 prose-pre:border prose-pre:border-gray-700
              prose-blockquote:border-l-amber-500 prose-blockquote:bg-gray-800/50 prose-blockquote:py-1 prose-blockquote:px-4
              prose-li:text-gray-300
              prose-hr:border-gray-700
            ">
              <ReactMarkdown>
                {content || '*No content available*'}
              </ReactMarkdown>
            </div>
          </article>
        )}
      </main>
    </div>
  );
}
