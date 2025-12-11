import { create } from 'zustand';
import type { Node, Edge, Viewport } from '../types/graph';

// View mode: graph for navigation, leaf for content reading
type ViewMode = 'graph' | 'leaf';

// Navigation breadcrumb entry
interface BreadcrumbEntry {
  id: string;
  title: string;
  emoji?: string;
  depth: number;
  isJump?: boolean;  // True if navigated via "similar nodes" jump
  jumpFromId?: string;  // Source node ID that we jumped from
  jumpFromTitle?: string;  // Source node title for display
  jumpFromEmoji?: string;  // Source node emoji for display
}

interface GraphState {
  nodes: Map<string, Node>;
  edges: Map<string, Edge>;
  viewport: Viewport;
  activeNodeId: string | null;

  // View mode state
  viewMode: ViewMode;              // 'graph' or 'leaf'
  leafNodeId: string | null;       // Node ID being viewed in leaf mode

  // Navigation state for drill-down
  currentDepth: number;             // Current hierarchy depth being viewed
  maxDepth: number;                 // Maximum depth (items are at maxDepth)
  currentParentId: string | null;   // Parent node whose children we're viewing (null = Universe)
  breadcrumbs: BreadcrumbEntry[];   // Navigation history for back button
  visibleNodes: Node[];             // Nodes currently visible at this depth

  // Actions
  setNodes: (nodes: Map<string, Node>) => void;
  addNode: (node: Node) => void;
  updateNode: (id: string, updates: Partial<Node>) => void;
  removeNode: (id: string) => void;

  setEdges: (edges: Map<string, Edge>) => void;
  addEdge: (edge: Edge) => void;
  removeEdge: (id: string) => void;

  setViewport: (viewport: Viewport) => void;
  setActiveNode: (id: string | null) => void;

  // View mode actions
  openLeaf: (nodeId: string) => void;  // Open item in leaf mode
  closeLeaf: () => void;               // Return to graph mode

  // Navigation actions
  setVisibleNodes: (nodes: Node[]) => void;
  setMaxDepth: (depth: number) => void;
  setCurrentDepth: (depth: number) => void;  // Switch to a specific depth
  navigateToNode: (node: Node) => void;  // Drill down into a node
  navigateBack: () => BreadcrumbEntry | null;  // Go back one level
  navigateToRoot: () => void;  // Go back to Universe (depth 0)
  jumpToNode: (node: Node, fromNode?: Node) => void;  // Jump to any node (from similar nodes)
}

