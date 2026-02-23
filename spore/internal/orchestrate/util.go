package orchestrate

import (
	"fmt"
	"strings"
)

// FormatDurationShort formats milliseconds into a compact human-readable string.
//
//	<1000ms  -> "0.Xs"
//	<60000ms -> "X.Xs"
//	<3600000 -> "XmYs"
//	else     -> "XhYm"
func FormatDurationShort(ms int64) string {
	switch {
	case ms < 1000:
		return fmt.Sprintf("0.%ds", ms/100)
	case ms < 60000:
		return fmt.Sprintf("%d.%ds", ms/1000, (ms%1000)/100)
	case ms < 3600000:
		minutes := ms / 60000
		seconds := (ms % 60000) / 1000
		return fmt.Sprintf("%dm%ds", minutes, seconds)
	default:
		hours := ms / 3600000
		minutes := (ms % 3600000) / 60000
		return fmt.Sprintf("%dh%dm", hours, minutes)
	}
}

// TruncateMiddle shortens a string by replacing the middle with "..." if it
// exceeds maxLen. Preserves roughly equal portions from start and end.
func TruncateMiddle(s string, maxLen int) string {
	if len(s) <= maxLen {
		return s
	}
	if maxLen <= 3 {
		return s[:maxLen]
	}
	// Split available space: first half gets one more char on odd splits
	available := maxLen - 3 // account for "..."
	firstHalf := (available + 1) / 2
	lastHalf := available / 2
	return s[:firstHalf] + "..." + s[len(s)-lastHalf:]
}

// IsLessonQuality checks whether content meets minimum quality thresholds for
// storage as a reusable lesson. Matches the Rust spore.rs implementation:
//   - At least 50 characters
//   - No "I don't know" or "I'm not sure" (case-insensitive)
//   - Doesn't start with "Error" or "error:"
//   - Contains at least one '.' or '\n'
func IsLessonQuality(content string) bool {
	if len(content) < 50 {
		return false
	}

	lower := strings.ToLower(content)
	if strings.Contains(lower, "i don't know") || strings.Contains(lower, "i'm not sure") {
		return false
	}

	if strings.HasPrefix(lower, "error") || strings.HasPrefix(lower, "error:") {
		return false
	}

	if !strings.ContainsAny(content, ".\n") {
		return false
	}

	return true
}

// IsSporeExcluded returns true for edge types excluded from Dijkstra context
// expansion. These are browser session edges that add noise, not signal.
func IsSporeExcluded(edgeType string) bool {
	switch strings.ToLower(edgeType) {
	case "clicked", "backtracked", "session_item":
		return true
	default:
		return false
	}
}

// AgentID returns the canonical node ID for a pipeline agent role.
func AgentID(role AgentRole) string {
	return "spore:" + string(role)
}

// Ptr returns a pointer to the given value. Useful for optional struct fields.
func Ptr[T any](v T) *T {
	return &v
}
