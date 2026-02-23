package orchestrate

import (
	"database/sql"
	"testing"

	"mycelica/spore/internal/db"

	_ "modernc.org/sqlite"
)

// setupVerdictTestDB creates an in-memory SQLite database with the edges schema.
func setupVerdictTestDB(t *testing.T) *db.DB {
	t.Helper()
	conn, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}

	_, err = conn.Exec(`
		CREATE TABLE nodes (
			id TEXT PRIMARY KEY,
			type TEXT NOT NULL DEFAULT 'page',
			title TEXT NOT NULL,
			url TEXT,
			content TEXT,
			created_at INTEGER NOT NULL,
			updated_at INTEGER NOT NULL,
			depth INTEGER NOT NULL DEFAULT 0,
			is_item INTEGER NOT NULL DEFAULT 1,
			is_universe INTEGER NOT NULL DEFAULT 0,
			parent_id TEXT,
			child_count INTEGER NOT NULL DEFAULT 0,
			ai_title TEXT,
			summary TEXT,
			tags TEXT,
			emoji TEXT,
			is_processed INTEGER NOT NULL DEFAULT 0,
			agent_id TEXT,
			node_class TEXT,
			meta_type TEXT,
			content_type TEXT,
			source TEXT,
			author TEXT,
			embedding BLOB
		);
		CREATE TABLE edges (
			id TEXT PRIMARY KEY,
			source_id TEXT NOT NULL,
			target_id TEXT NOT NULL,
			type TEXT NOT NULL,
			label TEXT,
			weight REAL,
			confidence REAL,
			agent_id TEXT,
			reason TEXT,
			content TEXT,
			created_at INTEGER NOT NULL,
			updated_at INTEGER,
			superseded_by TEXT,
			metadata TEXT
		);
	`)
	if err != nil {
		t.Fatal(err)
	}

	// Use db.TestNewDB to wrap the connection -- but since DB fields are
	// unexported, we need to go through OpenDB with a temp file or use
	// the Conn() accessor pattern. We'll create a helper that returns
	// a *db.DB wrapping our in-memory connection.
	//
	// Since db.DB has unexported conn, we open via the package's OpenDB
	// with a unique in-memory name to get a fresh DB.
	// Actually, the test helpers in db package aren't exported. Let's just
	// close this conn and use OpenDB with a shared-cache in-memory URI.

	conn.Close()

	// Each ":memory:" is unique per connection, so use a named in-memory DB
	// with shared cache so we can use the real OpenDB path.
	d, err := db.OpenDB(":memory:")
	if err != nil {
		t.Fatal(err)
	}

	// Create schema via the real connection
	_, err = d.Conn().Exec(`
		CREATE TABLE nodes (
			id TEXT PRIMARY KEY,
			type TEXT NOT NULL DEFAULT 'page',
			title TEXT NOT NULL,
			url TEXT,
			content TEXT,
			created_at INTEGER NOT NULL,
			updated_at INTEGER NOT NULL,
			depth INTEGER NOT NULL DEFAULT 0,
			is_item INTEGER NOT NULL DEFAULT 1,
			is_universe INTEGER NOT NULL DEFAULT 0,
			parent_id TEXT,
			child_count INTEGER NOT NULL DEFAULT 0,
			ai_title TEXT,
			summary TEXT,
			tags TEXT,
			emoji TEXT,
			is_processed INTEGER NOT NULL DEFAULT 0,
			agent_id TEXT,
			node_class TEXT,
			meta_type TEXT,
			content_type TEXT,
			source TEXT,
			author TEXT,
			embedding BLOB
		);
		CREATE TABLE edges (
			id TEXT PRIMARY KEY,
			source_id TEXT NOT NULL,
			target_id TEXT NOT NULL,
			type TEXT NOT NULL,
			label TEXT,
			weight REAL,
			confidence REAL,
			agent_id TEXT,
			reason TEXT,
			content TEXT,
			created_at INTEGER NOT NULL,
			updated_at INTEGER,
			superseded_by TEXT,
			metadata TEXT
		);
	`)
	if err != nil {
		t.Fatal(err)
	}

	return d
}

