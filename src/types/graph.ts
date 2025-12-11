// =============================================================================
// NODE TYPES - Metadata for Leaf mode rendering
// =============================================================================
//
// PRINCIPLE: Graph = navigation, Leaf = content display
//
// Graph rendering does NOT branch on node.type. It only uses:
//   - isItem: boolean (can drill into it, or opens in Leaf?)
//   - childCount: number (has children to navigate to?)
//   - clusterId: number (determines card color)
//   - title, summary, emoji (display fields)
//
// The `type` field is hidden metadata that only matters when Leaf mode
// opens content to determine HOW to render it:
//   - 'conversation' → chat bubble layout (imported Claude chats)
//   - 'note' → markdown renderer
//   - 'bookmark' → URL preview + fetched content
//   - 'cluster' → organizational node, shouldn't open in Leaf
//   - 'page' | 'thought' | 'context' → legacy types, treat as notes
//
export type NodeType =
  | 'conversation'  // Imported chat (Claude, etc.) - renders as chat bubbles
  | 'note'          // User note - renders as markdown
  | 'bookmark'      // URL/webpage - renders with URL preview
  | 'cluster'       // Organizational grouping - not openable in Leaf
  | 'page'          // Legacy: treat as note
  | 'thought'       // Legacy: treat as note
  | 'context';      // Legacy: treat as note

export type EdgeType = 'reference' | 'because' | 'related' | 'contains';

// =============================================================================
// NODE INTERFACE
// =============================================================================
//
// Dynamic hierarchy - no fixed level constants
// depth: 0 = Universe (root), increases toward items
// isItem: true = openable content (conversations, notes, etc.)
// isUniverse: true = single root node

export interface Node {
  id: string;

  // --- Metadata for Leaf mode (NOT used in Graph rendering) ---
  type: NodeType;           // Determines Leaf render mode
  url?: string;             // For bookmarks
  content?: string;         // Raw content from import

  // --- Display fields (used in Graph) ---
  title: string;            // Raw title from import
  aiTitle?: string;         // AI-generated clean title
  summary?: string;         // AI-generated summary
  tags?: string[];          // Parsed tags array
  emoji?: string;           // Topic emoji (AI-suggested or matched)

  // --- Graph navigation fields ---
  depth: number;            // 0 = Universe, increases toward items
  isItem: boolean;          // true = opens in Leaf, false = drill into children
  isUniverse: boolean;      // true = root node (exactly one)
  parentId?: string;        // Parent node ID (null for Universe)
  childCount: number;       // Number of direct children
  clusterId?: number;       // Semantic group ID (determines card color)
  clusterLabel?: string;    // Human-readable cluster name

  // --- Graph layout (not persisted) ---
  position: { x: number; y: number };

  // --- Timestamps & processing state ---
  createdAt: number;
  updatedAt: number;
  isProcessed: boolean;     // Whether AI has processed this node

  // --- Quick access (Sidebar) ---
  isPinned: boolean;        // User-pinned favorite
  lastAccessedAt?: number;  // For recency tracking in sidebar
}

export interface Edge {
  id: string;
  source: string;
  target: string;
  type: EdgeType;
  label?: string;
  weight?: number;  // Semantic similarity (0.0 to 1.0)
  createdAt: number;
}

export interface Viewport {
  x: number;
  y: number;
  zoom: number;
}

export interface Graph {
  nodes: Map<string, Node>;
  edges: Map<string, Edge>;
  viewport: Viewport;
  activeNodeId?: string;
}
