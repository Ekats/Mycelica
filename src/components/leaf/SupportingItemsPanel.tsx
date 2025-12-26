import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Lightbulb, Search, MessageSquare, BookOpen, Palette, ChevronDown, ChevronRight } from 'lucide-react';
import type { Node } from '../../types/graph';

interface SupportingItemsPanelProps {
  parentId: string;
  onSelectItem?: (nodeId: string) => void;
}

// SUPPORTING tier content types (lazy-loaded in leaf view)
// HIDDEN tier (code, debug, paste, trivial) is excluded entirely
type ContentTab = 'ideas' | 'investigation' | 'discussion' | 'reference' | 'creative';

const TAB_CONFIG: Record<ContentTab, { label: string; emoji: string; icon: typeof Lightbulb }> = {
  ideas: { label: 'Ideas', emoji: '💡', icon: Lightbulb },
  investigation: { label: 'Investigation', emoji: '🔍', icon: Search },
  discussion: { label: 'Discussion', emoji: '💬', icon: MessageSquare },
  reference: { label: 'Reference', emoji: '📚', icon: BookOpen },
  creative: { label: 'Creative', emoji: '🎨', icon: Palette },
};

export function SupportingItemsPanel({ parentId, onSelectItem }: SupportingItemsPanelProps) {
  const [activeTab, setActiveTab] = useState<ContentTab>('ideas');
  const [ideas, setIdeas] = useState<Node[]>([]);
  const [supportingItems, setSupportingItems] = useState<Node[]>([]);
  const [expandedIdeas, setExpandedIdeas] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(true);

  // Fetch items
  const fetchData = useCallback(async () => {
    setLoading(true);
    try {
      // Fetch graph children (VISIBLE tier only)
      const graphChildren = await invoke<Node[]>('get_graph_children', { parentId });
      setIdeas(graphChildren.filter(n => n.isItem));

      // Fetch supporting items (SUPPORTING tier only - investigation, discussion, reference, creative)
      const supporting = await invoke<Node[]>('get_supporting_items', { parentId });
      setSupportingItems(supporting);
    } catch (err) {
      console.error('Failed to fetch supporting items:', err);
    } finally {
      setLoading(false);
    }
  }, [parentId]);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  // Get associated items for an idea
  const getAssociatedItems = (ideaId: string): Node[] => {
    return supportingItems.filter(item => item.associatedIdeaId === ideaId);
  };

  // Get unassociated items (no associated_idea_id)
  const unassociatedItems = supportingItems.filter(item => !item.associatedIdeaId);

  // Toggle idea expansion
  const toggleIdea = (ideaId: string) => {
    setExpandedIdeas(prev => {
      const next = new Set(prev);
      if (next.has(ideaId)) {
        next.delete(ideaId);
      } else {
        next.add(ideaId);
      }
      return next;
    });
  };

  // Filter items by tab
  const getTabItems = (): Node[] => {
    switch (activeTab) {
      case 'ideas':
        return ideas;
      case 'investigation':
        return supportingItems.filter(n => n.contentType === 'investigation');
      case 'discussion':
        return supportingItems.filter(n => n.contentType === 'discussion');
      case 'reference':
        return supportingItems.filter(n => n.contentType === 'reference');
      case 'creative':
        return supportingItems.filter(n => n.contentType === 'creative');
      default:
        return [];
    }
  };

  // Get count for a specific tab
  const getTabCount = (tab: ContentTab): number => {
    if (tab === 'ideas') return ideas.length;
    return supportingItems.filter(n => n.contentType === tab).length;
  };

  const renderContentTypeIcon = (contentType: string | undefined) => {
    switch (contentType) {
      case 'investigation': return <Search className="w-4 h-4 text-blue-400" />;
      case 'discussion': return <MessageSquare className="w-4 h-4 text-green-400" />;
      case 'reference': return <BookOpen className="w-4 h-4 text-purple-400" />;
      case 'creative': return <Palette className="w-4 h-4 text-pink-400" />;
      default: return <Lightbulb className="w-4 h-4 text-yellow-400" />;
    }
  };

  if (loading) {
    return (
      <div className="p-4 text-gray-400 text-center">
        <div className="animate-pulse">Loading items...</div>
      </div>
    );
  }

  const totalSupporting = supportingItems.length;
  if (ideas.length === 0 && totalSupporting === 0) {
    return null; // No items to show
  }

  return (
    <div className="bg-gray-800 rounded-lg border border-gray-700">
      {/* Tabs */}
      <div className="flex border-b border-gray-700 overflow-x-auto">
        {Object.entries(TAB_CONFIG).map(([tab, config]) => {
          const count = getTabCount(tab as ContentTab);

          if (count === 0 && tab !== 'ideas') return null;

          const isActive = activeTab === tab;

          return (
            <button
              key={tab}
              onClick={() => setActiveTab(tab as ContentTab)}
              className={`flex items-center gap-2 px-4 py-2 text-sm font-medium transition-colors whitespace-nowrap
                ${isActive
                  ? 'text-amber-400 border-b-2 border-amber-400 -mb-px bg-gray-700/50'
                  : 'text-gray-400 hover:text-white hover:bg-gray-700/30'
                }`}
            >
              <span>{config.emoji}</span>
              <span>{config.label}</span>
              <span className={`px-1.5 py-0.5 rounded text-xs ${isActive ? 'bg-amber-500/20' : 'bg-gray-600'}`}>
                {count}
              </span>
            </button>
          );
        })}
      </div>

      {/* Content */}
      <div className="max-h-96 overflow-y-auto">
        {activeTab === 'ideas' ? (
          // Ideas view with inline associations
          <div className="divide-y divide-gray-700/50">
            {ideas.map(idea => {
              const associated = getAssociatedItems(idea.id);
              const isExpanded = expandedIdeas.has(idea.id);
              const hasAssociations = associated.length > 0;

              return (
                <div key={idea.id} className="p-3">
                  <div
                    className={`flex items-start gap-2 ${hasAssociations ? 'cursor-pointer' : ''}`}
                    onClick={() => hasAssociations && toggleIdea(idea.id)}
                  >
                    {hasAssociations && (
                      isExpanded
                        ? <ChevronDown className="w-4 h-4 mt-1 text-gray-500 shrink-0" />
                        : <ChevronRight className="w-4 h-4 mt-1 text-gray-500 shrink-0" />
                    )}
                    <div className="flex-1 min-w-0">
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          onSelectItem?.(idea.id);
                        }}
                        className="text-left hover:text-amber-400 transition-colors"
                      >
                        <span className="mr-2">{idea.emoji || '💡'}</span>
                        <span className="font-medium text-white">{idea.aiTitle || idea.title}</span>
                      </button>
                      {hasAssociations && (
                        <span className="ml-2 text-xs text-gray-500">
                          ({associated.length} associated)
                        </span>
                      )}
                      {idea.summary && (
                        <p className="text-sm text-gray-400 mt-1 line-clamp-2">{idea.summary}</p>
                      )}
                    </div>
                  </div>

                  {/* Associated items */}
                  {isExpanded && associated.length > 0 && (
                    <div className="mt-2 ml-6 space-y-1">
                      {associated.map(item => (
                        <button
                          key={item.id}
                          onClick={() => onSelectItem?.(item.id)}
                          className="flex items-center gap-2 px-2 py-1 w-full text-left text-sm text-gray-300 hover:text-white hover:bg-gray-700/50 rounded transition-colors"
                        >
                          {renderContentTypeIcon(item.contentType)}
                          <span className="truncate">{item.aiTitle || item.title}</span>
                        </button>
                      ))}
                    </div>
                  )}
                </div>
              );
            })}

            {/* Unassociated items */}
            {unassociatedItems.length > 0 && (
              <div className="p-3 bg-gray-900/50">
                <h4 className="text-sm font-medium text-gray-500 mb-2">Unassociated</h4>
                <div className="space-y-1">
                  {unassociatedItems.map(item => (
                    <button
                      key={item.id}
                      onClick={() => onSelectItem?.(item.id)}
                      className="flex items-center gap-2 px-2 py-1 w-full text-left text-sm text-gray-300 hover:text-white hover:bg-gray-700/50 rounded transition-colors"
                    >
                      {renderContentTypeIcon(item.contentType)}
                      <span className="truncate">{item.aiTitle || item.title}</span>
                    </button>
                  ))}
                </div>
              </div>
            )}
          </div>
        ) : (
          // Code/Debug/Paste tab - simple list
          <div className="divide-y divide-gray-700/50">
            {getTabItems().map(item => (
              <button
                key={item.id}
                onClick={() => onSelectItem?.(item.id)}
                className="flex items-start gap-3 p-3 w-full text-left hover:bg-gray-700/30 transition-colors"
              >
                {renderContentTypeIcon(item.contentType)}
                <div className="flex-1 min-w-0">
                  <div className="font-medium text-white truncate">{item.aiTitle || item.title}</div>
                  {item.associatedIdeaId && (
                    <div className="text-xs text-gray-500 mt-0.5">
                      ↳ linked to: {ideas.find(i => i.id === item.associatedIdeaId)?.title?.slice(0, 30)}...
                    </div>
                  )}
                  <div className="text-xs text-gray-500 mt-1">
                    {new Date(item.createdAt).toLocaleDateString()}
                  </div>
                </div>
              </button>
            ))}
            {getTabItems().length === 0 && (
              <div className="p-8 text-center text-gray-500">
                No items in this category
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
