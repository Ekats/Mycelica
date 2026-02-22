package db

import (
	"strings"
	"unicode"
)

var stopwords = map[string]bool{
	"the": true, "a": true, "an": true, "in": true, "on": true,
	"at": true, "to": true, "for": true, "of": true, "is": true,
	"it": true, "and": true, "or": true, "with": true, "from": true,
	"by": true, "this": true, "that": true, "as": true, "be": true,
}

// BuildFTSQuery preprocesses a natural language query for FTS5.
// Splits on whitespace, removes stopwords and words < 3 chars, trims punctuation,
// joins with " OR ".
func BuildFTSQuery(query string) string {
	words := strings.Fields(query)
	var filtered []string
	for _, w := range words {
		// Trim non-letter/digit chars from both ends
		trimmed := strings.TrimFunc(w, func(r rune) bool {
			return !unicode.IsLetter(r) && !unicode.IsDigit(r) && r != '_'
		})
		if len(trimmed) < 3 {
			continue
		}
		if stopwords[strings.ToLower(trimmed)] {
			continue
		}
		filtered = append(filtered, trimmed)
	}
	return strings.Join(filtered, " OR ")
}

// SearchNodes performs FTS5 search and returns matching nodes.
// Returns empty slice if the preprocessed query is empty or if FTS table doesn't exist.
func (d *DB) SearchNodes(query string) ([]Node, error) {
	ftsQuery := BuildFTSQuery(query)
	if ftsQuery == "" {
		return []Node{}, nil
	}

	rows, err := d.conn.Query(`
		SELECT n.id, n.type, n.title, n.url, n.content, n.created_at, n.updated_at,
		       n.depth, n.is_item, n.is_universe, n.parent_id, n.child_count,
		       n.ai_title, n.summary, n.tags, n.emoji, n.is_processed,
		       n.agent_id, n.node_class, n.meta_type, n.content_type, n.source, n.author
		FROM nodes n
		JOIN nodes_fts fts ON n.rowid = fts.rowid
		WHERE nodes_fts MATCH ?1
		ORDER BY rank
	`, ftsQuery)
	if err != nil {
		// Gracefully handle missing FTS table
		if strings.Contains(err.Error(), "no such table") {
			return []Node{}, nil
		}
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