export const useGraphStore = create<GraphState>((set, get) => ({
  nodes: new Map(),
  edges: new Map(),
  viewport: { x: 0, y: 0, zoom: 1 },
  activeNodeId: null,

  // View mode state - start in graph mode
  viewMode: 'graph',
  leafNodeId: null,

  // Navigation state - start at Universe (depth 0)
  currentDepth: 0,
  maxDepth: 0,
  currentParentId: null,
  breadcrumbs: [],
  visibleNodes: [],

  setNodes: (nodes) => set({ nodes }),

  addNode: (node) => set((state) => {
    const nodes = new Map(state.nodes);
    nodes.set(node.id, node);
    return { nodes };
  }),

  updateNode: (id, updates) => set((state) => {
    const nodes = new Map(state.nodes);
    const existing = nodes.get(id);
    if (existing) {
      nodes.set(id, { ...existing, ...updates, updatedAt: Date.now() });
    }
    return { nodes };
  }),

  removeNode: (id) => set((state) => {
    const nodes = new Map(state.nodes);
    nodes.delete(id);
    // Also remove edges connected to this node
    const edges = new Map(state.edges);
    for (const [edgeId, edge] of edges) {
      if (edge.source === id || edge.target === id) {
        edges.delete(edgeId);
      }
    }
    return { nodes, edges };
  }),

  setEdges: (edges) => set({ edges }),

  addEdge: (edge) => set((state) => {
    const edges = new Map(state.edges);
    edges.set(edge.id, edge);
    return { edges };
  }),

  removeEdge: (id) => set((state) => {
    const edges = new Map(state.edges);
    edges.delete(id);
    return { edges };
  }),

  setViewport: (viewport) => set({ viewport }),
  setActiveNode: (activeNodeId) => set({ activeNodeId }),

  // View mode actions
  openLeaf: (nodeId) => set({
    viewMode: 'leaf',
    leafNodeId: nodeId,
    activeNodeId: nodeId,
  }),

  closeLeaf: () => set({
    viewMode: 'graph',
    leafNodeId: null,
  }),

  // Navigation actions
  setVisibleNodes: (visibleNodes) => set({ visibleNodes }),

  setMaxDepth: (maxDepth) => set({ maxDepth }),

  setCurrentDepth: (depth) => set({
    currentDepth: depth,
    currentParentId: null,
    breadcrumbs: [],
    visibleNodes: [],
  }),

  navigateToNode: (node) => set((state) => {
    // If it's an item, don't navigate into it (it opens in Leaf mode)
    if (node.isItem) {
      return state;
    }

    // Add current location to breadcrumbs
    const newBreadcrumb: BreadcrumbEntry = {
      id: node.id,
      title: node.aiTitle || node.title,
      emoji: node.emoji,
      depth: node.depth,
    };

    return {
      currentDepth: node.depth + 1,  // Go one level deeper
      currentParentId: node.id,
      breadcrumbs: [...state.breadcrumbs, newBreadcrumb],
      visibleNodes: [],  // Will be loaded by the component
    };
  }),

  navigateBack: () => {
    const state = get();
    if (state.breadcrumbs.length === 0) return null;

    const newBreadcrumbs = [...state.breadcrumbs];
    const popped = newBreadcrumbs.pop()!;

    // Determine new parent: the previous breadcrumb's id, or null if going to root
    const newParentId = newBreadcrumbs.length > 0
      ? newBreadcrumbs[newBreadcrumbs.length - 1].id
      : null;

    // Determine new depth: the popped item's depth (we're going back to that depth)
    const newDepth = popped.depth;

    set({
      currentDepth: newDepth,
      currentParentId: newParentId,
      breadcrumbs: newBreadcrumbs,
      visibleNodes: [],
    });

    return popped;
  },

  navigateToRoot: () => set({
    currentDepth: 0,  // Go back to Universe (depth 0)
    currentParentId: null,
    breadcrumbs: [],
    visibleNodes: [],
  }),

  jumpToNode: (node, fromNode) => set((state) => {
    // Jump to any node - replaces last breadcrumb if it was a jump, otherwise adds
    // Navigate to the node's parent so we see the node in context
    const targetParentId = node.parentId || null;
    const targetDepth = node.depth;

    // Create jump breadcrumb showing where we jumped to
    const jumpBreadcrumb: BreadcrumbEntry = {
      id: node.id,
      title: node.aiTitle || node.title,
      emoji: node.emoji,
      depth: targetDepth,
      isJump: true,
      jumpFromId: fromNode?.id,
      jumpFromTitle: fromNode?.aiTitle || fromNode?.title,
      jumpFromEmoji: fromNode?.emoji,
    };

    // If the last breadcrumb was a jump, replace it (continuing from same similar panel)
    // Otherwise, add new jump breadcrumb
    let newBreadcrumbs: BreadcrumbEntry[];
    if (state.breadcrumbs.length > 0 && state.breadcrumbs[state.breadcrumbs.length - 1].isJump) {
      newBreadcrumbs = [...state.breadcrumbs.slice(0, -1), jumpBreadcrumb];
    } else {
      newBreadcrumbs = [...state.breadcrumbs, jumpBreadcrumb];
    }

    return {
      currentDepth: targetDepth,
      currentParentId: targetParentId,
      breadcrumbs: newBreadcrumbs,
      visibleNodes: [],
      activeNodeId: node.id,  // Highlight the jumped-to node
    };
  }),
}));
