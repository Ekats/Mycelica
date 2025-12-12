import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { readTextFile } from '@tauri-apps/plugin-fs';
import {
  X,
  Key,
  Database,
  RefreshCw,
  Trash2,
  Upload,
  Check,
  AlertCircle,
  Loader2,
  FolderOpen,
  HardDrive,
  Clock,
  Zap,
} from 'lucide-react';
import { ConfirmDialog } from './ConfirmDialog';

interface ApiKeyStatus {
  hasKey: boolean;
  maskedKey: string | null;
  source: string;
}

interface DbStats {
  totalNodes: number;
  totalItems: number;
  processedItems: number;
  itemsWithEmbeddings: number;
}

interface ProcessingStats {
  totalAiProcessingSecs: number;
  totalRebuildSecs: number;
  lastAiProcessingSecs: number;
  lastRebuildSecs: number;
  aiProcessingRuns: number;
  rebuildRuns: number;
  totalAnthropicInputTokens: number;
  totalAnthropicOutputTokens: number;
  totalOpenaiTokens: number;
}

interface SettingsPanelProps {
  open: boolean;
  onClose: () => void;
  onDataChanged?: () => void;
}

type ConfirmAction = 'deleteAll' | 'resetAi' | 'resetClustering' | 'clearEmbeddings' | 'clearHierarchy' | 'fullRebuild' | 'flattenHierarchy' | 'consolidateRoot' | 'tidyDatabase' | null;

interface TidyReport {
  chainsFlattened: number;
  emptiesRemoved: number;
  childCountsFixed: number;
  depthsFixed: number;
  orphansReparented: number;
  deadEdgesPruned: number;
  duplicateEdgesRemoved: number;
  durationMs: number;
}

