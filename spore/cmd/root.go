package cmd

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/spf13/cobra"
	"mycelica/spore/internal/db"
)

var dbPath string

var rootCmd = &cobra.Command{
	Use:   "spore",
	Short: "Spore graph analysis and orchestration",
}

func Execute() {
	if err := rootCmd.Execute(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func init() {
	rootCmd.PersistentFlags().StringVar(&dbPath, "db", "", "Path to .mycelica.db database")
}

// DiscoverDB finds the database path using priority: env > flag > walk-up > XDG fallback
func DiscoverDB() (string, error) {
	// 1. Environment variable
	if envPath := os.Getenv("MYCELICA_DB"); envPath != "" {
		if _, err := os.Stat(envPath); err == nil {
			return envPath, nil
		}
	}

	// 2. CLI flag
	if dbPath != "" {
		if _, err := os.Stat(dbPath); err == nil {
			return dbPath, nil
		}
		return "", fmt.Errorf("database not found at --db path: %s", dbPath)
	}

	// 3. Walk up from CWD
	dir, err := os.Getwd()
	if err == nil {
		for {
			candidate := filepath.Join(dir, ".mycelica.db")
			if _, err := os.Stat(candidate); err == nil {
				return candidate, nil
			}
			parent := filepath.Dir(dir)
			if parent == dir {
				break
			}
			dir = parent
		}
	}

	// 4. XDG fallback
	home, err := os.UserHomeDir()
	if err == nil {
		xdgPath := filepath.Join(home, ".local", "share", "com.mycelica.app", "mycelica.db")
		if _, err := os.Stat(xdgPath); err == nil {
			return xdgPath, nil
		}
	}

	return "", fmt.Errorf("no .mycelica.db found (set MYCELICA_DB, use --db, or run from a directory containing .mycelica.db)")
}

// OpenDatabase discovers and opens the database
func OpenDatabase() (*db.DB, error) {
	path, err := DiscoverDB()
	if err != nil {
		return nil, err
	}
	return db.OpenDB(path)
}

// ResolveNode finds a node by full ID, ID prefix, or title search.
// Port of team.rs:110-145.
func ResolveNode(d *db.DB, reference string) (*db.Node, error) {
	// 1. Exact ID match
	node, err := d.GetNode(reference)
	if err == nil && node != nil {
		return node, nil
	}

	// 2. ID prefix match (≥6 hex/dash chars)
	if len(reference) >= 6 && isHexDash(reference) {
		matches, err := d.SearchByIDPrefix(reference, 10)
		if err == nil {
			switch len(matches) {
			case 1:
				return &matches[0], nil
			case 0:
				// fall through to FTS
			default:
				lines := make([]string, len(matches))
				for i, m := range matches {
					id := m.ID
					if len(id) > 8 {
						id = id[:8]
					}
					lines[i] = fmt.Sprintf("  %s %s", id, m.Title)
				}
				return nil, fmt.Errorf("ambiguous reference '%s'. %d matches:\n%s\nUse a full node ID instead.",
					reference, len(matches), joinLines(lines))
			}
		}
	}

	// 3. FTS search
	ftsResults, err := d.SearchNodes(reference)
	if err == nil {
		var items []db.Node
		for _, n := range ftsResults {
			if n.IsItem {
				items = append(items, n)
			}
		}
		switch len(items) {
		case 1:
			return &items[0], nil
		case 0:
			// fall through to not found
		default:
			limit := 10
			if len(items) < limit {
				limit = len(items)
			}
			lines := make([]string, limit)
			for i := 0; i < limit; i++ {
				id := items[i].ID
				if len(id) > 8 {
					id = id[:8]
				}
				lines[i] = fmt.Sprintf("  %s %s", id, items[i].Title)
			}
			return nil, fmt.Errorf("ambiguous reference '%s'. %d matches:\n%s\nUse a node ID instead.",
				reference, len(items), joinLines(lines))
		}
	}

	return nil, fmt.Errorf("node not found: %s", reference)
}

func isHexDash(s string) bool {
	for _, c := range s {
		if !((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F') || c == '-') {
			return false
		}
	}
	return true
}

func joinLines(lines []string) string {
	result := ""
	for i, l := range lines {
		if i > 0 {
			result += "\n"
		}
		result += l
	}
	return result
}
