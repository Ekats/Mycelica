package db

// scanNode scans a row into a Node. The row must have all 23 columns in standard order.
func scanNode(scanner interface{ Scan(dest ...any) error }) (Node, error) {
	var n Node
	err := scanner.Scan(
		&n.ID, &n.NodeType, &n.Title, &n.URL, &n.Content,
		&n.CreatedAt, &n.UpdatedAt, &n.Depth, &n.IsItem, &n.IsUniverse,
		&n.ParentID, &n.ChildCount, &n.AITitle, &n.Summary, &n.Tags,
		&n.Emoji, &n.IsProcessed, &n.AgentID, &n.NodeClass, &n.MetaType,
		&n.ContentType, &n.Source, &n.Author,
	)
	return n, err
}

// AllNodes returns all nodes ordered by created_at descending
func (d *DB) AllNodes() ([]Node, error) {
	rows, err := d.conn.Query(`
		SELECT id, type, title, url, content, created_at, updated_at,
		       depth, is_item, is_universe, parent_id, child_count,
		       ai_title, summary, tags, emoji, is_processed,
		       agent_id, node_class, meta_type, content_type, source, author
		FROM nodes ORDER BY created_at DESC
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var nodes []Node
	for rows.Next() {
		n, err := scanNode(rows)
		if err != nil {
			return nil, err
		}
		nodes = append(nodes, n)
	}
	return nodes, rows.Err()
}

// GetNode returns a single node by ID, or nil if not found
func (d *DB) GetNode(id string) (*Node, error) {
	row := d.conn.QueryRow(`
		SELECT id, type, title, url, content, created_at, updated_at,
		       depth, is_item, is_universe, parent_id, child_count,
		       ai_title, summary, tags, emoji, is_processed,
		       agent_id, node_class, meta_type, content_type, source, author
		FROM nodes WHERE id = ?
	`, id)

	n, err := scanNode(row)
	if err != nil {
		return nil, err
	}
	return &n, nil
}

// SearchByIDPrefix finds nodes whose ID starts with the given prefix.
func (d *DB) SearchByIDPrefix(prefix string, limit int) ([]Node, error) {
	rows, err := d.conn.Query(`
		SELECT id, type, title, url, content, created_at, updated_at,
		       depth, is_item, is_universe, parent_id, child_count,
		       ai_title, summary, tags, emoji, is_processed,
		       agent_id, node_class, meta_type, content_type, source, author
		FROM nodes WHERE id LIKE ? LIMIT ?
	`, prefix+"%", limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var nodes []Node
	for rows.Next() {
		n, err := scanNode(rows)
		if err != nil {
			return nil, err
		}
		nodes = append(nodes, n)
	}
	return nodes, rows.Err()
}
