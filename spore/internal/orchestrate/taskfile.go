package orchestrate

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"mycelica/spore/internal/db"
	"mycelica/spore/internal/graph"
)

// anchorNode is a search result used as a Dijkstra expansion source.
type anchorNode struct {
	ID    string
	Title string
	Score float64
	Source string // "semantic" or "fts"
}

// contextRow is a single row in the rendered Graph Context table.
type contextRow struct {
	Rank      int
	NodeID    string
	Title     string
	Relevance float64 // 0-1
	Via       string  // edge-type path, e.g. "supports -> derives_from"
	Anchor    string  // anchor title this was reached from
	Tags      string  // raw tags JSON
	Content   string  // first 500 chars of node content
	NodeClass string
	IsCode    bool // has file_path in tags
}

// lesson is a past-run lesson relevant to this task.
type lesson struct {
	Title   string
	Summary string // extracted pattern/situation section
	Fix     string // extracted fix section
}

// codeTags holds parsed fields from a code node's tags JSON.
type codeTags struct {
	FilePath  string `json:"file_path"`
	StartLine int    `json:"line_start"`
	EndLine   int    `json:"line_end"`
	Language  string `json:"language"`
}

// GenerateTaskFile creates a markdown task file with graph context for a pipeline agent.
//
// It performs semantic + FTS anchor search, Dijkstra context expansion, lesson
// matching, and renders the result as structured markdown. The file is written
// to outputDir/task-<role>-<runID[:8]>.md.
//
// Returns (filepath, contextNodeCount, error).
func GenerateTaskFile(
	d *db.DB,
	task string,
	role AgentRole,
	runID, taskNodeID string,
	bounce, maxBounces int,
	lastImplID string,
	lastVerdict Verdict,
	config TaskFileConfig,
	outputDir string,
) (string, int, error) {

	// 1. Find anchor nodes
	anchors, err := findAnchors(d, task, taskNodeID, config)
	if err != nil {
		// Non-fatal: proceed with empty anchors
		fmt.Fprintf(os.Stderr, "[task-file] warning: anchor search failed: %v\n", err)
		anchors = nil
	}

	// 2. Expand anchors via Dijkstra
	context := gatherContext(d, anchors, taskNodeID, config)

	// 3. Find relevant lessons from past runs
	lessons := findLessons(d, task, taskNodeID, config)

	// 4. Render markdown
	md := renderMarkdown(d, task, role, runID, taskNodeID,
		bounce, maxBounces, lastImplID, lastVerdict,
		anchors, context, lessons)

	// 5. Write to disk
	if err := os.MkdirAll(outputDir, 0o755); err != nil {
		return "", 0, fmt.Errorf("creating output dir: %w", err)
	}

	shortRunID := runID
	if len(shortRunID) > 8 {
		shortRunID = shortRunID[:8]
	}
	filename := fmt.Sprintf("task-%s-%s.md", role, shortRunID)
	path := filepath.Join(outputDir, filename)
	if err := os.WriteFile(path, []byte(md), 0o644); err != nil {
		return "", 0, fmt.Errorf("writing task file: %w", err)
	}

	lineCount := strings.Count(md, "\n") + 1
	fmt.Fprintf(os.Stderr, "[task-file] Generated: %d lines\n", lineCount)

	return path, len(context), nil
}

