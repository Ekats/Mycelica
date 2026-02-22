package db

import "testing"

func TestBuildFTSQuery_StopwordRemoval(t *testing.T) {
	got := BuildFTSQuery("Add the flag to a function for parsing")
	want := "Add OR flag OR function OR parsing"
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestBuildFTSQuery_ShortWords(t *testing.T) {
	got := BuildFTSQuery("go do run fast")
	want := "run OR fast"
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestBuildFTSQuery_PunctuationTrimming(t *testing.T) {
	got := BuildFTSQuery("generate_task_file() function, (spore.rs)")
	want := "generate_task_file OR function OR spore.rs"
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestBuildFTSQuery_AllStopwords(t *testing.T) {
	got := BuildFTSQuery("the a an in on at")
	if got != "" {
		t.Errorf("expected empty, got %q", got)
	}
}

func TestBuildFTSQuery_MixedCase(t *testing.T) {
	got := BuildFTSQuery("The AND From THIS function")
	want := "function"
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestBuildFTSQuery_Empty(t *testing.T) {
	got := BuildFTSQuery("")
	if got != "" {
		t.Errorf("expected empty, got %q", got)
	}
}
