import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  Node, Edge, PersonalNode, PersonalEdge, TeamConfig,
  CreateNodeRequest, PatchNodeRequest, CreateEdgeRequest,
  DisplayNode, DisplayEdge, PersonalData, SavedPosition,
  BreadcrumbEntry,
} from "../types";

interface TeamStore {
  // Team data (from server snapshot)
  nodes: Map<string, Node>;
  edges: Edge[];
  authors: string[];

  // Personal data (from local.db)
  personalNodes: Map<string, PersonalNode>;
  personalEdges: PersonalEdge[];
  savedPositions: Map<string, { x: number; y: number }>;

  // In-memory positions (updated by GraphView after each render, NOT persisted)
  currentPositions: Map<string, { x: number; y: number }>;
  setCurrentPositions: (positions: Map<string, { x: number; y: number }>) => void;

  // Local category overrides (node IDs created as categories, survives refresh)
  localCategories: Set<string>;
  addLocalCategory: (id: string) => void;

  // Navigation (view layers)
  universeId: string | null;
  currentParentId: string | null;   // null = root view (children of universe)
  breadcrumbs: BreadcrumbEntry[];
  navigateToCategory: (nodeId: string) => void;
  navigateBack: () => void;
  navigateToRoot: () => void;
  navigateToBreadcrumb: (nodeId: string) => void;
  navigateToNodeParent: (nodeId: string) => void;

  // UI
  selectedNodeId: string | null;
  searchQuery: string;
  searchResults: Node[];
  showRecent: boolean;
  showSettings: boolean;
  showQuickAdd: boolean;

  // Status
  lastRefreshed: Date | null;
  isRefreshing: boolean;
  connected: boolean;
  config: TeamConfig | null;
  error: string | null;

  // Computed
  getDisplayNodes: () => DisplayNode[];
  getDisplayEdges: () => DisplayEdge[];
  // Actions
  refresh: () => Promise<void>;
  loadPersonalData: () => Promise<void>;
  loadPositions: () => Promise<void>;
  loadSettings: () => Promise<void>;

  createNode: (req: CreateNodeRequest) => Promise<string | null>;
  updateNode: (id: string, req: PatchNodeRequest) => Promise<void>;
  deleteNode: (id: string) => Promise<void>;
  createEdge: (req: CreateEdgeRequest) => Promise<void>;
  search: (query: string) => Promise<void>;

  createPersonalNode: (title: string, content?: string, contentType?: string, tags?: string) => Promise<PersonalNode>;
  deletePersonalNode: (id: string) => Promise<void>;
  updatePersonalNode: (id: string, updates: { title?: string; content?: string; contentType?: string; tags?: string }) => Promise<void>;
  createPersonalEdge: (sourceId: string, targetId: string, edgeType?: string, reason?: string) => Promise<PersonalEdge>;

  savePositions: (positions: Array<{ node_id: string; x: number; y: number }>) => Promise<void>;
  saveSettings: (config: TeamConfig) => Promise<void>;

  setSelectedNodeId: (id: string | null) => void;
  setShowRecent: (show: boolean) => void;
  setShowSettings: (show: boolean) => void;
  setShowQuickAdd: (show: boolean) => void;
  clearError: () => void;

  panToNodeId: string | null;
  setPanToNodeId: (id: string | null) => void;
}

