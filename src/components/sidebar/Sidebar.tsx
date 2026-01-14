import { useState, useEffect, useCallback, useDeferredValue, useMemo } from 'react';
import { Search, X, ChevronRight, ChevronDown, Settings, Pin, Clock, PinOff, GripVertical, Rabbit } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useGraphStore } from '../../stores/graphStore';
import { getEmojiForNode, initLearnedMappings } from '../../utils/emojiMatcher';
import { SessionsPanel } from '../sessions/SessionsPanel';
import type { Node } from '../../types/graph';

type SidebarTab = 'pinned' | 'search' | 'sessions';

const MIN_SIDEBAR_WIDTH = 200;
const MAX_SIDEBAR_WIDTH = 400;

interface SidebarProps {
  onOpenSettings?: () => void;
}

export function Sidebar({ onOpenSettings }: SidebarProps) {
  const { nodes, activeNodeId, setActiveNode, navigateToRoot, jumpToNode } = useGraphStore();
  const [activeTab, setActiveTab] = useState<SidebarTab>('pinned');
  const [searchQuery, setSearchQuery] = useState('');

  // Resize state
  const [sidebarWidth, setSidebarWidth] = useState(MIN_SIDEBAR_WIDTH);
  const [isResizing, setIsResizing] = useState(false);

  // Quick access state
  const [recentNodes, setRecentNodes] = useState<Node[]>([]);
  const [pinnedNodes, setPinnedNodes] = useState<Node[]>([]);
  const [pinnedIds, setPinnedIds] = useState<Set<string>>(new Set());
  const [recentExpanded, setRecentExpanded] = useState(true); // Will adjust after pinnedNodes load

  // Load learned emojis on mount
  useEffect(() => {
    loadLearnedEmojis();
  }, []);

  // Load quick access data
  useEffect(() => {
    loadRecentNodes();
    loadPinnedNodes();
  }, []);

  // Listen for pins-changed event (from Graph details pane)
  useEffect(() => {
    const unlisten = listen('pins-changed', () => {
      loadPinnedNodes();
    });
    return () => { unlisten.then(f => f()); };
  }, []);

  // Touch node whenever activeNodeId changes (from graph or sidebar clicks)
  useEffect(() => {
    if (activeNodeId) {
      invoke('touch_node', { nodeId: activeNodeId })
        .then(() => loadRecentNodes())
        .catch(console.error);
    }
  }, [activeNodeId]);

  // Sidebar resize handling
  useEffect(() => {
    if (!isResizing) return;

    const handleMouseMove = (e: MouseEvent) => {
      const newWidth = Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, e.clientX));
      setSidebarWidth(newWidth);
    };

    const handleMouseUp = () => {
      setIsResizing(false);
    };

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isResizing]);

  const loadLearnedEmojis = async () => {
    try {
      const mappings = await invoke<Record<string, string>>('get_learned_emojis');
      initLearnedMappings(mappings);
    } catch (err) {
      console.error('Failed to load learned emojis:', err);
    }
  };

  const loadRecentNodes = async () => {
    try {
      const recent = await invoke<Node[]>('get_recent_nodes', { limit: 15 });
      setRecentNodes(recent);
    } catch (err) {
      console.error('Failed to load recent nodes:', err);
    }
  };

  const loadPinnedNodes = async () => {
    try {
      const pinned = await invoke<Node[]>('get_pinned_nodes');
      setPinnedNodes(pinned);
      setPinnedIds(new Set(pinned.map(n => n.id)));
      // Auto-expand recent if no pinned nodes
      if (pinned.length === 0) {
        setRecentExpanded(true);
      }
    } catch (err) {
      console.error('Failed to load pinned nodes:', err);
    }
  };

  // Helper to get emoji for a node - uses stored emoji or falls back to matcher
  const getNodeEmoji = (node: { emoji?: string; title?: string; aiTitle?: string; tags?: string[]; content?: string }) => {
    if (node.emoji) return node.emoji;
    return getEmojiForNode({
      title: node.aiTitle || node.title,
      tags: node.tags,
      content: node.content
    });
  };

  const handleNodeClick = useCallback((nodeId: string) => {
    const node = nodes.get(nodeId);
    if (node) {
      // Jump to node (navigate to its parent so the node is visible)
      jumpToNode(node, undefined);
    }
    // touch_node is handled by the useEffect on activeNodeId
  }, [nodes, jumpToNode]);

  const handleTogglePin = useCallback(async (nodeId: string, currentlyPinned: boolean) => {
    try {
      await invoke('set_node_pinned', { nodeId, pinned: !currentlyPinned });
      // Refresh pinned list
      loadPinnedNodes();
    } catch (err) {
      console.error('Failed to toggle pin:', err);
    }
  }, []);

  // Debounced search query (React 18's useDeferredValue defers updates during typing)
  const deferredQuery = useDeferredValue(searchQuery);

  // Memoized search results to prevent unnecessary recalculations
  const searchResults = useMemo(() => {
    if (activeTab !== 'search' || !deferredQuery) return [];

    const query = deferredQuery.toLowerCase();
    return Array.from(nodes.values()).filter(node =>
      node.title.toLowerCase().includes(query) ||
      node.aiTitle?.toLowerCase().includes(query) ||
      node.content?.toLowerCase().includes(query) ||
      node.summary?.toLowerCase().includes(query)
    );
  }, [activeTab, deferredQuery, nodes]);

  // Render a single node item
  const renderNodeItem = (node: Node, showPinButton = true) => {
    const isPinned = pinnedIds.has(node.id);  // O(1) lookup

    return (
      <div
        key={node.id}
        className={`group flex items-center gap-2 px-2 py-2 rounded-lg text-sm transition-all duration-150 ${
          node.id === activeNodeId
            ? 'bg-amber-500/20 ring-1 ring-amber-500/50'
            : 'hover:bg-gray-700/50'
        }`}
      >
        <span className="text-base shrink-0">{getNodeEmoji(node)}</span>

        <button
          onClick={() => handleNodeClick(node.id)}
          className="flex-1 min-w-0 text-left"
        >
          <div className={`font-medium truncate ${
            node.id === activeNodeId ? 'text-amber-200' : 'text-gray-200'
          }`}>
            {node.aiTitle || node.title}
          </div>
          {node.summary && (
            <div className="text-xs text-gray-500 mt-0.5 truncate">
              {node.summary.slice(0, 60)}...
            </div>
          )}
        </button>

        {showPinButton && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              handleTogglePin(node.id, isPinned);
            }}
            className={`p-1 rounded transition-colors shrink-0 ${
              isPinned
                ? 'text-amber-400 hover:text-amber-300'
                : 'text-gray-500 hover:text-gray-300'
            }`}
            title={isPinned ? 'Unpin' : 'Pin'}
          >
            {isPinned ? <PinOff className="w-3.5 h-3.5" /> : <Pin className="w-3.5 h-3.5" />}
          </button>
        )}
      </div>
    );
  };

  return (
    <aside
      className="h-full bg-gray-800/95 backdrop-blur-sm border-r border-gray-700/50 flex flex-col relative"
      style={{ width: sidebarWidth }}
    >
      {/* Resize handle */}
      <div
        className={`absolute top-0 right-0 w-1 h-full cursor-col-resize group z-10 ${
          isResizing ? 'bg-amber-500/50' : 'hover:bg-amber-500/30'
        }`}
        onMouseDown={(e) => {
          e.preventDefault();
          setIsResizing(true);
        }}
      >
        <div className={`absolute top-1/2 -translate-y-1/2 -left-1 w-3 h-8 flex items-center justify-center rounded ${
          isResizing ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'
        } transition-opacity`}>
          <GripVertical className="w-3 h-3 text-gray-400" />
        </div>
      </div>
      {/* Header */}
      <div className="p-4 border-b border-gray-700/50">
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-2">
            <span className="text-2xl">🍄</span>
            <h1 className="text-lg font-semibold text-white">Mycelica</h1>
          </div>
          <button
            onClick={onOpenSettings}
            className="p-2 rounded-lg transition-colors text-gray-400 hover:text-gray-200 hover:bg-gray-700/50"
            title="Settings"
          >
            <Settings className="w-4 h-4" />
          </button>
        </div>

        {/* Tabs */}
        <div className="flex gap-1 bg-gray-900/50 p-1 rounded-lg">
          <button
            onClick={() => setActiveTab('pinned')}
            className={`flex-1 flex items-center justify-center gap-1.5 px-2 py-1.5 rounded-md text-xs font-medium transition-colors ${
              activeTab === 'pinned'
                ? 'bg-gray-700 text-white'
                : 'text-gray-400 hover:text-gray-200'
            }`}
          >
            <Pin className="w-3.5 h-3.5" />
            Pinned
          </button>
          <button
            onClick={() => setActiveTab('search')}
            className={`flex-1 flex items-center justify-center gap-1.5 px-2 py-1.5 rounded-md text-xs font-medium transition-colors ${
              activeTab === 'search'
                ? 'bg-gray-700 text-white'
                : 'text-gray-400 hover:text-gray-200'
            }`}
          >
            <Search className="w-3.5 h-3.5" />
            Search
          </button>
          <button
            onClick={() => setActiveTab('sessions')}
            className={`flex-1 flex items-center justify-center gap-1.5 px-2 py-1.5 rounded-md text-xs font-medium transition-colors ${
              activeTab === 'sessions'
                ? 'bg-gray-700 text-white'
                : 'text-gray-400 hover:text-gray-200'
            }`}
            title="Browsing Sessions"
          >
            <Rabbit className="w-3.5 h-3.5" />
            Sessions
          </button>
        </div>
      </div>

      {/* Search input (only on search tab) */}
      {activeTab === 'search' && (
        <div className="px-4 py-3 border-b border-gray-700/50">
          <div className="relative">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-500" strokeWidth={1.5} />
            <input
              type="text"
              placeholder="Search nodes..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              autoFocus
              className="w-full pl-9 pr-8 py-2 bg-gray-900/50 border border-gray-700/50 rounded-lg text-sm text-white placeholder-gray-500 focus:bg-gray-900 focus:border-amber-500/50 focus:ring-1 focus:ring-amber-500/20 focus:outline-none transition-colors duration-150"
            />
            {searchQuery && (
              <button
                onClick={() => setSearchQuery('')}
                className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-gray-500 hover:text-gray-300"
              >
                <X className="w-4 h-4" />
              </button>
            )}
          </div>
          {searchQuery && (
            <div className="mt-2 text-xs text-gray-400">
              {searchResults.length} result{searchResults.length !== 1 ? 's' : ''}
            </div>
          )}
        </div>
      )}

      {/* Node list */}
      <div className="flex-1 overflow-y-auto p-2">
        {/* Pinned tab */}
        {activeTab === 'pinned' && (() => {
          // Find Universe node
          const universeNode = Array.from(nodes.values()).find(n => n.isUniverse);
          // User-pinned nodes (excluding Universe)
          const userPinnedNodes = pinnedNodes.filter(n => !n.isUniverse);

          return (
            <>
              {/* Universe - always shown at top */}
              {universeNode && (
                <div className="mb-2">
                  <div
                    className={`group flex items-center gap-2 px-3 py-2 rounded-lg text-sm transition-colors cursor-pointer ${
                      activeNodeId === universeNode.id
                        ? 'bg-purple-500/20 text-purple-300'
                        : 'text-gray-300 hover:bg-gray-700/50'
                    }`}
                    onClick={() => {
                      navigateToRoot();
                      setActiveNode(universeNode.id);
                    }}
                  >
                    <span className="text-lg">🌌</span>
                    <span className="flex-1 truncate font-medium">Universe</span>
                  </div>
                </div>
              )}

              {/* User pinned nodes */}
              {userPinnedNodes.length > 0 ? (
                <div className="space-y-1">
                  {userPinnedNodes.map(node => renderNodeItem(node, true))}
                </div>
              ) : (
                <div className="p-8 text-center text-gray-500">
                  <Pin className="w-8 h-8 mx-auto mb-2 opacity-50" />
                  <p className="text-sm">No pinned nodes</p>
                  <p className="text-xs mt-1">Pin nodes for quick access</p>
                </div>
              )}
            </>
          );
        })()}

        {/* Search tab */}
        {activeTab === 'search' && (
          <>
            {searchQuery ? (
              searchResults.length > 0 ? (
                <div className="space-y-1">
                  {searchResults.slice(0, 50).map(node => renderNodeItem(node))}
                  {searchResults.length > 50 && (
                    <div className="px-3 py-2 text-xs text-gray-500 text-center">
                      +{searchResults.length - 50} more results...
                    </div>
                  )}
                </div>
              ) : (
                <div className="p-8 text-center text-gray-500">
                  <Search className="w-8 h-8 mx-auto mb-2 opacity-50" />
                  <p className="text-sm">No results found</p>
                  <p className="text-xs mt-1">Try a different search term</p>
                </div>
              )
            ) : (
              <div className="p-8 text-center text-gray-500">
                <Search className="w-8 h-8 mx-auto mb-2 opacity-50" />
                <p className="text-sm">Search your knowledge</p>
                <p className="text-xs mt-1">Type to search titles, summaries, and content</p>
              </div>
            )}
          </>
        )}

        {/* Sessions tab */}
        {activeTab === 'sessions' && (
          <SessionsPanel />
        )}
      </div>

      {/* Collapsible Recent section */}
      <div className="border-t border-gray-700/50 bg-gray-850">
        <button
          onClick={() => setRecentExpanded(!recentExpanded)}
          className="w-full flex items-center justify-between px-3 py-2 text-xs font-medium text-gray-400 hover:text-gray-200 hover:bg-gray-700/30 transition-colors"
        >
          <div className="flex items-center gap-1.5">
            {recentExpanded ? (
              <ChevronDown className="w-3.5 h-3.5" />
            ) : (
              <ChevronRight className="w-3.5 h-3.5" />
            )}
            <Clock className="w-3.5 h-3.5" />
            <span>Recent</span>
            {recentNodes.length > 0 && (
              <span className="text-gray-500">({recentNodes.length})</span>
            )}
          </div>
        </button>
        {recentExpanded && (
          <div className="px-2 pb-2 max-h-60 overflow-y-auto">
            {recentNodes.length > 0 ? (
              <div className="space-y-0.5">
                {recentNodes.slice(0, 10).map(node => {
                  const isActive = node.id === activeNodeId;
                  const isPinned = pinnedIds.has(node.id);
                  return (
                  <div
                    key={node.id}
                    className={`group flex items-center gap-1.5 px-1.5 py-1 rounded text-xs transition-colors ${
                      isActive
                        ? 'bg-amber-500/20 text-amber-300'
                        : 'text-gray-400 hover:text-gray-200 hover:bg-gray-700/30'
                    }`}
                  >
                    <span className="shrink-0">{getNodeEmoji(node)}</span>
                    <button
                      onClick={() => handleNodeClick(node.id)}
                      className="flex-1 text-left truncate"
                    >
                      <span className="truncate">{node.aiTitle || node.title}</span>
                    </button>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        handleTogglePin(node.id, isPinned);
                      }}
                      className={`p-1 rounded transition-colors shrink-0 ${
                        isPinned
                          ? 'text-amber-400 hover:text-amber-300'
                          : 'text-gray-500 hover:text-gray-300'
                      }`}
                      title={isPinned ? 'Unpin' : 'Pin'}
                    >
                      {isPinned ? <PinOff className="w-3 h-3" /> : <Pin className="w-3 h-3" />}
                    </button>
                    <button
                      onClick={async (e) => {
                        e.stopPropagation();
                        try {
                          await invoke('clear_recent', { nodeId: node.id });
                          setRecentNodes(prev => prev.filter(n => n.id !== node.id));
                        } catch (err) {
                          console.error('Failed to remove from recents:', err);
                        }
                      }}
                      className="p-1 text-gray-500 hover:text-red-400 rounded transition-colors shrink-0"
                      title="Remove from recents"
                    >
                      <X className="w-3 h-3" />
                    </button>
                  </div>
                  );
                })}
              </div>
            ) : (
              <div className="px-2 py-3 text-xs text-gray-500 text-center">
                No recent activity
              </div>
            )}
          </div>
        )}
      </div>

    </aside>
  );
}
