export type ContentType =
  | 'concept' | 'question' | 'decision' | 'reference' | 'idea'
  | 'insight' | 'exploration' | 'synthesis' | 'planning';

export type EdgeType =
  | 'related' | 'reference' | 'because' | 'contains'
  | 'prerequisite' | 'contradicts' | 'supports' | 'evolved_from' | 'questions';

export interface Node {
  id: string;
  title: string;
  aiTitle?: string;
  content?: string;
  tags?: string;
  contentType?: string;
  author?: string;
  humanCreated: boolean;
  humanEdited?: string;
  parentId?: string;
  isItem: boolean;
  isUniverse?: boolean;
  childCount: number;
  createdAt: number;
  updatedAt: number;
  x?: number;
  y?: number;
}

export interface Edge {
  id: string;
  source: string;
  target: string;
  type: EdgeType;
  weight?: number;
  edgeSource?: string;
  author?: string;
  reason?: string;
  createdAt: number;
}

export interface PersonalNode {
  id: string;
  title: string;
  content?: string;
  contentType?: string;
  tags?: string;
  createdAt: number;
  updatedAt: number;
}

export interface PersonalEdge {
  id: string;
  sourceId: string;
  targetId: string;
  edgeType: string;
  reason?: string;
  createdAt: number;
}

export interface TeamSnapshot {
  nodes: Node[];
  edges: Edge[];
}

export interface TeamConfig {
  server_url: string;
  author: string;
}

export interface CreateNodeRequest {
  title: string;
  content?: string;
  url?: string;
  content_type?: string;
  tags?: string;
  author?: string;
  connects_to?: string[];
  is_item?: boolean;
}

export interface PatchNodeRequest {
  title?: string;
  content?: string;
  tags?: string;
  content_type?: string;
  parent_id?: string;
  author?: string;
}

export interface CreateEdgeRequest {
  source: string;
  target: string;
  edge_type?: string;
  reason?: string;
  author?: string;
}

export interface PersonalData {
  nodes: PersonalNode[];
  edges: PersonalEdge[];
}

export interface SavedPosition {
  node_id: string;
  x: number;
  y: number;
}

// Unified display node for the graph (merges team + personal)
export interface DisplayNode {
  id: string;
  title: string;
  content?: string;
  contentType?: string;
  tags?: string;
  author?: string;
  isPersonal: boolean;
  isItem: boolean;
  parentId?: string;
  childCount: number;
  createdAt: number;
  updatedAt: number;
  x?: number;
  y?: number;
}

// Unified display edge
export interface DisplayEdge {
  id: string;
  source: string;
  target: string;
  type: string;
  reason?: string;
  author?: string;
  edgeSource?: string;
  isPersonal: boolean;
}

export interface BreadcrumbEntry {
  id: string;
  title: string;
}
