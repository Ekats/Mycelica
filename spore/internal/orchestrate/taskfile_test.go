package orchestrate

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"mycelica/spore/internal/db"
)

// openTestDB walks up from the test directory to find .mycelica.db.
func openTestDB(t *testing.T) *db.DB {
	t.Helper()
	dir, _ := os.Getwd()
	for {
		candidate := filepath.Join(dir, ".mycelica.db")
		if _, err := os.Stat(candidate); err == nil {
			d, err := db.OpenDB(candidate)
			if err != nil {
				t.Skipf("cannot open DB: %v", err)
			}
			return d
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			t.Skip("no .mycelica.db found")
		}
		dir = parent
	}
}

func TestGenerateTaskFile_DryRun(t *testing.T) {
	d := openTestDB(t)
	defer d.Close()

	tmpDir := t.TempDir()

	path, contextCount, err := GenerateTaskFile(
		d,
		"fix the login bug in the authentication module",
		RoleCoder,
		"abcdef12-3456-7890-abcd-ef1234567890",
		"test-task-node-id",
		0, 3,
		"", VerdictUnknown,
		DefaultTaskFileConfig(),
		tmpDir,
	)
	if err != nil {
		t.Fatalf("GenerateTaskFile failed: %v", err)
	}

	// Verify file was created
	if _, err := os.Stat(path); os.IsNotExist(err) {
		t.Fatalf("task file not created at %s", path)
	}

	content, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("failed to read task file: %v", err)
	}
	md := string(content)

	// Verify expected sections exist
	requiredSections := []string{
		"# Task:",
		"## Task",
		"## Graph Context",
		"## Checklist",
		"**Run:**",
		"**Agent:** coder",
		"**Bounce:** 1/3",
	}
	for _, section := range requiredSections {
		if !strings.Contains(md, section) {
			t.Errorf("missing section %q in task file", section)
		}
	}

	t.Logf("Generated task file: %s (%d context nodes, %d bytes)", path, contextCount, len(content))
}

func TestFindAnchors_WithDB(t *testing.T) {
	d := openTestDB(t)
	defer d.Close()

	anchors, err := findAnchors(d, "clustering algorithm implementation", "nonexistent-task-id", DefaultTaskFileConfig())
	if err != nil {
		t.Fatalf("findAnchors failed: %v", err)
	}

	// Should find something in a populated DB (FTS at minimum)
	if len(anchors) == 0 {
		t.Log("warning: no anchors found -- DB may lack relevant content")
	} else {
		t.Logf("found %d anchors:", len(anchors))
		for _, a := range anchors {
			t.Logf("  [%s] %s (score=%.3f, source=%s)", a.ID[:8], a.Title, a.Score, a.Source)
		}
	}
}

func TestFindAnchors_EmptyQuery(t *testing.T) {
	d := openTestDB(t)
	defer d.Close()

	anchors, err := findAnchors(d, "", "nonexistent-task-id", DefaultTaskFileConfig())
	if err != nil {
		t.Fatalf("findAnchors with empty query should not error: %v", err)
	}
	// Empty query should produce no FTS results; semantic may still work if task node has embedding
	t.Logf("found %d anchors for empty query", len(anchors))
}

func TestRenderMarkdown_Basic(t *testing.T) {
	context := []contextRow{
		{Rank: 1, NodeID: "abc123456789", Title: "TestFunction", Relevance: 0.85, Via: "direct", Anchor: "search"},
		{Rank: 2, NodeID: "def987654321", Title: "AnotherNode", Relevance: 0.60, Via: "supports -> derives_from", Anchor: "TestFunction"},
	}
	lessons := []lesson{
		{Title: "Lesson: Always check nil", Summary: "Check for nil before dereferencing pointers.", Fix: "Add nil guard."},
	}

	md := renderMarkdown(nil, "implement the search feature", RoleCoder,
		"run12345678", "task-node-id",
		0, 3, "", VerdictUnknown,
		nil, context, lessons)

	requiredSections := []string{
		"# Task: implement the search feature",
		"## Task",
		"## Graph Context",
		"| # | Node | ID | Relevance | Via |",
		"| 1 | TestFunction",
		"| 2 | AnotherNode",
		"85%",
		"60%",
		"## Lessons from Past Runs",
		"Always check nil",
		"**Fix:** Add nil guard.",
		"## Checklist",
	}
	for _, section := range requiredSections {
		if !strings.Contains(md, section) {
			t.Errorf("missing section %q in rendered markdown", section)
		}
	}
}