// findAnchors performs two-source anchor selection: semantic (embedding similarity)
// and FTS (keyword search). Semantic results have priority; FTS fills remaining slots.
// Ports spore.rs lines 2135-2220.
func findAnchors(d *db.DB, task, taskNodeID string, config TaskFileConfig) ([]anchorNode, error) {
	maxAnchors := config.MaxAnchors
	if maxAnchors <= 0 {
		maxAnchors = 5
	}

	var semanticAnchors []anchorNode
	var ftsAnchors []anchorNode

	// --- Semantic search ---
	taskEmb, err := d.GetNodeEmbedding(taskNodeID)
	if err == nil && taskEmb != nil {
		allEmbs, err := d.GetNodesWithEmbeddings()
		if err == nil {
			similar := graph.FindSimilar(
				taskEmb, allEmbs, taskNodeID,
				config.SimilarTop, float32(config.Threshold),
			)
			for _, s := range similar {
				// Resolve node to check class
				node, err := d.GetNode(s.ID)
				if err != nil || node == nil {
					continue
				}
				if node.NodeClass != nil && *node.NodeClass == "operational" {
					continue
				}
				title := node.Title
				if node.AITitle != nil {
					title = *node.AITitle
				}
				semanticAnchors = append(semanticAnchors, anchorNode{
					ID:    s.ID,
					Title: title,
					Score: float64(s.Similarity),
					Source: "semantic",
				})
				if len(semanticAnchors) >= maxAnchors {
					break
				}
			}
		}
	}

	if len(semanticAnchors) > 0 {
		fmt.Fprintf(os.Stderr, "[task-file] Semantic search found %d candidate(s)\n", len(semanticAnchors))
	}

	// --- FTS keyword search ---
	ftsQuery := db.BuildFTSQuery(task)
	if ftsQuery != "" {
		ftsNodes, err := d.SearchNodes(task)
		if err == nil {
			for _, n := range ftsNodes {
				if n.ID == taskNodeID {
					continue
				}
				if n.NodeClass != nil && *n.NodeClass == "operational" {
					continue
				}
				title := n.Title
				if n.AITitle != nil {
					title = *n.AITitle
				}
				ftsAnchors = append(ftsAnchors, anchorNode{
					ID:    n.ID,
					Title: title,
					Score: 0, // FTS doesn't produce a similarity score
					Source: "fts",
				})
				if len(ftsAnchors) >= maxAnchors {
					break
				}
			}
		}
	}

	if len(ftsAnchors) > 0 {
		fmt.Fprintf(os.Stderr, "[task-file] FTS search found %d candidate(s)\n", len(ftsAnchors))
	}

	// --- Merge: semantic first, then FTS deduped ---
	seen := make(map[string]bool)
	var merged []anchorNode
	for _, a := range semanticAnchors {
		if !seen[a.ID] {
			seen[a.ID] = true
			merged = append(merged, a)
		}
	}
	for _, a := range ftsAnchors {
		if !seen[a.ID] {
			seen[a.ID] = true
			merged = append(merged, a)
		}
	}
	if len(merged) > maxAnchors {
		merged = merged[:maxAnchors]
	}

	fmt.Fprintf(os.Stderr, "[task-file] %d anchor(s) after merge+dedup\n", len(merged))
	return merged, nil
}

// gatherContext expands each anchor via Dijkstra and merges results.
// For each node ID, keeps the highest relevance score.
func gatherContext(d *db.DB, anchors []anchorNode, taskNodeID string, config TaskFileConfig) []contextRow {
	budget := config.Budget
	if budget <= 0 {
		budget = 7
	}
	maxHops := config.MaxHops
	if maxHops <= 0 {
		maxHops = 4
	}
	maxCost := config.MaxCost
	if maxCost <= 0 {
		maxCost = 2.0
	}

	// seen tracks the best entry per node ID
	type seenEntry struct {
		relevance float64
		title     string
		anchor    string
		via       string
	}
	seen := make(map[string]seenEntry)

	for _, anchor := range anchors {
		ctxConfig := &db.ContextConfig{
			Budget:           budget,
			MaxHops:          maxHops,
			MaxCost:          maxCost,
			ExcludeEdgeTypes: []string{"clicked", "backtracked", "session_item"},
			NotSuperseded:    true,
			ItemsOnly:        true,
		}

		ctxNodes, err := d.ContextForTask(anchor.ID, ctxConfig)
		if err != nil {
			continue
		}

		for _, cn := range ctxNodes {
			// Skip operational nodes
			if cn.NodeClass != nil && *cn.NodeClass == "operational" {
				continue
			}

			via := "direct"
			if len(cn.Path) > 0 {
				parts := make([]string, len(cn.Path))
				for i, hop := range cn.Path {
					parts[i] = hop.EdgeType
				}
				via = strings.Join(parts, " -> ")
			}

			prev, exists := seen[cn.NodeID]
			if !exists || cn.Relevance > prev.relevance {
				seen[cn.NodeID] = seenEntry{
					relevance: cn.Relevance,
					title:     cn.NodeTitle,
					anchor:    anchor.Title,
					via:       via,
				}
			}
		}

		// Include the anchor itself if not already present
		if _, exists := seen[anchor.ID]; !exists {
			sourceLabel := "Semantic match"
			if anchor.Source == "fts" {
				sourceLabel = "FTS match"
			}
			seen[anchor.ID] = seenEntry{
				relevance: 1.0,
				title:     anchor.Title,
				anchor:    "search",
				via:       sourceLabel,
			}
		}
	}

	// Filter out the task node itself
	delete(seen, taskNodeID)

	// Sort by relevance descending
	type kv struct {
		id string
		e  seenEntry
	}
	var sorted []kv
	for id, e := range seen {
		sorted = append(sorted, kv{id, e})
	}
	sort.Slice(sorted, func(i, j int) bool {
		return sorted[i].e.relevance > sorted[j].e.relevance
	})

	// Build contextRow structs
	rows := make([]contextRow, len(sorted))
	for i, item := range sorted {
		// Look up full node for tags and content
		var tags, content, nodeClass string
		var isCode bool
		node, err := d.GetNode(item.id)
		if err == nil && node != nil {
			if node.Tags != nil {
				tags = *node.Tags
				isCode = strings.Contains(tags, "file_path")
			}
			if node.Content != nil {
				content = *node.Content
				if len(content) > 500 {
					content = content[:500]
				}
			}
			if node.NodeClass != nil {
				nodeClass = *node.NodeClass
			}
		}

		rows[i] = contextRow{
			Rank:      i + 1,
			NodeID:    item.id,
			Title:     item.e.title,
			Relevance: item.e.relevance,
			Via:       item.e.via,
			Anchor:    item.e.anchor,
			Tags:      tags,
			Content:   content,
			NodeClass: nodeClass,
			IsCode:    isCode,
		}
	}

	return rows
}