func insertTestEdge(t *testing.T, d *db.DB, id, source, target, edgeType string, agentID *string) {
	t.Helper()
	_, err := d.Conn().Exec(
		`INSERT INTO edges (id, source_id, target_id, type, agent_id, created_at) VALUES (?, ?, ?, ?, ?, 1000)`,
		id, source, target, edgeType, agentID,
	)
	if err != nil {
		t.Fatal(err)
	}
}

// --- Layer 3: ParseVerdictFromText ---

func TestParseVerdictFromText_Supports(t *testing.T) {
	got := ParseVerdictFromText("The implementation looks good. Verdict: PASS")
	if got != VerdictSupports {
		t.Errorf("expected VerdictSupports, got %v", got)
	}
}

func TestParseVerdictFromText_Contradicts(t *testing.T) {
	got := ParseVerdictFromText("Tests are broken. Verdict: FAIL")
	if got != VerdictContradicts {
		t.Errorf("expected VerdictContradicts, got %v", got)
	}
}

func TestParseVerdictFromText_SupportsWord(t *testing.T) {
	got := ParseVerdictFromText("The evidence supports the implementation")
	if got != VerdictSupports {
		t.Errorf("expected VerdictSupports, got %v", got)
	}
}

func TestParseVerdictFromText_ContradictsWord(t *testing.T) {
	got := ParseVerdictFromText("The evidence contradicts the implementation")
	if got != VerdictContradicts {
		t.Errorf("expected VerdictContradicts, got %v", got)
	}
}

func TestParseVerdictFromText_Unknown(t *testing.T) {
	got := ParseVerdictFromText("I reviewed the code and it looks interesting")
	if got != VerdictUnknown {
		t.Errorf("expected VerdictUnknown, got %v", got)
	}
}

func TestParseVerdictFromText_LastWins(t *testing.T) {
	// Both keywords present -- last one wins
	got := ParseVerdictFromText("Initially I thought it would PASS but on closer inspection it FAILS")
	if got != VerdictContradicts {
		t.Errorf("expected VerdictContradicts (FAILS appears last), got %v", got)
	}

	// Reversed: fail then pass
	got2 := ParseVerdictFromText("The first test fails but overall the implementation passes")
	if got2 != VerdictSupports {
		t.Errorf("expected VerdictSupports (passes appears last), got %v", got2)
	}
}

// --- Layer 2: ParseVerifierVerdictJSON ---

func TestParseVerifierVerdictJSON_Valid(t *testing.T) {
	text := `Some preamble text.
<verdict>{"verdict": "supports", "reason": "All tests pass", "confidence": 0.95}</verdict>
Some trailing text.`

	vv := ParseVerifierVerdictJSON(text)
	if vv == nil {
		t.Fatal("expected non-nil verdict")
	}
	if vv.Verdict != VerdictSupports {
		t.Errorf("expected VerdictSupports, got %v", vv.Verdict)
	}
	if vv.Reason != "All tests pass" {
		t.Errorf("expected reason 'All tests pass', got %q", vv.Reason)
	}
	if vv.Confidence != 0.95 {
		t.Errorf("expected confidence 0.95, got %f", vv.Confidence)
	}
}

func TestParseVerifierVerdictJSON_Contradicts(t *testing.T) {
	text := `<verdict>{"verdict": "contradicts", "reason": "Build fails", "confidence": 0.9}</verdict>`

	vv := ParseVerifierVerdictJSON(text)
	if vv == nil {
		t.Fatal("expected non-nil verdict")
	}
	if vv.Verdict != VerdictContradicts {
		t.Errorf("expected VerdictContradicts, got %v", vv.Verdict)
	}
	if vv.Reason != "Build fails" {
		t.Errorf("expected reason 'Build fails', got %q", vv.Reason)
	}
}

func TestParseVerifierVerdictJSON_NoBlock(t *testing.T) {
	text := "The implementation looks good and all tests pass."
	vv := ParseVerifierVerdictJSON(text)
	if vv != nil {
		t.Errorf("expected nil for text without verdict block, got %+v", vv)
	}
}

