import { useState, useEffect, useCallback } from 'react';
import { Search, X, ChevronRight, ChevronDown, Settings, Key, Check, AlertCircle, Pin, Clock, PinOff } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { useGraphStore } from '../../stores/graphStore';
import { getEmojiForNode, initLearnedMappings } from '../../utils/emojiMatcher';
import type { Node } from '../../types/graph';

interface ApiKeyStatus {
  hasKey: boolean;
  maskedKey: string | null;
  source: string;
}

type SidebarTab = 'pinned' | 'search';

export function Sidebar() {
  const { nodes, activeNodeId, setActiveNode } = useGraphStore();
  const [activeTab, setActiveTab] = useState<SidebarTab>('pinned');
  const [searchQuery, setSearchQuery] = useState('');
  const [showSettings, setShowSettings] = useState(false);
  const [apiKeyStatus, setApiKeyStatus] = useState<ApiKeyStatus | null>(null);
  const [apiKeyInput, setApiKeyInput] = useState('');
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  // OpenAI API key state
  const [openaiKeyStatus, setOpenaiKeyStatus] = useState<string | null>(null); // masked key or null
  const [openaiKeyInput, setOpenaiKeyInput] = useState('');
  const [savingOpenai, setSavingOpenai] = useState(false);
  const [openaiSaveError, setOpenaiSaveError] = useState<string | null>(null);

  // Quick access state
  const [recentNodes, setRecentNodes] = useState<Node[]>([]);
  const [pinnedNodes, setPinnedNodes] = useState<Node[]>([]);
  const [pinnedIds, setPinnedIds] = useState<Set<string>>(new Set());
  const [recentExpanded, setRecentExpanded] = useState(true); // Will adjust after pinnedNodes load

  // Load API key status and learned emojis on mount
  useEffect(() => {
    loadApiKeyStatus();
    loadOpenaiKeyStatus();
    loadLearnedEmojis();
  }, []);

  // Load quick access data
  useEffect(() => {
    loadRecentNodes();
    loadPinnedNodes();
  }, []);

  // Touch node whenever activeNodeId changes (from graph or sidebar clicks)
  useEffect(() => {
    if (activeNodeId) {
      invoke('touch_node', { nodeId: activeNodeId })
        .then(() => loadRecentNodes())
        .catch(console.error);
    }
  }, [activeNodeId]);

  const loadApiKeyStatus = async () => {
    try {
      const status = await invoke<ApiKeyStatus>('get_api_key_status');
      setApiKeyStatus(status);
    } catch (err) {
      console.error('Failed to load API key status:', err);
    }
  };

  const loadOpenaiKeyStatus = async () => {
    try {
      const maskedKey = await invoke<string | null>('get_openai_api_key_status');
      setOpenaiKeyStatus(maskedKey);
    } catch (err) {
      console.error('Failed to load OpenAI API key status:', err);
    }
  };

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

  const handleSaveApiKey = async () => {
    if (!apiKeyInput.trim()) return;

    setSaving(true);
    setSaveError(null);

    try {
      await invoke('save_api_key', { key: apiKeyInput.trim() });
      await loadApiKeyStatus();
      setApiKeyInput('');
    } catch (err) {
      setSaveError(err as string);
    } finally {
      setSaving(false);
    }
  };

  const handleClearApiKey = async () => {
    try {
      await invoke('clear_api_key');
      await loadApiKeyStatus();
    } catch (err) {
      console.error('Failed to clear API key:', err);
    }
  };

  const handleSaveOpenaiKey = async () => {
    if (!openaiKeyInput.trim()) return;

    setSavingOpenai(true);
    setOpenaiSaveError(null);

    try {
      await invoke('save_openai_api_key', { key: openaiKeyInput.trim() });
      await loadOpenaiKeyStatus();
      setOpenaiKeyInput('');
    } catch (err) {
      setOpenaiSaveError(err as string);
    } finally {
      setSavingOpenai(false);
    }
  };

  const handleClearOpenaiKey = async () => {
    try {
      await invoke('clear_openai_api_key');
      await loadOpenaiKeyStatus();
    } catch (err) {
      console.error('Failed to clear OpenAI API key:', err);
    }
  };

  const handleNodeClick = useCallback((nodeId: string) => {
    setActiveNode(nodeId);
    // touch_node is handled by the useEffect on activeNodeId
  }, [setActiveNode]);

  const handleTogglePin = useCallback(async (nodeId: string, currentlyPinned: boolean) => {
    try {
      await invoke('set_node_pinned', { nodeId, pinned: !currentlyPinned });
      // Refresh pinned list
      loadPinnedNodes();
    } catch (err) {
      console.error('Failed to toggle pin:', err);
    }
  }, []);

  // Filter nodes by search query (only when search tab is active)
  const searchResults = activeTab === 'search' && searchQuery
    ? Array.from(nodes.values()).filter(node => {
        const query = searchQuery.toLowerCase();
        return (
          node.title.toLowerCase().includes(query) ||
          node.aiTitle?.toLowerCase().includes(query) ||
          node.content?.toLowerCase().includes(query) ||
          node.summary?.toLowerCase().includes(query)
        );
      })
    : [];

  // Render a single node item
  const renderNodeItem = (node: Node, showPinButton = true) => {
    const isPinned = pinnedIds.has(node.id);  // O(1) lookup

    return (
      <div
        key={node.id}
        className={`group flex items-center gap-2 px-3 py-2 rounded-lg text-sm transition-all duration-150 ${
          node.id === activeNodeId
            ? 'bg-amber-500/20 ring-1 ring-amber-500/50'
            : 'hover:bg-gray-700/50'
        }`}
      >
        <button
          onClick={() => handleNodeClick(node.id)}
          className="flex-1 min-w-0 text-left"
        >
          <div className={`font-medium truncate ${
            node.id === activeNodeId ? 'text-amber-200' : 'text-gray-200'
          }`}>
            <span className="mr-1.5">{getNodeEmoji(node)}</span>
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
            className={`p-1 rounded transition-colors opacity-0 group-hover:opacity-100 ${
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
    <aside className="w-72 h-full bg-gray-800/95 backdrop-blur-sm border-r border-gray-700/50 flex flex-col">
      {/* Header */}
      <div className="p-4 border-b border-gray-700/50">
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-2">
            <span className="text-2xl">🍄</span>
            <h1 className="text-lg font-semibold text-white">Mycelica</h1>
          </div>
          <button
            onClick={() => setShowSettings(!showSettings)}
            className={`p-2 rounded-lg transition-colors ${
              showSettings
                ? 'bg-amber-500/20 text-amber-400'
                : 'text-gray-400 hover:text-gray-200 hover:bg-gray-700/50'
            }`}
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
        {activeTab === 'pinned' && (
          <>
            {pinnedNodes.length > 0 ? (
              <div className="space-y-1">
                {pinnedNodes.map(node => renderNodeItem(node, true))}
              </div>
            ) : (
              <div className="p-8 text-center text-gray-500">
                <Pin className="w-8 h-8 mx-auto mb-2 opacity-50" />
                <p className="text-sm">No pinned nodes</p>
                <p className="text-xs mt-1">Pin nodes for quick access</p>
              </div>
            )}
          </>
        )}

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
            {recentNodes.filter(n => n.id !== activeNodeId).length > 0 ? (
              <div className="space-y-0.5">
                {recentNodes.filter(n => n.id !== activeNodeId).slice(0, 10).map(node => (
                  <div
                    key={node.id}
                    className="group flex items-center gap-1 px-2 py-1.5 rounded text-xs transition-colors text-gray-400 hover:text-gray-200 hover:bg-gray-700/30"
                  >
                    <button
                      onClick={() => handleNodeClick(node.id)}
                      className="flex-1 flex items-center gap-2 text-left truncate"
                    >
                      <span>{getNodeEmoji(node)}</span>
                      <span className="truncate">{node.aiTitle || node.title}</span>
                    </button>
                    <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          handleTogglePin(node.id, node.isPinned);
                        }}
                        className={`p-1 rounded transition-colors ${
                          pinnedIds.has(node.id)
                            ? 'text-amber-400 hover:text-amber-300'
                            : 'text-gray-500 hover:text-gray-300'
                        }`}
                        title={pinnedIds.has(node.id) ? 'Unpin' : 'Pin'}
                      >
                        {pinnedIds.has(node.id) ? <PinOff className="w-3 h-3" /> : <Pin className="w-3 h-3" />}
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
                        className="p-1 text-gray-500 hover:text-red-400 rounded transition-colors"
                        title="Remove from recents"
                      >
                        <X className="w-3 h-3" />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              <div className="px-2 py-3 text-xs text-gray-500 text-center">
                No recent activity
              </div>
            )}
          </div>
        )}
      </div>

      {/* Settings panel */}
      {showSettings && (
        <div className="border-t border-gray-700/50 bg-gray-800/95 p-4">
          <div className="flex items-center gap-2 mb-3">
            <Key className="w-4 h-4 text-gray-400" />
            <span className="text-sm font-medium text-gray-200">Anthropic API Key</span>
          </div>

          {/* Status indicator */}
          {apiKeyStatus && (
            <div className={`flex items-center gap-2 mb-3 text-xs ${
              apiKeyStatus.hasKey ? 'text-green-400' : 'text-amber-400'
            }`}>
              {apiKeyStatus.hasKey ? (
                <>
                  <Check className="w-3 h-3" />
                  <span>
                    Key configured
                    {apiKeyStatus.source === 'env' && ' (from environment)'}
                  </span>
                </>
              ) : (
                <>
                  <AlertCircle className="w-3 h-3" />
                  <span>No API key set</span>
                </>
              )}
            </div>
          )}

          {/* Show masked key if set */}
          {apiKeyStatus?.maskedKey && (
            <div className="mb-3 px-2 py-1 bg-gray-900/50 rounded text-xs text-gray-400 font-mono">
              {apiKeyStatus.maskedKey}
            </div>
          )}

          {/* API key input */}
          <div className="space-y-2">
            <input
              type="password"
              placeholder="sk-ant-api03-..."
              value={apiKeyInput}
              onChange={(e) => setApiKeyInput(e.target.value)}
              className="w-full px-3 py-2 bg-gray-900/50 border border-gray-700/50 rounded text-sm text-white placeholder-gray-500 focus:border-amber-500/50 focus:ring-1 focus:ring-amber-500/20 focus:outline-none"
            />

            {saveError && (
              <p className="text-xs text-red-400">{saveError}</p>
            )}

            <div className="flex gap-2">
              <button
                onClick={handleSaveApiKey}
                disabled={saving || !apiKeyInput.trim()}
                className="flex-1 px-3 py-1.5 bg-amber-500/20 text-amber-200 rounded text-sm font-medium hover:bg-amber-500/30 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
              >
                {saving ? 'Saving...' : 'Save Key'}
              </button>

              {apiKeyStatus?.hasKey && apiKeyStatus.source !== 'env' && (
                <button
                  onClick={handleClearApiKey}
                  className="px-3 py-1.5 bg-red-500/20 text-red-300 rounded text-sm font-medium hover:bg-red-500/30 transition-colors"
                >
                  Clear
                </button>
              )}
            </div>
          </div>

          <p className="mt-3 text-xs text-gray-500">
            Get your API key from{' '}
            <a
              href="https://console.anthropic.com/settings/keys"
              target="_blank"
              rel="noopener noreferrer"
              className="text-amber-400 hover:underline"
            >
              console.anthropic.com
            </a>
          </p>

          {/* OpenAI API Key Section */}
          <div className="mt-6 pt-4 border-t border-gray-700/50">
            <div className="flex items-center gap-2 mb-3">
              <Key className="w-4 h-4 text-gray-400" />
              <span className="text-sm font-medium text-gray-200">OpenAI API Key</span>
              <span className="text-xs text-gray-500">(for embeddings)</span>
            </div>

            {/* Status indicator */}
            <div className={`flex items-center gap-2 mb-3 text-xs ${
              openaiKeyStatus ? 'text-green-400' : 'text-gray-500'
            }`}>
              {openaiKeyStatus ? (
                <>
                  <Check className="w-3 h-3" />
                  <span>Key configured</span>
                </>
              ) : (
                <>
                  <AlertCircle className="w-3 h-3" />
                  <span>Optional - enables semantic similarity</span>
                </>
              )}
            </div>

            {/* Show masked key if set */}
            {openaiKeyStatus && (
              <div className="mb-3 px-2 py-1 bg-gray-900/50 rounded text-xs text-gray-400 font-mono">
                {openaiKeyStatus}
              </div>
            )}

            {/* OpenAI API key input */}
            <div className="space-y-2">
              <input
                type="password"
                placeholder="sk-..."
                value={openaiKeyInput}
                onChange={(e) => setOpenaiKeyInput(e.target.value)}
                className="w-full px-3 py-2 bg-gray-900/50 border border-gray-700/50 rounded text-sm text-white placeholder-gray-500 focus:border-amber-500/50 focus:ring-1 focus:ring-amber-500/20 focus:outline-none"
              />

              {openaiSaveError && (
                <p className="text-xs text-red-400">{openaiSaveError}</p>
              )}

              <div className="flex gap-2">
                <button
                  onClick={handleSaveOpenaiKey}
                  disabled={savingOpenai || !openaiKeyInput.trim()}
                  className="flex-1 px-3 py-1.5 bg-amber-500/20 text-amber-200 rounded text-sm font-medium hover:bg-amber-500/30 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                >
                  {savingOpenai ? 'Saving...' : 'Save Key'}
                </button>

                {openaiKeyStatus && (
                  <button
                    onClick={handleClearOpenaiKey}
                    className="px-3 py-1.5 bg-red-500/20 text-red-300 rounded text-sm font-medium hover:bg-red-500/30 transition-colors"
                  >
                    Clear
                  </button>
                )}
              </div>
            </div>

            <p className="mt-3 text-xs text-gray-500">
              Get your API key from{' '}
              <a
                href="https://platform.openai.com/api-keys"
                target="_blank"
                rel="noopener noreferrer"
                className="text-amber-400 hover:underline"
              >
                platform.openai.com
              </a>
            </p>
          </div>
        </div>
      )}
    </aside>
  );
}
