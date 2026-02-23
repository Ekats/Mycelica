package orchestrate

import (
	"encoding/json"
	"regexp"
	"strings"

	"mycelica/spore/internal/db"
)

// CheckVerdictFromGraph queries graph edges targeting the impl node for
// supports/contradicts verdicts. Prefers edges from the verifier agent.
// Returns VerdictUnknown if no relevant edges exist.
//
// This is Layer 1 (most authoritative) of the 3-layer verdict detection.
// Matches Rust check_verdict() in spore.rs.
func CheckVerdictFromGraph(d *db.DB, implNodeID string) Verdict {
	edges, err := d.GetEdgesForNode(implNodeID)
	if err != nil {
		return VerdictUnknown
	}

	// First pass: look for edges from the verifier agent specifically
	for _, e := range edges {
		if e.TargetID != implNodeID {
			continue
		}
		if e.AgentID == nil || *e.AgentID != "spore:verifier" {
			continue
		}
		if e.SupersededBy != nil {
			continue
		}
		switch e.EdgeType {
		case "supports":
			return VerdictSupports
		case "contradicts":
			return VerdictContradicts
		}
	}

	// Second pass: accept any non-superseded supports/contradicts edge
	// (handles edges created via CLI link command without agent_id)
	for _, e := range edges {
		if e.TargetID != implNodeID {
			continue
		}
		if e.SupersededBy != nil {
			continue
		}
		switch e.EdgeType {
		case "supports":
			return VerdictSupports
		case "contradicts":
			return VerdictContradicts
		}
	}

	return VerdictUnknown
}

// verdictTagRe matches <verdict>{...}</verdict> blocks.
// The (?s) flag enables dot-all mode so . matches newlines.
var verdictTagRe = regexp.MustCompile(`(?s)<verdict>\s*(\{.*?\})\s*</verdict>`)

// rawVerdictRe matches bare "verdict":"..." JSON patterns outside tags.
var rawVerdictRe = regexp.MustCompile(`"verdict"\s*:\s*"(supports|contradicts|pass|fail)"`)

// verdictJSON is the internal representation for JSON parsing.
type verdictJSON struct {
	Verdict    string  `json:"verdict"`
	Result     string  `json:"result"`
	Reason     string  `json:"reason"`
	Confidence float64 `json:"confidence"`
}

// ParseVerifierVerdictJSON looks for structured verdict JSON in verifier output.
// Checks for <verdict>{...}</verdict> blocks first, then falls back to raw
// "verdict":"..." patterns in the text.
//
// This is Layer 2 of the 3-layer verdict detection.
// Matches Rust parse_verifier_verdict() in spore.rs.
//
// Returns nil if no verdict block found. Returns a VerifierVerdict with
// VerdictUnknown if the block exists but cannot be parsed.
func ParseVerifierVerdictJSON(text string) *VerifierVerdict {
	// Try tagged block first
	if m := verdictTagRe.FindStringSubmatch(text); len(m) == 2 {
		return parseVerdictJSONBlock(m[1])
	}

	// Fallback: raw JSON verdict pattern (some agents emit without tags)
	if m := rawVerdictRe.FindStringSubmatch(text); len(m) == 2 {
		v := mapVerdictString(m[1])
		if v != VerdictUnknown {
			return &VerifierVerdict{
				Verdict:    v,
				Reason:     "",
				Confidence: 0.8, // lower confidence for raw match
			}
		}
	}

	return nil
}

// parseVerdictJSONBlock parses the JSON content from inside a <verdict> tag.
func parseVerdictJSONBlock(jsonStr string) *VerifierVerdict {
	var parsed verdictJSON
	if err := json.Unmarshal([]byte(jsonStr), &parsed); err != nil {
		return &VerifierVerdict{
			Verdict:    VerdictUnknown,
			Reason:     "",
			Confidence: 0.0,
		}
	}

	confidence := parsed.Confidence
	if confidence == 0 {
		confidence = 0.9 // default when not specified
	}
	if confidence < 0 {
		confidence = 0
	}
	if confidence > 1 {
		confidence = 1
	}

	// Check "verdict" field first, then "result" as synonym
	for _, field := range []string{parsed.Verdict, parsed.Result} {
		v := mapVerdictString(field)
		if v != VerdictUnknown {
			return &VerifierVerdict{
				Verdict:    v,
				Reason:     parsed.Reason,
				Confidence: confidence,
			}
		}
	}

	return &VerifierVerdict{
		Verdict:    VerdictUnknown,
		Reason:     parsed.Reason,
		Confidence: 0.0,
	}
}

