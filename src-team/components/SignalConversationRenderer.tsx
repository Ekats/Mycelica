import { useMemo, useRef, useEffect } from "react";
import { useTeamStore } from "../stores/teamStore";
import type { Node } from "../types";

// Deterministic author colors
const AUTHOR_COLORS = ['#60a5fa', '#34d399', '#fb923c', '#c084fc', '#f472b6', '#fbbf24'];
function authorColor(name: string): string {
  let hash = 0;
  for (const c of name) hash = ((hash << 5) - hash + c.charCodeAt(0)) | 0;
  return AUTHOR_COLORS[Math.abs(hash) % AUTHOR_COLORS.length];
}

// Thread stripe colors (distinct from author colors)
const THREAD_COLORS = ['#6366f1', '#14b8a6', '#f97316', '#a855f7', '#ec4899', '#eab308', '#06b6d4', '#84cc16'];
function threadColor(threadId: string): string {
  let hash = 0;
  for (const c of threadId) hash = ((hash << 5) - hash + c.charCodeAt(0)) | 0;
  return THREAD_COLORS[Math.abs(hash) % THREAD_COLORS.length];
}

interface ParsedTags {
  thread_id?: string;
  reactions?: Array<{ emoji: string; fromId: string; author?: string }>;
  urls_found?: string[];
  has_attachments?: boolean;
  decision_detected?: boolean;
  agreement_count?: number;
  agreeing_authors?: string[];
  raw_sent_at?: number;
  signal_type?: string;
}

function parseTags(tags?: string): ParsedTags {
  if (!tags) return {};
  try {
    return JSON.parse(tags);
  } catch {
    return {};
  }
}