func TestRenderMarkdown_Verifier(t *testing.T) {
	md := renderMarkdown(nil, "verify the implementation", RoleVerifier,
		"run12345678", "task-node-id",
		0, 3, "impl-node-12345678", VerdictUnknown,
		nil, nil, nil)

	if !strings.Contains(md, "## Implementation to Check") {
		t.Error("verifier task file should contain 'Implementation to Check' section")
	}
	if !strings.Contains(md, "impl-node-12345678") {
		t.Error("verifier task file should reference the implementation node ID")
	}
	// Should NOT contain "Previous Bounce" since this is a verifier
	if strings.Contains(md, "## Previous Bounce") {
		t.Error("verifier task file should not contain 'Previous Bounce' section")
	}
}

func TestRenderMarkdown_Summarizer(t *testing.T) {
	md := renderMarkdown(nil, "summarize the changes", RoleSummarizer,
		"run12345678", "task-node-id",
		0, 3, "impl-node-12345678", VerdictSupports,
		nil, nil, nil)

	if !strings.Contains(md, "## Implementation to Summarize") {
		t.Error("summarizer task file should contain 'Implementation to Summarize' section")
	}
	if !strings.Contains(md, "impl-node-12345678") {
		t.Error("summarizer task file should reference the implementation node ID")
	}
}

func TestRenderMarkdown_Bounce(t *testing.T) {
	md := renderMarkdown(nil, "fix the bug again", RoleCoder,
		"run12345678", "task-node-id",
		1, 3, "impl-node-failed", VerdictContradicts,
		nil, nil, nil)

	if !strings.Contains(md, "## Previous Bounce") {
		t.Error("bounce task file should contain 'Previous Bounce' section")
	}
	if !strings.Contains(md, "Verifier found issues") {
		t.Error("bounce with contradicts verdict should mention verifier issues")
	}
	if !strings.Contains(md, "impl-node-failed") {
		t.Error("bounce task file should reference the failed implementation node")
	}
	if !strings.Contains(md, "Bounce:** 2/3") {
		t.Errorf("expected bounce 2/3, got different value")
	}
}

func TestRenderMarkdown_BounceUnknownVerdict(t *testing.T) {
	md := renderMarkdown(nil, "fix the bug again", RoleCoder,
		"run12345678", "task-node-id",
		1, 3, "impl-node-failed", VerdictUnknown,
		nil, nil, nil)

	if !strings.Contains(md, "could not parse a verdict") {
		t.Error("bounce with unknown verdict should mention parse failure")
	}
}

func TestRenderMarkdown_EmptyContext(t *testing.T) {
	md := renderMarkdown(nil, "do something", RoleCoder,
		"run12345678", "task-node-id",
		0, 3, "", VerdictUnknown,
		nil, nil, nil)

	if !strings.Contains(md, "_No relevant nodes found in the graph._") {
		t.Error("empty context should show 'no relevant nodes' message")
	}
	// Should still have header and checklist
	if !strings.Contains(md, "# Task:") {
		t.Error("empty context should still have header")
	}
	if !strings.Contains(md, "## Checklist") {
		t.Error("empty context should still have checklist")
	}
}

func TestRenderMarkdown_CodeSnippets(t *testing.T) {
	codeContext := []contextRow{
		{
			Rank:      1,
			NodeID:    "code-abc123",
			Title:     "fn handle_request",
			Relevance: 0.9,
			Via:       "direct",
			Anchor:    "search",
			Tags:      `{"file_path":"src/server.rs","line_start":10,"line_end":50,"language":"rust"}`,
			IsCode:    true,
		},
		{
			Rank:      2,
			NodeID:    "code-def456",
			Title:     "struct Config",
			Relevance: 0.7,
			Via:       "defined_in",
			Anchor:    "handle_request",
			Tags:      `{"file_path":"src/config.rs","line_start":1,"line_end":20,"language":"rust"}`,
			IsCode:    true,
		},
	}

	md := renderMarkdown(nil, "update the server", RoleCoder,
		"run12345678", "task-node-id",
		0, 3, "", VerdictUnknown,
		nil, codeContext, nil)

	// Should have Code Locations
	if !strings.Contains(md, "### Code Locations") {
		t.Error("should have Code Locations section")
	}
	if !strings.Contains(md, "`src/server.rs` L10-50") {
		t.Error("should list server.rs code location")
	}

	// Should have Files Likely Touched
	if !strings.Contains(md, "### Files Likely Touched") {
		t.Error("should have Files Likely Touched section")
	}
	if !strings.Contains(md, "`src/server.rs`") {
		t.Error("should list server.rs in files likely touched")
	}
}

