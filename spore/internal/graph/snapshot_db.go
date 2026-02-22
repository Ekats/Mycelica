package graph

import "mycelica/spore/internal/db"

// SnapshotFromDB loads a GraphSnapshot from the database
func SnapshotFromDB(d *db.DB) (*GraphSnapshot, error) {
	dbNodes, err := d.AllNodes()
	if err != nil {
		return nil, err
	}
	dbEdges, err := d.AllEdges()
	if err != nil {
		return nil, err
	}

	nodes := make([]*NodeInfo, 0, len(dbNodes))
	for _, n := range dbNodes {
		var parentID *string
		if n.ParentID != nil {
			p := *n.ParentID
			parentID = &p
		}
		nodes = append(nodes, &NodeInfo{
			ID:        n.ID,
			Title:     n.Title,
			NodeType:  n.NodeType,
			CreatedAt: n.CreatedAt,
			UpdatedAt: n.UpdatedAt,
			ParentID:  parentID,
			Depth:     n.Depth,
			IsItem:    n.IsItem,
		})
	}

	edges := make([]EdgeInfo, 0, len(dbEdges))
	for _, e := range dbEdges {
		var updatedAt *int64
		if e.UpdatedAt != nil {
			v := *e.UpdatedAt
			updatedAt = &v
		}
		edges = append(edges, EdgeInfo{
			ID:        e.ID,
			Source:    e.SourceID,
			Target:   e.TargetID,
			EdgeType: e.EdgeType,
			CreatedAt: e.CreatedAt,
			UpdatedAt: updatedAt,
		})
	}

	return NewSnapshot(nodes, edges), nil
}
