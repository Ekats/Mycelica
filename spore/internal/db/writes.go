package db

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// CreateNodeOpts holds optional fields for node creation
type CreateNodeOpts struct {
	Content   string
	AgentID   string
	NodeClass string // "knowledge", "meta", "operational"
	MetaType  string // "task", "implementation", "summary", "escalation"
	Source    string
	Author    string
}

// CreateNode creates a node via mycelica-cli and returns its UUID.
// Shells out to preserve embedding generation, FTS indexing, and hierarchy processing.
func (d *DB) CreateNode(title string, opts CreateNodeOpts) (string, error) {
	binary, err := FindCLIBinary()
	if err != nil {
		return "", fmt.Errorf("finding CLI binary: %w", err)
	}

	args := []string{"node", "create", "--title", title, "--json", "--db", d.Path}

	if opts.Content != "" {
		args = append(args, "--content", opts.Content)
	}
	if opts.AgentID != "" {
		args = append(args, "--agent-id", opts.AgentID)
	}
	if opts.NodeClass != "" {
		args = append(args, "--node-class", opts.NodeClass)
	}
	if opts.MetaType != "" {
		args = append(args, "--meta-type", opts.MetaType)
	}
	if opts.Source != "" {
		args = append(args, "--source", opts.Source)
	}
	if opts.Author != "" {
		args = append(args, "--author", opts.Author)
	}

	cmd := exec.Command(binary, args...)
	out, err := cmd.Output()
	if err != nil {
		stderr := ""
		if exitErr, ok := err.(*exec.ExitError); ok {
			stderr = strings.TrimSpace(string(exitErr.Stderr))
		}
		return "", fmt.Errorf("creating node: %w (stderr: %s)", err, stderr)
	}

	return parseCreatedID(out)
}

// CreateEdgeOpts holds optional fields for edge creation
type CreateEdgeOpts struct {
	Content    string
	Reason     string
	Agent      string
	Confidence float64
	Metadata   string // JSON string
	Supersedes string
}

// CreateEdge creates an edge via mycelica-cli and returns its UUID.
// Shells out to preserve embedding generation, FTS indexing, and hierarchy processing.
func (d *DB) CreateEdge(sourceID, targetID, edgeType string, opts CreateEdgeOpts) (string, error) {
	binary, err := FindCLIBinary()
	if err != nil {
		return "", fmt.Errorf("finding CLI binary: %w", err)
	}

	args := []string{
		"spore", "create-edge",
		"--from", sourceID,
		"--to", targetID,
		"--type", edgeType,
		"--json", "--db", d.Path,
	}

	if opts.Content != "" {
		args = append(args, "--content", opts.Content)
	}
	if opts.Reason != "" {
		args = append(args, "--reason", opts.Reason)
	}
	if opts.Agent != "" {
		args = append(args, "--agent", opts.Agent)
	}
	if opts.Confidence > 0 {
		args = append(args, "--confidence", fmt.Sprintf("%.2f", opts.Confidence))
	}
	if opts.Metadata != "" {
		args = append(args, "--metadata", opts.Metadata)
	}
	if opts.Supersedes != "" {
		args = append(args, "--supersedes", opts.Supersedes)
	}

	cmd := exec.Command(binary, args...)
	out, err := cmd.Output()
	if err != nil {
		stderr := ""
		if exitErr, ok := err.(*exec.ExitError); ok {
			stderr = strings.TrimSpace(string(exitErr.Stderr))
		}
		return "", fmt.Errorf("creating edge: %w (stderr: %s)", err, stderr)
	}

	return parseCreatedID(out)
}

// DeleteNode deletes a node via mycelica-cli. Edges are cascade-deleted by SQLite.
func (d *DB) DeleteNode(id string) error {
	binary, err := FindCLIBinary()
	if err != nil {
		return fmt.Errorf("finding CLI binary: %w", err)
	}

	cmd := exec.Command(binary, "node", "delete", id, "--db", d.Path)
	out, err := cmd.Output()
	if err != nil {
		stderr := ""
		if exitErr, ok := err.(*exec.ExitError); ok {
			stderr = strings.TrimSpace(string(exitErr.Stderr))
		}
		return fmt.Errorf("deleting node %s: %w (stderr: %s) (stdout: %s)", id, err, stderr, strings.TrimSpace(string(out)))
	}
	return nil
}

// parseCreatedID extracts the "id" field from JSON output like {"id":"<uuid>",...}
func parseCreatedID(output []byte) (string, error) {
	var result map[string]interface{}
	if err := json.Unmarshal(output, &result); err != nil {
		return "", fmt.Errorf("parsing CLI JSON output: %w (raw: %s)", err, strings.TrimSpace(string(output)))
	}

	idVal, ok := result["id"]
	if !ok {
		return "", fmt.Errorf("CLI output missing 'id' field: %s", strings.TrimSpace(string(output)))
	}

	id, ok := idVal.(string)
	if !ok {
		return "", fmt.Errorf("CLI output 'id' field is not a string: %v", idVal)
	}

	return id, nil
}

// FindCLIBinary locates the mycelica-cli binary.
// Search order: MYCELICA_CLI env var, PATH lookup, ~/.cargo/bin/mycelica-cli
func FindCLIBinary() (string, error) {
	// 1. Environment variable override
	if envPath := os.Getenv("MYCELICA_CLI"); envPath != "" {
		if _, err := os.Stat(envPath); err == nil {
			return envPath, nil
		}
	}

	// 2. PATH lookup
	if path, err := exec.LookPath("mycelica-cli"); err == nil {
		return path, nil
	}

	// 3. Default cargo install location
	home, err := os.UserHomeDir()
	if err == nil {
		cargoPath := filepath.Join(home, ".cargo", "bin", "mycelica-cli")
		if _, err := os.Stat(cargoPath); err == nil {
			return cargoPath, nil
		}
	}

	return "", fmt.Errorf("mycelica-cli not found: set MYCELICA_CLI env var, add to PATH, or install via cargo")
}
