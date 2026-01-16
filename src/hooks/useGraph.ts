import { useEffect, useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useGraphStore } from '../stores/graphStore';
import type { Node, Edge } from '../types/graph';

// =============================================================================
// useGraph - Fetches nodes/edges from Rust backend
// =============================================================================
//
// PRINCIPLE: Graph = navigation, Leaf = content display
//
// Graph rendering uses these fields:
//   - isItem, childCount: navigation behavior
//   - clusterId: card color
//   - title/aiTitle, summary, emoji: display
//
// The `type` field is passed through but NOT used by Graph.tsx.
// It's metadata for future Leaf mode to decide rendering style.
//
// Backend sends these field names (camelCase due to serde rename)
interface BackendNode {
  id: string;
  type: string;  // Rust uses #[serde(rename = "type")]
  title: string;
  url: string | null;
  content: string | null;
  position: { x: number; y: number };
  createdAt: number;  // Rust uses #[serde(rename = "createdAt")]
  updatedAt: number;  // Rust uses #[serde(rename = "updatedAt")]
  clusterId: number | null;  // Rust uses #[serde(rename = "clusterId")]
  clusterLabel: string | null;  // Rust uses #[serde(rename = "clusterLabel")]
  // Dynamic hierarchy fields
  depth: number;
  isItem: boolean;  // Rust uses #[serde(rename = "isItem")]
  isUniverse: boolean;  // Rust uses #[serde(rename = "isUniverse")]
  parentId: string | null;  // Rust uses #[serde(rename = "parentId")]
  childCount: number;  // Rust uses #[serde(rename = "childCount")]
  // AI-processed fields
  aiTitle: string | null;  // Rust uses #[serde(rename = "aiTitle")]
  summary: string | null;
  tags: string | null;  // JSON string from Rust
  emoji: string | null;
  isProcessed: boolean;  // Rust uses #[serde(rename = "isProcessed")]
  // Quick access fields
  isPinned: boolean;  // Rust uses #[serde(rename = "isPinned")]
  lastAccessedAt: number | null;  // Rust uses #[serde(rename = "lastAccessedAt")]
  // Hierarchy date propagation
  latestChildDate: number | null;  // Rust uses #[serde(rename = "latestChildDate")]
  // Privacy filtering
  isPrivate: boolean | null;  // Rust uses #[serde(rename = "isPrivate")]
  privacy: number | null;  // Float score 0.0-1.0 (no serde rename - field name matches)
  privacyReason: string | null;  // Rust uses #[serde(rename = "privacyReason")]
  // Content classification (mini-clustering)
  contentType: string | null;  // Rust uses #[serde(rename = "contentType")]: "idea" | "code" | "debug" | "paste"
  associatedIdeaId: string | null;  // Rust uses #[serde(rename = "associatedIdeaId")]
  // Paper fields
  source: string | null;  // e.g., "openaire", "openaire-pdf"
  pdfAvailable: boolean | null;  // Rust uses #[serde(rename = "pdfAvailable")]
}

interface BackendEdge {
  id: string;
  source: string;
  target: string;
  type: string;  // Rust uses #[serde(rename = "type")]
  label: string | null;
  weight: number | null;  // Semantic similarity (0.0 to 1.0)
  createdAt: number;  // Rust uses #[serde(rename = "createdAt")]
}

