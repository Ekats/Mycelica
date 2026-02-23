package db

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestParseCreatedID_Valid(t *testing.T) {
	input := []byte(`{"id":"abc-123","title":"test"}`)
	got, err := parseCreatedID(input)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if got != "abc-123" {
		t.Errorf("got %q, want %q", got, "abc-123")
	}
}

func TestParseCreatedID_NoID(t *testing.T) {
	input := []byte(`{"title":"test"}`)
	_, err := parseCreatedID(input)
	if err == nil {
		t.Fatal("expected error for missing id field")
	}
	if !strings.Contains(err.Error(), "missing 'id' field") {
		t.Errorf("error should mention missing id field, got: %v", err)
	}
}

func TestParseCreatedID_EmptyJSON(t *testing.T) {
	input := []byte(`{}`)
	_, err := parseCreatedID(input)
	if err == nil {
		t.Fatal("expected error for empty JSON object")
	}
	if !strings.Contains(err.Error(), "missing 'id' field") {
		t.Errorf("error should mention missing id field, got: %v", err)
	}
}

func TestParseCreatedID_InvalidJSON(t *testing.T) {
	input := []byte(`not json`)
	_, err := parseCreatedID(input)
	if err == nil {
		t.Fatal("expected error for invalid JSON")
	}
	if !strings.Contains(err.Error(), "parsing CLI JSON output") {
		t.Errorf("error should mention parsing, got: %v", err)
	}
}

func TestParseCreatedID_IDNotString(t *testing.T) {
	input := []byte(`{"id":42}`)
	_, err := parseCreatedID(input)
	if err == nil {
		t.Fatal("expected error for non-string id")
	}
	if !strings.Contains(err.Error(), "not a string") {
		t.Errorf("error should mention not a string, got: %v", err)
	}
}

func TestFindCLIBinary(t *testing.T) {
	path, err := FindCLIBinary()
	if err != nil {
		t.Skipf("mycelica-cli not found: %v", err)
	}
	if path == "" {
		t.Fatal("FindCLIBinary returned empty path")
	}
	// Verify the file actually exists
	if _, err := os.Stat(path); err != nil {
		t.Errorf("returned path does not exist: %s", path)
	}
}

func TestFindCLIBinary_EnvOverride(t *testing.T) {
	// Set env to a known binary
	realPath, err := FindCLIBinary()
	if err != nil {
		t.Skipf("mycelica-cli not found: %v", err)
	}

	t.Setenv("MYCELICA_CLI", realPath)
	got, err := FindCLIBinary()
	if err != nil {
		t.Fatalf("unexpected error with env override: %v", err)
	}
	if got != realPath {
		t.Errorf("got %q, want %q", got, realPath)
	}
}

func TestFindCLIBinary_EnvNonexistent(t *testing.T) {
	t.Setenv("MYCELICA_CLI", "/nonexistent/path/mycelica-cli")
	// Should fall through to other methods
	path, err := FindCLIBinary()
	if err != nil {
		// Acceptable if CLI isn't installed via other methods either
		t.Skipf("CLI not found via fallback: %v", err)
	}
	// If found via fallback, it should not be the nonexistent path
	if path == "/nonexistent/path/mycelica-cli" {
		t.Error("should not have returned the nonexistent env path")
	}
}

// findTestDB walks up from the working directory to find .mycelica.db
func findTestDB(t *testing.T) string {
	t.Helper()
	dir, err := os.Getwd()
	if err != nil {
		t.Skipf("cannot get working directory: %v", err)
	}
	for {
		dbPath := filepath.Join(dir, ".mycelica.db")
		if _, err := os.Stat(dbPath); err == nil {
			return dbPath
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}
	t.Skip("database not found (.mycelica.db)")
	return ""
}

func TestCreateNode_Integration(t *testing.T) {
	if _, err := FindCLIBinary(); err != nil {
		t.Skip("mycelica-cli not found")
	}
	dbPath := findTestDB(t)
	d, err := OpenDB(dbPath)
	if err != nil {
		t.Fatalf("opening database: %v", err)
	}
	defer d.Close()

	id, err := d.CreateNode("go-port-test-node", CreateNodeOpts{
		Content:   "Integration test node -- safe to delete",
		AgentID:   "spore:test",
		NodeClass: "operational",
		MetaType:  "task",
		Source:    "writes_test.go",
		Author:    "test",
	})
	if err != nil {
		t.Fatalf("CreateNode failed: %v", err)
	}

	// Verify UUID-like format (8-4-4-4-12 hex)
	if len(id) < 32 {
		t.Errorf("ID too short to be UUID: %q", id)
	}
	if !strings.Contains(id, "-") {
		t.Errorf("ID does not look like UUID: %q", id)
	}

	t.Logf("Created node: %s", id)

	// Clean up
	if err := d.DeleteNode(id); err != nil {
		t.Errorf("cleanup failed -- manually delete node %s: %v", id, err)
	}
}

func TestCreateEdge_Integration(t *testing.T) {
	if _, err := FindCLIBinary(); err != nil {
		t.Skip("mycelica-cli not found")
	}
	dbPath := findTestDB(t)
	d, err := OpenDB(dbPath)
	if err != nil {
		t.Fatalf("opening database: %v", err)
	}
	defer d.Close()

	// Create two nodes
	nodeA, err := d.CreateNode("go-port-test-edge-source", CreateNodeOpts{
		AgentID: "spore:test",
		Source:  "writes_test.go",
	})
	if err != nil {
		t.Fatalf("CreateNode (source) failed: %v", err)
	}

	nodeB, err := d.CreateNode("go-port-test-edge-target", CreateNodeOpts{
		AgentID: "spore:test",
		Source:  "writes_test.go",
	})
	if err != nil {
		// Clean up nodeA before failing
		d.DeleteNode(nodeA)
		t.Fatalf("CreateNode (target) failed: %v", err)
	}

	// Create edge between them
	edgeID, err := d.CreateEdge(nodeA, nodeB, "supports", CreateEdgeOpts{
		Content:    "Test edge content",
		Reason:     "integration test",
		Agent:      "spore:test",
		Confidence: 0.85,
		Metadata:   `{"test":true}`,
	})
	if err != nil {
		d.DeleteNode(nodeA)
		d.DeleteNode(nodeB)
		t.Fatalf("CreateEdge failed: %v", err)
	}

	if len(edgeID) < 32 {
		t.Errorf("Edge ID too short to be UUID: %q", edgeID)
	}
	if !strings.Contains(edgeID, "-") {
		t.Errorf("Edge ID does not look like UUID: %q", edgeID)
	}

	t.Logf("Created edge: %s (from %s to %s)", edgeID, nodeA, nodeB)

	// Clean up -- deleting nodes cascades to edges
	if err := d.DeleteNode(nodeA); err != nil {
		t.Errorf("cleanup nodeA failed: %v", err)
	}
	if err := d.DeleteNode(nodeB); err != nil {
		t.Errorf("cleanup nodeB failed: %v", err)
	}
}
