package db

import (
	"encoding/binary"
	"math"
)

// NodeEmbedding pairs a node ID with its deserialized embedding vector.
type NodeEmbedding struct {
	ID        string
	Embedding []float32
}

// bytesToEmbedding converts a little-endian byte slice to []float32.
// Each 4 bytes = one LE float32. Short trailing chunk → 0.0.
func bytesToEmbedding(data []byte) []float32 {
	n := len(data) / 4
	if len(data)%4 != 0 {
		n++ // include partial chunk as 0.0
	}
	result := make([]float32, n)
	for i := 0; i < len(data)/4; i++ {
		bits := binary.LittleEndian.Uint32(data[i*4 : i*4+4])
		result[i] = math.Float32frombits(bits)
	}
	return result
}

// GetNodeEmbedding returns the embedding for a single node, or nil if not set.
func (d *DB) GetNodeEmbedding(id string) ([]float32, error) {
	var data []byte
	err := d.conn.QueryRow("SELECT embedding FROM nodes WHERE id = ?", id).Scan(&data)
	if err != nil {
		return nil, err
	}
	if data == nil {
		return nil, nil
	}
	return bytesToEmbedding(data), nil
}

// GetNodesWithEmbeddings returns all (id, embedding) pairs for nodes that have embeddings.
func (d *DB) GetNodesWithEmbeddings() ([]NodeEmbedding, error) {
	rows, err := d.conn.Query("SELECT id, embedding FROM nodes WHERE embedding IS NOT NULL")
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var result []NodeEmbedding
	for rows.Next() {
		var id string
		var data []byte
		if err := rows.Scan(&id, &data); err != nil {
			return nil, err
		}
		result = append(result, NodeEmbedding{
			ID:        id,
			Embedding: bytesToEmbedding(data),
		})
	}
	return result, rows.Err()
}

// CountNodesWithEmbeddings returns the count of nodes with non-null embeddings.
func (d *DB) CountNodesWithEmbeddings() (int, error) {
	var count int
	err := d.conn.QueryRow("SELECT COUNT(*) FROM nodes WHERE embedding IS NOT NULL").Scan(&count)
	return count, err
}
