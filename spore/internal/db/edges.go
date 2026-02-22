package db

import "sort"

// scanEdge scans a row into an Edge. The row must have all 14 columns in standard order.
func scanEdge(scanner interface{ Scan(dest ...any) error }) (Edge, error) {
	var e Edge
	err := scanner.Scan(
		&e.ID, &e.SourceID, &e.TargetID, &e.EdgeType, &e.Label,
		&e.Weight, &e.Confidence, &e.AgentID, &e.Reason, &e.Content,
		&e.CreatedAt, &e.UpdatedAt, &e.SupersededBy, &e.Metadata,
	)
	return e, err
}

// AllEdges returns all edges
func (d *DB) AllEdges() ([]Edge, error) {
	rows, err := d.conn.Query(`
		SELECT id, source_id, target_id, type, label, weight, confidence,
		       agent_id, reason, content, created_at, updated_at,
		       superseded_by, metadata
		FROM edges
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var edges []Edge
	for rows.Next() {
		e, err := scanEdge(rows)
		if err != nil {
			return nil, err
		}
		edges = append(edges, e)
	}
	return edges, rows.Err()
}

// GetEdgesForNode returns all edges where the given node is source OR target.
func (d *DB) GetEdgesForNode(nodeID string) ([]Edge, error) {
	rows, err := d.conn.Query(`
		SELECT id, source_id, target_id, type, label, weight, confidence,
		       agent_id, reason, content, created_at, updated_at,
		       superseded_by, metadata
		FROM edges WHERE source_id = ? OR target_id = ?
	`, nodeID, nodeID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var edges []Edge
	for rows.Next() {
		e, err := scanEdge(rows)
		if err != nil {
			return nil, err
		}
		edges = append(edges, e)
	}
	return edges, rows.Err()
}

// EdgeTypePriority returns the traversal priority for an edge type.
// Higher priority = lower traversal cost in Dijkstra.
// Matches schema.rs:5665-5672.
func EdgeTypePriority(edgeType string) float64 {
	switch edgeType {
	case "contradicts", "flags":
		return 1.0
	case "derives_from", "summarizes", "resolves", "supersedes":
		return 0.7
	case "supports", "questions", "prerequisite", "evolved_from":
		return 0.5
	default:
		return 0.3
	}
}

// IsStructuralEdge returns true for edge types that represent structural
// relationships (same file, hierarchy) rather than semantic ones.
func IsStructuralEdge(edgeType string) bool {
	switch edgeType {
	case "defined_in", "belongs_to", "sibling":
		return true
	default:
		return false
	}
}

// EdgesForContext returns the top-N most relevant edges for a node,
// scored by 0.3*recency + 0.3*confidence + 0.4*type_priority.
// Matches schema.rs:5674-5709.
func (d *DB) EdgesForContext(nodeID string, topN int, notSuperseded bool) ([]Edge, error) {
	all, err := d.GetEdgesForNode(nodeID)
	if err != nil {
		return nil, err
	}

	if notSuperseded {
		filtered := all[:0]
		for _, e := range all {
			if e.SupersededBy == nil {
				filtered = append(filtered, e)
			}
		}
		all = filtered
	}

	if len(all) == 0 {
		return all, nil
	}

	// Compute time range for recency normalization
	oldest := all[0].CreatedAt
	newest := all[0].CreatedAt
	for _, e := range all[1:] {
		if e.CreatedAt < oldest {
			oldest = e.CreatedAt
		}
		if e.CreatedAt > newest {
			newest = e.CreatedAt
		}
	}
	timeRange := float64(newest - oldest)

	// Score and sort
	type scored struct {
		score float64
		edge  Edge
	}
	items := make([]scored, len(all))
	for i, e := range all {
		recency := 1.0
		if timeRange > 0 {
			recency = float64(e.CreatedAt-oldest) / timeRange
		}
		confidence := 0.5
		if e.Confidence != nil {
			confidence = *e.Confidence
		}
		typePriority := EdgeTypePriority(e.EdgeType)
		items[i] = scored{
			score: 0.3*recency + 0.3*confidence + 0.4*typePriority,
			edge:  e,
		}
	}

	sort.Slice(items, func(i, j int) bool {
		return items[i].score > items[j].score
	})

	if len(items) > topN {
		items = items[:topN]
	}

	result := make([]Edge, len(items))
	for i, s := range items {
		result[i] = s.edge
	}
	return result, nil
}
