package orchestrate

import (
	"os"
	"path/filepath"
	"testing"
)

func TestReadTasks_File(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "tasks.txt")
	content := `# This is a comment
Implement feature A

# Another comment
Fix bug in parser
Add unit tests for module X
`
	if err := os.WriteFile(path, []byte(content), 0644); err != nil {
		t.Fatalf("writing temp file: %v", err)
	}

	tasks, err := ReadTasks(path)
	if err != nil {
		t.Fatalf("ReadTasks: %v", err)
	}

	if len(tasks) != 3 {
		t.Fatalf("expected 3 tasks, got %d: %v", len(tasks), tasks)
	}
	if tasks[0] != "Implement feature A" {
		t.Errorf("task[0] = %q, want %q", tasks[0], "Implement feature A")
	}
	if tasks[1] != "Fix bug in parser" {
		t.Errorf("task[1] = %q, want %q", tasks[1], "Fix bug in parser")
	}
	if tasks[2] != "Add unit tests for module X" {
		t.Errorf("task[2] = %q, want %q", tasks[2], "Add unit tests for module X")
	}
}

func TestReadTasks_MultiLine(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "tasks.txt")
	content := `Implement feature A
with multiple lines of description
---
Fix bug in parser
also multi-line
---
`
	if err := os.WriteFile(path, []byte(content), 0644); err != nil {
		t.Fatalf("writing temp file: %v", err)
	}

	tasks, err := ReadTasks(path)
	if err != nil {
		t.Fatalf("ReadTasks: %v", err)
	}

	if len(tasks) != 2 {
		t.Fatalf("expected 2 tasks, got %d: %v", len(tasks), tasks)
	}
	if tasks[0] != "Implement feature A with multiple lines of description" {
		t.Errorf("task[0] = %q", tasks[0])
	}
	if tasks[1] != "Fix bug in parser also multi-line" {
		t.Errorf("task[1] = %q", tasks[1])
	}
}

func TestReadTasks_Empty(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "empty.txt")
	content := `# only comments

# nothing here
`
	if err := os.WriteFile(path, []byte(content), 0644); err != nil {
		t.Fatalf("writing temp file: %v", err)
	}

	_, err := ReadTasks(path)
	if err == nil {
		t.Fatal("expected error for empty task file")
	}
	if got := err.Error(); !contains(got, "no tasks found") {
		t.Errorf("error = %q, want to contain 'no tasks found'", got)
	}
}

func TestReadTasks_NonExistent(t *testing.T) {
	_, err := ReadTasks("/tmp/nonexistent-spore-task-file-xyz.txt")
	if err == nil {
		t.Fatal("expected error for nonexistent file")
	}
}

func TestLoopState_Persistence(t *testing.T) {
	dir := t.TempDir()
	statePath := filepath.Join(dir, "test.loop-state.json")

	// Create initial state
	state := newLoopState(statePath, "test-source.txt")
	state.recordResult(&LoopTaskResult{
		Task:   "task one",
		Status: "verified",
		Cost:   1.50,
	})
	if err := state.save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	// Verify the task is marked as verified
	if !state.isVerified("task one") {
		t.Error("expected 'task one' to be verified")
	}
	if state.isVerified("task two") {
		t.Error("expected 'task two' to NOT be verified")
	}

	// Reload from disk
	loaded := loadLoopState(statePath, "test-source.txt")
	if !loaded.isVerified("task one") {
		t.Error("after reload, expected 'task one' to still be verified")
	}
	if loaded.TotalCost != 1.50 {
		t.Errorf("after reload, total_cost = %f, want 1.50", loaded.TotalCost)
	}
	if len(loaded.Runs) != 1 {
		t.Errorf("after reload, runs = %d, want 1", len(loaded.Runs))
	}
}

func TestLoopState_Reset(t *testing.T) {
	dir := t.TempDir()
	statePath := filepath.Join(dir, "test.loop-state.json")

	// Create and save state
	state := newLoopState(statePath, "test-source.txt")
	state.recordResult(&LoopTaskResult{
		Task:   "task one",
		Status: "verified",
		Cost:   2.00,
	})
	if err := state.save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	// Delete the state file (simulating --reset)
	if err := os.Remove(statePath); err != nil {
		t.Fatalf("removing state file: %v", err)
	}

	// Load should return fresh state
	loaded := loadLoopState(statePath, "test-source.txt")
	if loaded.isVerified("task one") {
		t.Error("after reset, 'task one' should not be verified")
	}
	if loaded.TotalCost != 0 {
		t.Errorf("after reset, total_cost = %f, want 0", loaded.TotalCost)
	}
}