export function SettingsPanel({ open, onClose, onDataChanged }: SettingsPanelProps) {
  // API Keys
  const [apiKeyStatus, setApiKeyStatus] = useState<ApiKeyStatus | null>(null);
  const [apiKeyInput, setApiKeyInput] = useState('');
  const [savingApiKey, setSavingApiKey] = useState(false);
  const [apiKeyError, setApiKeyError] = useState<string | null>(null);

  const [openaiKeyStatus, setOpenaiKeyStatus] = useState<string | null>(null);
  const [openaiKeyInput, setOpenaiKeyInput] = useState('');
  const [savingOpenaiKey, setSavingOpenaiKey] = useState(false);
  const [openaiKeyError, setOpenaiKeyError] = useState<string | null>(null);

  // Database stats and path
  const [dbStats, setDbStats] = useState<DbStats | null>(null);
  const [dbPath, setDbPath] = useState<string>('');
  const [switchingDb, setSwitchingDb] = useState(false);

  // Processing stats
  const [processingStats, setProcessingStats] = useState<ProcessingStats | null>(null);

  // Import
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<string | null>(null);

  // Confirmation dialog
  const [confirmAction, setConfirmAction] = useState<ConfirmAction>(null);
  const [actionLoading, setActionLoading] = useState(false);
  const [actionResult, setActionResult] = useState<string | null>(null);

  // Operations
  const [isFullRebuilding, setIsFullRebuilding] = useState(false);
  const [isFlattening, setIsFlattening] = useState(false);
  const [isConsolidating, setIsConsolidating] = useState(false);
  const [isTidying, setIsTidying] = useState(false);
  const [operationResult, setOperationResult] = useState<string | null>(null);

  // Load data on mount
  useEffect(() => {
    if (open) {
      loadApiKeyStatus();
      loadOpenaiKeyStatus();
      loadDbStats();
      loadDbPath();
      loadProcessingStats();
      setImportResult(null);
      setActionResult(null);
    }
  }, [open]);

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

  const loadDbStats = async () => {
    try {
      const stats = await invoke<DbStats>('get_db_stats');
      setDbStats(stats);
    } catch (err) {
      console.error('Failed to load db stats:', err);
    }
  };

  const loadDbPath = async () => {
    try {
      const path = await invoke<string>('get_db_path');
      setDbPath(path);
    } catch (err) {
      console.error('Failed to load db path:', err);
    }
  };

  const loadProcessingStats = async () => {
    try {
      const stats = await invoke<ProcessingStats>('get_processing_stats');
      setProcessingStats(stats);
    } catch (err) {
      console.error('Failed to load processing stats:', err);
    }
  };

  // Format seconds to human readable
  const formatTime = (secs: number): string => {
    if (secs < 60) return `${Math.round(secs)}s`;
    const mins = Math.floor(secs / 60);
    const remainSecs = Math.round(secs % 60);
    if (mins < 60) return `${mins}m ${remainSecs}s`;
    const hours = Math.floor(mins / 60);
    const remainMins = mins % 60;
    return `${hours}h ${remainMins}m`;
  };

  // Database selector
  const handleSelectDatabase = async () => {
    setSwitchingDb(true);
    try {
      const file = await openDialog({
        title: 'Select Mycelica database',
        filters: [{ name: 'SQLite Database', extensions: ['db'] }],
      });
      if (file && typeof file === 'string') {
        const stats = await invoke<DbStats>('switch_database', { dbPath: file });
        setDbPath(file);
        setDbStats(stats);
        setActionResult(`Switched to database: ${file.split('/').pop()} - Restart app to apply changes`);
        onDataChanged?.();
      }
    } catch (err) {
      setActionResult(`Error: ${err}`);
    } finally {
      setSwitchingDb(false);
    }
  };

  // API Key handlers
  const handleSaveApiKey = async () => {
    if (!apiKeyInput.trim()) return;
    setSavingApiKey(true);
    setApiKeyError(null);
    try {
      await invoke('save_api_key', { key: apiKeyInput.trim() });
      await loadApiKeyStatus();
      setApiKeyInput('');
    } catch (err) {
      setApiKeyError(err as string);
    } finally {
      setSavingApiKey(false);
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
    setSavingOpenaiKey(true);
    setOpenaiKeyError(null);
    try {
      await invoke('save_openai_api_key', { key: openaiKeyInput.trim() });
      await loadOpenaiKeyStatus();
      setOpenaiKeyInput('');
    } catch (err) {
      setOpenaiKeyError(err as string);
    } finally {
      setSavingOpenaiKey(false);
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

  // Import handler
  const handleImport = async () => {
    setImporting(true);
    setImportResult(null);
    try {
      const file = await openDialog({
        title: 'Select Claude conversations JSON',
        filters: [{ name: 'JSON', extensions: ['json'] }],
      });
      if (file && typeof file === 'string') {
        const content = await readTextFile(file);
        const result = await invoke<{ conversationsImported: number; exchangesImported: number }>(
          'import_claude_conversations',
          { jsonContent: content }
        );
        setImportResult(`Imported ${result.conversationsImported} conversations, ${result.exchangesImported} exchanges`);
        await loadDbStats();
        onDataChanged?.();
      }
    } catch (err) {
      setImportResult(`Error: ${err}`);
    } finally {
      setImporting(false);
    }
  };

  // Confirm action handlers
  const confirmActionConfig: Record<NonNullable<ConfirmAction>, { title: string; message: string; handler: () => Promise<string> }> = {
    deleteAll: {
      title: 'Delete All Data',
      message: 'This will permanently delete ALL nodes and edges. This cannot be undone. Are you sure?',
      handler: async () => {
        const result = await invoke<{ nodesDeleted: number; edgesDeleted: number }>('delete_all_data');
        return `Deleted ${result.nodesDeleted} nodes and ${result.edgesDeleted} edges`;
      },
    },
    resetAi: {
      title: 'Reset AI Processing',
      message: 'This will mark all items as unprocessed and clear their AI-generated titles, summaries, and tags. You\'ll need to run "Process AI" again.',
      handler: async () => {
        const count = await invoke<number>('reset_ai_processing');
        return `Reset AI processing for ${count} items`;
      },
    },
    resetClustering: {
      title: 'Reset Clustering',
      message: 'This will mark all items as needing re-clustering. You\'ll need to run "Full Rebuild" to re-cluster them.',
      handler: async () => {
        const count = await invoke<number>('reset_clustering');
        return `Reset clustering for ${count} items`;
      },
    },
    clearEmbeddings: {
      title: 'Clear Embeddings',
      message: 'This will delete all embeddings and semantic edges. You\'ll need to run "Full Rebuild" to re-generate them.',
      handler: async () => {
        const count = await invoke<number>('clear_embeddings');
        return `Cleared embeddings for ${count} nodes`;
      },
    },
    clearHierarchy: {
      title: 'Clear Hierarchy',
      message: 'This will delete all intermediate category nodes (keeping your items). You\'ll need to run "Full Rebuild" to recreate the hierarchy.',
      handler: async () => {
        const count = await invoke<number>('clear_hierarchy');
        return `Deleted ${count} hierarchy nodes`;
      },
    },
    fullRebuild: {
      title: 'Full Rebuild',
      message: 'This will run AI clustering AND rebuild the entire hierarchy. This is a destructive operation that replaces all organization. This uses API credits and can take a long time for large databases.',
      handler: async () => {
        setIsFullRebuilding(true);
        setOperationResult(null);
        try {
          const result = await invoke<{
            clusteringResult: { itemsProcessed: number; clustersCreated: number; itemsAssigned: number } | null;
            hierarchyResult: { levelsCreated: number; intermediateNodesCreated: number; itemsOrganized: number; maxDepth: number };
            levelsCreated: number;
            groupingIterations: number;
          }>('build_full_hierarchy', { runClustering: true });
          const msg = result.clusteringResult
            ? `Clustered ${result.clusteringResult.itemsAssigned} items into ${result.clusteringResult.clustersCreated} clusters, created ${result.hierarchyResult.intermediateNodesCreated} hierarchy nodes`
            : `Created ${result.hierarchyResult.intermediateNodesCreated} hierarchy nodes`;
          setOperationResult(msg);
          return msg;
        } finally {
          setIsFullRebuilding(false);
        }
      },
    },
    flattenHierarchy: {
      title: 'Flatten Empty Levels',
      message: 'This will remove empty passthrough levels (like "Uncategorized and Related") and reparent their children to the grandparent. This cleans up the hierarchy structure.',
      handler: async () => {
        setIsFlattening(true);
        setOperationResult(null);
        try {
          const count = await invoke<number>('flatten_hierarchy');
          const msg = `Flattened ${count} empty levels`;
          setOperationResult(msg);
          return msg;
        } finally {
          setIsFlattening(false);
        }
      },
    },
    consolidateRoot: {
      title: 'Consolidate Root',
      message: 'This will group Universe\'s direct children into 4-8 uber-categories with single-word ALL-CAPS names (like TECH, LIFE, MIND, WORK). Use this to create a cleaner top-level navigation.',
      handler: async () => {
        setIsConsolidating(true);
        setOperationResult(null);
        try {
          const result = await invoke<{ uberCategoriesCreated: number; childrenReparented: number }>('consolidate_root');
          const msg = `Created ${result.uberCategoriesCreated} uber-categories, reparented ${result.childrenReparented} children`;
          setOperationResult(msg);
          return msg;
        } finally {
          setIsConsolidating(false);
        }
      },
    },
    tidyDatabase: {
      title: 'Tidy Database',
      message: 'Run safe cleanup: flatten single-child chains, remove empties, fix counts/depths, reparent orphans, prune dead edges.',
      handler: async () => {
        setIsTidying(true);
        setOperationResult(null);
        try {
          const report = await invoke<TidyReport>('tidy_database');
          const parts: string[] = [];
          if (report.chainsFlattened > 0) parts.push(`flattened ${report.chainsFlattened} chains`);
          if (report.emptiesRemoved > 0) parts.push(`removed ${report.emptiesRemoved} empties`);
          if (report.childCountsFixed > 0) parts.push(`fixed ${report.childCountsFixed} counts`);
          if (report.depthsFixed > 0) parts.push(`fixed ${report.depthsFixed} depths`);
          if (report.orphansReparented > 0) parts.push(`reparented ${report.orphansReparented} orphans`);
          if (report.deadEdgesPruned > 0) parts.push(`pruned ${report.deadEdgesPruned} edges`);
          if (report.duplicateEdgesRemoved > 0) parts.push(`deduped ${report.duplicateEdgesRemoved} edges`);

          const msg = parts.length > 0
            ? `Tidied in ${report.durationMs}ms: ${parts.join(', ')}`
            : `Database already tidy (${report.durationMs}ms)`;
          setOperationResult(msg);
          return msg;
        } finally {
          setIsTidying(false);
        }
      },
    },
  };

  const handleConfirmAction = async () => {
    if (!confirmAction) return;
    setActionLoading(true);
    setActionResult(null);
    try {
      const config = confirmActionConfig[confirmAction];
      const result = await config.handler();
      setActionResult(result);
      await loadDbStats();
      onDataChanged?.();
    } catch (err) {
      setActionResult(`Error: ${err}`);
    } finally {
      setActionLoading(false);
      setConfirmAction(null);
    }
  };

  // Close on escape
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && open && !confirmAction) {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [open, confirmAction, onClose]);

  if (!open) return null;

  return (
    <>
      <div className="fixed inset-0 z-50 flex items-center justify-center">
        {/* Backdrop */}
        <div
          className="absolute inset-0 bg-black/60 backdrop-blur-sm"
          onClick={onClose}
        />

        {/* Panel */}
        <div className="relative bg-gray-800 rounded-xl border border-gray-700 shadow-2xl max-w-xl w-full mx-4 max-h-[85vh] overflow-hidden flex flex-col">
          {/* Header */}
          <div className="flex items-center justify-between px-6 py-4 border-b border-gray-700">
            <h2 className="text-xl font-semibold text-white">Settings</h2>
            <button
              onClick={onClose}
              className="p-2 text-gray-400 hover:text-white hover:bg-gray-700 rounded-lg transition-colors"
            >
              <X className="w-5 h-5" />
            </button>
          </div>

          {/* Content */}
          <div className="flex-1 overflow-y-auto p-6 space-y-6">
            {/* Database Section */}
            <section>
              <div className="flex items-center gap-2 mb-4">
                <HardDrive className="w-5 h-5 text-green-400" />
                <h3 className="text-lg font-medium text-white">Database</h3>
              </div>

              {/* Current database path */}
              <div className="bg-gray-900/50 rounded-lg p-4 mb-3">
                <div className="flex items-center justify-between mb-2">
                  <span className="text-sm font-medium text-gray-200">Current Database</span>
                  <button
                    onClick={handleSelectDatabase}
                    disabled={switchingDb}
                    className="flex items-center gap-1.5 px-3 py-1.5 bg-gray-700 hover:bg-gray-600 text-gray-200 rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {switchingDb ? (
                      <Loader2 className="w-3 h-3 animate-spin" />
                    ) : (
                      <FolderOpen className="w-3 h-3" />
                    )}
                    Select...
                  </button>
                </div>
                <div className="px-2 py-1.5 bg-gray-800 rounded text-xs text-gray-400 font-mono truncate" title={dbPath}>
                  {dbPath || 'Loading...'}
                </div>
              </div>

              {/* Database Stats */}
              {dbStats && (
                <div className="bg-gray-900/50 rounded-lg p-4 text-sm">
                  <div className="grid grid-cols-2 gap-4">
                    <div>
                      <span className="text-gray-400">Total Nodes:</span>
                      <span className="ml-2 text-white font-medium">{dbStats.totalNodes}</span>
                    </div>
                    <div>
                      <span className="text-gray-400">Items:</span>
                      <span className="ml-2 text-white font-medium">{dbStats.totalItems}</span>
                    </div>
                    <div>
                      <span className="text-gray-400">AI Processed:</span>
                      <span className="ml-2 text-white font-medium">{dbStats.processedItems}</span>
                    </div>
                    <div>
                      <span className="text-gray-400">With Embeddings:</span>
                      <span className="ml-2 text-white font-medium">{dbStats.itemsWithEmbeddings}</span>
                    </div>
                  </div>
                </div>
              )}

              {/* Processing Stats */}
              {processingStats && (processingStats.aiProcessingRuns > 0 || processingStats.rebuildRuns > 0) && (
                <div className="bg-gray-900/50 rounded-lg p-4 text-sm mt-3">
                  <div className="flex items-center gap-1.5 text-xs text-gray-500 uppercase tracking-wider mb-2">
                    <Clock className="w-3 h-3" />
                    Processing Time
                  </div>
                  <div className="grid grid-cols-2 gap-4">
                    {processingStats.aiProcessingRuns > 0 && (
                      <>
                        <div>
                          <span className="text-gray-400">AI Processing:</span>
                          <span className="ml-2 text-white font-medium">{formatTime(processingStats.totalAiProcessingSecs)}</span>
                          <span className="text-gray-500 text-xs ml-1">({processingStats.aiProcessingRuns} runs)</span>
                        </div>
                        <div>
                          <span className="text-gray-400">Last AI Run:</span>
                          <span className="ml-2 text-white font-medium">{formatTime(processingStats.lastAiProcessingSecs)}</span>
                        </div>
                      </>
                    )}
                    {processingStats.rebuildRuns > 0 && (
                      <>
                        <div>
                          <span className="text-gray-400">Rebuild:</span>
                          <span className="ml-2 text-white font-medium">{formatTime(processingStats.totalRebuildSecs)}</span>
                          <span className="text-gray-500 text-xs ml-1">({processingStats.rebuildRuns} runs)</span>
                        </div>
                        <div>
                          <span className="text-gray-400">Last Rebuild:</span>
                          <span className="ml-2 text-white font-medium">{formatTime(processingStats.lastRebuildSecs)}</span>
                        </div>
                      </>
                    )}
                  </div>
                </div>
              )}

              {/* Token Usage Stats */}
              {processingStats && (processingStats.totalAnthropicInputTokens > 0 || processingStats.totalOpenaiTokens > 0) && (
                <div className="bg-gray-900/50 rounded-lg p-4 text-sm mt-3">
                  <div className="flex items-center gap-1.5 text-xs text-gray-500 uppercase tracking-wider mb-2">
                    <Zap className="w-3 h-3" />
                    Token Usage
                  </div>
                  <div className="space-y-2">
                    {processingStats.totalAnthropicInputTokens > 0 && (
                      <div className="flex justify-between">
                        <span className="text-gray-400">Anthropic:</span>
                        <span className="text-white font-medium">
                          {(processingStats.totalAnthropicInputTokens + processingStats.totalAnthropicOutputTokens).toLocaleString()} tokens
                          <span className="text-gray-500 text-xs ml-1">
                            ({processingStats.totalAnthropicInputTokens.toLocaleString()} in / {processingStats.totalAnthropicOutputTokens.toLocaleString()} out)
                          </span>
                        </span>
                      </div>
                    )}
                    {processingStats.totalOpenaiTokens > 0 && (
                      <div className="flex justify-between">
                        <span className="text-gray-400">OpenAI:</span>
                        <span className="text-white font-medium">
                          {processingStats.totalOpenaiTokens.toLocaleString()} tokens
                        </span>
                      </div>
                    )}
                  </div>
                </div>
              )}
            </section>

            {/* API Keys Section */}
            <section>
              <div className="flex items-center gap-2 mb-4">
                <Key className="w-5 h-5 text-amber-400" />
                <h3 className="text-lg font-medium text-white">API Keys</h3>
              </div>

              {/* Anthropic API Key */}
              <div className="bg-gray-900/50 rounded-lg p-4 mb-3">
                <div className="flex items-center justify-between mb-2">
                  <span className="text-sm font-medium text-gray-200">Anthropic API Key</span>
                  {apiKeyStatus?.hasKey ? (
                    <span className="flex items-center gap-1 text-xs text-green-400">
                      <Check className="w-3 h-3" />
                      {apiKeyStatus.source === 'env' ? 'From environment' : 'Configured'}
                    </span>
                  ) : (
                    <span className="flex items-center gap-1 text-xs text-amber-400">
                      <AlertCircle className="w-3 h-3" />
                      Not set
                    </span>
                  )}
                </div>
                {apiKeyStatus?.maskedKey && (
                  <div className="mb-2 px-2 py-1 bg-gray-800 rounded text-xs text-gray-400 font-mono">
                    {apiKeyStatus.maskedKey}
                  </div>
                )}
                <div className="flex gap-2">
                  <input
                    type="password"
                    placeholder="sk-ant-api03-..."
                    value={apiKeyInput}
                    onChange={(e) => setApiKeyInput(e.target.value)}
                    className="flex-1 px-3 py-1.5 bg-gray-800 border border-gray-700 rounded text-sm text-white placeholder-gray-500 focus:border-amber-500/50 focus:ring-1 focus:ring-amber-500/20 focus:outline-none"
                  />
                  <button
                    onClick={handleSaveApiKey}
                    disabled={savingApiKey || !apiKeyInput.trim()}
                    className="px-3 py-1.5 bg-amber-500/20 text-amber-200 rounded text-sm font-medium hover:bg-amber-500/30 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                  >
                    {savingApiKey ? 'Saving...' : 'Save'}
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
                {apiKeyError && <p className="mt-2 text-xs text-red-400">{apiKeyError}</p>}
                <p className="mt-2 text-xs text-gray-500">
                  Required for AI processing.{' '}
                  <a href="https://console.anthropic.com/settings/keys" target="_blank" rel="noopener noreferrer" className="text-amber-400 hover:underline">
                    Get your key
                  </a>
                </p>
              </div>

              {/* OpenAI API Key */}
              <div className="bg-gray-900/50 rounded-lg p-4">
                <div className="flex items-center justify-between mb-2">
                  <span className="text-sm font-medium text-gray-200">OpenAI API Key <span className="text-gray-500">(optional)</span></span>
                  {openaiKeyStatus ? (
                    <span className="flex items-center gap-1 text-xs text-green-400">
                      <Check className="w-3 h-3" />
                      Configured
                    </span>
                  ) : (
                    <span className="text-xs text-gray-500">For embeddings</span>
                  )}
                </div>
                {openaiKeyStatus && (
                  <div className="mb-2 px-2 py-1 bg-gray-800 rounded text-xs text-gray-400 font-mono">
                    {openaiKeyStatus}
                  </div>
                )}
                <div className="flex gap-2">
                  <input
                    type="password"
                    placeholder="sk-..."
                    value={openaiKeyInput}
                    onChange={(e) => setOpenaiKeyInput(e.target.value)}
                    className="flex-1 px-3 py-1.5 bg-gray-800 border border-gray-700 rounded text-sm text-white placeholder-gray-500 focus:border-amber-500/50 focus:ring-1 focus:ring-amber-500/20 focus:outline-none"
                  />
                  <button
                    onClick={handleSaveOpenaiKey}
                    disabled={savingOpenaiKey || !openaiKeyInput.trim()}
                    className="px-3 py-1.5 bg-amber-500/20 text-amber-200 rounded text-sm font-medium hover:bg-amber-500/30 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                  >
                    {savingOpenaiKey ? 'Saving...' : 'Save'}
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
                {openaiKeyError && <p className="mt-2 text-xs text-red-400">{openaiKeyError}</p>}
                <p className="mt-2 text-xs text-gray-500">
                  Enables semantic similarity.{' '}
                  <a href="https://platform.openai.com/api-keys" target="_blank" rel="noopener noreferrer" className="text-amber-400 hover:underline">
                    Get your key
                  </a>
                </p>
              </div>
            </section>

            {/* Operations Section */}
            <section>
              <div className="flex items-center gap-2 mb-4">
                <RefreshCw className="w-5 h-5 text-green-400" />
                <h3 className="text-lg font-medium text-white">Operations</h3>
              </div>

              <div className="space-y-3">
                {/* Full Rebuild */}
                <div className="bg-gray-900/50 rounded-lg p-4">
                  <button
                    onClick={() => setConfirmAction('fullRebuild')}
                    disabled={isFullRebuilding || isFlattening || isConsolidating || isTidying}
                    className="w-full flex items-center justify-center gap-2 px-4 py-2.5 bg-green-600 hover:bg-green-700 text-white rounded-lg font-medium transition-colors disabled:opacity-50"
                  >
                    {isFullRebuilding ? (
                      <Loader2 className="w-4 h-4 animate-spin" />
                    ) : (
                      <RefreshCw className="w-4 h-4" />
                    )}
                    {isFullRebuilding ? 'Rebuilding...' : 'Full Rebuild'}
                  </button>
                  <p className="mt-2 text-xs text-gray-500">
                    Runs clustering + hierarchy building. Replaces all organization.
                  </p>
                </div>

                {/* Flatten Hierarchy */}
                <div className="bg-gray-900/50 rounded-lg p-4">
                  <button
                    onClick={() => setConfirmAction('flattenHierarchy')}
                    disabled={isFlattening || isFullRebuilding || isConsolidating || isTidying}
                    className="w-full flex items-center justify-center gap-2 px-4 py-2.5 bg-amber-600 hover:bg-amber-700 text-white rounded-lg font-medium transition-colors disabled:opacity-50"
                  >
                    {isFlattening ? (
                      <Loader2 className="w-4 h-4 animate-spin" />
                    ) : (
                      <Zap className="w-4 h-4" />
                    )}
                    {isFlattening ? 'Flattening...' : 'Flatten Empty Levels'}
                  </button>
                  <p className="mt-2 text-xs text-gray-500">
                    Removes passthrough "Uncategorized" nodes and cleans up hierarchy.
                  </p>
                </div>

                {/* Consolidate Root */}
                <div className="bg-gray-900/50 rounded-lg p-4">
                  <button
                    onClick={() => setConfirmAction('consolidateRoot')}
                    disabled={isConsolidating || isFullRebuilding || isFlattening || isTidying}
                    className="w-full flex items-center justify-center gap-2 px-4 py-2.5 bg-purple-600 hover:bg-purple-700 text-white rounded-lg font-medium transition-colors disabled:opacity-50"
                  >
                    {isConsolidating ? (
                      <Loader2 className="w-4 h-4 animate-spin" />
                    ) : (
                      <Zap className="w-4 h-4" />
                    )}
                    {isConsolidating ? 'Consolidating...' : 'Consolidate Root'}
                  </button>
                  <p className="mt-2 text-xs text-gray-500">
                    Groups Universe's children into 4-8 uber-categories (TECH, LIFE, MIND, etc.)
                  </p>
                </div>

                {/* Tidy Database */}
                <div className="bg-gray-900/50 rounded-lg p-4">
                  <button
                    onClick={() => setConfirmAction('tidyDatabase')}
                    disabled={isTidying || isFullRebuilding || isFlattening || isConsolidating}
                    className="w-full flex items-center justify-center gap-2 px-4 py-2.5 bg-cyan-600 hover:bg-cyan-700 text-white rounded-lg font-medium transition-colors disabled:opacity-50"
                  >
                    {isTidying ? (
                      <Loader2 className="w-4 h-4 animate-spin" />
                    ) : (
                      <Zap className="w-4 h-4" />
                    )}
                    {isTidying ? 'Tidying...' : 'Tidy Database'}
                  </button>
                  <p className="mt-2 text-xs text-gray-500">
                    Fast cleanup: fix counts/depths, remove empties, flatten chains, prune edges.
                  </p>
                </div>

                {operationResult && (
                  <p className="text-xs text-green-400">{operationResult}</p>
                )}
              </div>
            </section>

            {/* Data Section */}
            <section>
              <div className="flex items-center gap-2 mb-4">
                <Database className="w-5 h-5 text-blue-400" />
                <h3 className="text-lg font-medium text-white">Data</h3>
              </div>

              <div className="space-y-3">
                {/* Import */}
                <div className="bg-gray-900/50 rounded-lg p-4">
                  <button
                    onClick={handleImport}
                    disabled={importing}
                    className="w-full flex items-center justify-center gap-2 px-4 py-2.5 bg-blue-600 hover:bg-blue-700 text-white rounded-lg font-medium transition-colors disabled:opacity-50"
                  >
                    {importing ? (
                      <Loader2 className="w-4 h-4 animate-spin" />
                    ) : (
                      <Upload className="w-4 h-4" />
                    )}
                    Import Claude Conversations
                  </button>
                  <p className="mt-2 text-xs text-gray-500">
                    Select a <code className="bg-gray-800 px-1 rounded">conversations.json</code> file exported from Claude.
                    Duplicates will be skipped automatically.
                  </p>
                  {importResult && (
                    <p className={`mt-2 text-xs ${importResult.startsWith('Error') ? 'text-red-400' : 'text-green-400'}`}>
                      {importResult}
                    </p>
                  )}
                </div>

                {/* Delete All */}
                <div className="bg-gray-900/50 rounded-lg p-4">
                  <button
                    onClick={() => setConfirmAction('deleteAll')}
                    className="w-full flex items-center justify-center gap-2 px-4 py-2.5 bg-red-600 hover:bg-red-700 text-white rounded-lg font-medium transition-colors"
                  >
                    <Trash2 className="w-4 h-4" />
                    Delete All Data
                  </button>
                  <p className="mt-2 text-xs text-gray-500">
                    Permanently delete all nodes and edges. Cannot be undone.
                  </p>
                </div>
              </div>
            </section>

            {/* Reset Flags Section */}
            <section>
              <div className="flex items-center gap-2 mb-4">
                <RefreshCw className="w-5 h-5 text-purple-400" />
                <h3 className="text-lg font-medium text-white">Reset Flags</h3>
              </div>

              <div className="grid grid-cols-2 gap-3">
                <button
                  onClick={() => setConfirmAction('resetAi')}
                  className="flex flex-col items-center gap-2 p-4 bg-gray-900/50 hover:bg-gray-900 rounded-lg transition-colors text-center"
                >
                  <span className="text-sm font-medium text-gray-200">Reset AI Processing</span>
                  <span className="text-xs text-gray-500">Re-run title/summary generation</span>
                </button>

                <button
                  onClick={() => setConfirmAction('resetClustering')}
                  className="flex flex-col items-center gap-2 p-4 bg-gray-900/50 hover:bg-gray-900 rounded-lg transition-colors text-center"
                >
                  <span className="text-sm font-medium text-gray-200">Reset Clustering</span>
                  <span className="text-xs text-gray-500">Re-assign topics</span>
                </button>

                <button
                  onClick={() => setConfirmAction('clearEmbeddings')}
                  className="flex flex-col items-center gap-2 p-4 bg-gray-900/50 hover:bg-gray-900 rounded-lg transition-colors text-center"
                >
                  <span className="text-sm font-medium text-gray-200">Clear Embeddings</span>
                  <span className="text-xs text-gray-500">Re-generate semantic vectors</span>
                </button>

                <button
                  onClick={() => setConfirmAction('clearHierarchy')}
                  className="flex flex-col items-center gap-2 p-4 bg-gray-900/50 hover:bg-gray-900 rounded-lg transition-colors text-center"
                >
                  <span className="text-sm font-medium text-gray-200">Clear Hierarchy</span>
                  <span className="text-xs text-gray-500">Delete category nodes</span>
                </button>
              </div>

              {actionResult && (
                <p className={`mt-3 text-sm text-center ${actionResult.startsWith('Error') ? 'text-red-400' : 'text-green-400'}`}>
                  {actionResult}
                </p>
              )}
            </section>
          </div>
        </div>
      </div>

      {/* Confirmation Dialog */}
      {confirmAction && (
        <ConfirmDialog
          open={!!confirmAction}
          title={confirmActionConfig[confirmAction].title}
          message={confirmActionConfig[confirmAction].message}
          confirmText={actionLoading ? 'Processing...' : 'Confirm'}
          variant={confirmAction === 'deleteAll' ? 'danger' : 'warning'}
          onConfirm={handleConfirmAction}
          onCancel={() => setConfirmAction(null)}
        />
      )}
    </>
  );
}
