import { useEffect, useCallback } from 'react';
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
  privacyReason: string | null;  // Rust uses #[serde(rename = "privacyReason")]
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
    privacyReason: n.privacyReason ?? undefined,
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

export function useGraph() {
  const { setNodes, setEdges, nodes, edges } = useGraphStore();

  const loadGraph = useCallback(async () => {
    try {
      console.log('Loading graph from backend...');

      const [backendNodes, backendEdges] = await Promise.all([
        invoke<BackendNode[]>('get_nodes'),
        invoke<BackendEdge[]>('get_edges'),
      ]);

      console.log(`Loaded ${backendNodes.length} nodes, ${backendEdges.length} edges`);
      if (backendNodes.length > 0) {
        console.log('First node:', JSON.stringify(backendNodes[0], null, 2));
      }

      const nodeMap = new Map<string, Node>();
      for (const n of backendNodes) {
        nodeMap.set(n.id, mapBackendNode(n));
      }

      const edgeMap = new Map<string, Edge>();
      for (const e of backendEdges) {
        edgeMap.set(e.id, mapBackendEdge(e));
      }

      setNodes(nodeMap);
      setEdges(edgeMap);
    } catch (error) {
      console.error('Failed to load graph:', error);
    }
  }, [setNodes, setEdges]);

  useEffect(() => {
    loadGraph();
  }, [loadGraph]);

  return { nodes, edges, reload: loadGraph };
}
