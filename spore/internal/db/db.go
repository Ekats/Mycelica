package db

import (
	"database/sql"
	"fmt"

	_ "modernc.org/sqlite"
)

// DB wraps a SQLite database connection
type DB struct {
	conn *sql.DB
	Path string
}

// OpenDB opens a SQLite database with WAL mode and foreign keys enabled
func OpenDB(path string) (*DB, error) {
	conn, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, fmt.Errorf("opening database: %w", err)
	}

	// Enable WAL mode for concurrent reads
	if _, err := conn.Exec("PRAGMA journal_mode=WAL"); err != nil {
		conn.Close()
		return nil, fmt.Errorf("setting WAL mode: %w", err)
	}

	// Enable foreign keys
	if _, err := conn.Exec("PRAGMA foreign_keys=ON"); err != nil {
		conn.Close()
		return nil, fmt.Errorf("enabling foreign keys: %w", err)
	}

	return &DB{conn: conn, Path: path}, nil
}

// Close closes the database connection
func (d *DB) Close() error {
	return d.conn.Close()
}

// Conn returns the underlying sql.DB for custom queries
func (d *DB) Conn() *sql.DB {
	return d.conn
}