func TestParseVerifierVerdictJSON_MalformedJSON(t *testing.T) {
	text := `<verdict>{not valid json}</verdict>`
	vv := ParseVerifierVerdictJSON(text)
	if vv == nil {
		t.Fatal("expected non-nil verdict for malformed JSON (should return Unknown)")
	}
	if vv.Verdict != VerdictUnknown {
		t.Errorf("expected VerdictUnknown for malformed JSON, got %v", vv.Verdict)
	}
}

func TestParseVerifierVerdictJSON_RawJSON(t *testing.T) {
	// Raw JSON pattern without <verdict> tags
	text := `Based on my analysis, the "verdict":"supports" for this implementation.`
	vv := ParseVerifierVerdictJSON(text)
	if vv == nil {
		t.Fatal("expected non-nil verdict for raw JSON pattern")
	}
	if vv.Verdict != VerdictSupports {
		t.Errorf("expected VerdictSupports from raw JSON, got %v", vv.Verdict)
	}
}

// --- DetermineVerdict (orchestrator entry point) ---

func TestDetermineVerdict_TextFallback(t *testing.T) {
	// nil db -- should skip graph check and fall through to text
	vv := DetermineVerdict(nil, "", "Verification result: **PASS**")
	if vv == nil {
		t.Fatal("expected non-nil verdict")
	}
	if vv.Verdict != VerdictSupports {
		t.Errorf("expected VerdictSupports from text fallback, got %v", vv.Verdict)
	}
}

func TestDetermineVerdict_JSONOverText(t *testing.T) {
	// JSON says supports, but text says FAIL -- JSON should win
	text := `<verdict>{"verdict": "supports", "reason": "JSON wins", "confidence": 0.9}</verdict>
But overall the test FAILS.`

	vv := DetermineVerdict(nil, "", text)
	if vv == nil {
		t.Fatal("expected non-nil verdict")
	}
	if vv.Verdict != VerdictSupports {
		t.Errorf("expected VerdictSupports (JSON over text), got %v", vv.Verdict)
	}
	if vv.Reason != "JSON wins" {
		t.Errorf("expected reason 'JSON wins', got %q", vv.Reason)
	}
}

// --- Layer 1: CheckVerdictFromGraph ---

func TestCheckVerdictFromGraph_NoEdges(t *testing.T) {
	d := setupVerdictTestDB(t)
	defer d.Close()

	got := CheckVerdictFromGraph(d, "nonexistent-node")
	if got != VerdictUnknown {
		t.Errorf("expected VerdictUnknown for node with no edges, got %v", got)
	}
}

func TestCheckVerdictFromGraph_VerifierEdge(t *testing.T) {
	d := setupVerdictTestDB(t)
	defer d.Close()

	agentID := "spore:verifier"
	insertTestEdge(t, d, "e1", "verifier-run", "impl-node", "supports", &agentID)

	got := CheckVerdictFromGraph(d, "impl-node")
	if got != VerdictSupports {
		t.Errorf("expected VerdictSupports from verifier edge, got %v", got)
	}
}

func TestCheckVerdictFromGraph_VerifierPriority(t *testing.T) {
	d := setupVerdictTestDB(t)
	defer d.Close()

	// Non-verifier edge says supports, verifier says contradicts
	otherAgent := "spore:coder"
	verifierAgent := "spore:verifier"
	insertTestEdge(t, d, "e1", "other-run", "impl-node", "supports", &otherAgent)
	insertTestEdge(t, d, "e2", "verifier-run", "impl-node", "contradicts", &verifierAgent)

	got := CheckVerdictFromGraph(d, "impl-node")
	if got != VerdictContradicts {
		t.Errorf("expected VerdictContradicts (verifier wins), got %v", got)
	}
}

func TestCheckVerdictFromGraph_FallbackToAnyEdge(t *testing.T) {
	d := setupVerdictTestDB(t)
	defer d.Close()

	// Edge without agent_id (created via CLI link)
	insertTestEdge(t, d, "e1", "manual-link", "impl-node", "supports", nil)

	got := CheckVerdictFromGraph(d, "impl-node")
	if got != VerdictSupports {
		t.Errorf("expected VerdictSupports from non-agent edge, got %v", got)
	}
}