// findLessons finds past-run lessons relevant to the current task.
// Uses embedding similarity when available, falls back to recency.
func findLessons(d *db.DB, task, taskNodeID string, config TaskFileConfig) []lesson {
	maxLessons := config.MaxLessons
	if maxLessons <= 0 {
		maxLessons = 5
	}

	// Query operational nodes with "Lesson:" title prefix
	rows, err := d.Conn().Query(
		`SELECT id, title, content FROM nodes
		 WHERE node_class = 'operational' AND title LIKE 'Lesson:%'
		 ORDER BY created_at DESC LIMIT 20`,
	)
	if err != nil {
		return nil
	}
	defer rows.Close()

	type rawLesson struct {
		id, title, content string
	}
	var allLessons []rawLesson
	for rows.Next() {
		var id, title string
		var content *string
		if err := rows.Scan(&id, &title, &content); err != nil {
			continue
		}
		c := ""
		if content != nil {
			c = *content
		}
		allLessons = append(allLessons, rawLesson{id, title, c})
	}
	if len(allLessons) == 0 {
		return nil
	}

	// Try to rank by embedding similarity to task
	taskEmb, err := d.GetNodeEmbedding(taskNodeID)
	var ranked []rawLesson
	if err == nil && taskEmb != nil {
		allEmbs, err := d.GetNodesWithEmbeddings()
		if err == nil && len(allEmbs) > 0 {
			// Build set of lesson IDs for filtering
			lessonIDs := make(map[string]bool)
			for _, l := range allLessons {
				lessonIDs[l.id] = true
			}
			// Filter to only lesson embeddings
			var lessonEmbs []db.NodeEmbedding
			for _, e := range allEmbs {
				if lessonIDs[e.ID] {
					lessonEmbs = append(lessonEmbs, e)
				}
			}
			if len(lessonEmbs) > 0 {
				similar := graph.FindSimilar(taskEmb, lessonEmbs, taskNodeID, maxLessons, 0.15)
				rankedIDs := make([]string, len(similar))
				for i, s := range similar {
					rankedIDs[i] = s.ID
				}
				// Build ordered result preserving similarity ranking
				rankedSet := make(map[string]bool)
				for _, id := range rankedIDs {
					rankedSet[id] = true
					for _, l := range allLessons {
						if l.id == id {
							ranked = append(ranked, l)
							break
						}
					}
				}
				// Pad with remaining by recency if < maxLessons
				for _, l := range allLessons {
					if len(ranked) >= maxLessons {
						break
					}
					if !rankedSet[l.id] {
						ranked = append(ranked, l)
					}
				}
			} else {
				// No lesson embeddings — fall back to recency
				ranked = allLessons
			}
		} else {
			ranked = allLessons
		}
	} else {
		// No task embedding — fall back to recency
		ranked = allLessons
	}

	if len(ranked) > maxLessons {
		ranked = ranked[:maxLessons]
	}

	// Extract pattern/fix sections from each lesson
	var result []lesson
	for _, rl := range ranked {
		pattern := extractSection(rl.content, "## Pattern", "## Situation")
		fix := extractSection(rl.content, "## Fix")

		summary := pattern
		if summary == "" {
			summary = strings.TrimPrefix(rl.title, "Lesson: ")
		}

		if !IsLessonQuality(summary) {
			continue
		}

		result = append(result, lesson{
			Title:   rl.title,
			Summary: summary,
			Fix:     fix,
		})
	}

	return result
}