function formatTime(ts: number): string {
  const d = new Date(ts);
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function formatDate(ts: number): string {
  const d = new Date(ts);
  return d.toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' });
}

interface SignalConversationRendererProps {
  containerId: string;
}

export default function SignalConversationRenderer({ containerId }: SignalConversationRendererProps) {
  const { nodes, edges, navigateBack } = useTeamStore();
  const scrollRef = useRef<HTMLDivElement>(null);

  // Get container and children
  const container = nodes.get(containerId);

  const { messages, repliesMap, authors, dateRange } = useMemo(() => {
    // Collect children of this container
    const children: Node[] = [];
    for (const n of nodes.values()) {
      if (n.parentId === containerId && n.isItem) {
        children.push(n);
      }
    }

    // Sort by sequenceIndex, fallback to createdAt
    children.sort((a, b) => {
      const ai = a.sequenceIndex ?? Infinity;
      const bi = b.sequenceIndex ?? Infinity;
      if (ai !== bi) return ai - bi;
      return a.createdAt - b.createdAt;
    });

    // Build RepliesTo map: target_id -> source node (the reply quotes target)
    const rMap = new Map<string, Node>();
    for (const e of edges) {
      if (e.type === 'replies_to') {
        const sourceNode = nodes.get(e.source);
        if (sourceNode) rMap.set(e.source, nodes.get(e.target)!);
      }
    }

    // Collect unique authors
    const authorSet = new Set<string>();
    let minDate = Infinity;
    let maxDate = -Infinity;
    for (const m of children) {
      if (m.author) authorSet.add(m.author);
      const tags = parseTags(m.tags);
      const ts = tags.raw_sent_at || m.createdAt;
      if (ts < minDate) minDate = ts;
      if (ts > maxDate) maxDate = ts;
    }

    return {
      messages: children,
      repliesMap: rMap,
      authors: Array.from(authorSet),
      dateRange: children.length > 0 ? { min: minDate, max: maxDate } : null,
    };
  }, [nodes, edges, containerId]);

  // Auto-scroll to bottom on first load
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages.length]);

  if (!container) {
    return <div className="flex-1 flex items-center justify-center text-gray-500">Container not found</div>;
  }

  // Time separator: insert when gap > 30 minutes
  const GAP_MS = 30 * 60 * 1000;

  return (
    <div className="flex-1 flex flex-col min-w-0">
      {/* Header */}
      <div className="px-4 py-3 border-b border-gray-700 flex-shrink-0"
        style={{ background: 'var(--bg-secondary)' }}>
        <div className="flex items-center gap-3">
          <button onClick={navigateBack}
            className="text-gray-400 hover:text-white transition-colors p-1 rounded hover:bg-gray-700">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M19 12H5M12 19l-7-7 7-7" />
            </svg>
          </button>
          <div className="min-w-0">
            <h2 className="text-lg font-semibold text-white truncate">
              {container.aiTitle || container.title}
            </h2>
            <div className="text-xs text-gray-400 flex gap-3">
              <span>{authors.length} participant{authors.length !== 1 ? 's' : ''}</span>
              <span>{messages.length} message{messages.length !== 1 ? 's' : ''}</span>
              {dateRange && (
                <span>{formatDate(dateRange.min)} — {formatDate(dateRange.max)}</span>
              )}
            </div>
          </div>
        </div>
        {/* Author legend */}
        <div className="flex gap-2 mt-2 flex-wrap">
          {authors.map(a => (
            <span key={a} className="text-xs px-2 py-0.5 rounded-full"
              style={{ background: authorColor(a) + '22', color: authorColor(a), border: `1px solid ${authorColor(a)}44` }}>
              {a}
            </span>
          ))}
        </div>
      </div>

      {/* Timeline */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto px-4 py-3 space-y-1"
        style={{ background: 'var(--bg-primary)' }}>
        {messages.length === 0 ? (
          <div className="text-gray-500 text-center py-8">No messages in this conversation</div>
        ) : (
          messages.map((msg, idx) => {
            const tags = parseTags(msg.tags);
            const ts = tags.raw_sent_at || msg.createdAt;
            const prevTs = idx > 0 ? (parseTags(messages[idx - 1].tags).raw_sent_at || messages[idx - 1].createdAt) : ts;
            const showSeparator = idx > 0 && (ts - prevTs) > GAP_MS;
            const quotedNode = repliesMap.get(msg.id);
            const isDecision = tags.decision_detected;
            const isLink = msg.nodeClass === 'reference';
            const color = msg.author ? authorColor(msg.author) : '#9ca3af';
            const tColor = tags.thread_id ? threadColor(tags.thread_id) : undefined;

            return (
              <div key={msg.id}>
                {/* Time separator */}
                {showSeparator && (
                  <div className="flex items-center gap-3 my-3">
                    <div className="flex-1 border-t border-gray-700" />
                    <span className="text-xs text-gray-500 whitespace-nowrap">
                      {formatDate(ts)} {formatTime(ts)}
                    </span>
                    <div className="flex-1 border-t border-gray-700" />
                  </div>
                )}

                {/* Message bubble */}
                <div className="flex gap-2 py-1 group"
                  style={{ borderLeft: tColor ? `3px solid ${tColor}` : '3px solid transparent', paddingLeft: '8px' }}>
                  <div className="flex-1 min-w-0">
                    {/* Author + time */}
                    <div className="flex items-baseline gap-2 mb-0.5">
                      <span className="text-xs font-semibold" style={{ color }}>
                        {msg.author || 'Unknown'}
                      </span>
                      <span className="text-xs text-gray-600">
                        {formatTime(ts)}
                      </span>
                      {isDecision && (
                        <span className="text-xs px-1.5 py-0.5 rounded text-green-300"
                          style={{ background: '#065f4620', border: '1px solid #065f4650' }}>
                          DECISION
                        </span>
                      )}
                    </div>

                    {/* Quoted message (reply) */}
                    {quotedNode && (
                      <div className="text-xs text-gray-500 pl-2 mb-1 truncate"
                        style={{ borderLeft: '2px solid #4b5563' }}>
                        <span className="text-gray-400">{quotedNode.author}:</span>{' '}
                        {(quotedNode.content || quotedNode.title).slice(0, 80)}
                        {(quotedNode.content || quotedNode.title).length > 80 ? '...' : ''}
                      </div>
                    )}

                    {/* Content */}
                    {isLink ? (
                      <div className="text-sm">
                        <span className="text-blue-400">
                          {(() => {
                            const urlMatch = (msg.content || msg.title).match(/https?:\/\/[^\s]+/);
                            if (urlMatch) {
                              try {
                                const u = new URL(urlMatch[0]);
                                return u.hostname;
                              } catch { return urlMatch[0]; }
                            }
                            return msg.title;
                          })()}
                        </span>
                        {msg.content && !msg.content.match(/^https?:\/\//) && (
                          <span className="text-gray-300 ml-2">{msg.content}</span>
                        )}
                      </div>
                    ) : isDecision ? (
                      <div className="text-sm text-green-200 rounded p-1.5 -ml-1.5"
                        style={{ background: '#065f4615' }}>
                        {msg.content || msg.title}
                        {tags.agreement_count != null && (
                          <div className="text-xs text-green-400/70 mt-1">
                            Agreed: {tags.agreeing_authors?.join(', ')} ({tags.agreement_count})
                          </div>
                        )}
                      </div>
                    ) : (
                      <div className="text-sm text-gray-200 whitespace-pre-wrap break-words">
                        {msg.content || msg.title}
                      </div>
                    )}

                    {/* Reactions */}
                    {tags.reactions && tags.reactions.length > 0 && (
                      <div className="flex gap-1 mt-0.5">
                        {tags.reactions.map((r, ri) => (
                          <span key={ri} className="text-xs px-1 py-0.5 rounded bg-gray-800 text-gray-400">
                            {r.emoji} {r.author || ''}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
