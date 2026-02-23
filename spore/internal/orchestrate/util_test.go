package orchestrate

import "testing"

func TestFormatDurationShort(t *testing.T) {
	tests := []struct {
		ms   int64
		want string
	}{
		{0, "0.0s"},
		{500, "0.5s"},
		{1200, "1.2s"},
		{65000, "1m5s"},
		{3700000, "1h1m"},
	}

	for _, tt := range tests {
		got := FormatDurationShort(tt.ms)
		if got != tt.want {
			t.Errorf("FormatDurationShort(%d) = %q, want %q", tt.ms, got, tt.want)
		}
	}
}

func TestTruncateMiddle(t *testing.T) {
	tests := []struct {
		s      string
		maxLen int
		want   string
	}{
		{"short", 10, "short"},           // under limit
		{"exact", 5, "exact"},            // exactly at limit
		{"abcdefghij", 7, "ab...ij"},     // over limit
		{"hello world!", 9, "hel...ld!"}, // asymmetric
		{"abc", 3, "abc"},                // exactly 3
		{"abcd", 3, "abc"},               // maxLen <= 3 edge case
	}

	for _, tt := range tests {
		got := TruncateMiddle(tt.s, tt.maxLen)
		if got != tt.want {
			t.Errorf("TruncateMiddle(%q, %d) = %q, want %q", tt.s, tt.maxLen, got, tt.want)
		}
		if len(got) > tt.maxLen {
			t.Errorf("TruncateMiddle(%q, %d) length %d exceeds max %d", tt.s, tt.maxLen, len(got), tt.maxLen)
		}
	}
}

func TestIsLessonQuality(t *testing.T) {
	tests := []struct {
		name    string
		content string
		want    bool
	}{
		{
			"good content",
			"This is a well-written lesson about error handling patterns in Go. It covers multiple scenarios.",
			true,
		},
		{
			"too short",
			"Short.",
			false,
		},
		{
			"contains I don't know",
			"I don't know what went wrong here, but something broke in the pipeline somehow or other right.",
			false,
		},
		{
			"contains I'm not sure",
			"I'm not sure what the correct approach is here but we should probably investigate further or something.",
			false,
		},
		{
			"starts with Error",
			"Error: something went wrong in the system and we should probably fix it before proceeding further.",
			false,
		},
		{
			"starts with error:",
			"error: compilation failed and we should really take a closer look at the root cause of this issue.",
			false,
		},
		{
			"no period or newline",
			"This is a long enough string that has more than fifty characters but no period or newline at all right",
			false,
		},
		{
			"has newline instead of period",
			"This is valid content that uses newlines for structure\nand it should pass the quality check easily enough",
			true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := IsLessonQuality(tt.content)
			if got != tt.want {
				t.Errorf("IsLessonQuality(%q) = %v, want %v", tt.content, got, tt.want)
			}
		})
	}
}

func TestIsSporeExcluded(t *testing.T) {
	tests := []struct {
		edgeType string
		want     bool
	}{
		{"clicked", true},
		{"backtracked", true},
		{"session_item", true},
		{"Clicked", true},  // case insensitive
		{"calls", false},
		{"belongs_to", false},
		{"documents", false},
		{"related", false},
	}

	for _, tt := range tests {
		got := IsSporeExcluded(tt.edgeType)
		if got != tt.want {
			t.Errorf("IsSporeExcluded(%q) = %v, want %v", tt.edgeType, got, tt.want)
		}
	}
}

func TestAgentID(t *testing.T) {
	tests := []struct {
		role AgentRole
		want string
	}{
		{RoleCoder, "spore:coder"},
		{RoleVerifier, "spore:verifier"},
		{RoleSummarizer, "spore:summarizer"},
		{RoleOperator, "spore:operator"},
	}

	for _, tt := range tests {
		got := AgentID(tt.role)
		if got != tt.want {
			t.Errorf("AgentID(%q) = %q, want %q", tt.role, got, tt.want)
		}
	}
}

func TestPtr(t *testing.T) {
	i := 42
	p := Ptr(i)
	if *p != 42 {
		t.Errorf("Ptr(42) = %d, want 42", *p)
	}

	s := "hello"
	sp := Ptr(s)
	if *sp != "hello" {
		t.Errorf("Ptr(\"hello\") = %q, want \"hello\"", *sp)
	}
}