// extractSection extracts text between a start header and the next ## header.
// Accepts multiple possible start headers (OR match).
func extractSection(content string, startHeaders ...string) string {
	lines := strings.Split(content, "\n")
	var collecting bool
	var parts []string

	for _, line := range lines {
		if collecting {
			if strings.HasPrefix(line, "## ") {
				break
			}
			parts = append(parts, line)
		} else {
			for _, h := range startHeaders {
				if strings.HasPrefix(line, h) {
					collecting = true
					break
				}
			}
		}
	}

	return strings.TrimSpace(strings.Join(parts, " "))
}

// renderMarkdown generates the full task file markdown from gathered data.
// The db parameter is optional (nil-safe) and used only for call graph edge lookups.
func renderMarkdown(
	d *db.DB,
	task string,
	role AgentRole,
	runID, taskNodeID string,
	bounce, maxBounces int,
	lastImplID string,
	lastVerdict Verdict,
	anchors []anchorNode,
	context []contextRow,
	lessons []lesson,
) string {
	var md strings.Builder
	now := time.Now().UTC()

	// Short title
	taskShort := task
	if len(taskShort) > 60 {
		taskShort = taskShort[:60]
	}
	shortRunID := runID
	if len(shortRunID) > 8 {
		shortRunID = shortRunID[:8]
	}

	// --- 1. Header ---
	md.WriteString(fmt.Sprintf("# Task: %s\n\n", taskShort))
	md.WriteString(fmt.Sprintf("- **Run:** %s\n", shortRunID))
	md.WriteString(fmt.Sprintf("- **Agent:** %s\n", role))
	md.WriteString(fmt.Sprintf("- **Bounce:** %d/%d\n", bounce+1, maxBounces))
	md.WriteString(fmt.Sprintf("- **Generated:** %s\n\n", now.Format("2006-01-02 15:04:05 UTC")))

	// --- 2. Task ---
	md.WriteString("## Task\n\n")
	md.WriteString(task)
	md.WriteString("\n\n")

	// --- 3. Conditional sections based on role/bounce ---
	if lastImplID != "" {
		switch role {
		case RoleVerifier:
			md.WriteString("## Implementation to Check\n\n")
			md.WriteString(fmt.Sprintf(
				"Implementation node ID: `%s`. Read it with `mycelica_read_content` to see what the coder changed and why.\n\n",
				lastImplID,
			))
		case RoleSummarizer:
			md.WriteString("## Implementation to Summarize\n\n")
			md.WriteString(fmt.Sprintf(
				"Implementation node ID: `%s`. Read it and the full bounce trail with `mycelica_read_content` and `mycelica_nav_edges`.\n\n",
				lastImplID,
			))
		default:
			// coder on bounce 2+: previous impl had issues
			md.WriteString("## Previous Bounce\n\n")
			if lastVerdict == VerdictUnknown {
				md.WriteString(fmt.Sprintf(
					"The verifier could not parse a verdict from the previous attempt (node `%s`). Review your changes carefully and ensure correctness.\n\n",
					lastImplID,
				))
			} else {
				md.WriteString(fmt.Sprintf(
					"Verifier found issues with node `%s`. Check its incoming `contradicts` edges and fix the code.\n\n",
					lastImplID,
				))
			}
		}
	}

	// --- 4. Graph Context ---
	md.WriteString("## Graph Context\n\n")
	md.WriteString("Relevant nodes found by search + Dijkstra traversal from the task description.\n")
	md.WriteString("Use `mycelica_node_get` or `mycelica_read_content` to read full content of any node.\n\n")

	if len(context) == 0 {
		md.WriteString("_No relevant nodes found in the graph._\n\n")
	} else {
		md.WriteString("| # | Node | ID | Relevance | Via |\n")
		md.WriteString("|---|------|----|-----------|-----|\n")
		for _, row := range context {
			titleShort := row.Title
			if len(titleShort) > 50 {
				titleShort = titleShort[:50]
			}
			idShort := row.NodeID
			if len(idShort) > 12 {
				idShort = idShort[:12]
			}
			md.WriteString(fmt.Sprintf(
				"| %d | %s | `%s` | %.0f%% | %s -> %s |\n",
				row.Rank, titleShort, idShort, row.Relevance*100.0, row.Anchor, row.Via,
			))
		}
		md.WriteString("\n")

		// --- 5. Code Locations ---
		codeRows := filterCodeRows(context)
		if len(codeRows) > 0 {
			md.WriteString("### Code Locations\n\n")
			md.WriteString("Use `Read` tool with these paths for direct file access (faster than MCP):\n\n")
			for _, cr := range codeRows {
				ct := parseCodeTags(cr.Tags)
				if ct.FilePath == "" {
					continue
				}
				titleShort := cr.Title
				if len(titleShort) > 40 {
					titleShort = titleShort[:40]
				}
				md.WriteString(fmt.Sprintf("- `%s` L%d-%d -- %s\n",
					ct.FilePath, ct.StartLine, ct.EndLine, titleShort))
			}
			md.WriteString("\n")

			// --- 6. Key Code Snippets (top 5) ---
			renderCodeSnippets(&md, codeRows)

			// --- 7. Files Likely Touched (top 8) ---
			renderFilesLikelyTouched(&md, codeRows)

			// --- 8. Call Graph (top 3 functions) ---
			if d != nil {
				renderCallGraphWithDB(&md, d, context)
			}
		}
	}

	// --- 9. Lessons from Past Runs ---
	if len(lessons) > 0 {
		md.WriteString("## Lessons from Past Runs\n\n")
		md.WriteString("These were extracted from previous orchestrator runs. Keep them in mind.\n\n")
		for _, l := range lessons {
			lessonName := strings.TrimPrefix(l.Title, "Lesson: ")
			if l.Fix == "" {
				md.WriteString(fmt.Sprintf("- **%s**: %s\n", lessonName, l.Summary))
			} else {
				md.WriteString(fmt.Sprintf("- **%s**: %s\n  **Fix:** %s\n", lessonName, l.Summary, l.Fix))
			}
		}
		md.WriteString("\n")
	}

	// --- 10. Checklist ---
	md.WriteString("## Checklist\n\n")
	md.WriteString("- [ ] Read relevant context nodes above before starting\n")
	md.WriteString("- [ ] Link implementation to modified code nodes with edges\n")

	return md.String()
}