// mapVerdictString converts a verdict string to a Verdict enum value.
func mapVerdictString(s string) Verdict {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "supports", "pass":
		return VerdictSupports
	case "contradicts", "fail":
		return VerdictContradicts
	default:
		return VerdictUnknown
	}
}

// ParseVerdictFromText is the last-resort keyword scanner for verdict detection.
// Looks for explicit verdict markers and edge type mentions in verifier output.
//
// This is Layer 3 of the 3-layer verdict detection.
// Matches Rust parse_verdict_from_text() in spore.rs.
func ParseVerdictFromText(text string) Verdict {
	lower := strings.ToLower(text)

	// Look for explicit verdict markers (more specific first)
	if strings.Contains(lower, "verification result: **pass**") ||
		strings.Contains(lower, "verdict: pass") ||
		strings.Contains(lower, "verdict: **pass**") {
		return VerdictSupports
	}
	if strings.Contains(lower, "verification result: **fail**") ||
		strings.Contains(lower, "verdict: fail") ||
		strings.Contains(lower, "verdict: **fail**") {
		return VerdictContradicts
	}

	// Fallback: edge type mentions
	if strings.Contains(lower, `edge_type: "supports"`) ||
		strings.Contains(lower, "edge_type: supports") {
		return VerdictSupports
	}
	if strings.Contains(lower, `edge_type: "contradicts"`) ||
		strings.Contains(lower, "edge_type: contradicts") {
		return VerdictContradicts
	}

	// Last resort: bare keyword scan. If both present, last one wins.
	lastPass := -1
	lastFail := -1

	// Check all keyword positions
	for _, kw := range []string{"pass", "passes", "supports"} {
		if idx := strings.LastIndex(lower, kw); idx > lastPass {
			lastPass = idx
		}
	}
	for _, kw := range []string{"fail", "fails", "contradicts"} {
		if idx := strings.LastIndex(lower, kw); idx > lastFail {
			lastFail = idx
		}
	}

	if lastPass >= 0 && lastFail >= 0 {
		// Both present: whichever appears last wins
		if lastFail > lastPass {
			return VerdictContradicts
		}
		return VerdictSupports
	}
	if lastPass >= 0 {
		return VerdictSupports
	}
	if lastFail >= 0 {
		return VerdictContradicts
	}

	return VerdictUnknown
}

// DetermineVerdict applies the 3-layer verdict detection in priority order:
//  1. Graph edges (most authoritative)
//  2. Structured JSON from verifier output
//  3. Text keywords from verifier output
//
// Returns a VerifierVerdict with VerdictUnknown if all layers fail.
// Handles nil db gracefully by skipping the graph check.
func DetermineVerdict(d *db.DB, implNodeID string, verifierOutput string) *VerifierVerdict {
	// Layer 1: graph edges
	if d != nil && implNodeID != "" {
		v := CheckVerdictFromGraph(d, implNodeID)
		if v != VerdictUnknown {
			return &VerifierVerdict{
				Verdict:    v,
				Reason:     "Verdict from graph edge",
				Confidence: 1.0,
			}
		}
	}

	// Layer 2: structured JSON
	if vv := ParseVerifierVerdictJSON(verifierOutput); vv != nil && vv.Verdict != VerdictUnknown {
		return vv
	}

	// Layer 3: text keywords
	v := ParseVerdictFromText(verifierOutput)
	if v != VerdictUnknown {
		reason := "Verdict inferred from verifier output text (keyword scan)"
		return &VerifierVerdict{
			Verdict:    v,
			Reason:     reason,
			Confidence: 0.6,
		}
	}

	// All layers failed
	return &VerifierVerdict{
		Verdict:    VerdictUnknown,
		Reason:     "",
		Confidence: 0.0,
	}
}