func TestCheckVerdictFromGraph_SupersededIgnored(t *testing.T) {
	d := setupVerdictTestDB(t)
	defer d.Close()

	agentID := "spore:verifier"
	// Insert a superseded edge
	_, err := d.Conn().Exec(
		`INSERT INTO edges (id, source_id, target_id, type, agent_id, created_at, superseded_by) VALUES (?, ?, ?, ?, ?, 1000, ?)`,
		"e1", "verifier-run", "impl-node", "supports", agentID, "e2",
	)
	if err != nil {
		t.Fatal(err)
	}

	got := CheckVerdictFromGraph(d, "impl-node")
	if got != VerdictUnknown {
		t.Errorf("expected VerdictUnknown (edge superseded), got %v", got)
	}
}

// --- Verdict.String() ---

func TestVerdictString(t *testing.T) {
	tests := []struct {
		v    Verdict
		want string
	}{
		{VerdictUnknown, "unknown"},
		{VerdictSupports, "supports"},
		{VerdictContradicts, "contradicts"},
	}
	for _, tt := range tests {
		if got := tt.v.String(); got != tt.want {
			t.Errorf("Verdict(%d).String() = %q, want %q", tt.v, got, tt.want)
		}
	}
}

// --- Additional edge cases ---

func TestParseVerifierVerdictJSON_ResultField(t *testing.T) {
	// "result" as synonym for "verdict"
	text := `<verdict>{"result": "pass", "reason": "All good"}</verdict>`
	vv := ParseVerifierVerdictJSON(text)
	if vv == nil {
		t.Fatal("expected non-nil verdict")
	}
	if vv.Verdict != VerdictSupports {
		t.Errorf("expected VerdictSupports from 'result' field, got %v", vv.Verdict)
	}
}

func TestParseVerifierVerdictJSON_DefaultConfidence(t *testing.T) {
	// No confidence field should default to 0.9
	text := `<verdict>{"verdict": "supports", "reason": "looks good"}</verdict>`
	vv := ParseVerifierVerdictJSON(text)
	if vv == nil {
		t.Fatal("expected non-nil verdict")
	}
	if vv.Confidence != 0.9 {
		t.Errorf("expected default confidence 0.9, got %f", vv.Confidence)
	}
}

func TestDetermineVerdict_AllUnknown(t *testing.T) {
	// No db, no JSON, no keywords -> all layers fail
	vv := DetermineVerdict(nil, "", "This is just a description with no verdict indicators.")
	if vv == nil {
		t.Fatal("expected non-nil verdict even when all layers fail")
	}
	if vv.Verdict != VerdictUnknown {
		t.Errorf("expected VerdictUnknown, got %v", vv.Verdict)
	}
	if vv.Confidence != 0.0 {
		t.Errorf("expected confidence 0.0, got %f", vv.Confidence)
	}
}

func TestParseVerdictFromText_ExplicitMarkers(t *testing.T) {
	// Verification result: **PASS** (markdown bold)
	got := ParseVerdictFromText("Verification result: **PASS**")
	if got != VerdictSupports {
		t.Errorf("expected VerdictSupports for 'Verification result: **PASS**', got %v", got)
	}

	got2 := ParseVerdictFromText("Verification result: **FAIL**")
	if got2 != VerdictContradicts {
		t.Errorf("expected VerdictContradicts for 'Verification result: **FAIL**', got %v", got2)
	}
}

func TestParseVerdictFromText_EdgeTypePattern(t *testing.T) {
	got := ParseVerdictFromText(`I created an edge with edge_type: "supports" for this implementation.`)
	if got != VerdictSupports {
		t.Errorf("expected VerdictSupports for edge_type pattern, got %v", got)
	}

	got2 := ParseVerdictFromText(`Created edge_type: contradicts for the broken implementation.`)
	if got2 != VerdictContradicts {
		t.Errorf("expected VerdictContradicts for edge_type pattern, got %v", got2)
	}
}