// filterCodeRows returns context rows that represent code nodes (have file_path in tags).
func filterCodeRows(rows []contextRow) []contextRow {
	var result []contextRow
	for _, r := range rows {
		if r.IsCode {
			result = append(result, r)
		}
	}
	return result
}

// parseCodeTags extracts file path, line range, and language from tags JSON.
func parseCodeTags(tagsJSON string) codeTags {
	if tagsJSON == "" {
		return codeTags{}
	}
	var ct codeTags
	if err := json.Unmarshal([]byte(tagsJSON), &ct); err != nil {
		return codeTags{}
	}
	if ct.EndLine == 0 {
		ct.EndLine = ct.StartLine
	}
	return ct
}

// renderCodeSnippets writes inline code snippets for top code nodes.
func renderCodeSnippets(md *strings.Builder, codeRows []contextRow) {
	const snippetLimit = 5
	const snippetMaxLines = 30

	// Filter to reasonable-sized functions, prefer functions over structs
	type candidate struct {
		row  contextRow
		tags codeTags
	}
	var candidates []candidate
	for _, cr := range codeRows {
		ct := parseCodeTags(cr.Tags)
		if ct.FilePath == "" {
			continue
		}
		lineRange := ct.EndLine - ct.StartLine
		if lineRange < 3 || lineRange > 200 {
			continue
		}
		candidates = append(candidates, candidate{cr, ct})
	}

	// Sort: functions first, then structs/enums
	sort.SliceStable(candidates, func(i, j int) bool {
		return isFunctionTitle(candidates[i].row.Title) && !isFunctionTitle(candidates[j].row.Title)
	})

	snippetsAdded := 0
	for _, c := range candidates {
		if snippetsAdded >= snippetLimit {
			break
		}

		fullPath := c.tags.FilePath
		if !filepath.IsAbs(fullPath) {
			cwd, err := os.Getwd()
			if err == nil {
				fullPath = filepath.Join(cwd, fullPath)
			}
		}

		content, err := os.ReadFile(fullPath)
		if err != nil {
			continue
		}
		lines := strings.Split(string(content), "\n")

		startIdx := c.tags.StartLine - 1 // 1-indexed to 0-indexed
		if startIdx < 0 {
			startIdx = 0
		}
		endIdx := c.tags.EndLine
		if endIdx > len(lines) {
			endIdx = len(lines)
		}
		snippetEnd := startIdx + snippetMaxLines
		if snippetEnd > endIdx {
			snippetEnd = endIdx
		}
		if startIdx >= len(lines) || snippetEnd <= startIdx {
			continue
		}

		if snippetsAdded == 0 {
			md.WriteString("### Key Code Snippets\n\n")
			md.WriteString("Top code sections -- read these before exploring further.\n\n")
		}

		titleShort := c.row.Title
		if len(titleShort) > 60 {
			titleShort = titleShort[:60]
		}
		md.WriteString(fmt.Sprintf("**%s** (`%s` L%d-%d):\n",
			titleShort, c.tags.FilePath, c.tags.StartLine, c.tags.EndLine))

		lang := langFromExtension(c.tags.FilePath)
		if c.tags.Language != "" {
			lang = c.tags.Language
		}
		md.WriteString(fmt.Sprintf("```%s\n", lang))
		for _, line := range lines[startIdx:snippetEnd] {
			md.WriteString(line)
			md.WriteString("\n")
		}
		if snippetEnd < endIdx {
			md.WriteString(fmt.Sprintf("// ... (%d more lines)\n", endIdx-snippetEnd))
		}
		md.WriteString("```\n\n")
		snippetsAdded++
	}
}

