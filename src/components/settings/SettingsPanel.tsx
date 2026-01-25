import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
import { readTextFile } from '@tauri-apps/plugin-fs';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
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
  ExternalLink,
  FilePlus,
  HardDrive,
  Clock,
  Zap,
  Shield,
  Download,
  Cpu,
  ChevronDown,
  ChevronRight,
  Plus,
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

type ConfirmAction = 'deleteAll' | 'resetAi' | 'resetClustering' | 'clearEmbeddings' | 'regenerateEdges' | 'clearHierarchy' | 'clearTags' | 'resetPrivacy' | 'fullRebuild' | 'flattenHierarchy' | 'consolidateRoot' | 'tidyDatabase' | 'scorePrivacy' | 'reclassifyPattern' | 'reclassifyAi' | 'rebuildLite' | 'nameClusters' | 'resetProcessing' | 'clearStructure' | null;

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
  edgesIndexed: number;
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

interface ExportPreview {
  included: number;
  excluded: number;
  unscored: number;
}

type SettingsTab = 'setup' | 'keys' | 'maintenance' | 'privacy' | 'info';

export function SettingsPanel({ open, onClose, onDataChanged }: SettingsPanelProps) {
  // Navigation reset for database switches + shared privacy threshold
  const { navigateToRoot, privacyThreshold, setPrivacyThreshold } = useGraphStore();

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

  const [openaireKeyStatus, setOpenaireKeyStatus] = useState<string | null>(null);
  const [openaireKeyInput, setOpenaireKeyInput] = useState('');
  const [savingOpenaireKey, setSavingOpenaireKey] = useState(false);
  const [openaireKeyError, setOpenaireKeyError] = useState<string | null>(null);

  // LLM Backend (anthropic or ollama)
  const [llmBackend, setLlmBackend] = useState<'anthropic' | 'ollama'>('anthropic');
  const [ollamaModel, setOllamaModel] = useState('qwen2.5:7b');
  const [ollamaStatus, setOllamaStatus] = useState<'unknown' | 'running' | 'stopped'>('unknown');

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

  // OpenAIRE import
  const [showOpenAireDialog, setShowOpenAireDialog] = useState(false);
  const [openAireQueryTags, setOpenAireQueryTags] = useState<string[]>([]);
  const [openAireQueryInput, setOpenAireQueryInput] = useState('');
  const [openAireMatchAll, setOpenAireMatchAll] = useState(false); // false = OR, true = AND
  const [openAireFromYear, setOpenAireFromYear] = useState<string>('');
  const [openAireToYear, setOpenAireToYear] = useState<string>('');
  const [openAireCountry, setOpenAireCountry] = useState<string>('EE');
  const [openAireFos, setOpenAireFos] = useState<string>('');
  const [openAireMaxPapers, setOpenAireMaxPapers] = useState(100);
  const [openAirePaperCount, setOpenAirePaperCount] = useState<number | null>(null);
  const [openAirePrevCount, setOpenAirePrevCount] = useState<number | null>(null);
  const [openAireCountLoading, setOpenAireCountLoading] = useState(false);
  const [openAireShowMore, setOpenAireShowMore] = useState(false);
  const [openAireDownloadPdfs, setOpenAireDownloadPdfs] = useState(true);
  const [openAireMaxPdfSize, setOpenAireMaxPdfSize] = useState(20);
  const openAireCountAbortRef = useRef<AbortController | null>(null);

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
  const [isScoringPrivacy, setIsScoringPrivacy] = useState(false);
  const [isExportingTrimmed, setIsExportingTrimmed] = useState(false);
  const [isReclassifyingPattern, setIsReclassifyingPattern] = useState(false);
  const [isReclassifyingAi, setIsReclassifyingAi] = useState(false);
  const [isRebuildingLite, setIsRebuildingLite] = useState(false);
  const [isNamingClusters, setIsNamingClusters] = useState(false);
  const [operationResult, setOperationResult] = useState<string | null>(null);

  // Setup flow operations
  const [isProcessingAi, setIsProcessingAi] = useState(false);
  const [isClustering, setIsClustering] = useState(false);
  const [isBuildingHierarchy, setIsBuildingHierarchy] = useState(false);
  const [isUpdatingDates, setIsUpdatingDates] = useState(false);
  const [isQuickAdding, setIsQuickAdding] = useState(false);
  const [inboxCount, setInboxCount] = useState(0);
  const [setupResult, setSetupResult] = useState<string | null>(null);
  const [isFullSetup, setIsFullSetup] = useState(false);
  const [fullSetupStep, setFullSetupStep] = useState('');

  // Privacy scanning (privacyThreshold is shared via store with Graph.tsx)
  const [privacyStats, setPrivacyStats] = useState<PrivacyStats | null>(null);
  const [privacyResult, setPrivacyResult] = useState<string | null>(null);
  const [exportPreview, setExportPreview] = useState<ExportPreview | null>(null);

  // Protection settings
  const [protectRecentNotes, setProtectRecentNotes] = useState(true);

  // Tips setting from store (shared with Graph)
  const { showTips, setShowTips, showHidden, setShowHidden, edgeWeightThreshold, setEdgeWeightThreshold } = useGraphStore();

  // Local embeddings
  const [useLocalEmbeddings, setUseLocalEmbeddings] = useState(false);
  const [isRegeneratingEmbeddings, setIsRegeneratingEmbeddings] = useState(false);
  const [regenerateProgress, setRegenerateProgress] = useState<{ current: number; total: number; status: string } | null>(null);

  // Clustering thresholds
  const [clusteringPrimary, setClusteringPrimary] = useState<string>('');
  const [clusteringSecondary, setClusteringSecondary] = useState<string>('');
  const [savingThresholds, setSavingThresholds] = useState(false);
  const [thresholdResult, setThresholdResult] = useState<string | null>(null);

  // Privacy threshold (for Personal category separation)
  const [privacyThresholdInput, setPrivacyThresholdInput] = useState<string>('0.5');
  const [savingPrivacyThreshold, setSavingPrivacyThreshold] = useState(false);
  const [privacyThresholdResult, setPrivacyThresholdResult] = useState<string | null>(null);

  // UI collapse states
  const [showAdvancedSetup, setShowAdvancedSetup] = useState(false);
  const [dangerZoneExpanded, setDangerZoneExpanded] = useState(false);

  // Smart operation suggestion after import
  const [suggestedOperation, setSuggestedOperation] = useState<'smartAdd' | 'fullSetup' | null>(null);
  const [suggestionReason, setSuggestionReason] = useState('');

  // Load data on mount
  useEffect(() => {
    if (open) {
      loadApiKeyStatus();
      loadOpenaiKeyStatus();
      loadOpenaireKeyStatus();
      loadLlmBackend();
      loadDbStats();
      loadDbPath();
      loadProcessingStats();
      loadPrivacyStats();
      loadProtectionSettings();
      loadShowTips();
      loadLocalEmbeddingsStatus();
      loadClusteringThresholds();
      loadPrivacyThresholdSetting();
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

  const loadOpenaireKeyStatus = async () => {
    try {
      const maskedKey = await invoke<string | null>('get_openaire_api_key_status');
      setOpenaireKeyStatus(maskedKey);
    } catch (err) {
      console.error('Failed to load OpenAIRE API key status:', err);
    }
  };

  const loadLlmBackend = async () => {
    try {
      const backend = await invoke<string>('get_llm_backend');
      setLlmBackend(backend as 'anthropic' | 'ollama');
      const model = await invoke<string>('get_ollama_model');
      setOllamaModel(model);
      const status = await invoke<{ running: boolean }>('check_ollama_status');
      setOllamaStatus(status.running ? 'running' : 'stopped');
    } catch (err) {
      console.error('Failed to load LLM backend:', err);
    }
  };

  const handleSetLlmBackend = async (backend: 'anthropic' | 'ollama') => {
    try {
      await invoke('set_llm_backend', { backend });
      setLlmBackend(backend);
    } catch (err) {
      console.error('Failed to set LLM backend:', err);
    }
  };

  const handleSetOllamaModel = async (model: string) => {
    try {
      await invoke('set_ollama_model', { model });
      setOllamaModel(model);
    } catch (err) {
      console.error('Failed to set Ollama model:', err);
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

  const loadExportPreview = async (threshold: number) => {
    try {
      const preview = await invoke<ExportPreview>('get_export_preview', { minPrivacy: threshold });
      setExportPreview(preview);
    } catch (err) {
      console.error('Failed to load export preview:', err);
    }
  };

  // Update export preview when threshold changes
  useEffect(() => {
    if (open && activeTab === 'privacy') {
      loadExportPreview(privacyThreshold);
    }
  }, [open, activeTab, privacyThreshold]);

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

  const loadShowTips = async () => {
    try {
      const enabled = await invoke<boolean>('get_show_tips');
      setShowTips(enabled);
    } catch (err) {
      console.error('Failed to load tips setting:', err);
    }
  };

  const handleToggleShowTips = async (enabled: boolean) => {
    try {
      await invoke('set_show_tips', { enabled });
      setShowTips(enabled); // Updates store, which updates Graph
    } catch (err) {
      console.error('Failed to toggle tips:', err);
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

  const loadClusteringThresholds = async () => {
    try {
      const [primary, secondary] = await invoke<[number | null, number | null]>('get_clustering_thresholds');
      setClusteringPrimary(primary !== null ? primary.toString() : '');
      setClusteringSecondary(secondary !== null ? secondary.toString() : '');
    } catch (err) {
      console.error('Failed to load clustering thresholds:', err);
    }
  };

  const handleSaveClusteringThresholds = async () => {
    setSavingThresholds(true);
    setThresholdResult(null);
    try {
      const primary = clusteringPrimary ? parseFloat(clusteringPrimary) : null;
      const secondary = clusteringSecondary ? parseFloat(clusteringSecondary) : null;

      // Validate ranges
      if (primary !== null && (primary < 0.3 || primary > 0.95)) {
        setThresholdResult('Primary threshold must be between 0.3 and 0.95');
        setSavingThresholds(false);
        return;
      }
      if (secondary !== null && (secondary < 0.2 || secondary > 0.85)) {
        setThresholdResult('Secondary threshold must be between 0.2 and 0.85');
        setSavingThresholds(false);
        return;
      }

      await invoke('set_clustering_thresholds', { primary, secondary });
      setThresholdResult(primary && secondary
        ? `Saved: ${primary}/${secondary}. Run Full Rebuild to apply.`
        : 'Using adaptive thresholds. Run Full Rebuild to apply.');
    } catch (err) {
      setThresholdResult(`Error: ${err}`);
    }
    setSavingThresholds(false);
  };

  const loadPrivacyThresholdSetting = async () => {
    try {
      const threshold = await invoke<number>('get_privacy_threshold');
      setPrivacyThresholdInput(threshold.toString());
    } catch (err) {
      console.error('Failed to load privacy threshold:', err);
    }
  };

  const handleSavePrivacyThreshold = async () => {
    setSavingPrivacyThreshold(true);
    setPrivacyThresholdResult(null);
    try {
      const threshold = parseFloat(privacyThresholdInput);

      // Validate range
      if (isNaN(threshold) || threshold < 0.0 || threshold > 1.0) {
        setPrivacyThresholdResult('Privacy threshold must be between 0.0 and 1.0');
        setSavingPrivacyThreshold(false);
        return;
      }

      await invoke('set_privacy_threshold', { threshold });
      setPrivacyThresholdResult(`Saved: ${threshold}. Run Full Rebuild to apply.`);
    } catch (err) {
      setPrivacyThresholdResult(`Error: ${err}`);
    }
    setSavingPrivacyThreshold(false);
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
  // Pricing: Haiku 4.5 $0.80/$4.00 MTok, Sonnet 4 $3/$15 MTok, OpenAI embed $0.02/MTok
  // Token usage (measured from actual runs):
  //   - Haiku classification: ~105 tokens/item (reclassify_ai, score_privacy)
  //   - Haiku generation: ~1200 tokens/item (process_ai titles/summaries)
  //   - Sonnet grouping: ~3000 tokens/call (hierarchy)
  // Grouping calls formula: Math.max(3, Math.ceil(topicsCount / 100) + 2)
  type HaikuOp = 'classify' | 'generate';
  const estimateCost = (calls: number, model: 'haiku' | 'sonnet' | 'openai', avgTokensPerCall: number = 1200, haikuOp: HaikuOp = 'generate'): string => {
    if (calls === 0) return '$0.00';
    const totalTokens = calls * avgTokensPerCall;
    let cost = 0;
    if (model === 'haiku') {
      // classify: 95% input / 5% output (single word/number)
      // generate: 83% input / 17% output (titles/summaries)
      const r = haikuOp === 'classify'
        ? { input: 0.95, output: 0.05 }
        : { input: 0.83, output: 0.17 };
      cost = (totalTokens * r.input / 1_000_000) * 0.80 + (totalTokens * r.output / 1_000_000) * 4.00;
    } else if (model === 'sonnet') {
      // Grouping: ~55% input, ~45% output
      cost = (totalTokens * 0.55 / 1_000_000) * 3 + (totalTokens * 0.45 / 1_000_000) * 15;
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
      // Use current database's directory as default location
      const defaultDir = dbPath ? dbPath.substring(0, dbPath.lastIndexOf('/') + 1) : undefined;
      const file = await openDialog({
        title: 'Select Mycelica database',
        defaultPath: defaultDir,
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
        // Reload to ensure clean state and update window title
        window.location.reload();
      }
    } catch (err) {
      setActionResult(`Error: ${err}`);
    } finally {
      setSwitchingDb(false);
    }
  };

  // Create new database
  const handleCreateDatabase = async () => {
    setSwitchingDb(true);
    try {
      // Use current database's directory as default location
      const defaultDir = dbPath ? dbPath.substring(0, dbPath.lastIndexOf('/') + 1) : undefined;
      const file = await saveDialog({
        title: 'Create new Mycelica database',
        defaultPath: defaultDir ? `${defaultDir}mycelica-new.db` : 'mycelica-new.db',
        filters: [{ name: 'SQLite Database', extensions: ['db'] }],
      });
      if (file && typeof file === 'string') {
        console.log('Creating new database at:', file);
        const stats = await invoke<DbStats>('switch_database', { dbPath: file });
        console.log('Switched to new database, stats:', stats);
        // Update local state with new db stats
        setDbStats(stats);
        setDbPath(file);
        // Reset navigation to universe root
        navigateToRoot();
        // Trigger graph data refresh
        onDataChanged?.();
        // Reload to ensure clean state with new database
        window.location.reload();
      }
    } catch (err) {
      console.error('Failed to create database:', err);
      setActionResult(`Error: ${err}`);
    } finally {
      setSwitchingDb(false);
    }
  };

  // Export trimmed database (no PDF blobs)
  const handleExportTrimmed = async () => {
    setIsExportingTrimmed(true);
    setOperationResult(null);
    try {
      const defaultDir = dbPath ? dbPath.substring(0, dbPath.lastIndexOf('/') + 1) : undefined;
      const defaultName = dbPath
        ? dbPath.substring(dbPath.lastIndexOf('/') + 1).replace('.db', '-trimmed.db')
        : 'mycelica-trimmed.db';
      const file = await saveDialog({
        title: 'Export trimmed database (no PDFs)',
        defaultPath: defaultDir ? `${defaultDir}${defaultName}` : defaultName,
        filters: [{ name: 'SQLite Database', extensions: ['db'] }],
      });
      if (file && typeof file === 'string') {
        const result = await invoke<string>('export_trimmed_database', { outputPath: file });
        setOperationResult(result);
      }
    } catch (err) {
      console.error('Failed to export trimmed database:', err);
      setOperationResult(`Error: ${err}`);
    } finally {
      setIsExportingTrimmed(false);
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

  const handleSaveOpenaireKey = async () => {
    if (!openaireKeyInput.trim()) return;
    setSavingOpenaireKey(true);
    setOpenaireKeyError(null);
    try {
      await invoke('save_openaire_api_key', { key: openaireKeyInput.trim() });
      await loadOpenaireKeyStatus();
      setOpenaireKeyInput('');
    } catch (err) {
      setOpenaireKeyError(err as string);
    } finally {
      setSavingOpenaireKey(false);
    }
  };

  const handleClearOpenaireKey = async () => {
    try {
      await invoke('clear_openaire_api_key');
      await loadOpenaireKeyStatus();
    } catch (err) {
      console.error('Failed to clear OpenAIRE API key:', err);
    }
  };

  // Smart operation suggestion after import
  const suggestOperationAfterImport = (newItems: number) => {
    const existingTopics = dbStats?.topicsCount || 0;

    if (existingTopics === 0) {
      // Fresh database - must do Full Setup
      setSuggestedOperation('fullSetup');
      setSuggestionReason(`${newItems} items imported. Run Full Setup to create initial structure.`);
    } else if (newItems <= 30) {
      // Small batch - Smart Add likely sufficient
      setSuggestedOperation('smartAdd');
      setSuggestionReason(`${newItems} items imported. Smart Add should place most in existing topics (~${Math.max(5, Math.ceil(newItems * 0.5))}s).`);
    } else {
      // Large batch - Full Setup recommended
      setSuggestedOperation('fullSetup');
      setSuggestionReason(`${newItems} items imported. Full Setup recommended for best organization (~$0.15, 5-7min).`);
    }
  };

  // Import handler
  const handleImport = async () => {
    setImporting(true);
    setImportResult(null);
    setSuggestedOperation(null);
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

        // Show smart operation suggestion if items were imported
        if (result.conversationsImported > 0) {
          setImportResult(`Imported ${result.conversationsImported} conversations`);
          suggestOperationAfterImport(result.conversationsImported);
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

  // ChatGPT import handler
  const handleImportChatGPT = async () => {
    setImporting(true);
    setImportResult(null);
    setSuggestedOperation(null);
    try {
      const file = await openDialog({
        title: 'Select ChatGPT conversations.json',
        filters: [{ name: 'JSON', extensions: ['json'] }],
      });
      if (file && typeof file === 'string') {
        const content = await readTextFile(file);
        const result = await invoke<{ conversationsImported: number; exchangesImported: number }>(
          'import_chatgpt_conversations',
          { jsonContent: content }
        );
        await loadDbStats();
        onDataChanged?.();

        // Show smart operation suggestion if items were imported
        if (result.conversationsImported > 0) {
          setImportResult(`Imported ${result.conversationsImported} ChatGPT conversations`);
          suggestOperationAfterImport(result.conversationsImported);
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
    setSuggestedOperation(null);
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

          // Show smart operation suggestion if items were imported
          if (result.exchangesImported > 0) {
            suggestOperationAfterImport(result.exchangesImported);
          }
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
    setSuggestedOperation(null);
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

        // Show smart operation suggestion if notes were imported
        if (result.notesImported > 0) {
          suggestOperationAfterImport(result.notesImported);
        }
      }
    } catch (err) {
      setImportResult(`Error: ${err}`);
    } finally {
      setImporting(false);
    }
  };

  // Code folder import handler
  const handleImportCode = async () => {
    setImporting(true);
    setImportResult(null);
    setSuggestedOperation(null);
    try {
      const folder = await openDialog({
        title: 'Select code folder to import',
        directory: true,
        multiple: false,
      });
      if (folder && typeof folder === 'string') {
        const result = await invoke<{
          functions: number;
          structs: number;
          enums: number;
          traits: number;
          impls: number;
          modules: number;
          macros: number;
          docs: number;
          files_processed: number;
          files_skipped: number;
          edges_created: number;
          doc_edges: number;
          errors: string[];
        }>('import_code', { path: folder, language: null });
        await loadDbStats();
        onDataChanged?.();

        const totalItems = result.functions + result.structs + result.enums + result.traits + result.impls + result.modules + result.macros + result.docs;
        const errorMsg = result.errors.length > 0 ? ` (${result.errors.length} errors)` : '';
        setImportResult(`Imported ${totalItems} code items from ${result.files_processed} files${errorMsg}`);

        // Show smart operation suggestion if items were imported
        if (totalItems > 0) {
          suggestOperationAfterImport(totalItems);
        }
      }
    } catch (err) {
      setImportResult(`Error: ${err}`);
    } finally {
      setImporting(false);
    }
  };

  // OpenAIRE tag handlers
  const handleAddOpenAireTag = () => {
    const trimmed = openAireQueryInput.trim();
    if (trimmed && !openAireQueryTags.includes(trimmed)) {
      setOpenAireQueryTags([...openAireQueryTags, trimmed]);
      setOpenAireQueryInput('');
    }
  };

  const handleRemoveOpenAireTag = (tagToRemove: string) => {
    setOpenAireQueryTags(openAireQueryTags.filter(t => t !== tagToRemove));
  };

  // Build query string based on match mode
  const buildOpenAireQueryString = () => {
    // No quotes - OpenAIRE treats quoted phrases as exact match only
    // Parentheses group the OR expression
    if (openAireMatchAll) {
      return openAireQueryTags.join(' AND ');
    }
    if (openAireQueryTags.length === 1) {
      return openAireQueryTags[0];
    }
    return `(${openAireQueryTags.join(') OR (')})`;
  };

  // OpenAIRE paper count preview (debounced)
  useEffect(() => {
    // Cancel any pending count
    if (openAireCountAbortRef.current) {
      openAireCountAbortRef.current.abort();
      openAireCountAbortRef.current = null;
    }

    if (!showOpenAireDialog || openAireQueryTags.length === 0) {
      // Cancel backend operation too
      invoke('cancel_openaire').catch(() => {});
      setOpenAirePaperCount(0);
      setOpenAirePrevCount(null);
      setOpenAireCountLoading(false);
      return;
    }

    const abortController = new AbortController();
    openAireCountAbortRef.current = abortController;

    const timer = setTimeout(async () => {
      if (abortController.signal.aborted) return;

      setOpenAireCountLoading(true);
      try {
        if (abortController.signal.aborted) return;
        const queryString = buildOpenAireQueryString();
        const [total] = await invoke<[number, number]>('count_openaire_papers', {
          query: queryString,
          country: openAireCountry || null,
          fos: openAireFos || null,
          fromYear: openAireFromYear || null,
          toYear: openAireToYear || null,
        });
        if (abortController.signal.aborted) return;
        setOpenAirePrevCount(openAirePaperCount);
        setOpenAirePaperCount(total);
      } catch (err) {
        if (abortController.signal.aborted) return;
        console.error('Failed to count papers:', err);
        setOpenAirePaperCount(null);
      } finally {
        if (!abortController.signal.aborted) {
          setOpenAireCountLoading(false);
        }
      }
    }, 500);  // 500ms debounce

    return () => {
      clearTimeout(timer);
      abortController.abort();
    };
  }, [showOpenAireDialog, openAireQueryTags, openAireMatchAll, openAireCountry, openAireFos, openAireFromYear, openAireToYear]);

  // OpenAIRE import handler
  const handleImportOpenAire = async () => {
    if (openAireQueryTags.length === 0) {
      setImportResult('Please add at least one search term');
      return;
    }
    setImporting(true);
    setImportResult(null);
    setSuggestedOperation(null);
    setShowOpenAireDialog(false);
    try {
      const result = await invoke<{
        papersImported: number;
        pdfsDownloaded: number;
        pdfsSkipped: number;
        duplicatesSkipped: number;
        errors: string[];
      }>('import_openaire', {
        query: buildOpenAireQueryString(),
        country: openAireCountry || null,
        fos: openAireFos || null,
        fromYear: openAireFromYear || null,
        toYear: openAireToYear || null,
        maxPapers: openAireMaxPapers,
        downloadPdfs: openAireDownloadPdfs,
        maxPdfSizeMb: openAireMaxPdfSize,
      });
      await loadDbStats();
      onDataChanged?.();

      const parts = [`Imported ${result.papersImported} paper${result.papersImported !== 1 ? 's' : ''}`];
      if (result.pdfsDownloaded > 0) {
        parts.push(`${result.pdfsDownloaded} PDF${result.pdfsDownloaded !== 1 ? 's' : ''} downloaded`);
      }
      if (result.duplicatesSkipped > 0) {
        parts.push(`${result.duplicatesSkipped} duplicate${result.duplicatesSkipped !== 1 ? 's' : ''} skipped`);
      }
      if (result.errors.length > 0) {
        parts.push(`${result.errors.length} error${result.errors.length !== 1 ? 's' : ''}`);
      }
      setImportResult(parts.join(', '));

      if (result.papersImported > 0) {
        suggestOperationAfterImport(result.papersImported);
      }
    } catch (err) {
      setImportResult(`Error: ${err}`);
    } finally {
      setImporting(false);
    }
  };

  // Privacy export handler
  const handleExportShareable = async () => {
    setPrivacyResult(null);
    try {
      const path = await invoke<string>('export_shareable_db', { minPrivacy: privacyThreshold });
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

  const handleFullSetup = async () => {
    setIsFullSetup(true);
    setSetupResult(null);
    try {
      // Step 0: Pre-classify (pattern matching, FREE)
      setFullSetupStep('Classifying items (pattern matching)...');
      const classifyResult = await invoke<{ classified: number; hiddenCount: number; visibleCount: number }>('preclassify_items');

      // Step 1: Process AI (skips hidden items)
      setFullSetupStep(`Processing AI (${classifyResult.hiddenCount} hidden items will skip)...`);
      const aiResult = await invoke<{ processed: number; skipped: number }>('process_nodes');

      // Step 2: Cluster
      setFullSetupStep('Clustering items by similarity...');
      const clusterResult = await invoke<{ clusters_created: number; nodes_clustered: number }>('run_clustering');

      // Step 3: Build Hierarchy
      setFullSetupStep('Building hierarchy with AI grouping...');
      const hierarchyResult = await invoke<{
        clusteringResult: { itemsProcessed: number; clustersCreated: number; itemsAssigned: number } | null;
        hierarchyResult: { levelsCreated: number; intermediateNodesCreated: number; itemsOrganized: number; maxDepth: number };
        levelsCreated: number;
        groupingIterations: number;
      }>('build_full_hierarchy', { runClustering: false });

      // Step 4: Flatten hierarchy (remove empty intermediate levels)
      setFullSetupStep('Flattening hierarchy...');
      const flattenResult = await invoke<number>('flatten_hierarchy');

      setFullSetupStep('');
      const flattenMsg = flattenResult > 0 ? `, ${flattenResult} empty levels removed` : '';
      setSetupResult(`Full setup complete: ${aiResult.processed} items processed, ${clusterResult.clusters_created} clusters, ${hierarchyResult.levelsCreated} hierarchy levels${flattenMsg}`);
      await loadDbStats();
      onDataChanged?.();
      window.location.reload();
    } catch (err) {
      setSetupResult(`Error: ${err}`);
      setFullSetupStep('');
      setIsFullSetup(false);
    }
  };

  const handleSmartAdd = async () => {
    setIsQuickAdding(true);
    setSetupResult(null);
    try {
      const result = await invoke<{
        orphansFound: number;
        matchedToExisting: number;
        newTopicsCreated: number;
        sentToInbox: number;
        processingTimeMs: number;
      }>('smart_add_to_hierarchy');
      setInboxCount(result.sentToInbox);
      if (result.matchedToExisting === 0 && result.newTopicsCreated === 0) {
        setSetupResult('No orphan items to add');
      } else if (result.sentToInbox > 10) {
        // Many items went to Inbox - suggest Full Rebuild
        setSetupResult(`Added ${result.matchedToExisting} to existing, ${result.sentToInbox} to Inbox. Consider Full Rebuild to better organize Inbox items.`);
      } else {
        setSetupResult(`Added ${result.matchedToExisting} to existing topics, ${result.newTopicsCreated} new topics, ${result.sentToInbox} to Inbox (${result.processingTimeMs}ms)`);
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
    regenerateEdges: {
      title: 'Regenerate Edges',
      message: 'This will delete existing semantic edges and recreate them from embeddings (threshold 0.3, max 10 per node). Use after importing new items or changing embeddings.',
      handler: async () => {
        const result = await invoke<{ deleted: number; created: number }>('regenerate_semantic_edges', { threshold: 0.3, maxEdges: 10 });
        return `Deleted ${result.deleted} old edges, created ${result.created} new edges`;
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
    clearTags: {
      title: 'Clear Tags',
      message: 'This will delete all persistent tags and item-tag assignments. Tags will regenerate on next rebuild.',
      handler: async () => {
        const count = await invoke<number>('clear_tags');
        return `Deleted ${count} tags`;
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
    scorePrivacy: {
      title: 'Score Privacy',
      message: `AI will score all items 0.0-1.0 for public shareability. Items < 0.3 are considered private and will be separated during clustering.`,
      handler: async () => {
        setIsScoringPrivacy(true);
        setOperationResult(null);
        try {
          const result = await invoke<{ itemsScored: number; batchesProcessed: number; errorCount: number }>('score_privacy_all_items', { forceRescore: false });
          const msg = `Scored ${result.itemsScored} items in ${result.batchesProcessed} batches${result.errorCount > 0 ? ` (${result.errorCount} errors)` : ''}`;
          setOperationResult(msg);
          return msg;
        } finally {
          setIsScoringPrivacy(false);
        }
      },
    },
    reclassifyPattern: {
      title: 'Reclassify (Pattern)',
      message: 'Re-classify all items using pattern matching. This is FREE and instant. Does not use AI.',
      handler: async () => {
        setIsReclassifyingPattern(true);
        setOperationResult(null);
        try {
          const count = await invoke<number>('reclassify_pattern');
          const msg = `Reclassified ${count} items using patterns (FREE)`;
          setOperationResult(msg);
          return msg;
        } finally {
          setIsReclassifyingPattern(false);
        }
      },
    },
    reclassifyAi: {
      title: 'Reclassify (AI)',
      message: 'Re-classify all items using AI (Haiku).',
      handler: async () => {
        setIsReclassifyingAi(true);
        setOperationResult(null);
        try {
          const count = await invoke<number>('reclassify_ai');
          const msg = `Reclassified ${count} items using AI (cheap)`;
          setOperationResult(msg);
          return msg;
        } finally {
          setIsReclassifyingAi(false);
        }
      },
    },
    rebuildLite: {
      title: 'Rebuild Lite',
      message: 'Reclassify items + recluster with existing embeddings. SAFE: Hierarchy untouched. FREE.',
      handler: async () => {
        setIsRebuildingLite(true);
        setOperationResult(null);
        try {
          const result = await invoke<{ itemsClassified: number; clustersCreated: number; hierarchyLevels: number; method: string }>('rebuild_lite');
          const msg = `Rebuilt: ${result.itemsClassified} items classified, ${result.clustersCreated} clusters (hierarchy untouched)`;
          setOperationResult(msg);
          return msg;
        } finally {
          setIsRebuildingLite(false);
        }
      },
    },
    nameClusters: {
      title: 'Name Clusters (AI)',
      message: 'Name clusters that have keyword-only names using AI. This improves cluster names without re-clustering.',
      handler: async () => {
        setIsNamingClusters(true);
        setOperationResult(null);
        try {
          const result = await invoke<{ clustersNamed: number; clustersSkipped: number }>('name_clusters');
          const msg = `Named ${result.clustersNamed} clusters, skipped ${result.clustersSkipped}`;
          setOperationResult(msg);
          return msg;
        } finally {
          setIsNamingClusters(false);
        }
      },
    },
    resetProcessing: {
      title: 'Reset Processing',
      message: 'This will reset AI processing AND embeddings. Items will be re-analyzed and get new embeddings. Run "Full Rebuild" after.',
      handler: async () => {
        const aiCount = await invoke<number>('reset_ai_processing');
        const embeddingCount = await invoke<number>('clear_embeddings');
        return `Reset AI for ${aiCount} items, cleared ${embeddingCount} embeddings`;
      },
    },
    clearStructure: {
      title: 'Clear Structure',
      message: 'This will clear clustering AND hierarchy. Items keep their embeddings. Run "Full Rebuild" to re-cluster.',
      handler: async () => {
        const clusterCount = await invoke<number>('reset_clustering');
        const hierarchyCount = await invoke<number>('clear_hierarchy');
        return `Reset clustering for ${clusterCount} items, deleted ${hierarchyCount} hierarchy nodes`;
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
                  <div className="flex items-center gap-2">
                    <button
                      onClick={() => dbPath && revealItemInDir(dbPath)}
                      disabled={!dbPath}
                      className="flex items-center gap-1.5 px-3 py-1.5 bg-gray-700 hover:bg-gray-600 text-gray-200 rounded text-xs font-medium transition-colors disabled:opacity-50"
                      title="Reveal in file explorer"
                    >
                      <ExternalLink className="w-3 h-3" />
                      Reveal
                    </button>
                    <button
                      onClick={handleCreateDatabase}
                      disabled={switchingDb}
                      className="flex items-center gap-1.5 px-3 py-1.5 bg-green-700 hover:bg-green-600 text-gray-200 rounded text-xs font-medium transition-colors disabled:opacity-50"
                      title="Create a new empty database"
                    >
                      <FilePlus className="w-3 h-3" />
                      New
                    </button>
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
                      Open...
                    </button>
                  </div>
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

                {/* Tips toggle */}
                <div className="flex items-center justify-between">
                  <div>
                    <div className="text-sm font-medium text-gray-200">Show Tips</div>
                    <div className="text-xs text-gray-500 mt-0.5">Show popup buttons on selected nodes</div>
                  </div>
                  <button
                    onClick={() => handleToggleShowTips(!showTips)}
                    className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
                      showTips ? 'bg-amber-500' : 'bg-gray-600'
                    }`}
                  >
                    <span
                      className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                        showTips ? 'translate-x-6' : 'translate-x-1'
                      }`}
                    />
                  </button>
                </div>

                {/* Show Hidden Items toggle */}
                <div className="flex items-center justify-between">
                  <div>
                    <div className="text-sm font-medium text-gray-200">Show Hidden Items</div>
                    <div className="text-xs text-gray-500 mt-0.5">Show debug, code snippets, and trivial items in graph</div>
                  </div>
                  <button
                    onClick={() => setShowHidden(!showHidden)}
                    className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
                      showHidden ? 'bg-amber-500' : 'bg-gray-600'
                    }`}
                  >
                    <span
                      className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                        showHidden ? 'translate-x-6' : 'translate-x-1'
                      }`}
                    />
                  </button>
                </div>

                {/* Edge Weight Threshold Slider */}
                <div className="pt-2 border-t border-gray-800">
                  <div className="flex items-center justify-between mb-2">
                    <div>
                      <div className="text-sm font-medium text-gray-200">Edge Display Threshold</div>
                      <div className="text-xs text-gray-500 mt-0.5">Only show edges with similarity above this value</div>
                    </div>
                    <div className="text-lg font-mono text-white">{edgeWeightThreshold.toFixed(1)}</div>
                  </div>
                  <input
                    type="range"
                    min="0.3"
                    max="0.9"
                    step="0.1"
                    value={edgeWeightThreshold}
                    onChange={(e) => setEdgeWeightThreshold(parseFloat(e.target.value))}
                    className="w-full h-2 rounded-lg appearance-none cursor-pointer"
                    style={{
                      background: `linear-gradient(to right,
                        rgb(59, 130, 246) 0%,
                        rgb(168, 85, 247) 50%,
                        rgb(236, 72, 153) 100%)`
                    }}
                  />
                  <div className="flex justify-between text-xs text-gray-500 mt-1">
                    <span>0.3 (more edges)</span>
                    <span>0.9 (fewer edges)</span>
                  </div>
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
                          <button
                            onClick={() => {
                              setShowImportSources(false);
                              handleImportChatGPT();
                            }}
                            className="w-full flex items-center gap-3 px-4 py-3 hover:bg-gray-700 transition-colors text-left border-t border-gray-700"
                          >
                            <span className="text-xl">🟢</span>
                            <div>
                              <div className="text-sm text-gray-200">ChatGPT</div>
                              <div className="text-xs text-gray-500">conversations.json</div>
                            </div>
                          </button>
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
                    <button
                      onClick={() => setShowOpenAireDialog(true)}
                      disabled={importing}
                      className="flex-1 flex flex-col items-center gap-2 p-3 bg-gray-800/50 hover:bg-gray-800 rounded-lg transition-colors disabled:opacity-50"
                    >
                      <span className="text-2xl">{importing ? <Loader2 className="w-6 h-6 animate-spin" /> : '📚'}</span>
                      <span className="text-xs text-gray-300">Papers</span>
                      <span className="text-xs text-gray-500">OpenAIRE</span>
                    </button>
                    <button
                      onClick={handleImportCode}
                      disabled={importing}
                      className="flex-1 flex flex-col items-center gap-2 p-3 bg-gray-800/50 hover:bg-gray-800 rounded-lg transition-colors disabled:opacity-50"
                    >
                      <span className="text-2xl">{importing ? <Loader2 className="w-6 h-6 animate-spin" /> : '💻'}</span>
                      <span className="text-xs text-gray-300">Code</span>
                      <span className="text-xs text-gray-500">folder</span>
                    </button>
                  </div>
                </div>

                {/* Smart operation suggestion (after import) */}
                {suggestedOperation && (
                  <div className="bg-blue-900/30 border border-blue-500/50 rounded-lg p-4">
                    <div className="flex items-start gap-3">
                      <span className="text-2xl">{suggestedOperation === 'smartAdd' ? '🧠' : '🚀'}</span>
                      <div className="flex-1">
                        <div className="text-sm text-blue-200">{suggestionReason}</div>
                      </div>
                      <div className="flex gap-2">
                        <button
                          onClick={() => setSuggestedOperation(null)}
                          disabled={isQuickAdding || isFullSetup}
                          className="px-3 py-1.5 text-gray-400 hover:text-white text-xs disabled:opacity-50"
                        >
                          Dismiss
                        </button>
                        <button
                          onClick={() => {
                            if (suggestedOperation === 'smartAdd') {
                              handleSmartAdd();
                            } else {
                              handleFullSetup();
                            }
                            setSuggestedOperation(null);
                          }}
                          disabled={isQuickAdding || isFullSetup || (suggestedOperation === 'fullSetup' && !apiKeyStatus?.hasKey)}
                          className={`px-3 py-1.5 text-white rounded text-xs font-medium disabled:opacity-50 ${
                            suggestedOperation === 'smartAdd'
                              ? 'bg-indigo-600 hover:bg-indigo-700'
                              : 'bg-gradient-to-r from-purple-600 to-green-600 hover:from-purple-700 hover:to-green-700'
                          }`}
                        >
                          {isQuickAdding || isFullSetup ? 'Running...' : `Run ${suggestedOperation === 'smartAdd' ? 'Smart Add' : 'Full Setup'}`}
                        </button>
                      </div>
                    </div>
                  </div>
                )}

                {/* Step 2: Full Setup (combines Process + Cluster + Build) */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-gradient-to-r from-purple-600 to-green-600 flex items-center justify-center text-white text-sm font-bold">2</span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Full Setup</div>
                    <div className="text-xs text-gray-500">Process AI → Cluster → Build Hierarchy</div>
                    {isFullSetup && fullSetupStep && (
                      <div className="text-xs text-amber-400 mt-0.5 flex items-center gap-1">
                        <Loader2 className="w-3 h-3 animate-spin" />
                        {fullSetupStep}
                      </div>
                    )}
                    {!isFullSetup && dbStats && (dbStats.unprocessedItems > 0 || dbStats.unclusteredItems > 0) && (
                      <div className="text-xs text-amber-400/80 mt-0.5">
                        {dbStats.unprocessedItems > 0 && `${dbStats.unprocessedItems} to process`}
                        {dbStats.unprocessedItems > 0 && dbStats.unclusteredItems > 0 && ' · '}
                        {dbStats.unclusteredItems > 0 && `${dbStats.unclusteredItems} to cluster`}
                        {' · '}Haiku ≤{estimateCost(dbStats.unprocessedItems, 'haiku', 1200)} <span className="text-gray-500">(papers skipped)</span>
                        {useLocalEmbeddings
                          ? <span className="text-cyan-400"> · Embeddings: FREE</span>
                          : ` + OpenAI ${estimateCost(dbStats.unprocessedItems, 'openai', 1000)}`
                        }
                        {' + '}Sonnet {estimateCost(Math.max(3, Math.ceil((dbStats.topicsCount || 10) / 100) + 2), 'sonnet', 3000)}
                      </div>
                    )}
                    {!isFullSetup && dbStats && dbStats.unprocessedItems === 0 && dbStats.unclusteredItems === 0 && (
                      <div className="text-xs text-green-500/70 mt-0.5">All items processed and clustered</div>
                    )}
                  </div>
                  <button
                    onClick={handleFullSetup}
                    disabled={isFullSetup || !apiKeyStatus?.hasKey}
                    style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-gradient-to-r from-purple-600 to-green-600 hover:from-purple-700 hover:to-green-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isFullSetup ? <Loader2 className="w-5 h-5 animate-spin" /> : '🚀'}
                  </button>
                </div>

                {/* Advanced Setup Steps (collapsed by default) */}
                <button
                  onClick={() => setShowAdvancedSetup(!showAdvancedSetup)}
                  className="w-full flex items-center gap-2 p-2 text-sm text-gray-400 hover:text-gray-200 transition-colors"
                >
                  {showAdvancedSetup ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
                  <span>Advanced: Individual Steps</span>
                  <span className="text-xs text-gray-600">(for manual control)</span>
                </button>

                {showAdvancedSetup && (
                  <div className="space-y-3 pl-2 border-l-2 border-gray-700/50 ml-2">
                    {/* Step 2: Process AI */}
                    <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                      <span className="flex-shrink-0 w-7 h-7 rounded-full bg-purple-600 flex items-center justify-center text-white text-sm font-bold">2</span>
                      <div className="flex-1 min-w-0">
                        <div className="text-sm font-medium text-gray-200">Process AI</div>
                        <div className="text-xs text-gray-500">Generate titles, summaries & embeddings</div>
                        {dbStats && dbStats.unprocessedItems > 0 && (
                          <div className="text-xs text-amber-400/80 mt-0.5">
                            {dbStats.unprocessedItems} items · Haiku ≤{estimateCost(dbStats.unprocessedItems, 'haiku', 1200)} <span className="text-gray-500">(papers skipped)</span>
                            {useLocalEmbeddings
                              ? <span className="text-cyan-400"> · Embeddings: FREE</span>
                              : ` + OpenAI ${estimateCost(dbStats.unprocessedItems, 'openai', 1000)}`
                            }
                          </div>
                        )}
                        {dbStats && dbStats.unprocessedItems === 0 && (
                          <div className="text-xs text-green-500/70 mt-0.5">All items processed</div>
                        )}
                      </div>
                      <button
                        onClick={handleProcessAi}
                        disabled={isProcessingAi || !apiKeyStatus?.hasKey}
                        style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-purple-600 hover:bg-purple-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
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
                        style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-cyan-600 hover:bg-cyan-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
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
                            ~{Math.max(3, Math.ceil(dbStats.topicsCount / 100) + 2)} calls · Sonnet {estimateCost(Math.max(3, Math.ceil(dbStats.topicsCount / 100) + 2), 'sonnet', 3000)}
                            {openaiKeyStatus && ` + OpenAI ${estimateCost(Math.ceil(dbStats.topicsCount / 8), 'openai', 500)}`}
                            <span className="text-gray-500 ml-1">({dbStats.topicsCount} topics)</span>
                          </div>
                        )}
                        {dbStats && dbStats.topicsCount <= 15 && dbStats.topicsCount > 0 && (
                          <div className="text-xs text-green-500/70 mt-0.5">
                            1 call · Sonnet {estimateCost(1, 'sonnet', 3000)}
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
                        style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-green-600 hover:bg-green-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                      >
                        {isBuildingHierarchy ? <Loader2 className="w-5 h-5 animate-spin" /> : '🏗️'}
                      </button>
                    </div>
                  </div>
                )}

                {/* Step 3: Update Dates (optional) */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-cyan-600 flex items-center justify-center text-white text-sm font-bold">3</span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Update Dates <span className="text-gray-500">(optional)</span></div>
                    <div className="text-xs text-gray-500">Propagate dates to groups (fast)</div>
                    <div className="text-xs text-gray-600 mt-0.5">No API calls</div>
                  </div>
                  <button
                    onClick={handleUpdateDates}
                    disabled={isUpdatingDates}
                    style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-cyan-600 hover:bg-cyan-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isUpdatingDates ? <Loader2 className="w-5 h-5 animate-spin" /> : '📅'}
                  </button>
                </div>

                {/* Step 4: Smart Add (for incremental imports) */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-indigo-600 flex items-center justify-center text-white text-sm font-bold">4</span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Smart Add <span className="text-gray-500">(incremental)</span></div>
                    <div className="text-xs text-gray-500">Add new items using embedding similarity (~5-15s)</div>
                    <div className="text-xs text-gray-600 mt-0.5">
                      <span className="text-cyan-400">FREE</span> · Uses stored embeddings for matching
                      {dbStats && dbStats.orphanItems > 0 && (
                        <span className="text-amber-400/80 ml-1">· {dbStats.orphanItems} items to add</span>
                      )}
                    </div>
                  </div>
                  <button
                    onClick={handleSmartAdd}
                    disabled={isQuickAdding}
                    style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-indigo-600 hover:bg-indigo-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isQuickAdding ? <Loader2 className="w-5 h-5 animate-spin" /> : '🧠'}
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

              {/* LLM Backend Toggle */}
              <div className="bg-gray-900/50 rounded-lg p-4 mb-3">
                <div className="flex items-center justify-between mb-2">
                  <span className="text-sm font-medium text-gray-200">AI Processing Backend</span>
                  <div className="flex items-center gap-1">
                    <button
                      onClick={() => handleSetLlmBackend('anthropic')}
                      className={`px-3 py-1 rounded-l text-xs font-medium transition-colors ${
                        llmBackend === 'anthropic'
                          ? 'bg-purple-600 text-white'
                          : 'bg-gray-700 text-gray-400 hover:text-gray-200'
                      }`}
                    >
                      Claude
                    </button>
                    <button
                      onClick={() => handleSetLlmBackend('ollama')}
                      className={`px-3 py-1 rounded-r text-xs font-medium transition-colors ${
                        llmBackend === 'ollama'
                          ? 'bg-orange-600 text-white'
                          : 'bg-gray-700 text-gray-400 hover:text-gray-200'
                      }`}
                    >
                      Ollama
                    </button>
                  </div>
                </div>
                {llmBackend === 'ollama' && (
                  <div className="mt-3">
                    <div className="flex items-center justify-between mb-2">
                      <span className="text-xs text-gray-400">Ollama Model</span>
                      <span className={`text-xs ${ollamaStatus === 'running' ? 'text-green-400' : 'text-red-400'}`}>
                        {ollamaStatus === 'running' ? '● Running' : '● Not running'}
                      </span>
                    </div>
                    <input
                      type="text"
                      value={ollamaModel}
                      onChange={(e) => setOllamaModel(e.target.value)}
                      onBlur={() => handleSetOllamaModel(ollamaModel)}
                      placeholder="qwen2.5:7b"
                      className="w-full px-3 py-1.5 bg-gray-800 border border-gray-700 rounded text-sm text-white placeholder-gray-500 focus:border-orange-500/50 focus:ring-1 focus:ring-orange-500/20 focus:outline-none"
                    />
                  </div>
                )}
                <p className="mt-2 text-xs text-gray-500">
                  Affects AI Processing (summaries, tags). Hierarchy build still uses Claude.
                </p>
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

              {/* OpenAIRE API Key */}
              <div className="bg-gray-900/50 rounded-lg p-4">
                <div className="flex items-center justify-between mb-2">
                  <span className="text-sm font-medium text-gray-200">OpenAIRE API Key <span className="text-gray-500">(optional)</span></span>
                  {openaireKeyStatus ? (
                    <span className="flex items-center gap-1 text-xs text-green-400">
                      <Check className="w-3 h-3" />
                      Authenticated
                    </span>
                  ) : (
                    <span className="text-xs text-gray-500">Public API</span>
                  )}
                </div>
                {openaireKeyStatus && (
                  <div className="mb-2 px-2 py-1 bg-gray-800 rounded text-xs text-gray-400 font-mono">
                    {openaireKeyStatus}
                  </div>
                )}
                <div className="flex gap-2">
                  <input
                    type="password"
                    placeholder="Bearer token..."
                    value={openaireKeyInput}
                    onChange={(e) => setOpenaireKeyInput(e.target.value)}
                    className="flex-1 px-3 py-1.5 bg-gray-800 border border-gray-700 rounded text-sm text-white placeholder-gray-500 focus:border-amber-500/50 focus:ring-1 focus:ring-amber-500/20 focus:outline-none"
                  />
                  <button
                    onClick={handleSaveOpenaireKey}
                    disabled={savingOpenaireKey || !openaireKeyInput.trim()}
                    className="px-3 py-1.5 bg-amber-500/20 text-amber-200 rounded text-sm font-medium hover:bg-amber-500/30 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                  >
                    {savingOpenaireKey ? 'Saving...' : 'Save'}
                  </button>
                  {openaireKeyStatus && (
                    <button
                      onClick={handleClearOpenaireKey}
                      className="px-3 py-1.5 bg-red-500/20 text-red-300 rounded text-sm font-medium hover:bg-red-500/30 transition-colors"
                    >
                      Clear
                    </button>
                  )}
                </div>
                {openaireKeyError && <p className="mt-2 text-xs text-red-400">{openaireKeyError}</p>}
                <p className="mt-2 text-xs text-gray-500">
                  Optional. Public API works fine, auth gives higher rate limits.{' '}
                  <a href="https://develop.openaire.eu/" target="_blank" rel="noopener noreferrer" className="text-amber-400 hover:underline">
                    Get token
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

                <p className="mt-3 text-xs text-gray-500">
                  <span className="text-gray-600">Regenerate embeddings in Maintenance tab.</span>
                  {' '}
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
                    {dbStats && (
                      <div className="text-xs text-amber-400/80 mt-0.5">
                        {/* Grouping cost (Sonnet) - runs when topics > 15 */}
                        {dbStats.topicsCount > 15 && (
                          <>
                            ~{Math.max(3, Math.ceil(dbStats.topicsCount / 100) + 2)} grouping calls · Sonnet {estimateCost(Math.max(3, Math.ceil(dbStats.topicsCount / 100) + 2), 'sonnet', 3000)}
                          </>
                        )}
                        {dbStats.topicsCount <= 15 && dbStats.topicsCount > 0 && (
                          <span className="text-cyan-400">Clustering: free (local embeddings)</span>
                        )}
                        <span className="text-gray-500 ml-1">({dbStats.topicsCount} topics, {dbStats.totalItems} items)</span>
                      </div>
                    )}
                  </div>
                  <button
                    onClick={() => setConfirmAction('fullRebuild')}
                    disabled={isFullRebuilding || isFlattening || isConsolidating || isTidying}
                    style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-green-600 hover:bg-green-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
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
                    style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-amber-600 hover:bg-amber-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
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
                      1 call · Sonnet {estimateCost(1, 'sonnet', 3000)}
                    </div>
                  </div>
                  <button
                    onClick={() => setConfirmAction('consolidateRoot')}
                    disabled={isConsolidating || isFullRebuilding || isFlattening || isTidying}
                    style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-purple-600 hover:bg-purple-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isConsolidating ? <Loader2 className="w-5 h-5 animate-spin" /> : '🏛️'}
                  </button>
                </div>

                {/* Budget Options Section */}
                <div className="mt-4 mb-2">
                  <h4 className="text-xs font-semibold text-gray-400 uppercase tracking-wide">Budget Options</h4>
                </div>

                {/* Rebuild Lite - SAFE */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3 border border-green-500/30">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-green-700 flex items-center justify-center text-white text-sm">
                    <RefreshCw className="w-3.5 h-3.5" />
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Rebuild Lite <span className="text-green-400">(SAFE)</span></div>
                    <div className="text-xs text-gray-500">Reclassify + recluster. Hierarchy untouched.</div>
                    <div className="text-xs text-green-400 mt-0.5">FREE · Reuses embeddings</div>
                  </div>
                  <button
                    onClick={() => setConfirmAction('rebuildLite')}
                    disabled={isRebuildingLite || isFullRebuilding || isFlattening || isConsolidating || isTidying}
                    style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-green-700 hover:bg-green-800 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isRebuildingLite ? <Loader2 className="w-5 h-5 animate-spin" /> : '🔄'}
                  </button>
                </div>

                {/* Name Clusters (AI) */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-amber-600 flex items-center justify-center text-white text-sm">
                    <Cpu className="w-3.5 h-3.5" />
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Name Clusters (AI)</div>
                    <div className="text-xs text-gray-500">Improve cluster names using AI (after Rebuild Lite)</div>
                    <div className="text-xs text-amber-400 mt-0.5">Haiku · Names keyword-only clusters</div>
                  </div>
                  <button
                    onClick={() => setConfirmAction('nameClusters')}
                    disabled={isNamingClusters || isRebuildingLite || isFullRebuilding}
                    style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-amber-600 hover:bg-amber-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isNamingClusters ? <Loader2 className="w-5 h-5 animate-spin" /> : '🏷️'}
                  </button>
                </div>

                {/* Reclassify (Pattern) */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-emerald-600 flex items-center justify-center text-white text-sm">
                    <Zap className="w-3.5 h-3.5" />
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Reclassify (Pattern)</div>
                    <div className="text-xs text-gray-500">Re-run content_type classification using pattern matching</div>
                    <div className="text-xs text-green-400 mt-0.5">FREE · Instant · No AI</div>
                  </div>
                  <button
                    onClick={() => setConfirmAction('reclassifyPattern')}
                    disabled={isReclassifyingPattern || isReclassifyingAi || isFullRebuilding || isRebuildingLite}
                    style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-emerald-600 hover:bg-emerald-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isReclassifyingPattern ? <Loader2 className="w-5 h-5 animate-spin" /> : '🏷️'}
                  </button>
                </div>

                {/* Reclassify (AI) */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-teal-600 flex items-center justify-center text-white text-sm">
                    <Cpu className="w-3.5 h-3.5" />
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Reclassify (AI)</div>
                    <div className="text-xs text-gray-500">Re-run content_type classification using AI (Haiku)</div>
                    {dbStats && (
                      <div className="text-xs text-cyan-400 mt-0.5">
                        {dbStats.totalItems} items · Haiku {estimateCost(dbStats.totalItems, 'haiku', 105, 'classify')}
                      </div>
                    )}
                  </div>
                  <button
                    onClick={() => setConfirmAction('reclassifyAi')}
                    disabled={isReclassifyingAi || isReclassifyingPattern || isFullRebuilding || isRebuildingLite}
                    style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-teal-600 hover:bg-teal-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isReclassifyingAi ? <Loader2 className="w-5 h-5 animate-spin" /> : '🤖'}
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
                    style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-cyan-600 hover:bg-cyan-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isTidying ? <Loader2 className="w-5 h-5 animate-spin" /> : '🧹'}
                  </button>
                </div>

                {/* Export Trimmed Database */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-indigo-600 flex items-center justify-center text-white text-sm">
                    <Download className="w-3.5 h-3.5" />
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Export Trimmed</div>
                    <div className="text-xs text-gray-500">Export database without PDF blobs for sharing</div>
                    <div className="text-xs text-indigo-400 mt-0.5">
                      Removes PDF blobs · PDFs download on-demand when viewing
                    </div>
                  </div>
                  <button
                    onClick={handleExportTrimmed}
                    disabled={isExportingTrimmed || isFullRebuilding}
                    style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-indigo-600 hover:bg-indigo-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isExportingTrimmed ? <Loader2 className="w-5 h-5 animate-spin" /> : '📦'}
                  </button>
                </div>

                {/* Score Privacy */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-rose-600 flex items-center justify-center text-white text-sm">
                    <Shield className="w-3.5 h-3.5" />
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Score Privacy</div>
                    <div className="text-xs text-gray-500">AI scores items 0.0 (private) to 1.0 (public) for clustering separation</div>
                    {dbStats && (
                      <div className="text-xs text-amber-400 mt-0.5">
                        {dbStats.totalItems} items · Haiku {estimateCost(dbStats.totalItems, 'haiku', 105, 'classify')}
                      </div>
                    )}
                  </div>
                  <button
                    onClick={() => setConfirmAction('scorePrivacy')}
                    disabled={isScoringPrivacy || isFullRebuilding || isFlattening || isConsolidating || isTidying}
                    style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-rose-600 hover:bg-rose-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isScoringPrivacy ? <Loader2 className="w-5 h-5 animate-spin" /> : '🔒'}
                  </button>
                </div>

                {operationResult && (
                  <p className="text-xs text-green-400">{operationResult}</p>
                )}
              </div>
            </section>

            {/* Embeddings Section */}
            <section>
              <div className="flex items-center gap-2 mb-4">
                <Cpu className="w-5 h-5 text-cyan-400" />
                <h3 className="text-lg font-medium text-white">Embeddings</h3>
              </div>

              <div className="space-y-3">
                {/* Current Source Indicator */}
                <div className="bg-gray-900/50 rounded-lg p-3">
                  <div className="flex items-center justify-between">
                    <div>
                      <div className="text-sm text-gray-300">
                        Using: {useLocalEmbeddings ? (
                          <span className="text-cyan-400">Local Model <span className="text-gray-500">(all-MiniLM-L6-v2)</span></span>
                        ) : (
                          <span className="text-amber-400">OpenAI <span className="text-gray-500">(text-embedding-3-small)</span></span>
                        )}
                      </div>
                      <div className="text-xs text-gray-500 mt-0.5">
                        Change embedding source in API Keys tab
                      </div>
                    </div>
                  </div>
                </div>

                {/* Regenerate All Embeddings */}
                <div className="bg-gray-900/50 rounded-lg p-3 flex items-center gap-3">
                  <span className="flex-shrink-0 w-7 h-7 rounded-full bg-cyan-600 flex items-center justify-center text-white text-sm">
                    <RefreshCw className="w-3.5 h-3.5" />
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gray-200">Regenerate All Embeddings</div>
                    <div className="text-xs text-gray-500">Re-compute all embeddings using current source</div>
                    {dbStats && (
                      <div className="text-xs mt-0.5">
                        {useLocalEmbeddings ? (
                          <span className="text-cyan-400">FREE · {dbStats.itemsWithEmbeddings} embeddings</span>
                        ) : (
                          <span className="text-amber-400">{dbStats.itemsWithEmbeddings} items · OpenAI {estimateCost(dbStats.itemsWithEmbeddings, 'openai', 1000)}</span>
                        )}
                      </div>
                    )}
                  </div>
                  <button
                    onClick={handleRegenerateEmbeddings}
                    disabled={isRegeneratingEmbeddings || (!useLocalEmbeddings && !openaiKeyStatus)}
                    style={{ fontSize: '2.5rem' }} className="flex-shrink-0 w-16 h-16 flex items-center justify-center bg-cyan-600 hover:bg-cyan-700 text-white rounded text-xs font-medium transition-colors disabled:opacity-50"
                  >
                    {isRegeneratingEmbeddings ? <Loader2 className="w-5 h-5 animate-spin" /> : '🔄'}
                  </button>
                </div>

                {/* Regeneration Progress */}
                {regenerateProgress && (
                  <div className="bg-gray-900/50 rounded-lg p-3">
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
              </div>
            </section>

            {/* Clustering Thresholds Section */}
            <section>
              <div className="flex items-center gap-2 mb-4">
                <Zap className="w-5 h-5 text-amber-400" />
                <h3 className="text-lg font-medium text-white">Clustering Thresholds</h3>
              </div>

              <div className="bg-gray-900/50 rounded-lg p-4 space-y-4">
                <p className="text-xs text-gray-400">
                  Higher = tighter clusters, more categories. Lower = looser clusters, fewer categories.
                  Default: <span className="text-amber-400">0.75 / 0.60</span> for accurate clustering.
                </p>

                <div className="flex gap-4">
                  <div className="flex-1">
                    <label className="text-sm text-gray-400">Primary (0.3-0.95)</label>
                    <input
                      type="number"
                      step="0.05"
                      min="0.3"
                      max="0.95"
                      placeholder="0.75"
                      value={clusteringPrimary}
                      onChange={(e) => setClusteringPrimary(e.target.value)}
                      className="w-full mt-1 px-3 py-2 bg-gray-800 border border-gray-700 rounded text-sm text-white placeholder-gray-500 focus:border-amber-500 focus:ring-1 focus:ring-amber-500 outline-none"
                    />
                  </div>
                  <div className="flex-1">
                    <label className="text-sm text-gray-400">Secondary (0.2-0.85)</label>
                    <input
                      type="number"
                      step="0.05"
                      min="0.2"
                      max="0.85"
                      placeholder="0.60"
                      value={clusteringSecondary}
                      onChange={(e) => setClusteringSecondary(e.target.value)}
                      className="w-full mt-1 px-3 py-2 bg-gray-800 border border-gray-700 rounded text-sm text-white placeholder-gray-500 focus:border-amber-500 focus:ring-1 focus:ring-amber-500 outline-none"
                    />
                  </div>
                </div>

                <div className="flex items-center justify-between">
                  <button
                    onClick={handleSaveClusteringThresholds}
                    disabled={savingThresholds}
                    className="px-4 py-2 bg-amber-600 hover:bg-amber-700 text-white rounded text-sm font-medium transition-colors disabled:opacity-50 flex items-center gap-2"
                  >
                    {savingThresholds ? <Loader2 className="w-4 h-4 animate-spin" /> : null}
                    Save Thresholds
                  </button>
                  {thresholdResult && (
                    <p className={`text-xs ${thresholdResult.startsWith('Error') ? 'text-red-400' : 'text-green-400'}`}>
                      {thresholdResult}
                    </p>
                  )}
                </div>

                <p className="text-xs text-gray-500">
                  Lower values (0.50-0.65) create fewer, broader clusters. Adjust and rebuild to experiment.
                </p>
              </div>
            </section>

            {/* Privacy Threshold Section */}
            <section>
              <div className="flex items-center gap-2 mb-4">
                <Shield className="w-5 h-5 text-rose-400" />
                <h3 className="text-lg font-medium text-white">Privacy Threshold</h3>
              </div>

              <div className="bg-gray-900/50 rounded-lg p-4 space-y-4">
                <p className="text-xs text-gray-400">
                  Items with privacy score below this threshold go to the Personal category.
                  Default: <span className="text-rose-400">0.5</span> (captures Highly Private + Personal tiers).
                </p>

                <div className="flex gap-4 items-end">
                  <div className="flex-1">
                    <label className="text-sm text-gray-400">Threshold (0.0-1.0)</label>
                    <input
                      type="number"
                      step="0.1"
                      min="0"
                      max="1"
                      placeholder="0.5"
                      value={privacyThresholdInput}
                      onChange={(e) => setPrivacyThresholdInput(e.target.value)}
                      className="w-full mt-1 px-3 py-2 bg-gray-800 border border-gray-700 rounded text-sm text-white placeholder-gray-500 focus:border-rose-500 focus:ring-1 focus:ring-rose-500 outline-none"
                    />
                  </div>
                  <button
                    onClick={handleSavePrivacyThreshold}
                    disabled={savingPrivacyThreshold}
                    className="px-4 py-2 bg-rose-600 hover:bg-rose-700 text-white rounded text-sm font-medium transition-colors disabled:opacity-50 flex items-center gap-2"
                  >
                    {savingPrivacyThreshold ? <Loader2 className="w-4 h-4 animate-spin" /> : null}
                    Save
                  </button>
                </div>

                {privacyThresholdResult && (
                  <p className={`text-xs ${privacyThresholdResult.startsWith('Error') ? 'text-red-400' : 'text-green-400'}`}>
                    {privacyThresholdResult}
                  </p>
                )}

                <p className="text-xs text-gray-500">
                  Higher = more items go to Personal. Lower = fewer items in Personal.
                </p>
              </div>
            </section>

            {/* Danger Zone Section (collapsed by default) */}
            <section>
              <button
                onClick={() => setDangerZoneExpanded(!dangerZoneExpanded)}
                className="w-full flex items-center gap-2 p-2 rounded-lg hover:bg-red-900/20 transition-colors"
              >
                {dangerZoneExpanded ? <ChevronDown className="w-4 h-4 text-red-400" /> : <ChevronRight className="w-4 h-4 text-red-400" />}
                <Trash2 className="w-5 h-5 text-red-400" />
                <h3 className="text-lg font-medium text-white">Danger Zone</h3>
                <span className="text-xs text-gray-500 ml-auto">Reset & clear operations</span>
              </button>

              {dangerZoneExpanded && (
                <div className="mt-4 space-y-4">
                  <div className="grid grid-cols-2 gap-3">
                    {/* Reset AI Processing (titles, summaries, tags) */}
                    <button
                      onClick={() => setConfirmAction('resetAi')}
                      className="flex flex-col items-center gap-2 p-4 bg-gray-900/50 hover:bg-gray-900 rounded-lg transition-colors text-center"
                    >
                      <span className="text-sm font-medium text-gray-200">Reset AI</span>
                      <span className="text-xs text-gray-500">Titles, summaries, tags</span>
                    </button>

                    {/* Clear Embeddings (for similarity) */}
                    <button
                      onClick={() => setConfirmAction('clearEmbeddings')}
                      className="flex flex-col items-center gap-2 p-4 bg-gray-900/50 hover:bg-gray-900 rounded-lg transition-colors text-center"
                    >
                      <span className="text-sm font-medium text-gray-200">Clear Embeddings</span>
                      <span className="text-xs text-gray-500">Similarity vectors</span>
                    </button>

                    {/* Regenerate Edges (rebuild semantic connections) */}
                    <button
                      onClick={() => setConfirmAction('regenerateEdges')}
                      className="flex flex-col items-center gap-2 p-4 bg-gray-900/50 hover:bg-gray-900 rounded-lg transition-colors text-center"
                    >
                      <span className="text-sm font-medium text-gray-200">Regenerate Edges</span>
                      <span className="text-xs text-gray-500">Semantic connections</span>
                    </button>

                    {/* Combined: Clear Structure (Clustering + Hierarchy) */}
                    <button
                      onClick={() => setConfirmAction('clearStructure')}
                      className="flex flex-col items-center gap-2 p-4 bg-gray-900/50 hover:bg-gray-900 rounded-lg transition-colors text-center"
                    >
                      <span className="text-sm font-medium text-gray-200">Clear Structure</span>
                      <span className="text-xs text-gray-500">Clustering + Hierarchy</span>
                    </button>

                    <button
                      onClick={() => setConfirmAction('clearTags')}
                      className="flex flex-col items-center gap-2 p-4 bg-gray-900/50 hover:bg-gray-900 rounded-lg transition-colors text-center"
                    >
                      <span className="text-sm font-medium text-gray-200">Clear Tags</span>
                      <span className="text-xs text-gray-500">Regenerate on rebuild</span>
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
                </div>
              )}
            </section>
            </>
            )}

            {/* PRIVACY TAB */}
            {activeTab === 'privacy' && (
            <section>
              <div className="flex items-center gap-2 mb-4">
                <Shield className="w-5 h-5 text-rose-400" />
                <h3 className="text-lg font-medium text-white">Privacy Export</h3>
              </div>

              {/* Description */}
              <div className="bg-gray-900/50 rounded-lg p-4 mb-4 text-sm text-gray-400 space-y-2">
                <p>
                  Items are scored on a <span className="text-rose-400">0.0</span> (private) to <span className="text-green-400">1.0</span> (public) scale by AI.
                  Use the slider to set your export threshold — items below the threshold are excluded.
                </p>
                <p className="text-xs text-gray-500">
                  <strong className="text-gray-400">Score Privacy</strong> in Maintenance tab to score unscored items.
                  Categories inherit the minimum privacy of their children.
                </p>
              </div>

              <div className="space-y-4">
                {/* Privacy Stats */}
                {privacyStats && (
                  <div className="bg-gray-900/50 rounded-lg p-4 text-sm">
                    <div className="grid grid-cols-2 gap-4">
                      <div>
                        <span className="text-gray-400">Total Items:</span>
                        <span className="ml-2 text-white font-medium">{privacyStats.total}</span>
                      </div>
                      <div>
                        <span className="text-gray-400">Scored:</span>
                        <span className="ml-2 text-white font-medium">{privacyStats.scanned}</span>
                      </div>
                      <div>
                        <span className="text-gray-400">🔒 Private (&lt;0.3):</span>
                        <span className="ml-2 text-rose-400 font-medium">{privacyStats.private}</span>
                      </div>
                      <div>
                        <span className="text-gray-400">✓ Public (&gt;0.7):</span>
                        <span className="ml-2 text-green-400 font-medium">{privacyStats.safe}</span>
                      </div>
                    </div>
                    {privacyStats.unscanned > 0 && (
                      <div className="mt-2 text-xs text-amber-400">
                        {privacyStats.unscanned} items not yet scored — use Score Privacy in Maintenance
                      </div>
                    )}
                  </div>
                )}

                {/* Privacy Threshold Slider */}
                <div className="bg-gray-900/50 rounded-lg p-4">
                  <div className="flex items-center justify-between mb-3">
                    <div className="text-sm font-medium text-gray-200">Export Threshold</div>
                    <div className="text-lg font-mono text-white">{privacyThreshold.toFixed(1)}</div>
                  </div>
                  <input
                    type="range"
                    min="0"
                    max="1"
                    step="0.1"
                    value={privacyThreshold}
                    onChange={(e) => setPrivacyThreshold(parseFloat(e.target.value))}
                    className="w-full h-2 rounded-lg appearance-none cursor-pointer"
                    style={{
                      background: `linear-gradient(to right,
                        rgb(34, 197, 94) 0%,
                        rgb(245, 158, 11) 50%,
                        rgb(244, 63, 94) 100%)`
                    }}
                  />
                  <div className="flex justify-between text-xs text-gray-500 mt-1">
                    <span className="text-green-400">Permissive</span>
                    <span>Moderate</span>
                    <span className="text-rose-400">Strict</span>
                  </div>
                  <p className="mt-3 text-xs text-gray-500">
                    {privacyThreshold <= 0.3 && <span className="text-green-400">Keeps most content, only removes very private items.</span>}
                    {privacyThreshold > 0.3 && privacyThreshold <= 0.5 && <span className="text-amber-400">Removes clearly private content.</span>}
                    {privacyThreshold > 0.5 && privacyThreshold <= 0.7 && <span className="text-amber-400">Removes private and borderline content.</span>}
                    {privacyThreshold > 0.7 && <span className="text-rose-400">Only keeps clearly public content.</span>}
                  </p>

                  {/* Export Preview Counts */}
                  {exportPreview && (
                    <div className="mt-3 p-2 bg-gray-800/50 rounded flex justify-between text-sm">
                      <span className="text-green-400">
                        ✓ {exportPreview.included} included
                      </span>
                      <span className="text-rose-400">
                        🔒 {exportPreview.excluded} excluded
                      </span>
                      {exportPreview.unscored > 0 && (
                        <span className="text-gray-400">
                          ⚠ {exportPreview.unscored} unscored
                        </span>
                      )}
                    </div>
                  )}
                </div>

                {/* Export Button */}
                <div className="bg-gray-900/50 rounded-lg p-4">
                  <button
                    onClick={handleExportShareable}
                    disabled={privacyStats?.scanned === 0}
                    className="w-full flex items-center justify-center gap-2 px-4 py-2.5 bg-emerald-600 hover:bg-emerald-700 text-white rounded-lg font-medium transition-colors disabled:opacity-50"
                  >
                    <Download className="w-4 h-4" />
                    Export Shareable DB (threshold {privacyThreshold.toFixed(1)})
                  </button>
                  <p className="mt-2 text-xs text-gray-500">
                    Creates a copy excluding items with privacy &lt; {privacyThreshold.toFixed(1)}
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

      {/* OpenAIRE Import Dialog */}
      {showOpenAireDialog && (
        <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 rounded-xl shadow-2xl max-w-md w-full p-6 space-y-4">
            <div className="flex items-center justify-between">
              <h3 className="text-lg font-semibold text-white flex items-center gap-2">
                <span>📚</span>
                Import from OpenAIRE
              </h3>
              <button
                onClick={() => setShowOpenAireDialog(false)}
                className="p-1 hover:bg-gray-700 rounded-lg transition-colors text-gray-400 hover:text-white"
              >
                <X className="w-5 h-5" />
              </button>
            </div>

            <p className="text-sm text-gray-400">
              Search the EU Open Research Graph for scientific papers.
            </p>

            <div className="space-y-3">
              {/* Search Query with Tags */}
              <div>
                <label className="block text-sm text-gray-300 mb-1">Search Terms *</label>
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={openAireQueryInput}
                    onChange={(e) => setOpenAireQueryInput(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') {
                        e.preventDefault();
                        handleAddOpenAireTag();
                      }
                    }}
                    placeholder="Type term and press Enter"
                    className="flex-1 px-3 py-2 bg-gray-900 border border-gray-700 rounded-lg text-white text-sm focus:border-amber-500 focus:outline-none"
                  />
                  <button
                    type="button"
                    onClick={handleAddOpenAireTag}
                    disabled={!openAireQueryInput.trim()}
                    className="px-3 py-2 bg-amber-600 hover:bg-amber-500 disabled:bg-gray-700 disabled:text-gray-500 rounded-lg text-sm transition-colors"
                  >
                    <Plus className="w-4 h-4" />
                  </button>
                </div>
                {openAireQueryTags.length > 0 && (
                  <div className="flex flex-wrap gap-2 mt-2">
                    {openAireQueryTags.map((tag) => (
                      <span key={tag} className="inline-flex items-center gap-1 px-2 py-1 bg-amber-600/20 text-amber-400 rounded text-sm">
                        {tag}
                        <button type="button" onClick={() => handleRemoveOpenAireTag(tag)} className="hover:text-amber-200">
                          <X className="w-3 h-3" />
                        </button>
                      </span>
                    ))}
                  </div>
                )}
              </div>

              {/* Match Mode Toggle */}
              <div className="flex items-center gap-4">
                <span className="text-sm text-gray-400">Match:</span>
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="radio"
                    checked={!openAireMatchAll}
                    onChange={() => setOpenAireMatchAll(false)}
                    className="w-4 h-4 text-amber-500"
                  />
                  <span className="text-sm text-gray-300">Any term</span>
                </label>
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="radio"
                    checked={openAireMatchAll}
                    onChange={() => setOpenAireMatchAll(true)}
                    className="w-4 h-4 text-amber-500"
                  />
                  <span className="text-sm text-gray-300">All terms</span>
                </label>
              </div>

              {/* Country + Field of Science */}
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="block text-sm text-gray-300 mb-1">Country</label>
                  <select
                    value={openAireCountry}
                    onChange={(e) => setOpenAireCountry(e.target.value)}
                    style={{ colorScheme: 'dark' }}
                    className="w-full px-3 py-2 bg-gray-900 border border-gray-700 rounded-lg text-white text-sm focus:border-amber-500 focus:outline-none"
                  >
                    <option value="">All countries</option>
                    <option value="EE">Estonia</option>
                    <option value="FI">Finland</option>
                    <option value="DE">Germany</option>
                    <option value="FR">France</option>
                    <option value="GB">United Kingdom</option>
                    <option value="US">United States</option>
                    <option value="NL">Netherlands</option>
                    <option value="SE">Sweden</option>
                    <option value="IT">Italy</option>
                    <option value="ES">Spain</option>
                  </select>
                </div>

                <div>
                  <label className="block text-sm text-gray-300 mb-1">Field of Science</label>
                  <select
                    value={openAireFos}
                    onChange={(e) => setOpenAireFos(e.target.value)}
                    style={{ colorScheme: 'dark' }}
                    className="w-full px-3 py-2 bg-gray-900 border border-gray-700 rounded-lg text-white text-sm focus:border-amber-500 focus:outline-none"
                  >
                    <option value="">All fields</option>
                    <option value="01 natural sciences">Natural Sciences</option>
                    <option value="02 engineering and technology">Engineering</option>
                    <option value="03 medical and health sciences">Medical & Health</option>
                    <option value="05 social sciences">Social Sciences</option>
                    <option value="06 humanities">Humanities</option>
                  </select>
                </div>
              </div>

              <div>
                <label className="block text-sm text-gray-300 mb-1">Max Papers</label>
                <div className="flex gap-2">
                  <input
                    type="number"
                    value={openAireMaxPapers}
                    onChange={(e) => setOpenAireMaxPapers(Math.max(1, parseInt(e.target.value) || 100))}
                    min={1}
                    className="flex-1 px-3 py-2 bg-gray-900 border border-gray-700 rounded-lg text-white text-sm focus:border-amber-500 focus:outline-none"
                  />
                  <button
                    type="button"
                    onClick={() => setOpenAireMaxPapers(openAirePaperCount ?? 100)}
                    disabled={!openAirePaperCount}
                    className="px-3 py-2 bg-gray-700 hover:bg-gray-600 disabled:bg-gray-800 disabled:text-gray-500 text-white rounded-lg text-sm transition-colors"
                  >
                    All
                  </button>
                </div>
                {/* Paper count preview */}
                <div className="mt-1 text-xs h-5">
                  {openAireCountLoading ? (
                    <span className="text-gray-500">Counting...</span>
                  ) : (
                    <span className="text-amber-400">
                      {(openAirePaperCount ?? 0).toLocaleString()} total found
                      {openAirePrevCount !== null && openAirePaperCount !== null && openAirePrevCount !== openAirePaperCount && (
                        <span className={openAirePaperCount > openAirePrevCount ? "text-green-400" : "text-red-400"}>
                          {' '}({openAirePaperCount > openAirePrevCount ? '+' : ''}{(openAirePaperCount - openAirePrevCount).toLocaleString()})
                        </span>
                      )}
                    </span>
                  )}
                </div>
              </div>

              {/* More options (collapsible) */}
              <div>
                <button
                  type="button"
                  onClick={() => setOpenAireShowMore(!openAireShowMore)}
                  className="flex items-center gap-1 text-sm text-gray-400 hover:text-gray-300"
                >
                  {openAireShowMore ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
                  More options
                </button>
                {openAireShowMore && (
                  <div className="mt-2 ml-5 space-y-3">
                    {/* Year Range */}
                    <div className="grid grid-cols-2 gap-3">
                      <div>
                        <label className="block text-xs text-gray-400 mb-1">From Year</label>
                        <input
                          type="number"
                          value={openAireFromYear}
                          onChange={(e) => setOpenAireFromYear(e.target.value)}
                          placeholder="e.g., 2020"
                          min="1900"
                          max="2030"
                          className="w-full px-2 py-1 bg-gray-900 border border-gray-700 rounded text-white text-sm focus:border-amber-500 focus:outline-none"
                        />
                      </div>
                      <div>
                        <label className="block text-xs text-gray-400 mb-1">To Year</label>
                        <input
                          type="number"
                          value={openAireToYear}
                          onChange={(e) => setOpenAireToYear(e.target.value)}
                          placeholder="e.g., 2025"
                          min="1900"
                          max="2030"
                          className="w-full px-2 py-1 bg-gray-900 border border-gray-700 rounded text-white text-sm focus:border-amber-500 focus:outline-none"
                        />
                      </div>
                    </div>

                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={openAireDownloadPdfs}
                        onChange={(e) => setOpenAireDownloadPdfs(e.target.checked)}
                        className="w-4 h-4 rounded border-gray-600 bg-gray-900 text-amber-500 focus:ring-amber-500"
                      />
                      <span className="text-sm text-gray-300">Download PDFs (when available)</span>
                    </label>

                    {openAireDownloadPdfs && (
                      <div className="ml-6">
                        <label className="block text-xs text-gray-400 mb-1">Max PDF size (MB)</label>
                        <input
                          type="number"
                          value={openAireMaxPdfSize}
                          onChange={(e) => setOpenAireMaxPdfSize(Math.max(1, Math.min(100, parseInt(e.target.value) || 20)))}
                          min={1}
                          max={100}
                          className="w-24 px-2 py-1 bg-gray-900 border border-gray-700 rounded text-white text-sm focus:border-amber-500 focus:outline-none"
                        />
                      </div>
                    )}
                  </div>
                )}
              </div>
            </div>

            <div className="flex gap-3 pt-2">
              <button
                onClick={() => setShowOpenAireDialog(false)}
                className="flex-1 px-4 py-2 bg-gray-700 hover:bg-gray-600 text-white rounded-lg text-sm transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleImportOpenAire}
                disabled={openAireQueryTags.length === 0}
                className="flex-1 px-4 py-2 bg-amber-600 hover:bg-amber-500 disabled:bg-gray-700 disabled:text-gray-500 text-white rounded-lg text-sm font-medium transition-colors"
              >
                Import Papers
              </button>
            </div>
          </div>
        </div>
      )}

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
