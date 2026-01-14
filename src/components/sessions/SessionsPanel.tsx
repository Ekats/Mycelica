import { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { Rabbit, ChevronRight, ChevronDown, Globe, Clock, ExternalLink, Pause, Play, Pencil, Trash2, Check, X } from 'lucide-react';

// Types matching backend responses
interface SessionSummary {
  id: string;
  title: string;
  start_time: number;
  duration_ms: number;
  item_count: number;
  status: string;
  entry_point: string | null;
  entry_title: string | null;
}

interface SessionItem {
  node_id: string;
  title: string;
  url: string;
  order: number;
  timestamp: number;
  dwell_time_ms: number;
  visit_count: number;
}

interface NavigationEdge {
  from_id: string;
  to_id: string;
  edge_type: string;
  timestamp: number;
}

interface SessionDetail {
  session: SessionSummary;
  items: SessionItem[];
  edges: NavigationEdge[];
}

const API_BASE = 'http://localhost:9876';

// Format duration from milliseconds
function formatDuration(ms: number): string {
  const seconds = Math.floor(ms / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);

  if (hours > 0) {
    return `${hours}h ${minutes % 60}m`;
  } else if (minutes > 0) {
    return `${minutes}m`;
  }
  return `${seconds}s`;
}

// Format timestamp as relative time
function formatRelativeTime(timestamp: number): string {
  const now = Date.now();
  const diff = now - timestamp;

  const minutes = Math.floor(diff / 60000);
  const hours = Math.floor(diff / 3600000);
  const days = Math.floor(diff / 86400000);

  if (days > 0) return `${days}d ago`;
  if (hours > 0) return `${hours}h ago`;
  if (minutes > 0) return `${minutes}m ago`;
  return 'Just now';
}

// Extract domain from URL
function extractDomain(url: string): string {
  try {
    return new URL(url).hostname.replace('www.', '');
  } catch {
    return url;
  }
}

export function SessionsPanel() {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expandedSession, setExpandedSession] = useState<string | null>(null);
  const [sessionDetail, setSessionDetail] = useState<SessionDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);

  // Editing state
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editTitle, setEditTitle] = useState('');
  const editInputRef = useRef<HTMLInputElement>(null);

  // Delete confirmation state
  const [deleteConfirmId, setDeleteConfirmId] = useState<string | null>(null);

  // Fetch sessions list
  const fetchSessions = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const response = await fetch(`${API_BASE}/holerabbit/sessions`);
      if (!response.ok) throw new Error('Failed to fetch sessions');
      const data = await response.json();
      setSessions(data.sessions || []);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load sessions');
    } finally {
      setLoading(false);
    }
  }, []);

  // Fetch session detail when expanded
  const fetchSessionDetail = useCallback(async (sessionId: string) => {
    try {
      setLoadingDetail(true);
      const response = await fetch(`${API_BASE}/holerabbit/session/${sessionId}`);
      if (!response.ok) throw new Error('Failed to fetch session detail');
      const data = await response.json();
      setSessionDetail(data);
    } catch (err) {
      console.error('Failed to load session detail:', err);
    } finally {
      setLoadingDetail(false);
    }
  }, []);

  // Initial load
  useEffect(() => {
    fetchSessions();
  }, [fetchSessions]);

  // Listen for visit events from backend (real-time updates)
  useEffect(() => {
    const unlisten = listen('holerabbit:visit', () => {
      fetchSessions();
      // Also refresh expanded session detail
      if (expandedSession) {
        fetchSessionDetail(expandedSession);
      }
    });
    return () => { unlisten.then(fn => fn()); };
  }, [fetchSessions, fetchSessionDetail, expandedSession]);

  // Load detail when session expanded
  useEffect(() => {
    if (expandedSession) {
      fetchSessionDetail(expandedSession);
    } else {
      setSessionDetail(null);
    }
  }, [expandedSession, fetchSessionDetail]);

  // Focus edit input when editing starts
  useEffect(() => {
    if (editingId && editInputRef.current) {
      editInputRef.current.focus();
      editInputRef.current.select();
    }
  }, [editingId]);

  const handleSessionClick = (sessionId: string) => {
    if (editingId || deleteConfirmId) return; // Don't toggle while editing/confirming
    setExpandedSession(expandedSession === sessionId ? null : sessionId);
  };

  // Session control handlers
  const handlePauseResume = async (e: React.MouseEvent, sessionId: string, currentStatus: string) => {
    e.stopPropagation();
    const action = currentStatus === 'live' ? 'pause' : 'resume';
    try {
      const response = await fetch(`${API_BASE}/holerabbit/session/${sessionId}/${action}`, {
        method: 'POST',
      });
      if (!response.ok) throw new Error(`Failed to ${action} session`);
      fetchSessions(); // Refresh list
    } catch (err) {
      console.error(`Failed to ${action} session:`, err);
    }
  };

  const handleStartEdit = (e: React.MouseEvent, session: SessionSummary) => {
    e.stopPropagation();
    setEditingId(session.id);
    setEditTitle(session.entry_title || session.title);
    setDeleteConfirmId(null);
  };

  const handleCancelEdit = (e: React.MouseEvent) => {
    e.stopPropagation();
    setEditingId(null);
    setEditTitle('');
  };

  const handleSaveEdit = async (e: React.MouseEvent, sessionId: string) => {
    e.stopPropagation();
    if (!editTitle.trim()) return;

    try {
      const response = await fetch(`${API_BASE}/holerabbit/session/${sessionId}/rename`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ title: editTitle.trim() }),
      });
      if (!response.ok) throw new Error('Failed to rename session');
      setEditingId(null);
      setEditTitle('');
      fetchSessions(); // Refresh list
    } catch (err) {
      console.error('Failed to rename session:', err);
    }
  };

  const handleDeleteClick = (e: React.MouseEvent, sessionId: string) => {
    e.stopPropagation();
    setDeleteConfirmId(sessionId);
    setEditingId(null);
  };

  const handleCancelDelete = (e: React.MouseEvent) => {
    e.stopPropagation();
    setDeleteConfirmId(null);
  };

  const handleConfirmDelete = async (e: React.MouseEvent, sessionId: string) => {
    e.stopPropagation();
    try {
      const response = await fetch(`${API_BASE}/holerabbit/session/${sessionId}`, {
        method: 'DELETE',
      });
      if (!response.ok) throw new Error('Failed to delete session');
      setDeleteConfirmId(null);
      if (expandedSession === sessionId) {
        setExpandedSession(null);
      }
      fetchSessions(); // Refresh list
    } catch (err) {
      console.error('Failed to delete session:', err);
    }
  };

  const getStatusColor = (status: string) => {
    switch (status) {
      case 'live': return 'text-green-400';
      case 'paused': return 'text-yellow-400';
      default: return 'text-gray-400';
    }
  };

  const getStatusIcon = (status: string) => {
    switch (status) {
      case 'live': return <div className="w-2 h-2 rounded-full bg-green-400 animate-pulse" />;
      case 'paused': return <Pause className="w-3 h-3 text-yellow-400" />;
      default: return <Clock className="w-3 h-3 text-gray-500" />;
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center h-32">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-amber-500"></div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="p-4 text-center">
        <p className="text-red-400 text-sm">{error}</p>
        <button
          onClick={fetchSessions}
          className="mt-2 text-xs text-amber-400 hover:text-amber-300"
        >
          Retry
        </button>
      </div>
    );
  }

  if (sessions.length === 0) {
    return (
      <div className="p-8 text-center text-gray-500">
        <Rabbit className="w-12 h-12 mx-auto mb-3 opacity-30" />
        <p className="text-sm">No browsing sessions yet</p>
        <p className="text-xs mt-1">Install the Holerabbit extension to track your web journeys</p>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {sessions.map((session) => (
        <div key={session.id} className="rounded-lg bg-gray-900/50 border border-gray-700/50">
          {/* Session Header */}
          <div className="w-full p-3 flex items-start gap-3 text-left">
            <button
              onClick={() => handleSessionClick(session.id)}
              className="shrink-0 mt-0.5 hover:bg-gray-700/50 rounded p-0.5 transition-colors"
            >
              {expandedSession === session.id ? (
                <ChevronDown className="w-4 h-4 text-gray-400" />
              ) : (
                <ChevronRight className="w-4 h-4 text-gray-400" />
              )}
            </button>

            <button
              onClick={() => handleSessionClick(session.id)}
              className="flex-1 min-w-0 text-left"
            >
              <div className="flex items-center gap-2">
                {getStatusIcon(session.status)}
                <span className={`text-xs font-medium ${getStatusColor(session.status)}`}>
                  {session.status}
                </span>
                <span className="text-xs text-gray-500">
                  {formatRelativeTime(session.start_time)}
                </span>
              </div>

              {/* Title - editable or static */}
              {editingId === session.id ? (
                <div className="flex items-center gap-1 mt-1" onClick={e => e.stopPropagation()}>
                  <input
                    ref={editInputRef}
                    type="text"
                    value={editTitle}
                    onChange={(e) => setEditTitle(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') handleSaveEdit(e as unknown as React.MouseEvent, session.id);
                      if (e.key === 'Escape') handleCancelEdit(e as unknown as React.MouseEvent);
                    }}
                    className="flex-1 text-sm font-medium bg-gray-800 border border-gray-600 rounded px-2 py-0.5 text-gray-200 focus:border-amber-500 focus:outline-none"
                  />
                  <button
                    onClick={(e) => handleSaveEdit(e, session.id)}
                    className="p-1 text-green-400 hover:text-green-300 hover:bg-gray-700/50 rounded"
                    title="Save"
                  >
                    <Check className="w-3.5 h-3.5" />
                  </button>
                  <button
                    onClick={handleCancelEdit}
                    className="p-1 text-gray-400 hover:text-gray-300 hover:bg-gray-700/50 rounded"
                    title="Cancel"
                  >
                    <X className="w-3.5 h-3.5" />
                  </button>
                </div>
              ) : (
                <h3 className="text-sm font-medium text-gray-200 mt-1 truncate">
                  {session.entry_title || session.title}
                </h3>
              )}

              <div className="flex items-center gap-3 mt-1 text-xs text-gray-500">
                <span>{session.item_count} pages</span>
                <span>{formatDuration(session.duration_ms)}</span>
              </div>
            </button>

            {/* Control buttons */}
            <div className="flex items-center gap-1 shrink-0">
              {deleteConfirmId === session.id ? (
                // Delete confirmation
                <div className="flex items-center gap-1 bg-red-900/30 rounded px-2 py-1">
                  <span className="text-xs text-red-400">Delete?</span>
                  <button
                    onClick={(e) => handleConfirmDelete(e, session.id)}
                    className="p-1 text-red-400 hover:text-red-300 hover:bg-red-800/50 rounded"
                    title="Confirm delete"
                  >
                    <Check className="w-3.5 h-3.5" />
                  </button>
                  <button
                    onClick={handleCancelDelete}
                    className="p-1 text-gray-400 hover:text-gray-300 hover:bg-gray-700/50 rounded"
                    title="Cancel"
                  >
                    <X className="w-3.5 h-3.5" />
                  </button>
                </div>
              ) : (
                <>
                  {/* Pause/Resume button */}
                  <button
                    onClick={(e) => handlePauseResume(e, session.id, session.status)}
                    className={`p-1.5 rounded transition-colors ${
                      session.status === 'live'
                        ? 'text-yellow-400 hover:text-yellow-300 hover:bg-yellow-900/30'
                        : 'text-green-400 hover:text-green-300 hover:bg-green-900/30'
                    }`}
                    title={session.status === 'live' ? 'Pause session' : 'Resume session'}
                  >
                    {session.status === 'live' ? (
                      <Pause className="w-3.5 h-3.5" />
                    ) : (
                      <Play className="w-3.5 h-3.5" />
                    )}
                  </button>

                  {/* Edit button */}
                  <button
                    onClick={(e) => handleStartEdit(e, session)}
                    className="p-1.5 text-gray-400 hover:text-gray-300 hover:bg-gray-700/50 rounded transition-colors"
                    title="Rename session"
                  >
                    <Pencil className="w-3.5 h-3.5" />
                  </button>

                  {/* Delete button */}
                  <button
                    onClick={(e) => handleDeleteClick(e, session.id)}
                    className="p-1.5 text-gray-400 hover:text-red-400 hover:bg-red-900/30 rounded transition-colors"
                    title="Delete session"
                  >
                    <Trash2 className="w-3.5 h-3.5" />
                  </button>
                </>
              )}
            </div>
          </div>

          {/* Expanded Session Detail */}
          {expandedSession === session.id && (
            <div className="border-t border-gray-700/50">
              {loadingDetail ? (
                <div className="p-4 text-center">
                  <div className="animate-spin rounded-full h-5 w-5 border-b-2 border-amber-500 mx-auto"></div>
                </div>
              ) : sessionDetail ? (
                <div className="p-2 space-y-1 max-h-64 overflow-y-auto">
                  {sessionDetail.items.map((item, index) => (
                    <a
                      key={item.node_id}
                      href={item.url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="flex items-center gap-2 px-2 py-1.5 rounded hover:bg-gray-700/50 transition-colors group"
                    >
                      <span className="text-xs text-gray-500 w-4 shrink-0">
                        {index + 1}.
                      </span>
                      <Globe className="w-3.5 h-3.5 text-gray-500 shrink-0" />
                      <div className="flex-1 min-w-0">
                        <div className="text-xs text-gray-200 truncate">
                          {item.title}
                        </div>
                        <div className="text-[10px] text-gray-500 truncate">
                          {extractDomain(item.url)}
                        </div>
                      </div>
                      <div className="flex items-center gap-2 shrink-0">
                        {item.visit_count > 1 && (
                          <span className="text-[10px] text-gray-500">
                            {item.visit_count}x
                          </span>
                        )}
                        <span className="text-[10px] text-gray-600">
                          {formatDuration(item.dwell_time_ms)}
                        </span>
                        <ExternalLink className="w-3 h-3 text-gray-500 opacity-0 group-hover:opacity-100 transition-opacity" />
                      </div>
                    </a>
                  ))}
                </div>
              ) : null}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