// renderFilesLikelyTouched groups code nodes by file and renders a ranked list.
func renderFilesLikelyTouched(md *strings.Builder, codeRows []contextRow) {
	// Group by file path
	fileNodes := make(map[string][]string) // file -> titles
	for _, cr := range codeRows {
		ct := parseCodeTags(cr.Tags)
		if ct.FilePath == "" {
			continue
		}
		fileNodes[ct.FilePath] = append(fileNodes[ct.FilePath], cr.Title)
	}

	if len(fileNodes) == 0 {
		return
	}

	// Sort by node count descending
	type fileEntry struct {
		path  string
		nodes []string
	}
	var entries []fileEntry
	for path, nodes := range fileNodes {
		entries = append(entries, fileEntry{path, nodes})
	}
	sort.Slice(entries, func(i, j int) bool {
		return len(entries[i].nodes) > len(entries[j].nodes)
	})

	md.WriteString("### Files Likely Touched\n\n")
	md.WriteString("Ranked by number of relevant code nodes per file:\n\n")

	limit := 8
	if len(entries) < limit {
		limit = len(entries)
	}

	for _, entry := range entries[:limit] {
		// Show up to 3 node names
		showCount := 3
		if len(entry.nodes) < showCount {
			showCount = len(entry.nodes)
		}
		names := make([]string, showCount)
		for i := 0; i < showCount; i++ {
			name := entry.nodes[i]
			if len(name) > 35 {
				name = name[:35]
			}
			names[i] = fmt.Sprintf("`%s`", name)
		}
		suffix := ""
		if len(entry.nodes) > 3 {
			suffix = fmt.Sprintf(" +%d more", len(entry.nodes)-3)
		}
		md.WriteString(fmt.Sprintf("1. **`%s`** (%d nodes) -- %s%s\n",
			entry.path, len(entry.nodes), strings.Join(names, ", "), suffix))
	}
	md.WriteString("\n")
}

