package db

// Node represents a row in the nodes table
type Node struct {
	ID          string  `json:"id"`
	NodeType    string  `json:"type"`         // "page", "thought", "context", "cluster", "paper", "bookmark"
	Title       string  `json:"title"`
	URL         *string `json:"url"`
	Content     *string `json:"content"`
	CreatedAt   int64   `json:"created_at"`   // Unix millis
	UpdatedAt   int64   `json:"updated_at"`   // Unix millis
	Depth       int     `json:"depth"`
	IsItem      bool    `json:"is_item"`
	IsUniverse  bool    `json:"is_universe"`
	ParentID    *string `json:"parent_id"`
	ChildCount  int     `json:"child_count"`
	AITitle     *string `json:"ai_title"`
	Summary     *string `json:"summary"`
	Tags        *string `json:"tags"`         // JSON string
	Emoji       *string `json:"emoji"`
	IsProcessed bool    `json:"is_processed"`
	AgentID     *string `json:"agent_id"`
	NodeClass   *string `json:"node_class"`   // "knowledge", "meta", "operational"
	MetaType    *string `json:"meta_type"`    // "summary", "contradiction", "status"
	ContentType *string `json:"content_type"`
	Source      *string `json:"source"`
	Author      *string `json:"author"`
}

// Edge represents a row in the edges table
type Edge struct {
	ID           string   `json:"id"`
	SourceID     string   `json:"source_id"`
	TargetID     string   `json:"target_id"`
	EdgeType     string   `json:"edge_type"`     // lowercase: "calls", "summarizes", etc.
	Label        *string  `json:"label"`
	Weight       *float64 `json:"weight"`
	Confidence   *float64 `json:"confidence"`
	AgentID      *string  `json:"agent_id"`
	Reason       *string  `json:"reason"`
	Content      *string  `json:"content"`
	CreatedAt    int64    `json:"created_at"`    // Unix millis
	UpdatedAt    *int64   `json:"updated_at"`
	SupersededBy *string  `json:"superseded_by"`
	Metadata     *string  `json:"metadata"`      // JSON string
}