function parseTags(tagsJson: string | null): string[] {
  if (!tagsJson) return [];
  try {
    const parsed = JSON.parse(tagsJson);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

function mapBackendNode(n: BackendNode): Node {
  return {
    id: n.id,
    type: n.type as Node['type'],
    title: n.title,
    url: n.url ?? undefined,
    content: n.content ?? undefined,
    position: n.position,
    createdAt: n.createdAt,
    updatedAt: n.updatedAt,
    clusterId: n.clusterId ?? undefined,
    clusterLabel: n.clusterLabel ?? undefined,
    // Dynamic hierarchy fields
    depth: n.depth ?? 0,
    isItem: n.isItem ?? false,
    isUniverse: n.isUniverse ?? false,
    parentId: n.parentId ?? undefined,
    childCount: n.childCount ?? 0,
    // AI-processed fields
    aiTitle: n.aiTitle ?? undefined,
    summary: n.summary ?? undefined,
    tags: parseTags(n.tags),
    emoji: n.emoji ?? undefined,
    isProcessed: n.isProcessed ?? false,
    // Quick access fields
    isPinned: n.isPinned ?? false,
    lastAccessedAt: n.lastAccessedAt ?? undefined,
    // Hierarchy date propagation
    latestChildDate: n.latestChildDate ?? undefined,
    // Privacy filtering
    isPrivate: n.isPrivate ?? undefined,
    privacy: n.privacy ?? undefined,
    privacyReason: n.privacyReason ?? undefined,
    // Content classification (mini-clustering)
    contentType: n.contentType as Node['contentType'] ?? undefined,
    associatedIdeaId: n.associatedIdeaId ?? undefined,
    // Paper fields
    source: n.source ?? undefined,
    pdfAvailable: n.pdfAvailable ?? undefined,
  };
}

function mapBackendEdge(e: BackendEdge): Edge {
  return {
    id: e.id,
    source: e.source,
    target: e.target,
    type: e.type as Edge['type'],
    label: e.label ?? undefined,
    weight: e.weight ?? undefined,
    createdAt: e.createdAt,
  };
}

// Module-level flag to prevent double-loading from multiple useGraph() calls
let isLoadingGraph = false;

export function useGraph() {
  const { setNodes, setEdges, nodes, edges, showHidden } = useGraphStore();
  const [loaded, setLoaded] = useState(false);
  const [loadedParents, setLoadedParents] = useState<Set<string>>(new Set());

  const loadGraph = useCallback(async (includeHidden: boolean) => {
    // Prevent concurrent loads from multiple useGraph() calls
    if (isLoadingGraph) {
      console.log('[PERF] loadGraph skipped - already loading');
      return;
    }
    isLoadingGraph = true;

    try {
      const start = performance.now();
      console.log(`Loading graph from backend (includeHidden=${includeHidden})...`);

      // Load all nodes - needed for similar nodes, search, cross-graph jumps
      const invokeStart = performance.now();
      const backendNodes = await invoke<BackendNode[]>('get_nodes', { includeHidden });
      const invokeTime = performance.now() - invokeStart;

      console.log(`[PERF] get_nodes invoke: ${invokeTime.toFixed(0)}ms (${backendNodes.length} nodes)`);

      const mapStart = performance.now();
      const nodeMap = new Map<string, Node>();
      for (const n of backendNodes) {
        nodeMap.set(n.id, mapBackendNode(n));
      }
      const mapTime = performance.now() - mapStart;

      setNodes(nodeMap);
      // Clear loaded parents cache when reloading with different visibility
      setLoadedParents(new Set());
      setLoaded(true);

      const totalTime = performance.now() - start;
      console.log(`[PERF] loadGraph total: ${totalTime.toFixed(0)}ms (invoke: ${invokeTime.toFixed(0)}ms, map: ${mapTime.toFixed(0)}ms)`);
    } catch (error) {
      console.error('Failed to load graph:', error);
      setLoaded(true); // Mark as loaded even on error so UI doesn't hang
    } finally {
      isLoadingGraph = false;
    }
  }, [setNodes]);

  // Lazy load children of a parent node (for graph navigation)
  // Uses get_graph_children to exclude supporting items (code/debug/paste/trivial)
  // unless showHidden is true
  const loadChildren = useCallback(async (parentId: string, _limit: number = 100) => {
    // Skip if already loaded
    if (loadedParents.has(parentId)) {
      return;
    }

    try {
      console.log(`Lazy loading children of ${parentId} (showHidden=${showHidden})...`);
      const backendNodes = await invoke<BackendNode[]>('get_graph_children', { parentId, includeHidden: showHidden });

      // Merge new nodes into existing map
      const newNodes = new Map(nodes);
      for (const n of backendNodes) {
        newNodes.set(n.id, mapBackendNode(n));
      }
      setNodes(newNodes);

      // Mark parent as loaded
      setLoadedParents(prev => new Set([...prev, parentId]));
      console.log(`Loaded ${backendNodes.length} children of ${parentId}`);
    } catch (error) {
      console.error(`Failed to load children of ${parentId}:`, error);
    }
  }, [nodes, setNodes, loadedParents, showHidden]);

  // Check if a parent's children are loaded
  const isParentLoaded = useCallback((parentId: string) => {
    return loadedParents.has(parentId);
  }, [loadedParents]);

  // Load graph on mount and reload when showHidden changes
  // Skip if nodes already loaded (prevents double-load from multiple useGraph() calls)
  useEffect(() => {
    if (nodes.size === 0) {
      loadGraph(showHidden);
    }
  }, [loadGraph, showHidden, nodes.size]);

  // Simple reload function that uses current showHidden value
  const reload = useCallback(() => {
    loadGraph(showHidden);
  }, [loadGraph, showHidden]);

  // Load edges for a specific view (where both endpoints are children of the given parent)
  // Uses indexed lookup for O(1) performance instead of client-side filtering
  const loadEdgesForView = useCallback(async (parentId: string) => {
    try {
      console.log(`Loading edges for view: ${parentId}`);
      const backendEdges = await invoke<BackendEdge[]>('get_edges_for_view', { parentId });

      const edgeMap = new Map<string, Edge>();
      for (const e of backendEdges) {
        edgeMap.set(e.id, mapBackendEdge(e));
      }

      setEdges(edgeMap);
      console.log(`Loaded ${backendEdges.length} edges for view ${parentId}`);
    } catch (err) {
      console.error('Failed to load view edges:', err);
    }
  }, [setEdges]);

  // Load precomputed edges for a FOS category (fast cached lookup)
  // Falls back to loadEdgesForView if fos_edges table is empty
  const loadEdgesForFos = useCallback(async (fosId: string) => {
    try {
      console.log(`Loading precomputed edges for FOS: ${fosId}`);
      const backendEdges = await invoke<BackendEdge[]>('get_edges_for_fos', { fosId });

      // Only use precomputed FOS edges if available
      // Otherwise fall back to per-view loading
      if (backendEdges.length === 0) {
        console.log(`No precomputed FOS edges for ${fosId}, using per-view loading`);
        await loadEdgesForView(fosId);
        return;
      }

      const edgeMap = new Map<string, Edge>();
      for (const e of backendEdges) {
        edgeMap.set(e.id, mapBackendEdge(e));
      }

      setEdges(edgeMap);
      console.log(`Loaded ${backendEdges.length} precomputed edges for FOS ${fosId}`);
    } catch (err) {
      console.error('Failed to load FOS edges:', err);
    }
  }, [setEdges, loadEdgesForView]);

  return { nodes, edges, reload, loaded, loadChildren, isParentLoaded, loadEdgesForView, loadEdgesForFos };
}
