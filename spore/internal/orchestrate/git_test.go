package orchestrate

import (
	"os"
	"testing"
)

func TestCaptureGitState_InRepo(t *testing.T) {
	// Use the actual Mycelica repo
	repoDir := "/home/spore/Mycelica"
	if _, err := os.Stat(repoDir + "/.git"); os.IsNotExist(err) {
		t.Skip("not inside a git repo at /home/spore/Mycelica")
	}

	state, err := CaptureGitState(repoDir)
	if err != nil {
		t.Fatalf("CaptureGitState failed: %v", err)
	}

	if state.Branch == "" {
		t.Error("expected non-empty branch")
	}
	if state.Commit == "" {
		t.Error("expected non-empty commit")
	}
	if state.Dirty == nil {
		t.Error("expected non-nil Dirty map")
	}
	if state.Untracked == nil {
		t.Error("expected non-nil Untracked map")
	}
	if state.Hashes == nil {
		t.Error("expected non-nil Hashes map")
	}

	t.Logf("branch=%s commit=%s dirty=%d untracked=%d hashes=%d",
		state.Branch, state.Commit, len(state.Dirty), len(state.Untracked), len(state.Hashes))
}

func TestDiffChangedFiles(t *testing.T) {
	before := &GitState{
		Branch:    "main",
		Commit:    "abc1234",
		Dirty:     map[string]bool{"file_a.go": true},
		Untracked: map[string]bool{"temp.txt": true},
		Hashes:    map[string]string{"file_a.go": "aaa111", "file_b.go": "bbb222"},
	}

	after := &GitState{
		Branch:    "main",
		Commit:    "def5678",
		Dirty:     map[string]bool{"file_a.go": true, "file_c.go": true},
		Untracked: map[string]bool{"temp.txt": true, "new.txt": true},
		Hashes:    map[string]string{"file_a.go": "aaa111", "file_b.go": "ccc333", "file_c.go": "ddd444"},
	}

	changed := DiffChangedFiles(before, after)

	expected := map[string]bool{
		"file_c.go": true, // newly dirty
		"new.txt":   true, // newly untracked
		"file_b.go": true, // hash changed
	}

	if len(changed) != len(expected) {
		t.Fatalf("expected %d changed files, got %d: %v", len(expected), len(changed), changed)
	}

	for _, f := range changed {
		if !expected[f] {
			t.Errorf("unexpected changed file: %s", f)
		}
	}

	// Verify sorted order
	for i := 1; i < len(changed); i++ {
		if changed[i] < changed[i-1] {
			t.Errorf("result not sorted: %v", changed)
			break
		}
	}
}

func TestDiffChangedFiles_Empty(t *testing.T) {
	state := &GitState{
		Dirty:     map[string]bool{},
		Untracked: map[string]bool{},
		Hashes:    map[string]string{},
	}

	changed := DiffChangedFiles(state, state)
	if len(changed) != 0 {
		t.Errorf("expected no changes, got: %v", changed)
	}
}

func TestCaptureFileHashes_NonexistentFile(t *testing.T) {
	repoDir := "/home/spore/Mycelica"
	if _, err := os.Stat(repoDir + "/.git"); os.IsNotExist(err) {
		t.Skip("not inside a git repo at /home/spore/Mycelica")
	}

	files := map[string]bool{
		"this-file-definitely-does-not-exist-abc123.txt": true,
	}

	hashes := CaptureFileHashes(repoDir, files)
	if len(hashes) != 0 {
		t.Errorf("expected empty hashes for nonexistent file, got: %v", hashes)
	}
}