func TestFindLessons_NoEmbedding(t *testing.T) {
	d := openTestDB(t)
	defer d.Close()

	// Use a non-existent task node ID so there's no embedding
	lessons := findLessons(d, "some task", "nonexistent-node-id", DefaultTaskFileConfig())
	// Should not panic; may return empty or recency-based results
	t.Logf("found %d lessons for non-existent node", len(lessons))
}

func TestParseCodeTags(t *testing.T) {
	tests := []struct {
		name     string
		input    string
		wantPath string
		wantLang string
	}{
		{
			name:     "valid tags",
			input:    `{"file_path":"src/foo.rs","line_start":10,"line_end":50,"language":"rust"}`,
			wantPath: "src/foo.rs",
			wantLang: "rust",
		},
		{
			name:     "empty string",
			input:    "",
			wantPath: "",
		},
		{
			name:     "invalid JSON",
			input:    "not json",
			wantPath: "",
		},
		{
			name:     "no file_path",
			input:    `{"language":"go"}`,
			wantPath: "",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			ct := parseCodeTags(tt.input)
			if ct.FilePath != tt.wantPath {
				t.Errorf("got FilePath=%q, want %q", ct.FilePath, tt.wantPath)
			}
			if tt.wantLang != "" && ct.Language != tt.wantLang {
				t.Errorf("got Language=%q, want %q", ct.Language, tt.wantLang)
			}
		})
	}
}

func TestIsFunctionTitle(t *testing.T) {
	tests := []struct {
		title string
		want  bool
	}{
		{"fn handle_request", true},
		{"pub fn new", true},
		{"pub(crate) fn parse", true},
		{"async fn fetch", true},
		{"pub async fn run", true},
		{"func main", true},
		{"function render", true},
		{"export function doSomething", true},
		{"struct Config", false},
		{"enum Status", false},
		{"type Foo = Bar", false},
		{"", false},
	}

	for _, tt := range tests {
		t.Run(tt.title, func(t *testing.T) {
			got := isFunctionTitle(tt.title)
			if got != tt.want {
				t.Errorf("isFunctionTitle(%q) = %v, want %v", tt.title, got, tt.want)
			}
		})
	}
}

func TestLangFromExtension(t *testing.T) {
	tests := []struct {
		path string
		want string
	}{
		{"src/main.rs", "rust"},
		{"lib/foo.ts", "typescript"},
		{"app.tsx", "typescript"},
		{"index.js", "javascript"},
		{"module.mjs", "javascript"},
		{"script.py", "python"},
		{"main.go", "go"},
		{"file.c", "c"},
		{"file.cpp", "cpp"},
		{"Main.java", "java"},
		{"README.md", "markdown"},
		{"data.json", ""},
	}

	for _, tt := range tests {
		t.Run(tt.path, func(t *testing.T) {
			got := langFromExtension(tt.path)
			if got != tt.want {
				t.Errorf("langFromExtension(%q) = %q, want %q", tt.path, got, tt.want)
			}
		})
	}
}

func TestExtractSection(t *testing.T) {
	content := `## Header
Some intro text.

## Pattern
This is the pattern description.
It spans multiple lines.

## Fix
Apply the fix here.

## Other
More stuff.`

	pattern := extractSection(content, "## Pattern")
	if !strings.Contains(pattern, "pattern description") {
		t.Errorf("expected pattern section, got %q", pattern)
	}

	fix := extractSection(content, "## Fix")
	if !strings.Contains(fix, "Apply the fix") {
		t.Errorf("expected fix section, got %q", fix)
	}

	// Multiple start headers
	multi := extractSection(content, "## Pattern", "## Situation")
	if !strings.Contains(multi, "pattern description") {
		t.Errorf("expected pattern from multi-header search, got %q", multi)
	}

	// Non-existent section
	missing := extractSection(content, "## NonExistent")
	if missing != "" {
		t.Errorf("expected empty for missing section, got %q", missing)
	}
}
