import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { readTextFile } from '@tauri-apps/plugin-fs';
import { useGraphStore } from '../../stores/graphStore';
import {
  X,
  Key,
  RefreshCw,
  Trash2,
  Check,
  AlertCircle,
  Loader2,
  FolderOpen,
  HardDrive,
  Clock,
  Zap,
  Shield,
  Download,
  Cpu,
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
  // For API cost estimation
  unprocessedItems: number;
  unclusteredItems: number;
  orphanItems: number;
  topicsCount: number;
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

type ConfirmAction = 'deleteAll' | 'resetAi' | 'resetClustering' | 'clearEmbeddings' | 'clearHierarchy' | 'resetPrivacy' | 'fullRebuild' | 'flattenHierarchy' | 'consolidateRoot' | 'tidyDatabase' | null;

interface TidyReport {
  sameNameMerged: number;
  chainsFlattened: number;
  emptiesRemoved: number;
  emptyItemsRemoved: number;
  childCountsFixed: number;
  depthsFixed: number;
  orphansReparented: number;
  deadEdgesPruned: number;
  duplicateEdgesRemoved: number;
  durationMs: number;
}

interface PrivacyStats {
  total: number;
  scanned: number;
  unscanned: number;
  private: number;
  safe: number;
  totalCategories: number;
  scannedCategories: number;
}

type SettingsTab = 'setup' | 'keys' | 'maintenance' | 'privacy' | 'info';

export function SettingsPanel({ open, onClose, onDataChanged }: SettingsPanelProps) {
  // Navigation reset for database switches
  const { navigateToRoot } = useGraphStore();

  // Active tab
  const [activeTab, setActiveTab] = useState<SettingsTab>('setup');

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
  const [showImportSources, setShowImportSources] = useState(false);
  const importDropdownRef = useRef<HTMLDivElement>(null);

  // Click outside to close import sources dropdown
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (importDropdownRef.current && !importDropdownRef.current.contains(event.target as Node)) {
        setShowImportSources(false);
      }
    };
    if (showImportSources) {
      document.addEventListener('mousedown', handleClickOutside);
    }
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [showImportSources]);

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

  // Setup flow operations
  const [isProcessingAi, setIsProcessingAi] = useState(false);
  const [isClustering, setIsClustering] = useState(false);
  const [isBuildingHierarchy, setIsBuildingHierarchy] = useState(false);
  const [isUpdatingDates, setIsUpdatingDates] = useState(false);
  const [isQuickAdding, setIsQuickAdding] = useState(false);
  const [inboxCount, setInboxCount] = useState(0);
  const [setupResult, setSetupResult] = useState<string | null>(null);

  // Quick Process (post-import)
  const [showQuickProcessPrompt, setShowQuickProcessPrompt] = useState(false);
  const [newItemCount, setNewItemCount] = useState(0);
  const [isQuickProcessing, setIsQuickProcessing] = useState(false);
  const [quickProcessStep, setQuickProcessStep] = useState('');

  // Privacy scanning
  const [privacyStats, setPrivacyStats] = useState<PrivacyStats | null>(null);
  const [isScanning, setIsScanning] = useState(false);
  const [scanProgress, setScanProgress] = useState<{ current: number; total: number; status: string } | null>(null);
  const [privacyResult, setPrivacyResult] = useState<string | null>(null);
  const [showcaseMode, setShowcaseMode] = useState(false);

  // Protection settings
  const [protectRecentNotes, setProtectRecentNotes] = useState(true);

  // Local embeddings
  const [useLocalEmbeddings, setUseLocalEmbeddings] = useState(false);
  const [isRegeneratingEmbeddings, setIsRegeneratingEmbeddings] = useState(false);
  const [regenerateProgress, setRegenerateProgress] = useState<{ current: number; total: number; status: string } | null>(null);

  // Load data on mount
  useEffect(() => {
    if (open) {
      loadApiKeyStatus();
      loadOpenaiKeyStatus();
      loadDbStats();
      loadDbPath();
      loadProcessingStats();
      loadPrivacyStats();
      loadProtectionSettings();
      loadLocalEmbeddingsStatus();
      setImportResult(null);
      setActionResult(null);
      setPrivacyResult(null);
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

  const loadPrivacyStats = async () => {
    try {
      const stats = await invoke<PrivacyStats>('get_privacy_stats');
      setPrivacyStats(stats);
    } catch (err) {
      console.error('Failed to load privacy stats:', err);
    }
  };

  const loadProtectionSettings = async () => {
    try {
      const protected_ = await invoke<boolean>('get_protect_recent_notes');
      setProtectRecentNotes(protected_);
    } catch (err) {
      console.error('Failed to load protection settings:', err);
    }
  };

  const handleToggleProtectRecentNotes = async (enabled: boolean) => {
    try {
      await invoke('set_protect_recent_notes', { protected: enabled });
      setProtectRecentNotes(enabled);
    } catch (err) {
      console.error('Failed to toggle protection:', err);
    }
  };

  const loadLocalEmbeddingsStatus = async () => {
    try {
      const enabled = await invoke<boolean>('get_use_local_embeddings');
      setUseLocalEmbeddings(enabled);
    } catch (err) {
      console.error('Failed to load local embeddings status:', err);
    }
  };

  const handleToggleLocalEmbeddings = async (enabled: boolean) => {
    // Check if OpenAI key is available when switching away from local
    if (!enabled && !openaiKeyStatus) {
      setActionResult('Error: OpenAI API key required for cloud embeddings');
      return;
    }

    try {
      await invoke('set_use_local_embeddings', { enabled });
      setUseLocalEmbeddings(enabled);
      setActionResult(null);
    } catch (err) {
      setActionResult(`Error: ${err}`);
    }
  };

  const handleRegenerateEmbeddings = async () => {
    // Check requirements
    if (!useLocalEmbeddings && !openaiKeyStatus) {
      setActionResult('Error: OpenAI API key required for cloud embeddings');
      return;
    }

    setIsRegeneratingEmbeddings(true);
    setRegenerateProgress({ current: 0, total: 0, status: 'starting' });
    setActionResult(null);

    // Listen for progress events
    const { listen } = await import('@tauri-apps/api/event');
    const unlisten = await listen<{
      current: number;
      total: number;
      status: string;
    }>('regenerate-progress', (event) => {
      setRegenerateProgress(event.payload);
    });

    try {
      const result = await invoke<{
        count: number;
        embeddingSource: string;
        durationSecs: number;
      }>('regenerate_all_embeddings');

      setActionResult(`Regenerated ${result.count} embeddings using ${result.embeddingSource} (${result.durationSecs.toFixed(1)}s)`);
      await loadDbStats();
    } catch (err) {
      setActionResult(`Error: ${err}`);
    } finally {
      unlisten();
      setIsRegeneratingEmbeddings(false);
      setRegenerateProgress(null);
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

  // Estimate API cost (returns formatted string)
  // Pricing: Haiku $1/$5 MTok, Sonnet $3/$15 MTok, OpenAI embed $0.02/MTok
  const estimateCost = (calls: number, model: 'haiku' | 'sonnet' | 'openai', avgTokensPerCall: number = 2000): string => {
    if (calls === 0) return '$0.00';
    const totalTokens = calls * avgTokensPerCall;
    let cost = 0;
    if (model === 'haiku') {
      // ~50% input, ~50% output tokens assumed
      cost = (totalTokens * 0.5 / 1_000_000) * 1 + (totalTokens * 0.5 / 1_000_000) * 5;
    } else if (model === 'sonnet') {
      cost = (totalTokens * 0.5 / 1_000_000) * 3 + (totalTokens * 0.5 / 1_000_000) * 15;
    } else {
      cost = (totalTokens / 1_000_000) * 0.02;
    }
    if (cost < 0.01) return '<$0.01';
    return `~$${cost.toFixed(2)}`;
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
        // Update local state with new db stats
        setDbStats(stats);
        setDbPath(file);
        // Reset navigation to universe root
        navigateToRoot();
        // Trigger graph data refresh
        onDataChanged?.();
        // Close settings panel to show the new data
        onClose();
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
    setShowQuickProcessPrompt(false);
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
        await loadDbStats();
        onDataChanged?.();

        // Show Quick Process prompt if items were imported
        if (result.conversationsImported > 0) {
          setNewItemCount(result.conversationsImported);
          setShowQuickProcessPrompt(true);
          setImportResult(null); // Clear result, prompt will show instead
        } else {
          setImportResult(`No new conversations found`);
        }
      }
    } catch (err) {
      setImportResult(`Error: ${err}`);
    } finally {
      setImporting(false);
    }
  };

  // Markdown import handler
  const handleImportMarkdown = async () => {
    setImporting(true);
    setImportResult(null);
    try {
      const files = await openDialog({
        title: 'Select Markdown files',
        filters: [{ name: 'Markdown', extensions: ['md', 'markdown', 'txt'] }],
        multiple: true,
      });
      if (files) {
        const filePaths = Array.isArray(files) ? files : [files];
        if (filePaths.length > 0) {
          const result = await invoke<{ exchangesImported: number; skipped: number; errors: string[] }>(
            'import_markdown_files',
            { filePaths }
          );
          await loadDbStats();
          onDataChanged?.();
          const errorMsg = result.errors.length > 0 ? ` (${result.errors.length} errors)` : '';
          setImportResult(`Imported ${result.exchangesImported} markdown file${result.exchangesImported !== 1 ? 's' : ''}${errorMsg}`);
        }
      }
    } catch (err) {
      setImportResult(`Error: ${err}`);
    } finally {
      setImporting(false);
    }
  };

  // Google Keep import handler
  const handleImportGoogleKeep = async () => {
    setImporting(true);
    setImportResult(null);
    try {
      const file = await openDialog({
        title: 'Select Google Takeout zip file',
        filters: [{ name: 'Zip Archive', extensions: ['zip'] }],
        multiple: false,
      });
      if (file && typeof file === 'string') {
        const result = await invoke<{ notesImported: number; skipped: number; warnings: string[]; errors: string[] }>(
          'import_google_keep',
          { zipPath: file }
        );
        await loadDbStats();
        onDataChanged?.();
        const errorMsg = result.errors.length > 0 ? ` (${result.errors.length} errors)` : '';
        const warnMsg = result.warnings.length > 0 ? ` (${result.warnings.length} warnings)` : '';
        setImportResult(`Imported ${result.notesImported} Google Keep note${result.notesImported !== 1 ? 's' : ''}${warnMsg}${errorMsg}`);

        // Show Quick Process prompt if notes were imported
        if (result.notesImported > 0) {
          setNewItemCount(result.notesImported);
          setShowQuickProcessPrompt(true);
          setImportResult(null);
        }
      }
    } catch (err) {
      setImportResult(`Error: ${err}`);
    } finally {
      setImporting(false);
    }
  };

  const handleQuickProcess = async () => {
    setIsQuickProcessing(true);
    setQuickProcessStep('');
    try {
      // Step 1: Process AI (only unprocessed items)
      setQuickProcessStep('Processing AI titles & summaries...');
      await invoke('process_nodes');

      // Step 2: Cluster (only unclustered items)
      setQuickProcessStep('Clustering into topics...');
      await invoke('run_clustering');

      // Step 3: Quick Add to hierarchy
      setQuickProcessStep('Adding to hierarchy...');
      const result = await invoke<{
        items_added: number;
        topics_created: number;
        inbox_count: number;
      }>('quick_add_to_hierarchy');

      setShowQuickProcessPrompt(false);
      setInboxCount(result.inbox_count);
      setImportResult(`Processed ${newItemCount} items → ${result.items_added} added, ${result.topics_created} new topics`);
      await loadDbStats();
      onDataChanged?.();
      window.location.reload();
    } catch (err) {
      setImportResult(`Error: ${err}`);
      setShowQuickProcessPrompt(false);
    } finally {
      setIsQuickProcessing(false);
      setQuickProcessStep('');
    }
  };

  // Privacy scan handlers
  const handlePrivacyScan = async () => {
    setIsScanning(true);
    setScanProgress({ current: 0, total: 0, status: 'starting' });
    setPrivacyResult(null);

    // Listen for progress events
    const { listen } = await import('@tauri-apps/api/event');
    const unlisten = await listen<{
      current: number;
      total: number;
      nodeTitle: string;
      isPrivate: boolean;
      reason: string | null;
      status: string;
      errorMessage: string | null;
    }>('privacy-progress', (event) => {
      const { current, total, status } = event.payload;
      setScanProgress({ current, total, status });

      if (status === 'complete' || status === 'cancelled') {
        unlisten();
        setIsScanning(false);
        loadPrivacyStats();
      }
    });

    try {
      const result = await invoke<{
        total: number;
        privateCount: number;
        safeCount: number;
        errorCount: number;
        cancelled: boolean;
      }>('analyze_all_privacy', { showcaseMode });

      if (result.cancelled) {
        setPrivacyResult('Scan cancelled');
      } else {
        const modeLabel = showcaseMode ? ' (showcase mode)' : '';
        setPrivacyResult(`Scanned ${result.total} items: ${result.privateCount} private, ${result.safeCount} safe${result.errorCount > 0 ? `, ${result.errorCount} errors` : ''}${modeLabel}`);
      }
    } catch (err) {
      setPrivacyResult(`Error: ${err}`);
      unlisten();
    } finally {
      setIsScanning(false);
      setScanProgress(null);
      await loadPrivacyStats();
    }
  };

  const handleCancelPrivacyScan = async () => {
    try {
      await invoke('cancel_privacy_scan');
    } catch (err) {
      console.error('Failed to cancel privacy scan:', err);
    }
  };

  const handleCategoryScan = async () => {
    setIsScanning(true);
    setScanProgress({ current: 0, total: 0, status: 'starting' });
    setPrivacyResult(null);

    const { listen } = await import('@tauri-apps/api/event');
    const unlisten = await listen<{
      current: number;
      total: number;
      nodeTitle: string;
      isPrivate: boolean;
      reason: string | null;
      status: string;
      errorMessage: string | null;
    }>('privacy-progress', (event) => {
      const { current, total, status } = event.payload;
      setScanProgress({ current, total, status });

      if (status === 'complete' || status === 'cancelled') {
        unlisten();
        setIsScanning(false);
        loadPrivacyStats();
      }
    });

    try {
      const result = await invoke<{
        categoriesScanned: number;
        categoriesPrivate: number;
        categoriesSafe: number;
        itemsPropagated: number;
        errorCount: number;
        cancelled: boolean;
      }>('analyze_categories_privacy', { showcaseMode });

      if (result.cancelled) {
        setPrivacyResult('Scan cancelled');
      } else {
        const modeLabel = showcaseMode ? ' (showcase)' : '';
        setPrivacyResult(`Scanned ${result.categoriesScanned} categories: ${result.categoriesPrivate} private, ${result.categoriesSafe} safe → ${result.itemsPropagated} items filtered${modeLabel}`);
      }
    } catch (err) {
      setPrivacyResult(`Error: ${err}`);
      unlisten();
    } finally {
      setIsScanning(false);
      setScanProgress(null);
      await loadPrivacyStats();
    }
  };

  const handleExportShareable = async () => {
    setPrivacyResult(null);
    try {
      const path = await invoke<string>('export_shareable_db');
      setPrivacyResult(`Exported: ${path}`);
    } catch (err) {
      setPrivacyResult(`Error: ${err}`);
    }
  };

  // Setup flow handlers
  const handleProcessAi = async () => {
    setIsProcessingAi(true);
    setSetupResult(null);
    try {
      const result = await invoke<{ processed: number; skipped: number }>('process_nodes');
      setSetupResult(`AI processing complete: ${result.processed} processed, ${result.skipped} skipped`);
      await loadDbStats();
      onDataChanged?.();
      setIsProcessingAi(false);
    } catch (err) {
      setSetupResult(`Error: ${err}`);
      setIsProcessingAi(false);
    }
  };

  const handleClustering = async () => {
    setIsClustering(true);
    setSetupResult(null);
    try {
      const result = await invoke<{ clusters_created: number; nodes_clustered: number }>('run_clustering');
      setSetupResult(`Clustering complete: ${result.clusters_created} clusters, ${result.nodes_clustered} nodes`);
      await loadDbStats();
      onDataChanged?.();
      window.location.reload();
    } catch (err) {
      setSetupResult(`Error: ${err}`);
      setIsClustering(false);
    }
  };

  const handleBuildHierarchy = async () => {
    setIsBuildingHierarchy(true);
    setSetupResult(null);
    try {
      const result = await invoke<{
        clusteringResult: { itemsProcessed: number; clustersCreated: number; itemsAssigned: number } | null;
        hierarchyResult: { levelsCreated: number; intermediateNodesCreated: number; itemsOrganized: number; maxDepth: number };
        levelsCreated: number;
        groupingIterations: number;
      }>('build_full_hierarchy', { runClustering: false });
      setSetupResult(`Hierarchy built: ${result.levelsCreated} levels, ${result.groupingIterations} AI grouping iterations`);
      await loadDbStats();
      onDataChanged?.();
      window.location.reload();
    } catch (err) {
      setSetupResult(`Error: ${err}`);
      setIsBuildingHierarchy(false);
    }
  };

  const handleUpdateDates = async () => {
    setIsUpdatingDates(true);
    setSetupResult(null);
    try {
      await invoke('propagate_latest_dates');
      setSetupResult('Dates updated');
      await loadDbStats();
      onDataChanged?.();
      window.location.reload();
    } catch (err) {
      setSetupResult(`Error: ${err}`);
      setIsUpdatingDates(false);
    }
  };

  const handleQuickAdd = async () => {
    setIsQuickAdding(true);
    setSetupResult(null);
    try {
      const result = await invoke<{
        items_added: number;
        topics_created: number;
        items_skipped: number;
        inbox_count: number;
      }>('quick_add_to_hierarchy');
      setInboxCount(result.inbox_count);
      if (result.items_added === 0 && result.topics_created === 0) {
        setSetupResult('No orphan items to add');
      } else {
        setSetupResult(`Added ${result.items_added} items, created ${result.topics_created} topics`);
      }
      await loadDbStats();
      onDataChanged?.();
      window.location.reload();
    } catch (err) {
      setSetupResult(`Error: ${err}`);
      setIsQuickAdding(false);
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
    resetPrivacy: {
      title: 'Reset Privacy Flags',
      message: 'This will clear all privacy scan results, marking all items as unscanned. Use this before re-scanning with different settings (e.g., showcase mode).',
      handler: async () => {
        const count = await invoke<number>('reset_privacy_flags');
        await loadPrivacyStats();
        return `Reset privacy flags for ${count} nodes`;
      },
    },
    fullRebuild: {
      title: 'Full Rebuild',
      message: 'This will run embedding clustering (free), AI hierarchy grouping (Sonnet), and flatten empty levels. Replaces all organization.',
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

          // Flatten empty passthrough levels
          const flattenCount = await invoke<number>('flatten_hierarchy');

          const msg = result.clusteringResult
            ? `Clustered ${result.clusteringResult.itemsAssigned} items → ${result.clusteringResult.clustersCreated} clusters, ${result.hierarchyResult.intermediateNodesCreated} nodes, flattened ${flattenCount}`
            : `Created ${result.hierarchyResult.intermediateNodesCreated} hierarchy nodes, flattened ${flattenCount}`;
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
      message: 'Run safe cleanup: merge same-name nodes, flatten chains, remove empties, fix counts/depths, prune edges.',
      handler: async () => {
        setIsTidying(true);
        setOperationResult(null);
        try {
          const report = await invoke<TidyReport>('tidy_database');
          const parts: string[] = [];
          if (report.sameNameMerged > 0) parts.push(`merged ${report.sameNameMerged} same-name`);
          if (report.chainsFlattened > 0) parts.push(`flattened ${report.chainsFlattened} chains`);
          if (report.emptiesRemoved > 0) parts.push(`removed ${report.emptiesRemoved} empty categories`);
          if (report.emptyItemsRemoved > 0) parts.push(`removed ${report.emptyItemsRemoved} empty items`);
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
        <div className="relative bg-gray-800 rounded-xl border border-gray-700 shadow-2xl max-w-2xl w-full mx-4 max-h-[85vh] overflow-hidden flex flex-col">
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

          {/* Tab Navigation */}
          <div className="flex border-b border-gray-700 px-4">
            {[
              { id: 'setup' as const, label: 'Setup', icon: '🚀' },
              { id: 'keys' as const, label: 'API Keys', icon: '🔑' },
              { id: 'maintenance' as const, label: 'Maintenance', icon: '🔧' },
              { id: 'privacy' as const, label: 'Privacy', icon: '🔒' },
              { id: 'info' as const, label: 'Info', icon: 'ℹ️' },
            ].map((tab) => (
              <button
                key={tab.id}
                onClick={() => setActiveTab(tab.id)}
                className={`px-4 py-2.5 text-sm font-medium transition-colors relative ${
                  activeTab === tab.id
                    ? 'text-amber-400'
                    : 'text-gray-400 hover:text-gray-200'
                }`}
              >
                <span className="mr-1.5">{tab.icon}</span>
                {tab.label}
                {activeTab === tab.id && (
                  <div className="absolute bottom-0 left-0 right-0 h-0.5 bg-amber-400" />
                )}
              </button>
            ))}
          </div>

          {/* Content */}
          <div className="flex-1 overflow-y-auto p-6 space-y-6">
            {/* INFO TAB - Database Section */}
            {activeTab === 'info' && (
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
                      <Loader2 className="w-5 h-5 animate-spin" />
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

              {/* Protection Settings */}
              <div className="bg-gray-900/50 rounded-lg p-4 mt-3">
                <div className="flex items-center justify-between">
                  <div>
                    <div className="text-sm font-medium text-gray-200">Protect Recent Notes</div>
                    <div className="text-xs text-gray-500 mt-0.5">Exclude from AI, clustering, hierarchy, and tidy operations</div>
                  </div>
                  <button
                    onClick={() => handleToggleProtectRecentNotes(!protectRecentNotes)}
                    className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
                      protectRecentNotes ? 'bg-amber-500' : 'bg-gray-600'
                    }`}
                  >
                    <span
                      className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                        protectRecentNotes ? 'translate-x-6' : 'translate-x-1'
                      }`}
                    />
                  </button>
                </div>
              </div>
            </section>
            )}

            {/* SETUP TAB - Setup Flow Section */}
            {activeTab === 'setup' && (
            <section>
              <div className="flex items-center gap-2 mb-4">
                <Zap className="w-5 h-5 text-amber-400" />
                <h3 className="text-lg font-medium text-white">Setup Flow</h3>
                <span className="text-xs text-gray-500">(follow in order)</span>
              </div>

              <div className="space-y-2">
                {/* Step 1: Import */}
                <div className="bg-gray-900/50 rounded-lg p-4">
                  <div className="flex items-center gap-2 mb-3">
                    <span className="flex-shrink-0 w-7 h-7 rounded-full bg-blue-600 flex items-center justify-center text-white text-sm font-bold">1</span>
                    <div className="text-sm font-medium text-gray-200">Import Data</div>
                    <span className="text-xs text-gray-600">No API calls</span>
                  </div>
                  <div className="flex gap-3">
                    <div className="flex-1 relative" ref={importDropdownRef}>
                      <button
                        onClick={() => setShowImportSources(!showImportSources)}
                        disabled={importing}
                        className="w-full flex flex-col items-center gap-2 p-3 bg-gray-800/50 hover:bg-gray-800 rounded-lg transition-colors disabled:opacity-50"
                      >
                        <span className="text-2xl">{importing ? <Loader2 className="w-6 h-6 animate-spin" /> : '💬'}</span>
                        <span className="text-xs text-gray-300">Conversations</span>
                        <span className="text-xs text-gray-500">select source ▾</span>
                      </button>
                      {showImportSources && (
                        <div className="absolute top-full left-0 right-0 mt-1 bg-gray-800 border border-gray-700 rounded-lg shadow-xl z-10 overflow-hidden">
                          <button
                            onClick={() => {
                              setShowImportSources(false);
                              handleImport();
                            }}
                            className="w-full flex items-center gap-3 px-4 py-3 hover:bg-gray-700 transition-colors text-left"
                          >
                            <span className="text-xl">🟠</span>
                            <div>
                              <div className="text-sm text-gray-200">Claude</div>
                              <div className="text-xs text-gray-500">conversations.json</div>
                            </div>
                          </button>
                          <div className="px-4 py-2 text-xs text-gray-600 border-t border-gray-700">
                            More sources coming soon...
                          </div>
                        </div>
                      )}
                    </div>
                    <button
                      onClick={handleImportMarkdown}
                      disabled={importing}
                      className="flex-1 flex flex-col items-center gap-2 p-3 bg-gray-800/50 hover:bg-gray-800 rounded-lg transition-colors disabled:opacity-50"
                    >
                      <span className="text-2xl">{importing ? <Loader2 className="w-6 h-6 animate-spin" /> : '📄'}</span>
                      <span className="text-xs text-gray-300">Markdown</span>
                      <span className="text-xs text-gray-500">.md</span>
                    </button>
                    <button
                      onClick={handleImportGoogleKeep}
                      disabled={importing}
                      className="flex-1 flex flex-col items-center gap-2 p-3 bg-gray-800/50 hover:bg-gray-800 rounded-lg transition-colors disabled:opacity-50"
                    >
                      <span className="text-2xl">{importing ? <Loader2 className="w-6 h-6 animate-spin" /> : '📝'}</span>
                      <span className="text-xs text-gray-300">Google Keep</span>
                      <span className="text-xs text-gray-500">.zip</span>
                    </button>
                  </div>
                </div>

                {/* Quick Process prompt (after import) */}
                {showQuickProcessPrompt && (
                  <div className="bg-blue-900/30 border border-blue-500/50 rounded-lg p-4">
                    <div className="flex items-start gap-3">
                      <span className="text-2xl">📥</span>
                      <div className="flex-1">
                        <div className="text-sm font-medium text-blue-200">
                          Imported {newItemCount} new conversation{newItemCount !== 1 ? 's' : ''}
                        </div>
                        <div className="text-xs text-gray-400 mt-1">
                          Process them now? (AI titles → Cluster → Add to hierarchy)
                        </div>
                        {isQuickProcessing && (
                          <div className="text-xs text-amber-400 mt-2 flex items-center gap-2">
                            <Loader2 className="w-5 h-5 animate-spin" />
                            {quickProcessStep}
                          </div>
                        )}
                      </div>
                      <div className="flex gap-2">
                        <button
                          onClick={() => setShowQuickProcessPrompt(false)}
                          disabled={isQuickProcessing}
                          className="px-3 py-1.5 text-gray-400 hover:text-white text-xs disabled:opacity-50"
                        >
                          Later
                        </button>
                        <button
                          onClick={handleQuickProcess}
                          disabled={isQuickProcessing || !apiKeyStatus?.hasKey}
                          className="px-3 py-1.5 bg-blue-600 hover:bg-blue-700 text-white rounded text-xs font-medium disabled:opacity-50"
                        >
                          {isQuickProcessing ? 'Processing...' : 'Quick Process'}
                        </button>
                      </div>
                    </div>
                  </div>
                )}

                {/* Step 2: Process AI */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-purple-600 flex items-center justify-center text-white text-sm font-bold">2</span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Process AI</div>
                    <div className="text-xs text-gray-500">Generate titles, summaries & embeddings</div>
                    {dbStats && dbStats.unprocessedItems > 0 && (
                      <div className="text-xs text-amber-400/80 mt-0.5">
                        {dbStats.unprocessedItems} calls · Haiku {estimateCost(dbStats.unprocessedItems, 'haiku', 3500)}
                        {openaiKeyStatus && ` + OpenAI ${estimateCost(dbStats.unprocessedItems, 'openai', 1500)}`}
                      </div>
                    )}
                    {dbStats && dbStats.unprocessedItems === 0 && (
                      <div className="text-xs text-green-500/70 mt-0.5">All items processed</div>
                    )}
                  </div>
                  <button
                    onClick={handleProcessAi}
                    disabled={isProcessingAi || !apiKeyStatus?.hasKey}
                    className="flex-shrink-0 w-12 h-12 flex items-center justify-center text-xl bg-purple-600 hover:bg-purple-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isProcessingAi ? <Loader2 className="w-5 h-5 animate-spin" /> : '🤖'}
                  </button>
                </div>

                {/* Step 3: Cluster */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-cyan-600 flex items-center justify-center text-white text-sm font-bold">3</span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Cluster</div>
                    <div className="text-xs text-gray-500">Group items by embedding similarity (free)</div>
                    {dbStats && dbStats.unclusteredItems > 0 && (
                      <div className="text-xs text-cyan-400 mt-0.5">
                        {dbStats.unclusteredItems} items to cluster · Local embeddings
                      </div>
                    )}
                    {dbStats && dbStats.unclusteredItems === 0 && (
                      <div className="text-xs text-green-500/70 mt-0.5">All items clustered</div>
                    )}
                  </div>
                  <button
                    onClick={handleClustering}
                    disabled={isClustering}
                    className="flex-shrink-0 w-12 h-12 flex items-center justify-center text-xl bg-cyan-600 hover:bg-cyan-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isClustering ? <Loader2 className="w-5 h-5 animate-spin" /> : '🎯'}
                  </button>
                </div>

                {/* Step 4: Build Hierarchy */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-green-600 flex items-center justify-center text-white text-sm font-bold">4</span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Build Hierarchy</div>
                    <div className="text-xs text-gray-500">Create navigation structure + group embeddings</div>
                    {dbStats && dbStats.topicsCount > 15 && (
                      <div className="text-xs text-amber-400/80 mt-0.5">
                        ~{Math.ceil(Math.log(dbStats.topicsCount / 10) / Math.log(10) * 3)} calls · Sonnet {estimateCost(Math.ceil(Math.log(dbStats.topicsCount / 10) / Math.log(10) * 3), 'sonnet', 5000)}
                        {openaiKeyStatus && ` + OpenAI ${estimateCost(Math.ceil(dbStats.topicsCount / 8), 'openai', 500)}`}
                        <span className="text-gray-500 ml-1">({dbStats.topicsCount} topics)</span>
                      </div>
                    )}
                    {dbStats && dbStats.topicsCount <= 15 && dbStats.topicsCount > 0 && (
                      <div className="text-xs text-green-500/70 mt-0.5">
                        1 call · Sonnet {estimateCost(1, 'sonnet', 5000)}
                        {openaiKeyStatus && ` + OpenAI ${estimateCost(dbStats.topicsCount, 'openai', 500)}`}
                      </div>
                    )}
                    {dbStats && dbStats.topicsCount === 0 && (
                      <div className="text-xs text-gray-500 mt-0.5">Run Cluster first</div>
                    )}
                  </div>
                  <button
                    onClick={handleBuildHierarchy}
                    disabled={isBuildingHierarchy || !apiKeyStatus?.hasKey}
                    className="flex-shrink-0 w-12 h-12 flex items-center justify-center text-xl bg-green-600 hover:bg-green-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isBuildingHierarchy ? <Loader2 className="w-5 h-5 animate-spin" /> : '🏗️'}
                  </button>
                </div>

                {/* Step 5: Update Dates (optional) */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-cyan-600 flex items-center justify-center text-white text-sm font-bold">5</span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Update Dates <span className="text-gray-500">(optional)</span></div>
                    <div className="text-xs text-gray-500">Propagate dates to groups (fast)</div>
                    <div className="text-xs text-gray-600 mt-0.5">No API calls</div>
                  </div>
                  <button
                    onClick={handleUpdateDates}
                    disabled={isUpdatingDates}
                    className="flex-shrink-0 w-12 h-12 flex items-center justify-center text-xl bg-cyan-600 hover:bg-cyan-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isUpdatingDates ? <Loader2 className="w-5 h-5 animate-spin" /> : '📅'}
                  </button>
                </div>

                {/* Step 6: Quick Add (for incremental imports) */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-indigo-600 flex items-center justify-center text-white text-sm font-bold">6</span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Quick Add <span className="text-gray-500">(incremental)</span></div>
                    <div className="text-xs text-gray-500">Fast add new items to hierarchy (~2s)</div>
                    <div className="text-xs text-gray-600 mt-0.5">
                      No API calls
                      {dbStats && dbStats.orphanItems > 0 && (
                        <span className="text-amber-400/80 ml-1">· {dbStats.orphanItems} items to add</span>
                      )}
                    </div>
                  </div>
                  <button
                    onClick={handleQuickAdd}
                    disabled={isQuickAdding}
                    className="flex-shrink-0 w-12 h-12 flex items-center justify-center text-xl bg-indigo-600 hover:bg-indigo-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isQuickAdding ? <Loader2 className="w-5 h-5 animate-spin" /> : '⚡'}
                  </button>
                </div>

                {/* Inbox warning */}
                {inboxCount > 20 && (
                  <div className="bg-amber-900/30 border border-amber-600/50 rounded-lg p-3 flex items-start gap-2">
                    <AlertCircle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
                    <div className="text-xs text-amber-200">
                      <strong>📥 Inbox has {inboxCount} items.</strong> Consider running "Build Hierarchy" (Step 4) to properly organize new topics.
                    </div>
                  </div>
                )}

                {setupResult && (
                  <p className={`text-xs mt-2 ${setupResult.startsWith('Error') ? 'text-red-400' : 'text-green-400'}`}>
                    {setupResult}
                  </p>
                )}

                {importResult && (
                  <p className={`text-xs ${importResult.startsWith('Error') ? 'text-red-400' : 'text-green-400'}`}>
                    {importResult}
                  </p>
                )}
              </div>
            </section>
            )}

            {/* KEYS TAB - API Keys Section */}
            {activeTab === 'keys' && (
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

              {/* Local Embeddings Toggle */}
              <div className="bg-gray-900/50 rounded-lg p-4 mt-3">
                <div className="flex items-center gap-2 mb-3">
                  <Cpu className="w-5 h-5 text-cyan-400" />
                  <span className="text-sm font-medium text-gray-200">Embedding Source</span>
                </div>

                <div className="flex items-center justify-between">
                  <div>
                    <div className="text-sm text-gray-300">
                      {useLocalEmbeddings ? (
                        <span className="flex items-center gap-2">
                          <span className="text-cyan-400">Local Model</span>
                          <span className="text-xs text-gray-500">(all-MiniLM-L6-v2, 384-dim)</span>
                        </span>
                      ) : (
                        <span className="flex items-center gap-2">
                          <span className="text-amber-400">OpenAI API</span>
                          <span className="text-xs text-gray-500">(text-embedding-3-small, 1536-dim)</span>
                        </span>
                      )}
                    </div>
                    <div className="text-xs text-gray-500 mt-1">
                      {useLocalEmbeddings
                        ? 'Free, offline, optimized for clustering'
                        : 'Requires API key, not optimized for clustering'}
                    </div>
                  </div>
                  <button
                    onClick={() => handleToggleLocalEmbeddings(!useLocalEmbeddings)}
                    disabled={isRegeneratingEmbeddings}
                    className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
                      useLocalEmbeddings ? 'bg-cyan-500' : 'bg-gray-600'
                    } ${isRegeneratingEmbeddings ? 'opacity-50 cursor-not-allowed' : ''}`}
                  >
                    <span
                      className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                        useLocalEmbeddings ? 'translate-x-6' : 'translate-x-1'
                      }`}
                    />
                  </button>
                </div>

                {/* Regenerate Button */}
                <div className="mt-3 flex items-center gap-2">
                  <button
                    onClick={handleRegenerateEmbeddings}
                    disabled={isRegeneratingEmbeddings || (!useLocalEmbeddings && !openaiKeyStatus)}
                    className="flex-1 flex items-center justify-center gap-2 px-3 py-2 bg-cyan-600 hover:bg-cyan-700 text-white rounded text-sm font-medium transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    {isRegeneratingEmbeddings ? (
                      <>
                        <Loader2 className="w-4 h-4 animate-spin" />
                        Regenerating...
                      </>
                    ) : (
                      <>
                        <RefreshCw className="w-4 h-4" />
                        Regenerate All Embeddings
                      </>
                    )}
                  </button>
                </div>

                {/* Regeneration Progress */}
                {regenerateProgress && (
                  <div className="mt-3">
                    <div className="flex items-center justify-between mb-1">
                      <span className="text-xs text-gray-400">
                        {regenerateProgress.status} {regenerateProgress.current}/{regenerateProgress.total}
                      </span>
                      <span className="text-xs text-cyan-400">
                        {regenerateProgress.total > 0
                          ? `${Math.round((regenerateProgress.current / regenerateProgress.total) * 100)}%`
                          : '0%'}
                      </span>
                    </div>
                    <div className="h-1.5 bg-gray-700 rounded-full overflow-hidden">
                      <div
                        className="h-full bg-cyan-500 transition-all duration-300"
                        style={{
                          width: regenerateProgress.total > 0
                            ? `${(regenerateProgress.current / regenerateProgress.total) * 100}%`
                            : '0%',
                        }}
                      />
                    </div>
                  </div>
                )}

                <p className="mt-3 text-xs text-gray-500">
                  {!useLocalEmbeddings && (
                    <span className="text-amber-400">OpenAI embeddings are higher-dimensional but not tuned for our clustering. </span>
                  )}
                  {!useLocalEmbeddings && !openaiKeyStatus && (
                    <span className="text-red-400">OpenAI key required.</span>
                  )}
                  {useLocalEmbeddings && (
                    <span>Local embeddings are optimized for semantic clustering and work offline.</span>
                  )}
                </p>
              </div>
            </section>
            )}

            {/* MAINTENANCE TAB - Operations, Reset Flags, Danger Zone */}
            {activeTab === 'maintenance' && (
            <>
            <section>
              <div className="flex items-center gap-2 mb-4">
                <RefreshCw className="w-5 h-5 text-green-400" />
                <h3 className="text-lg font-medium text-white">Operations</h3>
              </div>

              <div className="space-y-2">
                {/* Full Rebuild */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-green-600 flex items-center justify-center text-white text-sm">
                    <RefreshCw className="w-3.5 h-3.5" />
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Full Rebuild</div>
                    <div className="text-xs text-gray-500">Embedding clustering (free) + AI hierarchy. Replaces all organization.</div>
                    {dbStats && (dbStats.unclusteredItems > 0 || dbStats.topicsCount > 0) && (
                      <div className="text-xs text-amber-400/80 mt-0.5">
                        {dbStats.topicsCount > 15 && (
                          <>~{Math.ceil(Math.log(dbStats.topicsCount / 10) / Math.log(10) * 3)} hierarchy calls · Sonnet {estimateCost(Math.ceil(Math.log(dbStats.topicsCount / 10) / Math.log(10) * 3), 'sonnet', 5000)}</>
                        )}
                        {dbStats.topicsCount <= 15 && (
                          <span className="text-cyan-400">Clustering: free (local embeddings)</span>
                        )}
                        <span className="text-gray-500 ml-1">({dbStats.totalItems} items)</span>
                      </div>
                    )}
                    {dbStats && dbStats.unclusteredItems === 0 && dbStats.topicsCount <= 15 && (
                      <div className="text-xs text-green-500/70 mt-0.5">All items organized</div>
                    )}
                  </div>
                  <button
                    onClick={() => setConfirmAction('fullRebuild')}
                    disabled={isFullRebuilding || isFlattening || isConsolidating || isTidying}
                    className="flex-shrink-0 w-12 h-12 flex items-center justify-center text-xl bg-green-600 hover:bg-green-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isFullRebuilding ? <Loader2 className="w-5 h-5 animate-spin" /> : '🔄'}
                  </button>
                </div>

                {/* Flatten Hierarchy */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-amber-600 flex items-center justify-center text-white text-sm">
                    <Zap className="w-3.5 h-3.5" />
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Flatten Empty Levels</div>
                    <div className="text-xs text-gray-500">Remove passthrough "Uncategorized" nodes</div>
                    <div className="text-xs text-gray-600 mt-0.5">No API calls · Fast local operation</div>
                  </div>
                  <button
                    onClick={() => setConfirmAction('flattenHierarchy')}
                    disabled={isFlattening || isFullRebuilding || isConsolidating || isTidying}
                    className="flex-shrink-0 w-12 h-12 flex items-center justify-center text-xl bg-amber-600 hover:bg-amber-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isFlattening ? <Loader2 className="w-5 h-5 animate-spin" /> : '⚡'}
                  </button>
                </div>

                {/* Consolidate Root */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-purple-600 flex items-center justify-center text-white text-sm">
                    <Zap className="w-3.5 h-3.5" />
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Consolidate Root</div>
                    <div className="text-xs text-gray-500">Group into 4-8 uber-categories (TECH, LIFE, etc.)</div>
                    <div className="text-xs text-amber-400/80 mt-0.5">
                      1 call · Sonnet {estimateCost(1, 'sonnet', 5000)}
                    </div>
                  </div>
                  <button
                    onClick={() => setConfirmAction('consolidateRoot')}
                    disabled={isConsolidating || isFullRebuilding || isFlattening || isTidying}
                    className="flex-shrink-0 w-12 h-12 flex items-center justify-center text-xl bg-purple-600 hover:bg-purple-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isConsolidating ? <Loader2 className="w-5 h-5 animate-spin" /> : '🏛️'}
                  </button>
                </div>

                {/* Tidy Database */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-cyan-600 flex items-center justify-center text-white text-sm">
                    <Zap className="w-3.5 h-3.5" />
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Tidy Database</div>
                    <div className="text-xs text-gray-500">Fix counts/depths, remove empties, flatten chains, prune edges</div>
                    <div className="text-xs text-gray-600 mt-0.5">No API calls · Fast local operation</div>
                  </div>
                  <button
                    onClick={() => setConfirmAction('tidyDatabase')}
                    disabled={isTidying || isFullRebuilding || isFlattening || isConsolidating}
                    className="flex-shrink-0 w-12 h-12 flex items-center justify-center text-xl bg-cyan-600 hover:bg-cyan-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isTidying ? <Loader2 className="w-5 h-5 animate-spin" /> : '🧹'}
                  </button>
                </div>

                {operationResult && (
                  <p className="text-xs text-green-400">{operationResult}</p>
                )}
              </div>
            </section>

            {/* Danger Zone Section */}
            <section>
              <div className="flex items-center gap-2 mb-4">
                <Trash2 className="w-5 h-5 text-red-400" />
                <h3 className="text-lg font-medium text-white">Danger Zone</h3>
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

                <button
                  onClick={() => setConfirmAction('resetPrivacy')}
                  className="flex flex-col items-center gap-2 p-4 bg-gray-900/50 hover:bg-gray-900 rounded-lg transition-colors text-center"
                >
                  <span className="text-sm font-medium text-gray-200">Reset Privacy Flags</span>
                  <span className="text-xs text-gray-500">Re-scan with new settings</span>
                </button>
              </div>

              {actionResult && (
                <p className={`mt-3 text-sm text-center ${actionResult.startsWith('Error') ? 'text-red-400' : 'text-green-400'}`}>
                  {actionResult}
                </p>
              )}

              {/* Delete All Data - at the very bottom */}
              <div className="mt-6 pt-4 border-t border-gray-700/50">
                <button
                  onClick={() => setConfirmAction('deleteAll')}
                  className="w-full flex items-center justify-center gap-2 px-4 py-2.5 bg-red-600/20 hover:bg-red-600 text-red-400 hover:text-white rounded-lg font-medium transition-colors border border-red-600/30 hover:border-red-600"
                >
                  <Trash2 className="w-4 h-4" />
                  Delete All Data
                </button>
                <p className="mt-2 text-xs text-gray-500 text-center">
                  Permanently delete all nodes and edges. Cannot be undone.
                </p>
              </div>
            </section>
            </>
            )}

            {/* PRIVACY TAB */}
            {activeTab === 'privacy' && (
            <section>
              <div className="flex items-center gap-2 mb-4">
                <Shield className="w-5 h-5 text-rose-400" />
                <h3 className="text-lg font-medium text-white">Privacy Filter</h3>
              </div>

              {/* Description */}
              <div className="bg-gray-900/50 rounded-lg p-4 mb-4 text-sm text-gray-400 space-y-2">
                <p>
                  Uses AI to scan your items for sensitive content (personal info, credentials, private conversations, etc.) and marks them as <span className="text-rose-400">private</span> or <span className="text-green-400">safe</span>.
                </p>
                <p className="text-xs text-gray-500">
                  <strong className="text-gray-400">How it works:</strong> Categories automatically inherit privacy status — a category is hidden only when <em>all</em> its children are private. Use the 🔒 lock button in the top-right menu to toggle private content visibility. Export creates a shareable database with private items removed.
                </p>
                <p className="text-xs text-gray-500">
                  <strong className="text-gray-400">Adjusting sensitivity:</strong> Edit the <code className="text-gray-400 bg-gray-800 px-1 rounded">PRIVACY_PROMPT</code> in <code className="text-gray-400 bg-gray-800 px-1 rounded">src-tauri/src/commands/privacy.rs</code> (lines 15-45) to customize what's considered private. Add/remove categories, change "when uncertain" behavior, or adjust examples.
                </p>
              </div>

              <div className="space-y-3">
                {/* Privacy Stats */}
                {privacyStats && (
                  <div className="bg-gray-900/50 rounded-lg p-4 text-sm">
                    <div className="grid grid-cols-2 gap-4">
                      <div>
                        <span className="text-gray-400">Total Items:</span>
                        <span className="ml-2 text-white font-medium">{privacyStats.total}</span>
                      </div>
                      <div>
                        <span className="text-gray-400">Scanned:</span>
                        <span className="ml-2 text-white font-medium">{privacyStats.scanned}</span>
                      </div>
                      <div>
                        <span className="text-gray-400">🔒 Private:</span>
                        <span className="ml-2 text-rose-400 font-medium">{privacyStats.private}</span>
                      </div>
                      <div>
                        <span className="text-gray-400">✓ Safe:</span>
                        <span className="ml-2 text-green-400 font-medium">{privacyStats.safe}</span>
                      </div>
                    </div>
                    {privacyStats.unscanned > 0 && (
                      <div className="mt-2 text-xs text-amber-400">
                        {privacyStats.unscanned} items not yet scanned
                      </div>
                    )}
                  </div>
                )}

                {/* Scan Progress */}
                {scanProgress && (
                  <div className="bg-gray-900/50 rounded-lg p-4">
                    <div className="flex items-center justify-between mb-2">
                      <span className="text-sm text-gray-300">
                        Scanning... {scanProgress.current}/{scanProgress.total}
                      </span>
                      <span className="text-xs text-amber-400">
                        {scanProgress.total > 0 ? ((scanProgress.current / scanProgress.total) * 100).toFixed(0) : 0}%
                      </span>
                    </div>
                    <div className="h-2 bg-gray-700 rounded-full overflow-hidden">
                      <div
                        className="h-full bg-rose-500 transition-all duration-300"
                        style={{ width: scanProgress.total > 0 ? `${(scanProgress.current / scanProgress.total) * 100}%` : '0%' }}
                      />
                    </div>
                  </div>
                )}

                {/* Showcase Mode Toggle */}
                <div className="bg-gray-900/50 rounded-lg p-4">
                  <div className="flex items-center justify-between">
                    <div>
                      <div className="text-sm font-medium text-gray-200">Showcase Mode</div>
                      <div className="text-xs text-gray-500 mt-1">
                        Stricter filtering for demo databases — keeps only Mycelica, philosophy, and pure tech content
                      </div>
                    </div>
                    <button
                      onClick={() => setShowcaseMode(!showcaseMode)}
                      className={`relative w-12 h-6 rounded-full transition-colors ${
                        showcaseMode ? 'bg-amber-500' : 'bg-gray-600'
                      }`}
                    >
                      <div
                        className={`absolute top-1 w-4 h-4 bg-white rounded-full transition-all ${
                          showcaseMode ? 'left-7' : 'left-1'
                        }`}
                      />
                    </button>
                  </div>
                </div>

                {/* Scan Buttons */}
                <div className="bg-gray-900/50 rounded-lg p-4 space-y-3">
                  <div className="flex gap-2">
                    <button
                      onClick={handleCategoryScan}
                      disabled={isScanning || !apiKeyStatus?.hasKey}
                      className="flex-1 flex items-center justify-center gap-2 px-4 py-2.5 bg-amber-600 hover:bg-amber-700 text-white rounded-lg font-medium transition-colors disabled:opacity-50"
                    >
                      {isScanning ? (
                        <Loader2 className="w-4 h-4 animate-spin" />
                      ) : (
                        <Zap className="w-4 h-4" />
                      )}
                      Scan Categories
                    </button>
                    <button
                      onClick={handlePrivacyScan}
                      disabled={isScanning || !apiKeyStatus?.hasKey || privacyStats?.unscanned === 0}
                      className="flex-1 flex items-center justify-center gap-2 px-4 py-2.5 bg-rose-600 hover:bg-rose-700 text-white rounded-lg font-medium transition-colors disabled:opacity-50"
                    >
                      {isScanning ? (
                        <Loader2 className="w-4 h-4 animate-spin" />
                      ) : (
                        <Shield className="w-4 h-4" />
                      )}
                      Scan Items
                    </button>
                    {isScanning && (
                      <button
                        onClick={handleCancelPrivacyScan}
                        className="px-4 py-2.5 bg-red-600 hover:bg-red-700 text-white rounded-lg font-medium transition-colors"
                      >
                        Cancel
                      </button>
                    )}
                  </div>
                  <p className="text-xs text-gray-500">
                    <strong className="text-amber-400">Categories</strong> = fast, filters by topic name (~{privacyStats?.totalCategories || 0} calls).
                    <strong className="text-rose-400 ml-2">Items</strong> = thorough, scans content ({privacyStats?.unscanned || 0} remaining).
                  </p>
                </div>

                {/* Export Button */}
                <div className="bg-gray-900/50 rounded-lg p-4">
                  <button
                    onClick={handleExportShareable}
                    disabled={isScanning || privacyStats?.scanned === 0}
                    className="w-full flex items-center justify-center gap-2 px-4 py-2.5 bg-emerald-600 hover:bg-emerald-700 text-white rounded-lg font-medium transition-colors disabled:opacity-50"
                  >
                    <Download className="w-4 h-4" />
                    Export Shareable DB
                  </button>
                  <p className="mt-2 text-xs text-gray-500">
                    Creates a copy with private nodes removed
                  </p>
                </div>

                {privacyResult && (
                  <p className={`text-xs ${privacyResult.startsWith('Error') ? 'text-red-400' : 'text-green-400'}`}>
                    {privacyResult}
                  </p>
                )}
              </div>
            </section>
            )}
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
