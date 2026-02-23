package cmd

import (
	"testing"

	"mycelica/spore/internal/db"
)

func strPtr(s string) *string { return &s }

func TestExtractTaskFromNode(t *testing.T) {
	tests := []struct {
		name    string
		node    *db.Node
		want    string
	}{
		{
			name: "extracts from ## Task section",
			node: &db.Node{
				Title:   "Orchestration: Fix pagination",
				Content: strPtr("## Context\nsome context\n\n## Task\nFix the pagination bug in list view\n\n## Rules\nfollow conventions"),
			},
			want: "Fix the pagination bug in list view",
		},
		{
			name: "extracts multiline task section",
			node: &db.Node{
				Title:   "Orchestration: Refactor",
				Content: strPtr("## Task\nRefactor the parser\nto handle edge cases\nbetter\n\n## Constraints\ndon't break API"),
			},
			want: "Refactor the parser\nto handle edge cases\nbetter",
		},
		{
			name: "falls back to title stripping Orchestration prefix",
			node: &db.Node{
				Title:   "Orchestration: Fix pagination",
				Content: strPtr("no task section here"),
			},
			want: "Fix pagination",
		},
		{
			name: "falls back to title when content is nil",
			node: &db.Node{
				Title: "Orchestration: Add tests",
			},
			want: "Add tests",
		},
		{
			name: "title without Orchestration prefix",
			node: &db.Node{
				Title: "Some other task title",
			},
			want: "Some other task title",
		},
		{
			name: "empty title returns empty string",
			node: &db.Node{
				Title: "",
			},
			want: "",
		},
		{
			name: "task section at end of content (no following heading)",
			node: &db.Node{
				Title:   "test",
				Content: strPtr("## Task\nDo the thing\n"),
			},
			want: "Do the thing",
		},
		{
			name: "empty task section falls back to title",
			node: &db.Node{
				Title:   "Orchestration: Fallback task",
				Content: strPtr("## Task\n\n## Next\nsomething"),
			},
			want: "Fallback task",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := extractTaskFromNode(tt.node)
			if got != tt.want {
				t.Errorf("extractTaskFromNode() = %q, want %q", got, tt.want)
			}
		})
	}
}
