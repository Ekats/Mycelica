package orchestrate

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
)

// CaptureGitState snapshots the current repository state: branch, commit, dirty files,
// untracked files, and content hashes. Used before/after agent runs to detect changes.
func CaptureGitState(repoDir string) (*GitState, error) {
	branch, err := gitOutput(repoDir, "rev-parse", "--abbrev-ref", "HEAD")
	if err != nil {
		return nil, fmt.Errorf("git rev-parse --abbrev-ref HEAD: %w", err)
	}

	commit, err := gitOutput(repoDir, "rev-parse", "--short", "HEAD")
	if err != nil {
		return nil, fmt.Errorf("git rev-parse --short HEAD: %w", err)
	}

	// Staged changes
	stagedOut, err := gitOutput(repoDir, "diff", "--name-only", "--cached")
	if err != nil {
		return nil, fmt.Errorf("git diff --name-only --cached: %w", err)
	}

	// Unstaged changes
	unstagedOut, err := gitOutput(repoDir, "diff", "--name-only")
	if err != nil {
		return nil, fmt.Errorf("git diff --name-only: %w", err)
	}

	// Untracked files
	untrackedOut, err := gitOutput(repoDir, "ls-files", "--others", "--exclude-standard")
	if err != nil {
		return nil, fmt.Errorf("git ls-files --others: %w", err)
	}

	dirty := parseFileList(stagedOut)
	for f := range parseFileList(unstagedOut) {
		dirty[f] = true
	}

	untracked := parseFileList(untrackedOut)

	// Union of dirty + untracked for hashing
	hashSet := make(map[string]bool, len(dirty)+len(untracked))
	for f := range dirty {
		hashSet[f] = true
	}
	for f := range untracked {
		hashSet[f] = true
	}

	hashes := CaptureFileHashes(repoDir, hashSet)

	return &GitState{
		Branch:    branch,
		Commit:    commit,
		Dirty:     dirty,
		Untracked: untracked,
		Hashes:    hashes,
	}, nil
}

// CaptureFileHashes computes git content hashes for a set of files.
// Files that fail to hash (deleted, inaccessible) are silently skipped.
func CaptureFileHashes(repoDir string, files map[string]bool) map[string]string {
	hashes := make(map[string]string, len(files))
	for f := range files {
		absPath := filepath.Join(repoDir, f)
		hash, err := gitOutput(repoDir, "hash-object", absPath)
		if err != nil {
			continue // file deleted or inaccessible
		}
		if hash != "" {
			hashes[f] = hash
		}
	}
	return hashes
}

// DiffChangedFiles compares two GitState snapshots and returns a sorted, deduplicated
// list of files that changed between them. Detects new dirty files, new untracked files,
// and files whose content hash changed.
func DiffChangedFiles(before, after *GitState) []string {
	seen := make(map[string]bool)

	// Files newly dirty (in after but not in before)
	for f := range after.Dirty {
		if !before.Dirty[f] {
			seen[f] = true
		}
	}

	// Files newly untracked (in after but not in before)
	for f := range after.Untracked {
		if !before.Untracked[f] {
			seen[f] = true
		}
	}

	// Files whose hash changed
	for f, afterHash := range after.Hashes {
		beforeHash, exists := before.Hashes[f]
		if !exists || beforeHash != afterHash {
			seen[f] = true
		}
	}

	result := make([]string, 0, len(seen))
	for f := range seen {
		result = append(result, f)
	}
	sort.Strings(result)
	return result
}

// gitOutput runs a git command in repoDir and returns trimmed stdout.
func gitOutput(repoDir string, args ...string) (string, error) {
	cmd := exec.Command("git", args...)
	cmd.Dir = repoDir
	out, err := cmd.Output()
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(out)), nil
}

// parseFileList splits newline-separated git output into a set of file paths.
func parseFileList(output string) map[string]bool {
	result := make(map[string]bool)
	if output == "" {
		return result
	}
	for _, line := range strings.Split(output, "\n") {
		line = strings.TrimSpace(line)
		if line != "" {
			result[line] = true
		}
	}
	return result
}