func TestShouldExcludeFile(t *testing.T) {
	tests := []struct {
		path string
		want bool
	}{
		{"target/foo", true},
		{"target/release/binary", true},
		{"src/main.rs", false},
		{".env", true},
		{".env.local", true},
		{"foo.db", true},
		{"data.db-journal", true},
		{"data.db-wal", true},
		{"node_modules/x/y.js", true},
		{".git/config", false}, // .git/ is handled by git itself
		{"src/lib.rs", false},
		{"debug.log", true},
		{"Cargo.lock", true},
		{"package-lock.json", false},
		{"tasks.loop-state.json", true},
		{"README.md", false},
	}

	for _, tt := range tests {
		t.Run(tt.path, func(t *testing.T) {
			got := ShouldExcludeFile(tt.path)
			if got != tt.want {
				t.Errorf("ShouldExcludeFile(%q) = %v, want %v", tt.path, got, tt.want)
			}
		})
	}
}

func TestLoopStatePath(t *testing.T) {
	tests := []struct {
		source string
		want   string
	}{
		{"/tmp/tasks.txt", "/tmp/tasks.loop-state.json"},
		{"/home/user/my-tasks.md", "/home/user/my-tasks.loop-state.json"},
		{"file:/tmp/tasks.txt", "/tmp/tasks.loop-state.json"},
	}

	for _, tt := range tests {
		t.Run(tt.source, func(t *testing.T) {
			got := loopStatePath(tt.source)
			if got != tt.want {
				t.Errorf("loopStatePath(%q) = %q, want %q", tt.source, got, tt.want)
			}
		})
	}
}

func TestRunLoop_DryRun(t *testing.T) {
	d := openTestDB(t)
	defer d.Close()

	// Create a temp task file
	dir := t.TempDir()
	taskFile := filepath.Join(dir, "tasks.txt")
	content := "Implement feature A\nFix bug in parser\n"
	if err := os.WriteFile(taskFile, []byte(content), 0644); err != nil {
		t.Fatalf("writing task file: %v", err)
	}

	config := LoopConfig{
		Source:  taskFile,
		Budget:  10.0,
		MaxRuns: 50,
		OrchConfig: OrchestrationConfig{
			TaskFile:   DefaultTaskFileConfig(),
			MaxBounces: 3,
			MaxTurns:   50,
			DryRun:     true,
			OutputDir:  dir,
		},
	}

	result, err := RunLoop(d, config)
	if err != nil {
		t.Fatalf("RunLoop DryRun: %v", err)
	}

	if result.TotalCost != 0 {
		t.Errorf("dry run cost = %f, want 0", result.TotalCost)
	}
	if len(result.Tasks) != 0 {
		t.Errorf("dry run tasks = %d, want 0 (no agents spawned)", len(result.Tasks))
	}
}

func TestParseTaskContent(t *testing.T) {
	tests := []struct {
		name    string
		content string
		want    int
	}{
		{"simple", "task1\ntask2\ntask3\n", 3},
		{"comments", "# comment\ntask1\n# another\ntask2\n", 2},
		{"blank_lines", "\n\ntask1\n\ntask2\n\n", 2},
		{"delimiter", "line1\nline2\n---\nline3\n", 2},
		{"empty", "", 0},
		{"only_comments", "# comment\n# another\n", 0},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			tasks := parseTaskContent(tt.content)
			if len(tasks) != tt.want {
				t.Errorf("parseTaskContent(%q) returned %d tasks, want %d: %v",
					tt.name, len(tasks), tt.want, tasks)
			}
		})
	}
}

// contains is a test helper.
func contains(s, substr string) bool {
	return len(s) >= len(substr) && searchSubstring(s, substr)
}

func searchSubstring(s, substr string) bool {
	for i := 0; i+len(substr) <= len(s); i++ {
		if s[i:i+len(substr)] == substr {
			return true
		}
	}
	return false
}