export const useTeamStore = create<TeamStore>((set, get) => ({
  nodes: new Map(),
  edges: [],
  authors: [],
  personalNodes: new Map(),
  personalEdges: [],
  savedPositions: new Map(),
  currentPositions: new Map(),
  setCurrentPositions: (positions) => set({ currentPositions: positions }),
  localCategories: new Set(),
  addLocalCategory: (id) => set((s) => {
    const lc = new Set(s.localCategories);
    lc.add(id);
    return { localCategories: lc };
  }),

  universeId: null,
  currentParentId: null,
  breadcrumbs: [],

  navigateToCategory: (nodeId) => {
    const { nodes, localCategories } = get();
    const node = nodes.get(nodeId);
    if (!node) return;
    const isItem = localCategories.has(node.id) ? false : node.isItem;
    if (isItem) return;
    const title = node.aiTitle || node.title;
    set((s) => ({
      currentParentId: nodeId,
      breadcrumbs: [...s.breadcrumbs, { id: nodeId, title }],
      selectedNodeId: null,
    }));
  },

  navigateBack: () => {
    const { breadcrumbs } = get();
    if (breadcrumbs.length === 0) return;
    const newBreadcrumbs = breadcrumbs.slice(0, -1);
    const newParentId = newBreadcrumbs.length > 0
      ? newBreadcrumbs[newBreadcrumbs.length - 1].id
      : null;
    set({ currentParentId: newParentId, breadcrumbs: newBreadcrumbs, selectedNodeId: null });
  },

  navigateToRoot: () => {
    set({ currentParentId: null, breadcrumbs: [], selectedNodeId: null });
  },

  navigateToBreadcrumb: (nodeId) => {
    const { breadcrumbs } = get();
    const index = breadcrumbs.findIndex((b) => b.id === nodeId);
    if (index === -1) return;
    const newBreadcrumbs = breadcrumbs.slice(0, index + 1);
    set({ currentParentId: nodeId, breadcrumbs: newBreadcrumbs, selectedNodeId: null });
  },

  navigateToNodeParent: (nodeId) => {
    const { nodes, universeId } = get();
    const node = nodes.get(nodeId);
    if (!node) return;

    // Build breadcrumb trail by walking up the parentId chain
    const trail: BreadcrumbEntry[] = [];
    let cur = node.parentId ? nodes.get(node.parentId) : null;
    const ancestors: Node[] = [];
    while (cur && cur.id !== universeId) {
      ancestors.unshift(cur);
      cur = cur.parentId ? nodes.get(cur.parentId) : null;
    }
    for (const a of ancestors) {
      trail.push({ id: a.id, title: a.aiTitle || a.title });
    }

    // Navigate to the node's parent (or root if parentId is universe/null)
    const targetParent = node.parentId === universeId ? null : (node.parentId || null);
    set({
      currentParentId: targetParent,
      breadcrumbs: trail,
      selectedNodeId: nodeId,
      panToNodeId: nodeId,
    });
  },

  selectedNodeId: null,
  searchQuery: "",
  searchResults: [],
  showRecent: false,
  showSettings: false,
  showQuickAdd: false,

  lastRefreshed: null,
  isRefreshing: false,
  connected: false,
  config: null,
  error: null,

  getDisplayNodes: () => {
    const { nodes, personalNodes, localCategories, currentParentId, universeId } = get();
    const display: DisplayNode[] = [];

    for (const n of nodes.values()) {
      // Skip the universe node itself — it's structural
      if (n.isUniverse) continue;

      // Filter by current view level
      if (currentParentId !== null) {
        // Drilled into a category: show its direct children
        if ((n.parentId || null) !== currentParentId) continue;
      } else if (universeId) {
        // Root view with hierarchy: show children of universe
        if (n.parentId !== universeId) continue;
      } else {
        // No universe: show top-level nodes (no parent, or parent not in graph)
        if (n.parentId && nodes.has(n.parentId)) continue;
      }

      display.push({
        id: n.id,
        title: n.aiTitle || n.title,
        content: n.content,
        contentType: n.contentType,
        tags: n.tags,
        author: n.author,
        isPersonal: false,
        isItem: localCategories.has(n.id) ? false : n.isItem,
        parentId: n.parentId,
        childCount: n.childCount,
        createdAt: n.createdAt,
        updatedAt: n.updatedAt,
        x: n.x,
        y: n.y,
      });
    }

    // Personal nodes: always visible at every level
    for (const pn of personalNodes.values()) {
      display.push({
        id: pn.id,
        title: pn.title,
        content: pn.content,
        contentType: pn.contentType,
        tags: pn.tags,
        author: undefined,
        isPersonal: true,
        isItem: true,
        parentId: undefined,
        childCount: 0,
        createdAt: pn.createdAt,
        updatedAt: pn.updatedAt,
      });
    }
    return display;
  },

  getDisplayEdges: () => {
    const { edges, personalEdges } = get();
    // Only include edges where both endpoints are visible
    const visibleIds = new Set(get().getDisplayNodes().map((n) => n.id));
    const display: DisplayEdge[] = [];
    for (const e of edges) {
      if (!visibleIds.has(e.source) || !visibleIds.has(e.target)) continue;
      display.push({
        id: e.id,
        source: e.source,
        target: e.target,
        type: e.type,
        reason: e.reason,
        author: e.author,
        edgeSource: e.edgeSource,
        isPersonal: false,
      });
    }
    for (const pe of personalEdges) {
      if (!visibleIds.has(pe.sourceId) || !visibleIds.has(pe.targetId)) continue;
      display.push({
        id: pe.id,
        source: pe.sourceId,
        target: pe.targetId,
        type: pe.edgeType,
        reason: pe.reason,
        isPersonal: true,
      });
    }
    return display;
  },


  refresh: async () => {
    set({ isRefreshing: true, error: null });
    try {
      const snapshot = await invoke<{ nodes: Node[]; edges: Edge[] }>("team_refresh");
      const nodeMap = new Map<string, Node>();
      const authorSet = new Set<string>();
      for (const n of snapshot.nodes) {
        nodeMap.set(n.id, n);
        if (n.author) authorSet.add(n.author);
      }

      // Restore saved positions
      const { savedPositions } = get();
      for (const [id, pos] of savedPositions) {
        const node = nodeMap.get(id);
        if (node) {
          node.x = pos.x;
          node.y = pos.y;
        }
      }

      // Cache universe node ID — only trust the explicit isUniverse flag
      let uId: string | null = null;
      for (const n of nodeMap.values()) {
        if (n.isUniverse) { uId = n.id; break; }
      }

      set({
        nodes: nodeMap,
        edges: snapshot.edges,
        authors: Array.from(authorSet),
        universeId: uId,
        lastRefreshed: new Date(),
        isRefreshing: false,
        connected: true,
      });
    } catch (e) {
      set({ isRefreshing: false, connected: false });
    }
  },

  loadPersonalData: async () => {
    try {
      const data = await invoke<PersonalData>("team_get_personal_data");
      const nodeMap = new Map<string, PersonalNode>();
      for (const n of data.nodes) nodeMap.set(n.id, n);
      set({ personalNodes: nodeMap, personalEdges: data.edges });
    } catch (e) {
      console.error("Failed to load personal data:", e);
    }
  },

  loadPositions: async () => {
    try {
      const positions = await invoke<SavedPosition[]>("team_get_positions");
      const posMap = new Map<string, { x: number; y: number }>();
      for (const p of positions) posMap.set(p.node_id, { x: p.x, y: p.y });
      set({ savedPositions: posMap });
    } catch (e) {
      console.error("Failed to load positions:", e);
    }
  },

  loadSettings: async () => {
    try {
      const config = await invoke<TeamConfig>("team_get_settings");
      set({ config });
    } catch (e) {
      console.error("Failed to load settings:", e);
    }
  },

  createNode: async (req) => {
    try {
      const result = await invoke<{ node: Node }>("team_create_node", { req });
      set((s) => {
        const nodes = new Map(s.nodes);
        nodes.set(result.node.id, result.node);
        return { nodes };
      });
      // Auto-refresh to pick up server-created edges (from connects_to)
      get().refresh();
      return result.node.id;
    } catch (e) {
      set({ error: String(e) });
      return null;
    }
  },

  updateNode: async (id, req) => {
    try {
      const node = await invoke<Node>("team_update_node", { id, req });
      set((s) => {
        const nodes = new Map(s.nodes);
        nodes.set(node.id, node);
        return { nodes };
      });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  deleteNode: async (id) => {
    try {
      await invoke("team_delete_node", { id });
      set((s) => {
        const nodes = new Map(s.nodes);
        nodes.delete(id);
        const edges = s.edges.filter((e) => e.source !== id && e.target !== id);
        return { nodes, edges, selectedNodeId: s.selectedNodeId === id ? null : s.selectedNodeId };
      });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  createEdge: async (req) => {
    try {
      const result = await invoke<{ edge: Edge }>("team_create_edge", { req });
      set((s) => ({ edges: [...s.edges, result.edge] }));
      get().refresh();
    } catch (e) {
      set({ error: String(e) });
    }
  },

  search: async (query) => {
    set({ searchQuery: query });
    if (!query.trim()) {
      set({ searchResults: [] });
      return;
    }
    try {
      const results = await invoke<Node[]>("team_search", { query, limit: 20 });
      set({ searchResults: results });
    } catch (e) {
      console.error("Search failed:", e);
    }
  },

  createPersonalNode: async (title, content, contentType, tags) => {
    const node = await invoke<PersonalNode>("team_create_personal_node", {
      title, content, contentType, tags,
    });
    set((s) => {
      const personalNodes = new Map(s.personalNodes);
      personalNodes.set(node.id, node);
      return { personalNodes };
    });
    return node;
  },

  deletePersonalNode: async (id) => {
    try {
      await invoke("team_delete_personal_node", { id });
      set((s) => {
        const personalNodes = new Map(s.personalNodes);
        personalNodes.delete(id);
        const personalEdges = s.personalEdges.filter((e) => e.sourceId !== id && e.targetId !== id);
        return { personalNodes, personalEdges, selectedNodeId: s.selectedNodeId === id ? null : s.selectedNodeId };
      });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  updatePersonalNode: async (id, updates) => {
    try {
      const node = await invoke<PersonalNode>("team_update_personal_node", {
        id,
        title: updates.title,
        content: updates.content,
        contentType: updates.contentType,
        tags: updates.tags,
      });
      set((s) => {
        const personalNodes = new Map(s.personalNodes);
        personalNodes.set(node.id, node);
        return { personalNodes };
      });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  createPersonalEdge: async (sourceId, targetId, edgeType, reason) => {
    const edge = await invoke<PersonalEdge>("team_create_personal_edge", {
      sourceId, targetId, edgeType, reason,
    });
    set((s) => ({ personalEdges: [...s.personalEdges, edge] }));
    return edge;
  },

  savePositions: async (positions) => {
    try {
      await invoke("team_save_positions", { positions });
      set((s) => {
        const posMap = new Map(s.savedPositions);
        for (const p of positions) posMap.set(p.node_id, { x: p.x, y: p.y });
        return { savedPositions: posMap };
      });
    } catch (e) {
      console.error("Failed to save positions:", e);
    }
  },

  saveSettings: async (newConfig) => {
    try {
      await invoke("team_save_settings", { newConfig });
      set({ config: newConfig, showSettings: false });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  setSelectedNodeId: (id) => set({ selectedNodeId: id }),
  setShowRecent: (show) => set({ showRecent: show }),
  setShowSettings: (show) => set({ showSettings: show }),
  setShowQuickAdd: (show) => set({ showQuickAdd: show }),
  clearError: () => set({ error: null }),

  panToNodeId: null,
  setPanToNodeId: (id) => set({ panToNodeId: id }),
}));
