package orchestrate

import (
	"testing"
	"time"

	"mycelica/spore/internal/db"
)

func TestToolPermissions(t *testing.T) {
	tests := []struct {
		role            AgentRole
		wantAllowed     string
		wantDisallowed  string
	}{
		{RoleCoder, "Read,Write,Edit,Bash,mcp__mycelica__*", "Grep,Glob"},
		{RoleVerifier, "Read,Grep,Glob,Bash,mcp__mycelica__*", ""},
		{RoleSummarizer, "mcp__mycelica__*", "Bash,Edit,Write"},
		{RoleOperator, "", ""},
	}

	for _, tt := range tests {
		t.Run(string(tt.role), func(t *testing.T) {
			allowed, disallowed := ToolPermissions(tt.role)
			if allowed != tt.wantAllowed {
				t.Errorf("allowed = %q, want %q", allowed, tt.wantAllowed)
			}
			if disallowed != tt.wantDisallowed {
				t.Errorf("disallowed = %q, want %q", disallowed, tt.wantDisallowed)
			}
		})
	}
}

func TestTruncateTitle(t *testing.T) {
	tests := []struct {
		name string
		s    string
		max  int
		want string
	}{
		{"short", "hello", 10, "hello"},
		{"exact", "hello", 5, "hello"},
		{"long", "hello world", 5, "hello"},
		{"empty", "", 5, ""},
		{"zero max", "hello", 0, ""},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := TruncateTitle(tt.s, tt.max)
			if got != tt.want {
				t.Errorf("TruncateTitle(%q, %d) = %q, want %q", tt.s, tt.max, got, tt.want)
			}
		})
	}
}

func TestFindProjectRoot(t *testing.T) {
	// Run from somewhere under /home/spore/Mycelica which has .git
	root := FindProjectRoot("/home/spore/Mycelica/spore/internal/orchestrate")
	if root != "/home/spore/Mycelica" {
		t.Errorf("FindProjectRoot = %q, want /home/spore/Mycelica", root)
	}
}

func TestRunOrchestration_DryRun(t *testing.T) {
	d := openTestDB(t)
	defer d.Close()

	config := DefaultOrchestrationConfig()
	config.DryRun = true
	config.OutputDir = t.TempDir()

	result, err := RunOrchestration(d, "test dry run task for orchestration", config)
	if err != nil {
		t.Fatalf("RunOrchestration DryRun failed: %v", err)
	}

	if result.TaskNodeID == "" {
		t.Error("expected non-empty task node ID")
	}
	if result.RunID == "" {
		t.Error("expected non-empty run ID")
	}
	if result.Status != StatusSuccess {
		t.Errorf("expected status success, got %s", result.Status)
	}

	// Clean up: delete the task node
	if result.TaskNodeID != "" {
		if err := d.DeleteNode(result.TaskNodeID); err != nil {
			t.Logf("warning: cleanup failed: %v", err)
		}
	}
}

func TestRecordRunStatus(t *testing.T) {
	d := openTestDB(t)
	defer d.Close()

	// Create a test node
	nodeID, err := d.CreateNode("test-record-run-status", db.CreateNodeOpts{
		AgentID:   "spore:test",
		NodeClass: "operational",
		MetaType:  "task",
		Source:    "test",
	})
	if err != nil {
		t.Fatalf("creating test node: %v", err)
	}
	defer d.DeleteNode(nodeID)

	// Record run status
	mockResult := &ClaudeResult{
		ExitCode: 0,
		CostUSD:  1.23,
		NumTurns: 5,
		Duration: 30 * time.Second,
	}
	RecordRunStatus(d, nodeID, "test-run-id-12345678", "coder", "success", mockResult, "test-experiment")

	// Verify the Tracks edge exists
	edges, err := d.GetEdgesForNode(nodeID)
	if err != nil {
		t.Fatalf("querying edges: %v", err)
	}

	found := false
	for _, e := range edges {
		if e.EdgeType == "tracks" && e.SourceID == nodeID && e.TargetID == nodeID {
			found = true
			break
		}
	}
	if !found {
		t.Error("expected to find a self-referential tracks edge")
	}
}

func TestCreateEscalation(t *testing.T) {
	d := openTestDB(t)
	defer d.Close()

	// Create a test task node
	taskNodeID, err := d.CreateNode("test-create-escalation-task", db.CreateNodeOpts{
		AgentID:   "spore:test",
		NodeClass: "operational",
		MetaType:  "task",
		Source:    "test",
	})
	if err != nil {
		t.Fatalf("creating test task node: %v", err)
	}
	defer d.DeleteNode(taskNodeID)

	// Create escalation
	CreateEscalation(d, taskNodeID, "fake-impl-id", 3, "test escalation task")

	// Verify escalation node + edge exists
	edges, err := d.GetEdgesForNode(taskNodeID)
	if err != nil {
		t.Fatalf("querying edges: %v", err)
	}

	var escalationNodeID string
	for _, e := range edges {
		if e.EdgeType == "tracks" && e.TargetID == taskNodeID && e.SourceID != taskNodeID {
			escalationNodeID = e.SourceID
			break
		}
	}

	if escalationNodeID == "" {
		t.Error("expected to find an escalation tracks edge targeting the task node")
	} else {
		// Clean up escalation node
		defer d.DeleteNode(escalationNodeID)

		// Verify the escalation node exists
		node, err := d.GetNode(escalationNodeID)
		if err != nil {
			t.Fatalf("querying escalation node: %v", err)
		}
		if node == nil {
			t.Error("escalation node not found")
		}
	}
}