// renderCallGraphWithDB renders the call graph section using actual DB edge lookups.
func renderCallGraphWithDB(md *strings.Builder, d *db.DB, context []contextRow) {
	// Find function code nodes (top 3)
	var fnNodeIDs []string
	var fnTitles []string
	for _, cr := range context {
		if !cr.IsCode || !isFunctionTitle(cr.Title) {
			continue
		}
		ct := parseCodeTags(cr.Tags)
		lineRange := ct.EndLine - ct.StartLine
		if lineRange < 3 || lineRange > 500 {
			continue
		}
		fnNodeIDs = append(fnNodeIDs, cr.NodeID)
		fnTitles = append(fnTitles, cr.Title)
		if len(fnNodeIDs) >= 3 {
			break
		}
	}

	if len(fnNodeIDs) == 0 {
		return
	}

	var callLines []string
	for i, nodeID := range fnNodeIDs {
		edges, err := d.GetEdgesForNode(nodeID)
		if err != nil {
			continue
		}

		var callerNames, calleeNames []string
		for _, e := range edges {
			if e.EdgeType != "calls" {
				continue
			}
			if e.TargetID == nodeID {
				// This is a caller
				if n, err := d.GetNode(e.SourceID); err == nil && n != nil {
					name := n.Title
					if len(name) > 30 {
						name = name[:30]
					}
					callerNames = append(callerNames, fmt.Sprintf("`%s`", name))
					if len(callerNames) >= 3 {
						break
					}
				}
			}
		}
		for _, e := range edges {
			if e.EdgeType != "calls" {
				continue
			}
			if e.SourceID == nodeID {
				// This is a callee
				if n, err := d.GetNode(e.TargetID); err == nil && n != nil {
					name := n.Title
					if len(name) > 30 {
						name = name[:30]
					}
					calleeNames = append(calleeNames, fmt.Sprintf("`%s`", name))
					if len(calleeNames) >= 3 {
						break
					}
				}
			}
		}

		if len(callerNames) == 0 && len(calleeNames) == 0 {
			continue
		}

		fnTitle := fnTitles[i]
		if len(fnTitle) > 40 {
			fnTitle = fnTitle[:40]
		}
		line := fmt.Sprintf("- **`%s`**", fnTitle)
		if len(callerNames) > 0 {
			line += fmt.Sprintf(" -- called by: %s", strings.Join(callerNames, ", "))
		}
		if len(calleeNames) > 0 {
			if len(callerNames) > 0 {
				line += ";"
			}
			line += fmt.Sprintf(" calls: %s", strings.Join(calleeNames, ", "))
		}
		callLines = append(callLines, line)
	}

	if len(callLines) > 0 {
		md.WriteString("### Call Graph\n\n")
		md.WriteString("Who calls these functions and what do they call:\n\n")
		for _, line := range callLines {
			md.WriteString(line)
			md.WriteString("\n")
		}
		md.WriteString("\n")
	}
}

// isFunctionTitle checks if a code node title looks like a function definition.
func isFunctionTitle(title string) bool {
	t := strings.TrimSpace(title)
	prefixes := []string{
		"fn ", "pub fn ", "pub(crate) fn ",
		"async fn ", "pub async fn ", "pub(crate) async fn ",
		// Go
		"func ",
		// JS/TS
		"function ", "export function ", "export default function ",
		"async function ", "export async function ",
	}
	for _, p := range prefixes {
		if strings.HasPrefix(t, p) {
			return true
		}
	}
	return false
}

// langFromExtension maps a file extension to a markdown code fence language.
func langFromExtension(filePath string) string {
	ext := filepath.Ext(filePath)
	switch ext {
	case ".rs":
		return "rust"
	case ".ts", ".tsx":
		return "typescript"
	case ".js", ".jsx", ".mjs", ".cjs":
		return "javascript"
	case ".py", ".pyi":
		return "python"
	case ".go":
		return "go"
	case ".c", ".h":
		return "c"
	case ".cpp", ".hpp", ".cc", ".cxx":
		return "cpp"
	case ".java":
		return "java"
	case ".md":
		return "markdown"
	default:
		return ""
	}
}
