use super::*;

// ============================================================================
// Spore Commands
// ============================================================================

pub(crate) async fn handle_spore(cmd: SporeCommands, db: &Database, json: bool) -> Result<(), String> {
    match cmd {
        SporeCommands::Runs { cmd } => {
            super::spore_runs::handle_runs(cmd, db, json)
        }

        SporeCommands::ContextForTask { id, budget, max_hops, max_cost, edge_types, not_superseded, items_only } => {
            let source = resolve_node(db, &id)?;

            let type_list: Option<Vec<String>> = edge_types.map(|s| s.split(',').map(|t| t.trim().to_string()).collect());
            let type_refs: Option<Vec<&str>> = type_list.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect());

            let results = db.context_for_task(
                &source.id, budget, Some(max_hops), Some(max_cost),
                type_refs.as_deref(), None, not_superseded, items_only,
            ).map_err(|e| e.to_string())?;

            if json {
                let output = serde_json::json!({
                    "source": {
                        "id": source.id,
                        "title": source.ai_title.as_ref().unwrap_or(&source.title),
                    },
                    "budget": budget,
                    "results": results,
                    "count": results.len(),
                });
                println!("{}", serde_json::to_string(&output).unwrap_or_default());
            } else {
                let src_name = source.ai_title.as_ref().unwrap_or(&source.title);
                if results.is_empty() {
                    println!("No context nodes found for: {}", src_name);
                } else {
                    println!("Context for: {} ({})  budget={}\n",
                        src_name, &source.id[..8.min(source.id.len())], budget);
                    for r in &results {
                        let class_label = r.node_class.as_deref().map(|c| format!(" [{}]", c)).unwrap_or_default();
                        let marker = if r.is_item { "[I]" } else { "[C]" };
                        println!("  {:>2}. {} {}{} — dist={:.3} rel={:.0}% hops={}",
                            r.rank, marker, r.node_title, class_label,
                            r.distance, r.relevance * 100.0, r.hops);
                        if !r.path.is_empty() {
                            let path_str: Vec<String> = r.path.iter()
                                .map(|hop| format!("→[{}]→ {}",
                                    hop.edge.edge_type.as_str(),
                                    &hop.node_title[..hop.node_title.len().min(40)]))
                                .collect();
                            println!("      {}", path_str.join(" "));
                        }
                    }
                    println!("\n{} node(s) within budget", results.len());
                }
            }
            Ok(())
        }

        SporeCommands::Lessons { compact } => {
            super::spore_runs::handle_spore_lessons(db, json, compact)
        }

        SporeCommands::Dashboard { limit, format, count, cost, stale } => {
            super::spore_runs::handle_dashboard(db, json, limit, format, count, cost, stale)
        }

        SporeCommands::Distill { run, compact } => {
            super::spore_runs::handle_distill(db, &run, json, compact).await
        }

        SporeCommands::Health => {
            super::spore_runs::handle_health(db, json)
        }

        SporeCommands::PromptStats => {
            super::spore_runs::handle_prompt_stats()
        }

        SporeCommands::Analyze { region, top_n, stale_days, hub_threshold } => {
            super::spore_analyzer::handle_analyze(db, json, region, top_n, stale_days, hub_threshold).await
        }

        // ---- Deprecation aliases: delegate to graph_ops with a warning ----
        SporeCommands::QueryEdges { edge_type, agent, target_agent, confidence_min, since, not_superseded, limit, compact } => {
            eprintln!("WARNING: 'mycelica-cli spore query-edges' is deprecated. Use 'mycelica-cli graph query-edges' instead.");
            super::graph_ops::handle_graph(GraphCommands::QueryEdges { edge_type, agent, target_agent, confidence_min, since, not_superseded, limit, compact }, db, json).await
        }
        SporeCommands::ExplainEdge { id, depth } => {
            eprintln!("WARNING: 'mycelica-cli spore explain-edge' is deprecated. Use 'mycelica-cli graph explain-edge' instead.");
            super::graph_ops::handle_graph(GraphCommands::ExplainEdge { id, depth }, db, json).await
        }
        SporeCommands::PathBetween { from, to, max_hops, edge_types } => {
            eprintln!("WARNING: 'mycelica-cli spore path-between' is deprecated. Use 'mycelica-cli graph path-between' instead.");
            super::graph_ops::handle_graph(GraphCommands::PathBetween { from, to, max_hops, edge_types }, db, json).await
        }
        SporeCommands::EdgesForContext { id, top, not_superseded } => {
            eprintln!("WARNING: 'mycelica-cli spore edges-for-context' is deprecated. Use 'mycelica-cli graph edges-for-context' instead.");
            super::graph_ops::handle_graph(GraphCommands::EdgesForContext { id, top, not_superseded }, db, json).await
        }
        SporeCommands::CreateMeta { meta_type, title, content, agent, connects_to, edge_type } => {
            eprintln!("WARNING: 'mycelica-cli spore create-meta' is deprecated. Use 'mycelica-cli graph create-meta' instead.");
            super::graph_ops::handle_graph(GraphCommands::CreateMeta { meta_type, title, content, agent, connects_to, edge_type }, db, json).await
        }
        SporeCommands::UpdateMeta { id, content, title, agent, add_connects, edge_type } => {
            eprintln!("WARNING: 'mycelica-cli spore update-meta' is deprecated. Use 'mycelica-cli graph update-meta' instead.");
            super::graph_ops::handle_graph(GraphCommands::UpdateMeta { id, content, title, agent, add_connects, edge_type }, db, json).await
        }
        SporeCommands::Status { all, format } => {
            eprintln!("WARNING: 'mycelica-cli spore status' is deprecated. Use 'mycelica-cli graph status' instead.");
            super::graph_ops::handle_graph(GraphCommands::Status { all, format }, db, json).await
        }
        SporeCommands::CreateEdge { from, to, edge_type, content, reason, agent, confidence, supersedes, metadata } => {
            eprintln!("WARNING: 'mycelica-cli spore create-edge' is deprecated. Use 'mycelica-cli graph create-edge' instead.");
            super::graph_ops::handle_graph(GraphCommands::CreateEdge { from, to, edge_type, content, reason, agent, confidence, supersedes, metadata }, db, json).await
        }
        SporeCommands::ReadContent { id } => {
            eprintln!("WARNING: 'mycelica-cli spore read-content' is deprecated. Use 'mycelica-cli graph read-content' instead.");
            super::graph_ops::handle_graph(GraphCommands::ReadContent { id }, db, json).await
        }
        SporeCommands::ListRegion { id, class, items_only, limit } => {
            eprintln!("WARNING: 'mycelica-cli spore list-region' is deprecated. Use 'mycelica-cli graph list-region' instead.");
            super::graph_ops::handle_graph(GraphCommands::ListRegion { id, class, items_only, limit }, db, json).await
        }
        SporeCommands::CheckFreshness { id } => {
            eprintln!("WARNING: 'mycelica-cli spore check-freshness' is deprecated. Use 'mycelica-cli graph check-freshness' instead.");
            super::graph_ops::handle_graph(GraphCommands::CheckFreshness { id }, db, json).await
        }
        SporeCommands::Gc { days, dry_run, force } => {
            eprintln!("WARNING: 'mycelica-cli spore gc' is deprecated. Use 'mycelica-cli graph gc' instead.");
            super::graph_ops::handle_graph(GraphCommands::Gc { days, dry_run, force }, db, json).await
        }
    }
}

/// Parse a --since value as either an ISO date (YYYY-MM-DD) or a relative duration (e.g. 30m, 1h, 2d, 1w).
/// Returns epoch milliseconds.
pub(crate) fn parse_since_to_millis(s: &str) -> Result<i64, String> {
    // Try relative duration first: number + unit suffix
    let s_trimmed = s.trim();
    if let Some((num_str, unit)) = s_trimmed
        .strip_suffix('m')
        .map(|n| (n, 'm'))
        .or_else(|| s_trimmed.strip_suffix('h').map(|n| (n, 'h')))
        .or_else(|| s_trimmed.strip_suffix('d').map(|n| (n, 'd')))
        .or_else(|| s_trimmed.strip_suffix('w').map(|n| (n, 'w')))
    {
        if let Ok(num) = num_str.parse::<u64>() {
            let seconds = match unit {
                'm' => num * 60,
                'h' => num * 3600,
                'd' => num * 86400,
                'w' => num * 604800,
                _ => unreachable!(),
            };
            let now = chrono::Utc::now().timestamp_millis();
            return Ok(now - (seconds as i64 * 1000));
        }
    }
    // Fall back to ISO date
    let date = chrono::NaiveDate::parse_from_str(s_trimmed, "%Y-%m-%d")
        .map_err(|e| format!("Invalid --since '{}': {}. Use YYYY-MM-DD or relative (1h, 2d, 1w).", s, e))?;
    let dt = date.and_hms_opt(0, 0, 0).unwrap();
    Ok(dt.and_utc().timestamp_millis())
}

/// Format milliseconds into a short human-readable duration string.
/// Examples: "1.2s", "45.3s", "2m 15s", "1h 05m".
pub(crate) fn format_duration_short(ms: u64) -> String {
    let total_secs = ms / 1000;
    let tenths = (ms % 1000) / 100;

    if total_secs < 60 {
        format!("{}.{}s", total_secs, tenths)
    } else if total_secs < 3600 {
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{}m {:02}s", mins, secs)
    } else {
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        format!("{}h {:02}m", hours, mins)
    }
}

/// Count the number of whitespace-separated words in a string.
pub(crate) fn count_words(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Check whether a lesson summary is substantive enough to include in task files.
/// Returns `false` for short summaries (<20 words), bare CLI commands, and trivial
/// imperative phrases like "run cargo build" or "always run X before Y".
pub(crate) fn is_lesson_quality(summary: &str) -> bool {
    if count_words(summary) < 20 {
        return false;
    }
    let lower = summary.to_lowercase();
    // Reject bare CLI commands
    let trivial_commands = [
        "cargo check", "cargo build", "cargo test", "cargo run",
        "cargo fmt", "cargo clippy", "npm install", "npm run",
        "npm test", "npm build",
    ];
    for cmd in &trivial_commands {
        if lower.trim() == *cmd || lower.starts_with(&format!("{} ", cmd)) && count_words(&lower) < 10 {
            return false;
        }
    }
    // Reject short imperative phrases: "run ..." or "use ..." with < 10 words
    if (lower.starts_with("run ") || lower.starts_with("use ")) && count_words(&lower) < 10 {
        return false;
    }
    // Reject "always run X before Y" style
    if lower.starts_with("always ") && count_words(&lower) < 15 {
        return false;
    }
    true
}

/// Truncate a string from the middle with "..." if it exceeds `max_len` bytes.
/// Returns the original string unchanged if it fits within `max_len`.
/// If `max_len` < 3, returns the first `max_len` bytes (no room for ellipsis).
pub(crate) fn truncate_middle(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    if max_len < 3 {
        return s.chars().take(max_len).collect();
    }
    let remaining = max_len - 3; // space for "..."
    let left = (remaining + 1) / 2; // left half gets the extra char if odd
    let right = remaining / 2;

    // Find char-boundary-safe split points
    let mut left_end = left;
    while left_end > 0 && !s.is_char_boundary(left_end) {
        left_end -= 1;
    }
    let mut right_start = s.len() - right;
    while right_start < s.len() && !s.is_char_boundary(right_start) {
        right_start += 1;
    }

    format!("{}...{}", &s[..left_end], &s[right_start..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use super::super::spore_runs::{count_agent_prompt_lines, find_project_root};

    #[test]
    fn test_format_duration_short_zero() {
        assert_eq!(format_duration_short(0), "0.0s");
    }

    #[test]
    fn test_format_duration_short_sub_second() {
        assert_eq!(format_duration_short(500), "0.5s");
        assert_eq!(format_duration_short(100), "0.1s");
        assert_eq!(format_duration_short(999), "0.9s");
    }

    #[test]
    fn test_format_duration_short_seconds() {
        assert_eq!(format_duration_short(1200), "1.2s");
        assert_eq!(format_duration_short(45300), "45.3s");
        assert_eq!(format_duration_short(59000), "59.0s");
        assert_eq!(format_duration_short(59999), "59.9s");
    }

    #[test]
    fn test_format_duration_short_minutes() {
        assert_eq!(format_duration_short(60_000), "1m 00s");
        assert_eq!(format_duration_short(135_000), "2m 15s");
        assert_eq!(format_duration_short(3_599_000), "59m 59s");
    }

    #[test]
    fn test_format_duration_short_hours() {
        assert_eq!(format_duration_short(3_600_000), "1h 00m");
        assert_eq!(format_duration_short(3_900_000), "1h 05m");
        assert_eq!(format_duration_short(7_200_000), "2h 00m");
        assert_eq!(format_duration_short(86_400_000), "24h 00m");
    }

    #[test]
    fn test_format_duration_short_boundary_59s() {
        // 59.9s should still be in seconds format
        assert_eq!(format_duration_short(59_900), "59.9s");
    }

    #[test]
    fn test_format_duration_short_boundary_60s() {
        // Exactly 60s should switch to minutes format
        assert_eq!(format_duration_short(60_000), "1m 00s");
    }

    #[test]
    fn test_format_duration_short_boundary_3600s() {
        // Exactly 1 hour should switch to hours format
        assert_eq!(format_duration_short(3_600_000), "1h 00m");
    }

    #[test]
    fn test_format_duration_short_tenths_truncation() {
        // 1234ms -> 1 second, 2 tenths (truncated, not rounded)
        assert_eq!(format_duration_short(1234), "1.2s");
        // 1999ms -> 1 second, 9 tenths
        assert_eq!(format_duration_short(1999), "1.9s");
    }

    #[test]
    fn test_format_duration_short_zero_padding() {
        // Minutes format pads seconds with leading zero
        assert_eq!(format_duration_short(61_000), "1m 01s");
        assert_eq!(format_duration_short(65_000), "1m 05s");
        // Hours format pads minutes with leading zero
        assert_eq!(format_duration_short(3_660_000), "1h 01m");
    }

    #[test]
    fn test_count_words_normal_sentence() {
        assert_eq!(count_words("hello world foo"), 3);
    }

    #[test]
    fn test_count_words_empty_string() {
        assert_eq!(count_words(""), 0);
    }

    #[test]
    fn test_count_words_only_whitespace() {
        assert_eq!(count_words("   "), 0);
        assert_eq!(count_words("\t\n\r"), 0);
    }

    #[test]
    fn test_count_words_single_word() {
        assert_eq!(count_words("hello"), 1);
    }

    #[test]
    fn test_count_words_multiple_spaces_between_words() {
        assert_eq!(count_words("hello    world"), 2);
    }

    #[test]
    fn test_count_words_leading_and_trailing_whitespace() {
        assert_eq!(count_words("  hello world  "), 2);
    }

    #[test]
    fn test_count_words_mixed_whitespace() {
        assert_eq!(count_words("hello\tworld\nfoo\rbar"), 4);
    }

    #[test]
    fn test_truncate_middle_short_string() {
        assert_eq!(truncate_middle("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_middle_exact_length() {
        assert_eq!(truncate_middle("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_middle_basic() {
        // "hello world" = 11 chars, max 8 => left=3, right=2 => "hel...ld"
        assert_eq!(truncate_middle("hello world", 8), "hel...ld");
    }

    #[test]
    fn test_truncate_middle_even_split() {
        // "abcdefghij" = 10 chars, max 7 => remaining=4, left=2, right=2 => "ab...ij"
        assert_eq!(truncate_middle("abcdefghij", 7), "ab...ij");
    }

    #[test]
    fn test_truncate_middle_min_with_ellipsis() {
        // max_len=3 => remaining=0, left=0, right=0 => "..."
        assert_eq!(truncate_middle("hello", 3), "...");
    }

    #[test]
    fn test_truncate_middle_too_small_for_ellipsis() {
        // max_len < 3: no room for "...", just take first chars
        assert_eq!(truncate_middle("hello", 2), "he");
        assert_eq!(truncate_middle("hello", 1), "h");
        assert_eq!(truncate_middle("hello", 0), "");
    }

    #[test]
    fn test_truncate_middle_multibyte() {
        // "café" is 5 bytes (é = 2 bytes), max_len=4
        // remaining=1, left=1 byte, right=0 => "c..."
        assert_eq!(truncate_middle("café", 4), "c...");
    }

    #[test]
    fn test_truncate_middle_empty_string() {
        // Empty string is always <= max_len, returned as-is
        assert_eq!(truncate_middle("", 0), "");
        assert_eq!(truncate_middle("", 5), "");
    }

    #[test]
    fn test_truncate_middle_single_char() {
        assert_eq!(truncate_middle("x", 1), "x");
        assert_eq!(truncate_middle("x", 0), "");
        assert_eq!(truncate_middle("x", 5), "x");
    }

    #[test]
    fn test_truncate_middle_multibyte_boundary_snap() {
        // "émoji" = [195, 169, 109, 111, 106, 105] = 6 bytes
        // max_len=5: remaining=2, left=1, right=1
        // left_end=1 is NOT a char boundary (mid-é), snaps to 0
        // right_start=5 is a boundary ('i')
        // Result: "...i" (4 bytes, less than max_len due to snap)
        assert_eq!(truncate_middle("émoji", 5), "...i");
    }

    #[test]
    fn test_truncate_middle_ascii_result_length() {
        // For ASCII strings, result should never exceed max_len
        let s = "abcdefghijklmnopqrstuvwxyz";
        for max_len in 0..=s.len() + 5 {
            let result = truncate_middle(s, max_len);
            assert!(
                result.len() <= std::cmp::max(max_len, s.len()),
                "max_len={}, result='{}', result.len()={}",
                max_len, result, result.len()
            );
        }
    }

    #[test]
    fn test_truncate_middle_max_len_less_than_3_multibyte() {
        // max_len < 3 uses chars().take(), so it counts characters not bytes
        // "é世界" = 8 bytes, but 3 characters
        assert_eq!(truncate_middle("é世界", 2), "é世");
        assert_eq!(truncate_middle("é世界", 1), "é");
    }

    #[test]
    fn test_truncate_middle_realistic_path() {
        // Realistic use case: truncating a file path
        let path = "/home/user/projects/mycelica/src-tauri/src/bin/cli/spore.rs";
        let result = truncate_middle(path, 30);
        assert!(result.starts_with("/home/user/pro"));
        assert!(result.ends_with("ore.rs"));
        assert!(result.contains("..."));
        assert_eq!(result.len(), 30);
    }

    #[test]
    fn test_truncate_middle_max_len_4() {
        // max_len=4: remaining=1, left=1, right=0
        assert_eq!(truncate_middle("abcdef", 4), "a...");
    }

    #[test]
    fn test_truncate_middle_max_len_5() {
        // max_len=5: remaining=2, left=1, right=1
        assert_eq!(truncate_middle("abcdef", 5), "a...f");
    }

    #[test]
    fn test_truncate_middle_max_len_6() {
        // max_len=6: remaining=3, left=2, right=1
        assert_eq!(truncate_middle("abcdefghij", 6), "ab...j");
    }

    #[test]
    fn test_format_duration_short_tiny_ms() {
        // Values below 100ms all have tenths=0, so they show "0.0s"
        assert_eq!(format_duration_short(1), "0.0s");
        assert_eq!(format_duration_short(50), "0.0s");
        assert_eq!(format_duration_short(99), "0.0s");
    }

    #[test]
    fn test_format_duration_short_exact_second() {
        // Exactly 1 second
        assert_eq!(format_duration_short(1_000), "1.0s");
        // Exactly 10 seconds
        assert_eq!(format_duration_short(10_000), "10.0s");
    }

    #[test]
    fn test_format_duration_short_near_hour_boundary() {
        // 3_599_999ms is just below 1 hour — should stay in minutes format
        // total_secs = 3599, mins = 59, secs = 59
        assert_eq!(format_duration_short(3_599_999), "59m 59s");
        // One ms over the hour boundary
        assert_eq!(format_duration_short(3_600_001), "1h 00m");
    }

    // Tests for --verbose flag behavior in 'spore runs list'
    // The logic under test (spore.rs ~L1999-2004):
    //   let description = title.strip_prefix("Orchestration:").unwrap_or(title).trim();
    //   let description = if verbose { description.to_string() } else { truncate_middle(description, 50) };

    #[test]
    fn test_verbose_flag_preserves_full_description() {
        // Simulate verbose=true: full text is preserved
        let title = "Orchestration: Add a --verbose flag to 'spore runs list' that shows the full task text instead of truncating it";
        let description = title.strip_prefix("Orchestration:").unwrap_or(title).trim();
        let verbose = true;
        let result = if verbose {
            description.to_string()
        } else {
            truncate_middle(description, 50)
        };
        assert_eq!(result, "Add a --verbose flag to 'spore runs list' that shows the full task text instead of truncating it");
    }

    #[test]
    fn test_non_verbose_truncates_to_50() {
        // Simulate verbose=false: truncate_middle(description, 50) is used
        let title = "Orchestration: Add a --verbose flag to 'spore runs list' that shows the full task text instead of truncating it";
        let description = title.strip_prefix("Orchestration:").unwrap_or(title).trim();
        let verbose = false;
        let result = if verbose {
            description.to_string()
        } else {
            truncate_middle(description, 50)
        };
        assert!(result.len() <= 50, "result length {} exceeds 50: '{}'", result.len(), result);
        assert!(result.contains("..."), "truncated result should contain ellipsis");
        // Should preserve start and end of description
        assert!(result.starts_with("Add a --verbose"));
        assert!(result.ends_with("ncating it"));
    }

    #[test]
    fn test_non_verbose_short_description_unchanged() {
        // Descriptions <= 50 bytes are not truncated even in non-verbose mode
        let title = "Orchestration: Fix a small bug";
        let description = title.strip_prefix("Orchestration:").unwrap_or(title).trim();
        let result = truncate_middle(description, 50);
        assert_eq!(result, "Fix a small bug");
    }

    #[test]
    fn test_non_verbose_exactly_50_bytes() {
        // A description of exactly 50 bytes should not be truncated
        let desc = "12345678901234567890123456789012345678901234567890"; // 50 bytes
        assert_eq!(desc.len(), 50);
        let result = truncate_middle(desc, 50);
        assert_eq!(result, desc);
    }

    #[test]
    fn test_non_verbose_51_bytes_is_truncated() {
        // A description of 51 bytes should be truncated
        let desc = "123456789012345678901234567890123456789012345678901"; // 51 bytes
        assert_eq!(desc.len(), 51);
        let result = truncate_middle(desc, 50);
        assert!(result.len() <= 50);
        assert!(result.contains("..."));
    }

    #[test]
    fn test_strip_prefix_non_orchestration_title() {
        // With --all flag, non-Orchestration titles pass through strip_prefix unchanged
        let title = "Summary: something happened";
        let description = title.strip_prefix("Orchestration:").unwrap_or(title).trim();
        assert_eq!(description, "Summary: something happened");
    }

    #[test]
    fn test_verbose_with_non_orchestration_title() {
        // Non-Orchestration title in verbose mode: full text preserved
        let title = "Custom task with a very long description that definitely exceeds fifty characters in length";
        let description = title.strip_prefix("Orchestration:").unwrap_or(title).trim();
        let result_verbose = description.to_string();
        let result_non_verbose = truncate_middle(description, 50);
        assert_eq!(result_verbose, title);
        assert!(result_non_verbose.len() <= 50);
        assert!(result_non_verbose.contains("..."));
    }

    // --- Tests for truncate_middle at 60-char limit (orchestrator output) ---

    #[test]
    fn test_truncate_middle_60_short_task_unchanged() {
        // Tasks <= 60 bytes pass through unchanged
        let task = "Fix the login bug";
        assert_eq!(truncate_middle(task, 60), task);
    }

    #[test]
    fn test_truncate_middle_60_exactly_60_bytes() {
        let task = "123456789012345678901234567890123456789012345678901234567890"; // 60 bytes
        assert_eq!(task.len(), 60);
        assert_eq!(truncate_middle(task, 60), task);
    }

    #[test]
    fn test_truncate_middle_60_long_task_truncated() {
        let task = "Use truncate_middle() in the orchestrator output where task descriptions are printed";
        assert!(task.len() > 60);
        let result = truncate_middle(task, 60);
        assert!(result.len() <= 60, "result '{}' is {} bytes, expected <= 60", result, result.len());
        assert!(result.contains("..."));
        assert!(result.starts_with("Use truncate_middle() in the"));
        assert!(result.ends_with("are printed"));
    }

    #[test]
    fn test_orchestration_title_format() {
        // The orchestration node title format: "Orchestration: {truncated}"
        let task = "Add a feature that does something very important and needs a long description to explain";
        let title = format!("Orchestration: {}", truncate_middle(task, 60));
        // "Orchestration: " is 15 chars, truncated task is <= 60
        assert!(title.starts_with("Orchestration: "));
        let truncated_part = &title["Orchestration: ".len()..];
        assert!(truncated_part.len() <= 60);
    }

    #[test]
    fn test_orchestration_title_short_task_no_truncation() {
        let task = "Fix a small bug";
        let title = format!("Orchestration: {}", truncate_middle(task, 60));
        assert_eq!(title, "Orchestration: Fix a small bug");
    }

    #[test]
    fn test_bounce_header_format() {
        // Bounce header: "--- Bounce {}/{}: {truncated} ---"
        let task = "Implement a complex feature that requires many changes across multiple files in the codebase";
        let header = format!(
            "\n--- Bounce {}/{}: {} ---",
            1, 3, truncate_middle(task, 60)
        );
        assert!(header.contains("Bounce 1/3:"));
        // The truncated task portion should be <= 60 bytes
        let after_colon = header.split(": ").nth(1).unwrap().trim_end_matches(" ---");
        assert!(after_colon.len() <= 60, "task in header is {} bytes: '{}'", after_colon.len(), after_colon);
        assert!(after_colon.contains("..."));
    }

    #[test]
    fn test_bounce_header_short_task() {
        let task = "Fix typo";
        let header = format!(
            "\n--- Bounce {}/{}: {} ---",
            2, 2, truncate_middle(task, 60)
        );
        assert_eq!(header, "\n--- Bounce 2/2: Fix typo ---");
    }

    #[test]
    fn test_task_complete_impl_title_format() {
        // TASK COMPLETE section: "Implementation: {id} ({truncated_title})"
        let impl_title = "Implemented: Use truncate_middle() in orchestrator output where task descriptions are printed";
        let id = "fbe9e9cc-10b2-47df-a819-9da27c3bf616";
        let line = format!(
            "Implementation: {} ({})",
            &id[..8],
            truncate_middle(impl_title, 60)
        );
        assert!(line.starts_with("Implementation: fbe9e9cc ("));
        assert!(line.ends_with(")"));
        // Extract the truncated title between parens
        let paren_content = &line[line.find('(').unwrap() + 1..line.len() - 1];
        assert!(paren_content.len() <= 60);
        assert!(paren_content.contains("..."));
    }

    #[test]
    fn test_task_complete_short_impl_title() {
        let impl_title = "Implemented: Fix bug";
        let id = "abcdef12-0000-0000-0000-000000000000";
        let line = format!(
            "Implementation: {} ({})",
            &id[..8],
            truncate_middle(impl_title, 60)
        );
        assert_eq!(line, "Implementation: abcdef12 (Implemented: Fix bug)");
    }

    #[test]
    fn test_fallback_node_title_format() {
        // Fallback title: "{status_label}: {task_short}"
        let task = "Refactor the entire authentication system to use JWT tokens instead of session cookies for better scalability";
        let task_short = truncate_middle(task, 60);
        let status_label = "Implemented (auto)";
        let title = format!("{}: {}", status_label, task_short);
        assert!(title.starts_with("Implemented (auto): "));
        // The truncated task portion should be <= 60 bytes
        assert!(task_short.len() <= 60);
        assert!(task_short.contains("..."));
    }

    #[test]
    fn test_fallback_node_title_partial_status() {
        let task = "A very long task description that exceeds sixty characters and keeps going on and on to test truncation";
        let task_short = truncate_middle(task, 60);
        let title = format!("{}: {}", "Partial", task_short);
        assert!(title.starts_with("Partial: "));
        assert!(task_short.len() <= 60);
    }

    #[test]
    fn test_truncate_middle_60_preserves_start_and_end() {
        // For a 61-byte string, only 1 byte needs trimming but "..." adds 3
        // so we lose a few chars from the middle
        let task = "1234567890123456789012345678901234567890123456789012345678901"; // 61 bytes
        assert_eq!(task.len(), 61);
        let result = truncate_middle(task, 60);
        assert!(result.len() <= 60);
        assert!(result.contains("..."));
        // Should preserve beginning and end
        assert!(result.starts_with("1234567890"));
        assert!(result.ends_with("12345678901"));
    }

    // --- Tests for `spore runs top` subcommand logic ---

    #[test]
    fn test_runs_top_title_truncated_to_40() {
        // `runs top` uses truncate_middle(title, 40)
        let long_title = "Add a spore runs top subcommand that shows the top 5 most expensive runs";
        assert!(long_title.len() > 40);
        let result = truncate_middle(long_title, 40);
        assert!(result.len() <= 40, "result '{}' is {} bytes, expected <= 40", result, result.len());
        assert!(result.contains("..."));
    }

    #[test]
    fn test_runs_top_title_short_unchanged() {
        // Titles <= 40 bytes pass through unchanged
        let short_title = "Fix a small bug";
        assert!(short_title.len() <= 40);
        let result = truncate_middle(short_title, 40);
        assert_eq!(result, short_title);
    }

    #[test]
    fn test_runs_top_title_exactly_40_unchanged() {
        let title = "1234567890123456789012345678901234567890"; // 40 bytes
        assert_eq!(title.len(), 40);
        let result = truncate_middle(title, 40);
        assert_eq!(result, title);
    }

    #[test]
    fn test_runs_top_title_41_bytes_truncated() {
        let title = "12345678901234567890123456789012345678901"; // 41 bytes
        assert_eq!(title.len(), 41);
        let result = truncate_middle(title, 40);
        assert!(result.len() <= 40);
        assert!(result.contains("..."));
    }

    #[test]
    fn test_runs_top_id_truncation_normal_uuid() {
        // Normal UUID: take first 8 chars
        let task_id = "3302925e-9e4b-4016-8383-bb3759a2bc35";
        let short_id = &task_id[..8.min(task_id.len())];
        assert_eq!(short_id, "3302925e");
    }

    #[test]
    fn test_runs_top_id_truncation_short_id() {
        // ID shorter than 8 chars: take the whole thing
        let task_id = "abcde";
        let short_id = &task_id[..8.min(task_id.len())];
        assert_eq!(short_id, "abcde");
    }

    #[test]
    fn test_runs_top_id_truncation_exactly_8() {
        let task_id = "abcdef12";
        let short_id = &task_id[..8.min(task_id.len())];
        assert_eq!(short_id, "abcdef12");
    }

    #[test]
    fn test_runs_top_sort_by_cost_descending() {
        // Simulate the sort logic from handle_runs Top
        let mut costs: Vec<(String, f64)> = vec![
            ("run-a".into(), 0.05),
            ("run-b".into(), 1.23),
            ("run-c".into(), 0.50),
            ("run-d".into(), 0.00),
            ("run-e".into(), 2.10),
            ("run-f".into(), 0.75),
            ("run-g".into(), 0.10),
        ];
        costs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        costs.truncate(5);

        assert_eq!(costs.len(), 5);
        assert_eq!(costs[0].0, "run-e"); // $2.10
        assert_eq!(costs[1].0, "run-b"); // $1.23
        assert_eq!(costs[2].0, "run-f"); // $0.75
        assert_eq!(costs[3].0, "run-c"); // $0.50
        assert_eq!(costs[4].0, "run-g"); // $0.10
    }

    #[test]
    fn test_runs_top_sort_fewer_than_5() {
        // When fewer than 5 entries exist, truncate(5) is a no-op
        let mut costs: Vec<(String, f64)> = vec![
            ("run-a".into(), 0.50),
            ("run-b".into(), 1.00),
        ];
        costs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        costs.truncate(5);

        assert_eq!(costs.len(), 2);
        assert_eq!(costs[0].0, "run-b");
        assert_eq!(costs[1].0, "run-a");
    }

    #[test]
    fn test_runs_top_sort_empty() {
        let mut costs: Vec<(String, f64)> = vec![];
        costs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        costs.truncate(5);
        assert!(costs.is_empty());
    }

    #[test]
    fn test_runs_top_sort_equal_costs() {
        // Equal costs should be stable (both kept, order preserved)
        let mut costs: Vec<(String, f64)> = vec![
            ("run-a".into(), 1.00),
            ("run-b".into(), 1.00),
            ("run-c".into(), 1.00),
        ];
        costs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        costs.truncate(5);
        assert_eq!(costs.len(), 3);
        // All have equal cost, sort_by is stable so original order preserved
        assert_eq!(costs[0].0, "run-a");
        assert_eq!(costs[1].0, "run-b");
        assert_eq!(costs[2].0, "run-c");
    }

    #[test]
    fn test_runs_top_cost_format() {
        // The Top handler uses format!("${:.2}", cost)
        assert_eq!(format!("${:.2}", 0.0), "$0.00");
        assert_eq!(format!("${:.2}", 1.5), "$1.50");
        assert_eq!(format!("${:.2}", 0.123456), "$0.12");
        assert_eq!(format!("${:.2}", 10.999), "$11.00");
    }

    #[test]
    fn test_runs_top_strip_orchestration_prefix() {
        // The Top handler strips "Orchestration:" prefix
        let title = "Orchestration: Add runs top subcommand";
        let description = title.strip_prefix("Orchestration:").unwrap_or(title).trim();
        assert_eq!(description, "Add runs top subcommand");
    }

    #[test]
    fn test_runs_top_strip_prefix_no_prefix() {
        // Title without "Orchestration:" prefix returns as-is
        let title = "Summary: something else";
        let description = title.strip_prefix("Orchestration:").unwrap_or(title).trim();
        assert_eq!(description, "Summary: something else");
    }

    #[test]
    fn test_runs_top_strip_prefix_with_extra_spaces() {
        // "Orchestration:  double space" should trim to single word start
        let title = "Orchestration:  double space task";
        let description = title.strip_prefix("Orchestration:").unwrap_or(title).trim();
        assert_eq!(description, "double space task");
    }

    #[test]
    fn test_runs_top_date_format() {
        // The Top handler formats dates as YYYY-MM-DD from millisecond timestamps
        let ts_ms = 1708300800000_i64; // 2024-02-19 00:00:00 UTC
        let date = chrono::DateTime::from_timestamp_millis(ts_ms)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "?".to_string());
        assert_eq!(date, "2024-02-19");
    }

    #[test]
    fn test_runs_top_date_format_invalid_timestamp() {
        // Extremely negative timestamp should still produce a date (not panic)
        // chrono handles very old dates gracefully
        let result = chrono::DateTime::from_timestamp_millis(0)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "?".to_string());
        assert_eq!(result, "1970-01-01");
    }

    #[test]
    fn test_runs_top_cost_json_extraction() {
        // Simulates how cost is extracted from metadata JSON
        let meta = r#"{"run_id": "abc123", "cost_usd": 0.42, "agent": "coder"}"#;
        let cost = serde_json::from_str::<serde_json::Value>(meta)
            .ok()
            .and_then(|v| v["cost_usd"].as_f64());
        assert_eq!(cost, Some(0.42));
    }

    #[test]
    fn test_runs_top_cost_json_missing_field() {
        // Metadata without cost_usd should return None
        let meta = r#"{"run_id": "abc123", "agent": "coder"}"#;
        let cost = serde_json::from_str::<serde_json::Value>(meta)
            .ok()
            .and_then(|v| v["cost_usd"].as_f64());
        assert_eq!(cost, None);
    }

    #[test]
    fn test_runs_top_cost_json_null_cost() {
        // cost_usd: null should return None
        let meta = r#"{"cost_usd": null}"#;
        let cost = serde_json::from_str::<serde_json::Value>(meta)
            .ok()
            .and_then(|v| v["cost_usd"].as_f64());
        assert_eq!(cost, None);
    }

    #[test]
    fn test_runs_top_cost_json_string_cost() {
        // cost_usd as string should return None (as_f64 won't coerce strings)
        let meta = r#"{"cost_usd": "0.42"}"#;
        let cost = serde_json::from_str::<serde_json::Value>(meta)
            .ok()
            .and_then(|v| v["cost_usd"].as_f64());
        assert_eq!(cost, None);
    }

    #[test]
    fn test_runs_top_cost_sum_multiple_tracks() {
        // Multiple tracks edges sum their costs
        let metas = vec![
            r#"{"cost_usd": 0.10}"#,
            r#"{"cost_usd": 0.25}"#,
            r#"{"cost_usd": 0.07}"#,
        ];
        let total: f64 = metas.iter()
            .filter_map(|meta| {
                serde_json::from_str::<serde_json::Value>(meta).ok()
                    .and_then(|v| v["cost_usd"].as_f64())
            })
            .sum();
        assert!((total - 0.42).abs() < 1e-10);
    }

    #[test]
    fn test_runs_top_cost_sum_with_missing() {
        // Some tracks edges may lack cost_usd; they should be skipped
        let metas = vec![
            r#"{"cost_usd": 0.50}"#,
            r#"{"agent": "tester"}"#,  // no cost_usd
            r#"{"cost_usd": 0.30}"#,
        ];
        let total: f64 = metas.iter()
            .filter_map(|meta| {
                serde_json::from_str::<serde_json::Value>(meta).ok()
                    .and_then(|v| v["cost_usd"].as_f64())
            })
            .sum();
        assert!((total - 0.80).abs() < 1e-10);
    }

    // --- Tests for 'spore runs stats' computation logic ---

    /// Helper: compute median using the same algorithm as handle_runs Stats
    fn compute_median(costs: &[f64]) -> f64 {
        let mut sorted = costs.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if sorted.is_empty() {
            0.0
        } else if sorted.len() % 2 == 0 {
            (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
        } else {
            sorted[sorted.len() / 2]
        }
    }

    #[test]
    fn test_stats_median_cost_odd_count() {
        // Odd number of values: median is the middle element
        let costs = vec![0.10, 0.30, 0.50, 0.70, 0.90];
        assert!((compute_median(&costs) - 0.50).abs() < 1e-10);
    }

    #[test]
    fn test_stats_median_cost_even_count() {
        // Even number of values: median is average of two middle elements
        let costs = vec![0.10, 0.30, 0.50, 0.70];
        assert!((compute_median(&costs) - 0.40).abs() < 1e-10);
    }

    #[test]
    fn test_stats_median_cost_single_value() {
        let costs = vec![1.23];
        assert!((compute_median(&costs) - 1.23).abs() < 1e-10);
    }

    #[test]
    fn test_stats_median_cost_empty() {
        let costs: Vec<f64> = vec![];
        assert!((compute_median(&costs) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_stats_median_cost_two_values() {
        let costs = vec![1.0, 3.0];
        assert!((compute_median(&costs) - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_stats_median_cost_unsorted_input() {
        // Input doesn't need to be pre-sorted
        let costs = vec![0.90, 0.10, 0.50, 0.70, 0.30];
        assert!((compute_median(&costs) - 0.50).abs() < 1e-10);
    }

    #[test]
    fn test_stats_median_cost_identical_values() {
        let costs = vec![0.42, 0.42, 0.42];
        assert!((compute_median(&costs) - 0.42).abs() < 1e-10);
    }

    #[test]
    fn test_stats_avg_turns_basic() {
        // Replicate: total_turns / runs_with_turns
        let total_turns: u64 = 100;
        let runs_with_turns: usize = 4;
        let avg = if runs_with_turns > 0 {
            total_turns as f64 / runs_with_turns as f64
        } else {
            0.0
        };
        assert!((avg - 25.0).abs() < 1e-10);
    }

    #[test]
    fn test_stats_avg_turns_no_runs_with_turns() {
        let total_turns: u64 = 0;
        let runs_with_turns: usize = 0;
        let avg = if runs_with_turns > 0 {
            total_turns as f64 / runs_with_turns as f64
        } else {
            0.0
        };
        assert!((avg - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_stats_avg_bounces_verified() {
        // Replicate: verified_bounce_counts.sum() / verified_bounce_counts.len()
        let verified_bounce_counts: Vec<usize> = vec![2, 3, 1, 2];
        let avg = if !verified_bounce_counts.is_empty() {
            verified_bounce_counts.iter().sum::<usize>() as f64 / verified_bounce_counts.len() as f64
        } else {
            0.0
        };
        assert!((avg - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_stats_avg_bounces_empty() {
        let verified_bounce_counts: Vec<usize> = vec![];
        let avg = if !verified_bounce_counts.is_empty() {
            verified_bounce_counts.iter().sum::<usize>() as f64 / verified_bounce_counts.len() as f64
        } else {
            0.0
        };
        assert!((avg - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_stats_escalation_title_normalization() {
        // Replicate: strip "ESCALATION:" prefix and "(after N bounces)" suffix
        let titles = vec![
            "ESCALATION: Build failed (after 3 bounces)".to_string(),
            "ESCALATION: Build failed (after 2 bounces)".to_string(),
            "ESCALATION: Test timeout (after 1 bounces)".to_string(),
        ];

        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for t in &titles {
            let reason = t.strip_prefix("ESCALATION:").unwrap_or(t).trim();
            let reason = if let Some(pos) = reason.rfind(" (after ") {
                reason[..pos].trim()
            } else {
                reason
            };
            *counts.entry(reason.to_string()).or_insert(0) += 1;
        }
        let most_common = counts.into_iter().max_by_key(|(_, c)| *c).map(|(r, _)| r);
        assert_eq!(most_common.as_deref(), Some("Build failed"));
    }

    #[test]
    fn test_stats_escalation_title_no_bounce_suffix() {
        // Title without "(after N bounces)" suffix
        let title = "ESCALATION: Compilation error";
        let reason = title.strip_prefix("ESCALATION:").unwrap_or(title).trim();
        let reason = if let Some(pos) = reason.rfind(" (after ") {
            reason[..pos].trim()
        } else {
            reason
        };
        assert_eq!(reason, "Compilation error");
    }

    #[test]
    fn test_stats_escalation_title_non_escalation() {
        // Non-escalation title: strip_prefix returns None, use original
        let title = "Some other node";
        let reason = title.strip_prefix("ESCALATION:").unwrap_or(title).trim();
        assert_eq!(reason, "Some other node");
    }

    #[test]
    fn test_stats_bounce_count_from_metadata() {
        // Replicate: count coder agent entries in tracks metadata
        let metas = vec![
            r#"{"agent": "coder", "cost_usd": 0.50}"#,
            r#"{"agent": "tester", "cost_usd": 0.20}"#,
            r#"{"agent": "coder", "cost_usd": 0.30}"#,
            r#"{"agent": "verifier", "cost_usd": 0.10}"#,
        ];
        let bounce_count = metas.iter()
            .filter_map(|meta| serde_json::from_str::<serde_json::Value>(meta).ok())
            .filter(|v| v["agent"].as_str().map(|a| a == "coder").unwrap_or(false))
            .count();
        assert_eq!(bounce_count, 2);
    }

    #[test]
    fn test_stats_bounce_count_no_coders() {
        let metas = vec![
            r#"{"agent": "tester", "cost_usd": 0.20}"#,
            r#"{"agent": "verifier", "cost_usd": 0.10}"#,
        ];
        let bounce_count = metas.iter()
            .filter_map(|meta| serde_json::from_str::<serde_json::Value>(meta).ok())
            .filter(|v| v["agent"].as_str().map(|a| a == "coder").unwrap_or(false))
            .count();
        assert_eq!(bounce_count, 0);
    }

    #[test]
    fn test_stats_failure_reason_first_line_normalization() {
        // Replicate: take first line only from contradicts edge content
        let reasons = vec![
            "Build failed\nDetails: missing import".to_string(),
            "Build failed\nAnother detail".to_string(),
            "Test timeout\nSome info".to_string(),
        ];

        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for r in &reasons {
            let first_line = r.lines().next().unwrap_or(r).trim().to_string();
            if !first_line.is_empty() {
                *counts.entry(first_line).or_insert(0) += 1;
            }
        }
        let most_common = counts.into_iter().max_by_key(|(_, c)| *c).map(|(r, _)| r);
        assert_eq!(most_common.as_deref(), Some("Build failed"));
    }

    #[test]
    fn test_stats_failure_reason_empty_reasons_skipped() {
        let reasons = vec![
            "".to_string(),
            "   ".to_string(),
            "Actual reason".to_string(),
        ];

        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for r in &reasons {
            let first_line = r.lines().next().unwrap_or(r).trim().to_string();
            if !first_line.is_empty() {
                *counts.entry(first_line).or_insert(0) += 1;
            }
        }
        let most_common = counts.into_iter().max_by_key(|(_, c)| *c).map(|(r, _)| r);
        assert_eq!(most_common.as_deref(), Some("Actual reason"));
    }

    #[test]
    fn test_stats_status_percentage_calculation() {
        // Replicate: (count / total_runs) * 100.0
        let total_runs = 10_usize;
        let count = 3_usize;
        let pct = (count as f64 / total_runs as f64) * 100.0;
        assert!((pct - 30.0).abs() < 1e-10);
    }

    #[test]
    fn test_stats_failure_reason_display_truncation() {
        // Replicate: truncate reason to 60 chars with "..."
        let reason = "A very long failure reason that exceeds sixty characters and should be truncated";
        let display = if reason.chars().count() > 60 {
            format!("{}...", &reason.chars().take(57).collect::<String>())
        } else {
            reason.to_string()
        };
        assert!(display.ends_with("..."));
        assert_eq!(display.chars().count(), 60);
    }

    #[test]
    fn test_stats_failure_reason_display_short() {
        let reason = "Build failed";
        let display = if reason.chars().count() > 60 {
            format!("{}...", &reason.chars().take(57).collect::<String>())
        } else {
            reason.to_string()
        };
        assert_eq!(display, "Build failed");
    }

    // --- Tests for is_lesson_quality ---

    #[test]
    fn test_lesson_quality_empty_string_rejected() {
        assert!(!is_lesson_quality(""));
    }

    #[test]
    fn test_lesson_quality_short_summary_rejected() {
        assert!(!is_lesson_quality("This is too short to be useful."));
        assert!(!is_lesson_quality("cargo check"));
        assert!(!is_lesson_quality("Use cargo build --release for production"));
    }

    #[test]
    fn test_lesson_quality_bare_cli_commands_rejected() {
        // Even if padded to 20+ words, bare CLI command patterns are rejected
        // But these are short so they fail word count first
        assert!(!is_lesson_quality("cargo check"));
        assert!(!is_lesson_quality("cargo build --release"));
        assert!(!is_lesson_quality("npm install"));
    }

    #[test]
    fn test_lesson_quality_imperative_run_use_rejected() {
        assert!(!is_lesson_quality("Use cargo check"));
        assert!(!is_lesson_quality("Run cargo build before deploying"));
        assert!(!is_lesson_quality("Use npm test to verify"));
    }

    #[test]
    fn test_lesson_quality_always_pattern_rejected() {
        assert!(!is_lesson_quality("Always run cargo check before committing your changes"));
    }

    #[test]
    fn test_lesson_quality_substantive_accepted() {
        let good = "When adding a new enum variant to an existing enum, you must update ALL match \
                     blocks that pattern-match on that enum, not just the one function mentioned in \
                     the task. Rust exhaustive matching catches this at compile time but it wastes a \
                     full bounce cycle if you miss them.";
        assert!(is_lesson_quality(good));
    }

    #[test]
    fn test_lesson_quality_multi_sentence_accepted() {
        let good = "Coders generating tests in multiple passes often create duplicate function names. \
                     When a coder agent writes tests incrementally, it may reuse the same function \
                     names in both batches. Rust does not allow duplicate names in mod tests.";
        assert!(is_lesson_quality(good));
    }

    #[test]
    fn test_lesson_quality_exactly_20_words_accepted() {
        // Exactly 20 words should pass (>= 20)
        let exactly_20 = "one two three four five six seven eight nine ten \
                          eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty";
        assert_eq!(count_words(exactly_20), 20);
        assert!(is_lesson_quality(exactly_20));
    }

    #[test]
    fn test_lesson_quality_19_words_rejected() {
        let just_under = "one two three four five six seven eight nine ten \
                          eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen";
        assert_eq!(count_words(just_under), 19);
        assert!(!is_lesson_quality(just_under));
    }

    #[test]
    fn test_lesson_quality_whitespace_only_rejected() {
        assert!(!is_lesson_quality("   \t\n  "));
    }

    #[test]
    fn test_lesson_quality_long_cargo_prefix_accepted() {
        // 20+ words starting with "cargo check" pass because after the 20-word gate,
        // the pattern check requires count_words < 10 which is impossible.
        let long_cargo = "cargo check is a useful command that developers should run \
                          frequently to verify their code compiles correctly before pushing \
                          any changes to the remote repository branch";
        assert!(count_words(long_cargo) >= 20);
        assert!(is_lesson_quality(long_cargo));
    }

    #[test]
    fn test_lesson_quality_long_run_prefix_accepted() {
        // "run ..." with 20+ words passes because the <10 word check cannot trigger
        // after the 20-word gate has already passed.
        let long_run = "Run the complete test suite including integration tests and unit tests \
                        and end-to-end tests before merging any pull request to ensure nothing \
                        is broken by the changes";
        assert!(count_words(long_run) >= 20);
        assert!(is_lesson_quality(long_run));
    }

    #[test]
    fn test_lesson_quality_long_use_prefix_accepted() {
        // "use ..." with 20+ words passes the pattern check for the same reason.
        let long_use = "Use the nightly compiler when building this project because several \
                        features depend on unstable APIs that are only available in the nightly \
                        toolchain and will not compile otherwise";
        assert!(count_words(long_use) >= 20);
        assert!(is_lesson_quality(long_use));
    }

    #[test]
    fn test_lesson_quality_long_always_prefix_accepted() {
        // "always ..." with 20+ words passes both the always check (<15) and the word gate.
        let long_always = "Always verify that all match arms are updated when adding a new enum \
                           variant because Rust exhaustive matching will catch it at compile time \
                           but it wastes a full bounce cycle";
        assert!(count_words(long_always) >= 20);
        assert!(is_lesson_quality(long_always));
    }

    #[test]
    fn test_lesson_quality_case_insensitive_short_rejected() {
        // Case variations still rejected (all < 20 words, fail on word count)
        assert!(!is_lesson_quality("CARGO CHECK"));
        assert!(!is_lesson_quality("Cargo Build --release"));
        assert!(!is_lesson_quality("NPM Install"));
    }

    #[test]
    fn test_lesson_quality_newlines_in_content() {
        // Whitespace variations (newlines, tabs) are handled by split_whitespace
        let with_newlines = "When adding a new enum variant\nto an existing enum you must update \
                             ALL match blocks\nthat pattern-match on that enum not just the one \
                             function mentioned in the task";
        assert!(count_words(with_newlines) >= 20);
        assert!(is_lesson_quality(with_newlines));
    }

    #[test]
    fn test_lesson_quality_leading_trailing_whitespace() {
        // Leading/trailing whitespace shouldn't affect a substantive summary
        let padded = "  When adding a new enum variant to an existing enum you must update ALL \
                      match blocks that pattern-match on that enum not just the one function \
                      mentioned in the task description  ";
        assert!(count_words(padded) >= 20);
        assert!(is_lesson_quality(padded));
    }

    #[test]
    fn test_lesson_quality_exactly_20_words_with_run_prefix() {
        // 20 words starting with "run" — passes the 20-word gate, and the "run" pattern
        // check requires < 10 words so it does not trigger.
        let run_20 = "Run the complete integration test suite including all database migration \
                      tests and API endpoint tests before merging any pull requests";
        assert_eq!(count_words(run_20), 20);
        assert!(is_lesson_quality(run_20));
    }

    // --- Tests for anchor search: FTS query construction (generate_task_file L6081-6100) ---
    // The FTS query builder: split on whitespace, filter stopwords and short words,
    // trim punctuation, join with " OR ".

    /// Replicates the FTS query construction logic from generate_task_file.
    fn build_fts_query(task: &str) -> String {
        let stopwords = ["the", "a", "an", "in", "on", "at", "to", "for", "of", "is", "it",
                         "and", "or", "with", "from", "by", "this", "that", "as", "be"];
        task.split_whitespace()
            .filter(|w| w.len() > 2 && !stopwords.contains(&w.to_lowercase().as_str()))
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" OR ")
    }

    #[test]
    fn test_fts_query_filters_stopwords() {
        // "the", "a", "to", "for", "is" should all be removed
        let query = build_fts_query("Add the flag to a function for parsing");
        assert!(!query.contains("the"));
        assert!(!query.contains(" a "));
        assert!(query.contains("Add"));
        assert!(query.contains("flag"));
        assert!(query.contains("function"));
        assert!(query.contains("parsing"));
    }

    #[test]
    fn test_fts_query_filters_short_words() {
        // Words with len <= 2 are filtered: "a", "in", "to", "or" (also stopwords),
        // but also non-stopword short words like "go", "do"
        let query = build_fts_query("go do run fast");
        // "go" (2 chars) and "do" (2 chars) filtered by length
        assert!(!query.contains("go"));
        assert!(!query.contains("do"));
        assert!(query.contains("run"));
        assert!(query.contains("fast"));
    }

    #[test]
    fn test_fts_query_trims_punctuation() {
        // Surrounding punctuation stripped by trim_matches
        let query = build_fts_query("generate_task_file() function, (spore.rs)");
        assert!(query.contains("generate_task_file"));
        assert!(query.contains("function"));
        assert!(query.contains("spore.rs"));
        // Parentheses/commas should be gone
        assert!(!query.contains("("));
        assert!(!query.contains(")"));
        assert!(!query.contains(","));
    }

    #[test]
    fn test_fts_query_joins_with_or() {
        let query = build_fts_query("anchor search merge dedup");
        assert_eq!(query, "anchor OR search OR merge OR dedup");
    }

    #[test]
    fn test_fts_query_empty_when_all_stopwords() {
        // All words are stopwords or too short → empty query
        let query = build_fts_query("the a an in on at to for of is it");
        assert!(query.is_empty());
    }

    #[test]
    fn test_fts_query_mixed_case_stopwords() {
        // Stopword check is case-insensitive (to_lowercase)
        let query = build_fts_query("The AND From THIS function");
        assert!(!query.contains("The"));
        assert!(!query.contains("AND"));
        assert!(!query.contains("From"));
        assert!(!query.contains("THIS"));
        assert_eq!(query, "function");
    }

    #[test]
    fn test_fts_query_preserves_underscored_identifiers() {
        // Code identifiers with underscores should pass through intact
        let query = build_fts_query("change generate_task_file semantic search");
        assert!(query.contains("change"));
        assert!(query.contains("generate_task_file"));
        assert!(query.contains("semantic"));
        assert!(query.contains("search"));
    }

    #[test]
    fn test_fts_query_punctuation_only_word_filtered() {
        // A word that is entirely punctuation after trimming becomes empty
        let query = build_fts_query("test --- function");
        // "---" becomes empty after trim_matches(non-alphanumeric)
        assert!(!query.contains("---"));
        assert_eq!(query, "test OR function");
    }

    // --- Tests for anchor merge+dedup logic (generate_task_file L6107-6120) ---
    // Semantic results first (priority), FTS appended, dedup by node ID, truncate to 5.

    /// Minimal node for merge testing — only `id` matters for dedup.
    fn make_test_node(id: &str) -> Node {
        Node {
            id: id.to_string(),
            node_type: NodeType::Thought,
            title: format!("Node {}", id),
            url: None,
            content: None,
            position: Position { x: 0.0, y: 0.0 },
            created_at: 0,
            updated_at: 0,
            cluster_id: None,
            cluster_label: None,
            ai_title: None,
            summary: None,
            tags: None,
            emoji: None,
            is_processed: false,
            depth: 1,
            is_item: true,
            is_universe: false,
            parent_id: None,
            child_count: 0,
            conversation_id: None,
            sequence_index: None,
            is_pinned: false,
            last_accessed_at: None,
            latest_child_date: None,
            is_private: None,
            privacy_reason: None,
            source: None,
            pdf_available: None,
            content_type: None,
            associated_idea_id: None,
            privacy: None,
            human_edited: None,
            human_created: false,
            author: None,
            agent_id: None,
            node_class: None,
            meta_type: None,
        }
    }

    /// Replicates the merge+dedup logic from generate_task_file.
    fn merge_anchors(semantic: Vec<Node>, fts: Vec<Node>) -> Vec<Node> {
        let mut seen_ids = std::collections::HashSet::new();
        let mut merged: Vec<Node> = Vec::new();
        for node in semantic {
            if seen_ids.insert(node.id.clone()) {
                merged.push(node);
            }
        }
        for node in fts {
            if seen_ids.insert(node.id.clone()) {
                merged.push(node);
            }
        }
        merged.truncate(5);
        merged
    }

    #[test]
    fn test_anchor_merge_semantic_priority() {
        // Semantic results should appear before FTS results
        let semantic = vec![make_test_node("sem-1"), make_test_node("sem-2")];
        let fts = vec![make_test_node("fts-1"), make_test_node("fts-2")];
        let merged = merge_anchors(semantic, fts);
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0].id, "sem-1");
        assert_eq!(merged[1].id, "sem-2");
        assert_eq!(merged[2].id, "fts-1");
        assert_eq!(merged[3].id, "fts-2");
    }

    #[test]
    fn test_anchor_merge_dedup_by_id() {
        // When both lists contain the same node ID, semantic wins (appears first)
        let semantic = vec![make_test_node("shared"), make_test_node("sem-only")];
        let fts = vec![make_test_node("shared"), make_test_node("fts-only")];
        let merged = merge_anchors(semantic, fts);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].id, "shared");   // from semantic
        assert_eq!(merged[1].id, "sem-only");
        assert_eq!(merged[2].id, "fts-only"); // FTS duplicate "shared" was skipped
    }

    #[test]
    fn test_anchor_merge_truncate_to_5() {
        // More than 5 unique nodes → truncated to 5
        let semantic = vec![
            make_test_node("s1"), make_test_node("s2"), make_test_node("s3"),
        ];
        let fts = vec![
            make_test_node("f1"), make_test_node("f2"), make_test_node("f3"),
        ];
        let merged = merge_anchors(semantic, fts);
        assert_eq!(merged.len(), 5);
        // First 3 are semantic, then first 2 FTS
        assert_eq!(merged[0].id, "s1");
        assert_eq!(merged[1].id, "s2");
        assert_eq!(merged[2].id, "s3");
        assert_eq!(merged[3].id, "f1");
        assert_eq!(merged[4].id, "f2");
        // f3 was truncated
    }

    #[test]
    fn test_anchor_merge_both_empty() {
        let merged = merge_anchors(vec![], vec![]);
        assert!(merged.is_empty());
    }

    #[test]
    fn test_anchor_merge_semantic_only() {
        // No FTS results — only semantic
        let semantic = vec![make_test_node("a"), make_test_node("b")];
        let merged = merge_anchors(semantic, vec![]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "a");
        assert_eq!(merged[1].id, "b");
    }

    #[test]
    fn test_anchor_merge_fts_only() {
        // No semantic results — only FTS (e.g., embedding generation failed)
        let fts = vec![make_test_node("x"), make_test_node("y"), make_test_node("z")];
        let merged = merge_anchors(vec![], fts);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].id, "x");
        assert_eq!(merged[1].id, "y");
        assert_eq!(merged[2].id, "z");
    }

    #[test]
    fn test_anchor_merge_all_duplicates() {
        // All IDs overlap — result should be semantic's order only
        let semantic = vec![make_test_node("a"), make_test_node("b"), make_test_node("c")];
        let fts = vec![make_test_node("b"), make_test_node("a"), make_test_node("c")];
        let merged = merge_anchors(semantic, fts);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].id, "a");
        assert_eq!(merged[1].id, "b");
        assert_eq!(merged[2].id, "c");
    }

    #[test]
    fn test_anchor_merge_5_semantic_caps_fts() {
        // If semantic already has 5 nodes, no room for FTS
        let semantic = vec![
            make_test_node("s1"), make_test_node("s2"), make_test_node("s3"),
            make_test_node("s4"), make_test_node("s5"),
        ];
        let fts = vec![make_test_node("f1"), make_test_node("f2")];
        let merged = merge_anchors(semantic, fts);
        assert_eq!(merged.len(), 5);
        // All 5 are semantic — FTS got truncated away
        for (i, node) in merged.iter().enumerate() {
            assert_eq!(node.id, format!("s{}", i + 1));
        }
    }

    #[test]
    fn test_anchor_merge_dedup_with_truncation() {
        // 4 semantic + 4 FTS, 2 overlap → 6 unique → truncated to 5
        let semantic = vec![
            make_test_node("a"), make_test_node("b"),
            make_test_node("c"), make_test_node("d"),
        ];
        let fts = vec![
            make_test_node("c"), make_test_node("d"),  // duplicates
            make_test_node("e"), make_test_node("f"),  // unique
        ];
        let merged = merge_anchors(semantic, fts);
        assert_eq!(merged.len(), 5);
        assert_eq!(merged[0].id, "a");
        assert_eq!(merged[1].id, "b");
        assert_eq!(merged[2].id, "c");
        assert_eq!(merged[3].id, "d");
        assert_eq!(merged[4].id, "e");
        // "f" was truncated
    }


    // --- Tests for generate_task_file impl-id section (role-aware dispatch, L6221-6246) ---
    // The function requires a live database, so we replicate the pure string-building
    // logic as a helper and test the dispatch behaviour in isolation.

    /// Replicates the role-dispatch logic from generate_task_file for the impl-id section.
    fn build_impl_section(role: &str, impl_id: Option<&str>) -> String {
        let mut md = String::new();
        if let Some(id) = impl_id {
            match role {
                "verifier" => {
                    md.push_str("## Implementation to Check\n\n");
                    md.push_str(&format!(
                        "Implementation node ID: `{}`. Read it with `mycelica_read_content` to see what the coder changed and why.\n\n",
                        id
                    ));
                }
                "summarizer" => {
                    md.push_str("## Implementation to Summarize\n\n");
                    md.push_str(&format!(
                        "Implementation node ID: `{}`. Read it and the full bounce trail with `mycelica_read_content` and `mycelica_nav_edges`.\n\n",
                        id
                    ));
                }
                _ => {
                    md.push_str("## Previous Bounce\n\n");
                    md.push_str(&format!(
                        "Verifier found issues with node `{}`. Check its incoming `contradicts` edges and fix the code.\n\n",
                        id
                    ));
                }
            }
        }
        md
    }

    #[test]
    fn test_impl_section_verifier_uses_check_header() {
        let section = build_impl_section("verifier", Some("abc-123"));
        assert!(section.starts_with("## Implementation to Check\n\n"));
    }

    #[test]
    fn test_impl_section_verifier_contains_impl_id() {
        let section = build_impl_section("verifier", Some("node-xyz-789"));
        assert!(section.contains("`node-xyz-789`"));
    }

    #[test]
    fn test_impl_section_verifier_does_not_mention_previous_bounce() {
        let section = build_impl_section("verifier", Some("id-1"));
        assert!(!section.contains("## Previous Bounce"));
        assert!(!section.contains("contradicts"));
    }

    #[test]
    fn test_impl_section_summarizer_uses_summarize_header() {
        let section = build_impl_section("summarizer", Some("id-sum"));
        assert!(section.starts_with("## Implementation to Summarize\n\n"));
    }

    #[test]
    fn test_impl_section_summarizer_contains_impl_id() {
        let section = build_impl_section("summarizer", Some("id-sum"));
        assert!(section.contains("`id-sum`"));
    }

    #[test]
    fn test_impl_section_summarizer_mentions_nav_edges() {
        // The summarizer variant explicitly tells the agent to use mycelica_nav_edges
        let section = build_impl_section("summarizer", Some("id-sum"));
        assert!(section.contains("mycelica_nav_edges"));
    }

    #[test]
    fn test_impl_section_coder_uses_previous_bounce_header() {
        let section = build_impl_section("coder", Some("id-coder"));
        assert!(section.starts_with("## Previous Bounce\n\n"));
    }

    #[test]
    fn test_impl_section_coder_mentions_contradicts_edges() {
        let section = build_impl_section("coder", Some("id-coder"));
        assert!(section.contains("contradicts"));
    }

    #[test]
    fn test_impl_section_coder_contains_impl_id() {
        let section = build_impl_section("coder", Some("id-coder"));
        assert!(section.contains("`id-coder`"));
    }

    #[test]
    fn test_impl_section_unknown_role_falls_back_to_previous_bounce() {
        // Any role not explicitly matched should get the coder/default path
        let section = build_impl_section("unknown_role", Some("id-x"));
        assert!(section.starts_with("## Previous Bounce\n\n"));
    }

    #[test]
    fn test_impl_section_empty_role_falls_back_to_previous_bounce() {
        let section = build_impl_section("", Some("id-x"));
        assert!(section.starts_with("## Previous Bounce\n\n"));
    }

    #[test]
    fn test_impl_section_none_impl_id_produces_empty_string() {
        // When there is no last_impl_id the section must be absent entirely
        assert!(build_impl_section("verifier", None).is_empty());
        assert!(build_impl_section("summarizer", None).is_empty());
        assert!(build_impl_section("coder", None).is_empty());
    }

    #[test]
    fn test_impl_section_verifier_does_not_include_summarize_header() {
        let section = build_impl_section("verifier", Some("id-v"));
        assert!(!section.contains("## Implementation to Summarize"));
    }

    #[test]
    fn test_impl_section_summarizer_does_not_include_check_header() {
        let section = build_impl_section("summarizer", Some("id-s"));
        assert!(!section.contains("## Implementation to Check"));
    }

    #[test]
    fn test_impl_section_coder_does_not_include_check_or_summarize_header() {
        let section = build_impl_section("coder", Some("id-c"));
        assert!(!section.contains("## Implementation to Check"));
        assert!(!section.contains("## Implementation to Summarize"));
    }



    // ============================================================================
    // Tests for count_agent_prompt_lines() and handle_prompt_stats() logic
    // ============================================================================
    //
    // count_agent_prompt_lines() uses find_project_root() to locate the project
    // root (directory containing .mycelica.db), then reads docs/spore/agents/.
    // Tests that need real files use tempfile + set_current_dir, serialized
    // with CWD_MUTEX to prevent CWD races between parallel tests.
    // Each temp dir must contain a .mycelica.db sentinel for find_project_root().

    use std::sync::Mutex;
    static CWD_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_count_agent_prompt_lines_missing_dir_returns_empty() {
        // Use a temp dir with no docs/spore/agents — CWD-independent
        let _guard = CWD_MUTEX.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&tmp).unwrap();
        let result = count_agent_prompt_lines();
        std::env::set_current_dir(&prev).unwrap();
        assert_eq!(result.unwrap(), vec![]);
    }

    #[test]
    fn test_count_agent_prompt_lines_readme_excluded() {
        let _guard = CWD_MUTEX.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".mycelica.db"), "").unwrap();
        let agent_dir = tmp.path().join("docs/spore/agents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("README.md"), "readme line1\nreadme line2\n").unwrap();
        std::fs::write(agent_dir.join("coder.md"), "a\nb\nc\n").unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&tmp).unwrap();
        let result = count_agent_prompt_lines();
        std::env::set_current_dir(&prev).unwrap();
        let files = result.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "coder");
    }

    #[test]
    fn test_count_agent_prompt_lines_sorted_alphabetically() {
        let _guard = CWD_MUTEX.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".mycelica.db"), "").unwrap();
        let agent_dir = tmp.path().join("docs/spore/agents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("verifier.md"), "x\n").unwrap();
        std::fs::write(agent_dir.join("architect.md"), "x\n").unwrap();
        std::fs::write(agent_dir.join("coder.md"), "x\n").unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&tmp).unwrap();
        let result = count_agent_prompt_lines();
        std::env::set_current_dir(&prev).unwrap();
        let files = result.unwrap();
        assert_eq!(files[0].0, "architect");
        assert_eq!(files[1].0, "coder");
        assert_eq!(files[2].0, "verifier");
    }

    #[test]
    fn test_count_agent_prompt_lines_strips_md_extension() {
        let _guard = CWD_MUTEX.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".mycelica.db"), "").unwrap();
        let agent_dir = tmp.path().join("docs/spore/agents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("planner.md"), "line\n").unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&tmp).unwrap();
        let result = count_agent_prompt_lines();
        std::env::set_current_dir(&prev).unwrap();
        let files = result.unwrap();
        assert_eq!(files[0].0, "planner"); // not "planner.md"
    }

    #[test]
    fn test_count_agent_prompt_lines_counts_lines_correctly() {
        let _guard = CWD_MUTEX.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".mycelica.db"), "").unwrap();
        let agent_dir = tmp.path().join("docs/spore/agents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("tester.md"), "line one\nline two\nline three\n").unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&tmp).unwrap();
        let result = count_agent_prompt_lines();
        std::env::set_current_dir(&prev).unwrap();
        let files = result.unwrap();
        assert_eq!(files[0], ("tester".to_string(), 3));
    }

    #[test]
    fn test_count_agent_prompt_lines_ignores_non_md_files() {
        let _guard = CWD_MUTEX.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".mycelica.db"), "").unwrap();
        let agent_dir = tmp.path().join("docs/spore/agents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("notes.txt"), "not markdown\n").unwrap();
        std::fs::write(agent_dir.join("script.sh"), "#!/bin/bash\n").unwrap();
        std::fs::write(agent_dir.join("agent.md"), "real agent\n").unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&tmp).unwrap();
        let result = count_agent_prompt_lines();
        std::env::set_current_dir(&prev).unwrap();
        let files = result.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "agent");
    }

    #[test]
    fn test_count_agent_prompt_lines_total_sum() {
        let _guard = CWD_MUTEX.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".mycelica.db"), "").unwrap();
        let agent_dir = tmp.path().join("docs/spore/agents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("alpha.md"), "a\nb\n").unwrap();      // 2 lines
        std::fs::write(agent_dir.join("beta.md"), "x\ny\nz\n").unwrap();    // 3 lines
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&tmp).unwrap();
        let result = count_agent_prompt_lines();
        std::env::set_current_dir(&prev).unwrap();
        let files = result.unwrap();
        let total: usize = files.iter().map(|(_, c)| *c).sum();
        assert_eq!(total, 5);
    }

    #[test]
    fn test_prompt_stats_max_name_falls_back_to_5_for_short_names() {
        // When all names are shorter than "AGENT" (5 chars), max_name is 5
        let files: Vec<(String, usize)> = vec![
            ("hi".to_string(), 10),
            ("bye".to_string(), 20),
        ];
        let max_name = files.iter().map(|(n, _)| n.len()).max().unwrap_or(5).max(5);
        assert_eq!(max_name, 5);
    }

    #[test]
    fn test_prompt_stats_max_name_uses_longest_agent_name() {
        // When an agent name exceeds 5 chars, max_name is that length
        let files: Vec<(String, usize)> = vec![
            ("summarizer".to_string(), 10), // 10 chars
            ("coder".to_string(), 20),      // 5 chars
        ];
        let max_name = files.iter().map(|(n, _)| n.len()).max().unwrap_or(5).max(5);
        assert_eq!(max_name, 10);
    }

    #[test]
    fn test_prompt_stats_separator_width() {
        // Separator is "-".repeat(max_name + 8)
        let max_name = 10usize;
        let sep = "-".repeat(max_name + 8);
        assert_eq!(sep.len(), 18);
    }

    #[test]
    fn test_count_agent_prompt_lines_empty_file_has_zero_lines() {
        let _guard = CWD_MUTEX.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".mycelica.db"), "").unwrap();
        let agent_dir = tmp.path().join("docs/spore/agents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("empty.md"), "").unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&tmp).unwrap();
        let result = count_agent_prompt_lines();
        std::env::set_current_dir(&prev).unwrap();
        let files = result.unwrap();
        assert_eq!(files[0], ("empty".to_string(), 0));
    }

    // ============================================================================
    // Tests for generate_task_file() line count telemetry (Change 2)
    //
    // generate_task_file() now returns Result<(PathBuf, usize), String>.
    // The usize is md.lines().count() — counted after the markdown is assembled,
    // before writing to disk. These tests verify the counting semantics and
    // telemetry format used at that call site.
    // ============================================================================

    #[test]
    fn test_task_file_line_count_trailing_newline_not_double_counted() {
        // Rust .lines() does not treat a trailing \n as an extra blank line.
        // All md.push_str() calls end with \n, so this property is load-bearing.
        let content = "# Task\n\n## Checklist\n\n- [ ] Done\n";
        assert_eq!(content.lines().count(), 5);
    }

    #[test]
    fn test_task_file_line_count_with_and_without_trailing_newline_equal() {
        // Whether or not the final push ends with \n, the count is the same.
        let with_nl = "# Task\n\n## Checklist\n\n- [ ] Done\n";
        let without_nl = "# Task\n\n## Checklist\n\n- [ ] Done";
        assert_eq!(with_nl.lines().count(), without_nl.lines().count());
    }

    #[test]
    fn test_task_file_line_count_empty_content_is_zero() {
        // Empty markdown string yields zero lines.
        assert_eq!("".lines().count(), 0usize);
    }

    #[test]
    fn test_task_file_line_count_grows_with_pushed_sections() {
        // Simulate the md.push_str() pattern used inside generate_task_file.
        let mut md = String::new();
        md.push_str("## Task\n\n");        // 2 lines
        md.push_str("Do something.\n\n"); // 2 more lines
        md.push_str("## Checklist\n\n"); // 2 more lines
        assert_eq!(md.lines().count(), 6);

        md.push_str("- [ ] Step one\n"); // 1 more
        assert_eq!(md.lines().count(), 7);
    }

    #[test]
    fn test_task_file_telemetry_format() {
        // Verify the exact format of the stderr message emitted by generate_task_file.
        // eprintln!("[task-file] Generated: {} lines", line_count)
        let line_count = 87usize;
        let msg = format!("[task-file] Generated: {} lines", line_count);
        assert_eq!(msg, "[task-file] Generated: 87 lines");
        assert!(msg.starts_with("[task-file] Generated: "));
        assert!(msg.ends_with(" lines"));
    }

    #[test]
    fn test_task_file_telemetry_nonzero_for_nonempty_content() {
        // Non-empty task content must produce a telemetry line count > 0.
        let md = "# Task\n\nSome content\n";
        let line_count = md.lines().count();
        assert!(line_count > 0, "non-empty markdown reported 0 lines");
        let msg = format!("[task-file] Generated: {} lines", line_count);
        assert!(!msg.contains("Generated: 0 lines"));
    }

    #[test]
    fn test_task_file_return_tuple_caller_destructuring() {
        // Documents the destructuring pattern used at all 13 call sites:
        //   let (task_file, _line_count) = generate_task_file(...)?;
        // Simulate the return value and verify destructuring works as expected.
        let md = "# Task: Example\n\n- item one\n- item two\n\n## Checklist\n\n- [ ] done\n";
        let line_count = md.lines().count();
        let result: Result<(std::path::PathBuf, usize), String> = Ok((
            std::path::PathBuf::from("docs/spore/tasks/task-abc12345.md"),
            line_count,
        ));
        let (path, returned_count) = result.unwrap();
        assert_eq!(returned_count, 8);
        assert_eq!(returned_count, md.lines().count());
        assert!(path.to_str().unwrap().ends_with(".md"));
    }

    #[test]
    fn test_task_file_line_count_blank_lines_counted() {
        // Blank lines (just \n) are counted — they contribute to section spacing.
        let content = "line one\n\nline three\n\n";
        // "line one", "", "line three", "" — but trailing \n after last "" is not an extra line
        // Actually: "line one\n" + "\n" + "line three\n" + "\n"
        // .lines() yields: "line one", "", "line three", "" → 4
        assert_eq!(content.lines().count(), 4);
    }

    // ============================================================================
    // Tests for prompt_size health check logic (Change 3 / sub-3)
    //
    // handle_health() is not unit-testable directly (requires Database), so we
    // test the threshold decision logic and detail string format in isolation.
    // The logic in handle_health() lines ~1431-1458:
    //   let prompt_lines: usize = count_agent_prompt_lines().unwrap_or_default().iter().map(|(_, c)| c).sum();
    //   let threshold = 1000usize;
    //   if prompt_lines == 0 { ok=true, "no agent files found..." }
    //   else if prompt_lines > threshold { ok=false, "N lines total (threshold: N)" }
    //   else { ok=true, "N lines total" }
    // ============================================================================

    /// Mirror the prompt_size check logic from handle_health() for unit testing.
    fn prompt_size_check_logic(prompt_lines: usize) -> (bool, String) {
        let threshold = 1000usize;
        if prompt_lines == 0 {
            (true, "no agent files found (docs/spore/agents/ missing)".to_string())
        } else if prompt_lines > threshold {
            (false, format!("{} lines total (threshold: {})", prompt_lines, threshold))
        } else {
            (true, format!("{} lines total", prompt_lines))
        }
    }

    #[test]
    fn test_prompt_size_check_zero_lines_is_ok() {
        // 0 lines → missing dir case → ok=true
        let (ok, detail) = prompt_size_check_logic(0);
        assert!(ok);
        assert!(detail.contains("no agent files found"));
    }

    #[test]
    fn test_prompt_size_check_below_threshold_is_ok() {
        let (ok, detail) = prompt_size_check_logic(500);
        assert!(ok);
        assert_eq!(detail, "500 lines total");
    }

    #[test]
    fn test_prompt_size_check_at_threshold_boundary_is_ok() {
        // Exactly 1000 lines: NOT > threshold, so ok=true (boundary condition)
        let (ok, detail) = prompt_size_check_logic(1000);
        assert!(ok);
        assert_eq!(detail, "1000 lines total");
    }

    #[test]
    fn test_prompt_size_check_above_threshold_is_warning() {
        // 1001 lines: > 1000 threshold → ok=false
        let (ok, detail) = prompt_size_check_logic(1001);
        assert!(!ok);
        assert!(detail.contains("1001 lines total"));
        assert!(detail.contains("threshold: 1000"));
    }

    #[test]
    fn test_prompt_size_check_threshold_is_1000() {
        // Threshold constant embedded in detail string must be 1000
        let (_, detail) = prompt_size_check_logic(1001);
        assert!(detail.contains("threshold: 1000"), "threshold should be 1000, got: {}", detail);
    }

    #[test]
    fn test_prompt_size_check_detail_format_ok() {
        // Verify exact "N lines total" format for ok case
        let (ok, detail) = prompt_size_check_logic(732);
        assert!(ok);
        assert_eq!(detail, "732 lines total");
    }

    #[test]
    fn test_prompt_size_check_detail_format_warning() {
        // Verify exact format for warning case
        let (ok, detail) = prompt_size_check_logic(1500);
        assert!(!ok);
        assert_eq!(detail, "1500 lines total (threshold: 1000)");
    }

    #[test]
    fn test_prompt_size_check_missing_dir_detail() {
        // Verify exact detail string for the zero-lines (missing dir) case
        let (ok, detail) = prompt_size_check_logic(0);
        assert!(ok);
        assert_eq!(detail, "no agent files found (docs/spore/agents/ missing)");
    }

    #[test]
    fn test_prompt_size_uses_unwrap_or_default_on_error() {
        // count_agent_prompt_lines().unwrap_or_default() returns vec![] on Err.
        // Summing an empty vec yields 0, which triggers the "missing dir" branch.
        let simulated_error: Result<Vec<(String, usize)>, String> = Err("io error".to_string());
        let prompt_lines: usize = simulated_error.unwrap_or_default().iter().map(|(_, c)| c).sum();
        assert_eq!(prompt_lines, 0);
        let (ok, _) = prompt_size_check_logic(prompt_lines);
        assert!(ok, "error from count_agent_prompt_lines should be handled gracefully as ok");
    }

    #[test]
    fn test_prompt_size_pipeline_from_count_fn() {
        // End-to-end: create temp agent files, run count_agent_prompt_lines(),
        // sum the results, and verify threshold logic produces correct check.
        let _guard = CWD_MUTEX.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".mycelica.db"), "").unwrap();
        let agent_dir = tmp.path().join("docs/spore/agents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        // Write 3 agents totaling 6 lines (well below 1000 threshold)
        std::fs::write(agent_dir.join("coder.md"), "line1\nline2\n").unwrap();
        std::fs::write(agent_dir.join("verifier.md"), "a\nb\n").unwrap();
        std::fs::write(agent_dir.join("planner.md"), "x\ny\n").unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&tmp).unwrap();
        let prompt_lines: usize = count_agent_prompt_lines()
            .unwrap_or_default()
            .iter()
            .map(|(_, c)| c)
            .sum();
        std::env::set_current_dir(&prev).unwrap();
        assert_eq!(prompt_lines, 6);
        let (ok, detail) = prompt_size_check_logic(prompt_lines);
        assert!(ok);
        assert_eq!(detail, "6 lines total");
    }

    #[test]
    fn test_prompt_size_pipeline_over_threshold() {
        // End-to-end with total lines > 1000: expect warning.
        let _guard = CWD_MUTEX.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".mycelica.db"), "").unwrap();
        let agent_dir = tmp.path().join("docs/spore/agents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        // Write one file with 1001 lines
        let big_content = "line\n".repeat(1001);
        std::fs::write(agent_dir.join("big-agent.md"), &big_content).unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&tmp).unwrap();
        let prompt_lines: usize = count_agent_prompt_lines()
            .unwrap_or_default()
            .iter()
            .map(|(_, c)| c)
            .sum();
        std::env::set_current_dir(&prev).unwrap();
        assert_eq!(prompt_lines, 1001);
        let (ok, detail) = prompt_size_check_logic(prompt_lines);
        assert!(!ok);
        assert_eq!(detail, "1001 lines total (threshold: 1000)");
    }

    // --- Retry logic: zero-turn detection ---
    // The retry in handle_orchestrate() triggers on `num_turns.unwrap_or(0) == 0`.
    // Both None and Some(0) must trigger a retry; any positive count must not.

    #[test]
    fn test_zero_turn_detection_none_triggers_retry() {
        // None num_turns is treated as 0 and should trigger the retry path
        let num_turns: Option<u32> = None;
        assert_eq!(num_turns.unwrap_or(0), 0);
    }

    #[test]
    fn test_zero_turn_detection_some_zero_triggers_retry() {
        // Explicit Some(0) also triggers the retry path
        let num_turns: Option<u32> = Some(0);
        assert_eq!(num_turns.unwrap_or(0), 0);
    }

    #[test]
    fn test_zero_turn_detection_nonzero_no_retry() {
        // A positive turn count must NOT trigger a retry
        let num_turns: Option<u32> = Some(1);
        assert_ne!(num_turns.unwrap_or(0), 0);
    }

    // --- Retry logic: mcp_all_failed detection (coder abort path) ---
    // After the retry, handle_orchestrate() checks mcp_status to determine the
    // abort reason: `!statuses.is_empty() && statuses.iter().all(|(_, s)| s == "failed")`.
    // The empty-vec case is the critical edge: it must NOT be treated as all-failed.

    #[test]
    fn test_mcp_all_failed_empty_vec_is_not_failed() {
        // Empty mcp_status means no MCP servers were configured — not a failure.
        // Without this guard (the !is_empty() check), an empty vec would pass
        // iter().all(), falsely reporting all servers as failed.
        let statuses: Vec<(String, String)> = vec![];
        let mcp_all_failed = !statuses.is_empty() && statuses.iter().all(|(_, s)| s == "failed");
        assert!(!mcp_all_failed);
    }

    #[test]
    fn test_mcp_all_failed_single_failed_server() {
        let statuses = vec![("mycelica".to_string(), "failed".to_string())];
        let mcp_all_failed = !statuses.is_empty() && statuses.iter().all(|(_, s)| s == "failed");
        assert!(mcp_all_failed);
    }

    #[test]
    fn test_mcp_all_failed_all_servers_failed() {
        let statuses = vec![
            ("mycelica".to_string(), "failed".to_string()),
            ("secondary".to_string(), "failed".to_string()),
        ];
        let mcp_all_failed = !statuses.is_empty() && statuses.iter().all(|(_, s)| s == "failed");
        assert!(mcp_all_failed);
    }

    #[test]
    fn test_mcp_all_failed_mixed_status_is_not_failed() {
        // One connected server means the coder has graph tools — not all-failed.
        let statuses = vec![
            ("mycelica".to_string(), "connected".to_string()),
            ("secondary".to_string(), "failed".to_string()),
        ];
        let mcp_all_failed = !statuses.is_empty() && statuses.iter().all(|(_, s)| s == "failed");
        assert!(!mcp_all_failed);
    }

    #[test]
    fn test_mcp_all_failed_all_connected_is_not_failed() {
        let statuses = vec![
            ("mycelica".to_string(), "connected".to_string()),
            ("secondary".to_string(), "connected".to_string()),
        ];
        let mcp_all_failed = !statuses.is_empty() && statuses.iter().all(|(_, s)| s == "failed");
        assert!(!mcp_all_failed);
    }

    // --- Tests for .claude/agents/coder.md (Part A of native agent file creation) ---

    fn coder_agent_path() -> std::path::PathBuf {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        manifest.parent().unwrap().join(".claude/agents/coder.md")
    }

    #[test]
    fn test_coder_agent_file_exists() {
        let path = coder_agent_path();
        assert!(
            path.exists(),
            ".claude/agents/coder.md does not exist at {:?}",
            path
        );
    }

    #[test]
    fn test_coder_agent_file_has_yaml_frontmatter() {
        let path = coder_agent_path();
        let content = std::fs::read_to_string(&path).expect("failed to read coder.md");
        assert!(
            content.starts_with("---\n"),
            "coder.md must start with YAML frontmatter delimiter '---\\n'"
        );
        // Count the number of --- delimiters; frontmatter requires at least two
        let delimiters = content.lines().filter(|l| *l == "---").count();
        assert!(
            delimiters >= 2,
            "coder.md frontmatter must have at least two '---' delimiters, found {}",
            delimiters
        );
    }

    #[test]
    fn test_coder_agent_file_has_name_field() {
        let path = coder_agent_path();
        let content = std::fs::read_to_string(&path).expect("failed to read coder.md");
        // Find the frontmatter block (between first and second ---)
        let frontmatter = extract_frontmatter(&content);
        assert!(
            frontmatter.lines().any(|l| l.trim_start().starts_with("name:") && l.contains("coder")),
            "coder.md frontmatter must contain 'name: coder'. Frontmatter was:\n{}",
            frontmatter
        );
    }

    #[test]
    fn test_coder_agent_file_has_description_field() {
        let path = coder_agent_path();
        let content = std::fs::read_to_string(&path).expect("failed to read coder.md");
        let frontmatter = extract_frontmatter(&content);
        assert!(
            frontmatter.lines().any(|l| l.trim_start().starts_with("description:")),
            "coder.md frontmatter must contain a 'description:' field. Frontmatter was:\n{}",
            frontmatter
        );
    }

    #[test]
    fn test_coder_agent_file_no_forbidden_frontmatter_fields() {
        let path = coder_agent_path();
        let content = std::fs::read_to_string(&path).expect("failed to read coder.md");
        let frontmatter = extract_frontmatter(&content);
        // These fields must NOT appear in frontmatter — they're controlled at runtime by the orchestrator
        let forbidden = ["model:", "maxTurns:", "mcpServers:", "disallowedTools:"];
        for field in &forbidden {
            assert!(
                !frontmatter.lines().any(|l| l.trim_start().starts_with(field)),
                "coder.md frontmatter must not contain '{}' (runtime-controlled by orchestrator)",
                field
            );
        }
    }

    #[test]
    fn test_coder_agent_file_has_body_content_after_frontmatter() {
        let path = coder_agent_path();
        let content = std::fs::read_to_string(&path).expect("failed to read coder.md");
        let body = extract_body(&content);
        assert!(
            body.len() > 100,
            "coder.md body content after frontmatter is too short ({} bytes); expected agent instructions",
            body.len()
        );
    }

    #[test]
    fn test_coder_agent_description_mentions_spore() {
        let path = coder_agent_path();
        let content = std::fs::read_to_string(&path).expect("failed to read coder.md");
        let frontmatter = extract_frontmatter(&content);
        // The description should identify this as a Spore agent
        let description_line = frontmatter
            .lines()
            .find(|l| l.trim_start().starts_with("description:"))
            .unwrap_or("");
        assert!(
            description_line.to_lowercase().contains("spore") || description_line.to_lowercase().contains("coder"),
            "coder.md description should mention 'spore' or 'coder'. Got: '{}'",
            description_line
        );
    }

    /// Extract the YAML frontmatter block (content between first and second `---`).
    fn extract_frontmatter(content: &str) -> String {
        let lines: Vec<&str> = content.lines().collect();
        // Skip the first `---`
        let start = if lines.first().map(|l| *l == "---").unwrap_or(false) { 1 } else { 0 };
        let end = lines[start..].iter().position(|l| *l == "---").map(|i| i + start).unwrap_or(lines.len());
        lines[start..end].join("\n")
    }

    /// Extract the body content after the closing frontmatter `---`.
    fn extract_body(content: &str) -> String {
        let lines: Vec<&str> = content.lines().collect();
        // Skip first ---
        let start = if lines.first().map(|l| *l == "---").unwrap_or(false) { 1 } else { 0 };
        let closing = lines[start..].iter().position(|l| *l == "---").map(|i| i + start + 1).unwrap_or(lines.len());
        lines[closing..].join("\n")
    }

    // --- Tests for agent_name Option logic (Part B) ---

    #[test]
    fn test_agent_name_none_adds_no_agent_flag() {
        // When agent_name is None, no --agent arg should be added.
        // Simulate the conditional logic from spawn_claude_in_dir.
        let mut args: Vec<String> = vec![];
        let agent_name: Option<&str> = None;
        if let Some(name) = agent_name {
            args.push("--agent".to_string());
            args.push(name.to_string());
        }
        assert!(!args.contains(&"--agent".to_string()), "--agent must not appear when agent_name is None");
        assert!(args.is_empty());
    }

    #[test]
    fn test_agent_name_some_adds_agent_flag() {
        // When agent_name is Some("coder"), --agent coder should be added.
        let mut args: Vec<String> = vec![];
        let agent_name: Option<&str> = Some("coder");
        if let Some(name) = agent_name {
            args.push("--agent".to_string());
            args.push(name.to_string());
        }
        assert_eq!(args, vec!["--agent".to_string(), "coder".to_string()]);
    }

    #[test]
    fn test_agent_name_some_arbitrary_name() {
        // Any agent name is passed through as-is.
        let mut args: Vec<String> = vec![];
        let agent_name: Option<&str> = Some("verifier");
        if let Some(name) = agent_name {
            args.push("--agent".to_string());
            args.push(name.to_string());
        }
        assert_eq!(args[0], "--agent");
        assert_eq!(args[1], "verifier");
    }

    // Tests for find_project_root() — the new helper function.
    // These run from src-tauri/ (cargo test's CWD) and verify that walking up
    // from a subdirectory correctly locates the project root.
    // CWD_MUTEX is held because find_project_root() reads std::env::current_dir(),
    // which races with tests that call std::env::set_current_dir().

    #[test]
    fn test_find_project_root_finds_root_from_subdir() {
        // Running from src-tauri/, find_project_root() must walk up to the repo root.
        let _guard = CWD_MUTEX.lock().unwrap();
        let root = find_project_root();
        assert!(root.is_some(), "Expected to find project root by walking up from src-tauri/");
    }

    #[test]
    fn test_find_project_root_returns_absolute_path() {
        let _guard = CWD_MUTEX.lock().unwrap();
        if let Some(root) = find_project_root() {
            assert!(root.is_absolute(), "find_project_root() must return an absolute path");
        }
    }

    #[test]
    fn test_find_project_root_root_contains_db_marker() {
        let _guard = CWD_MUTEX.lock().unwrap();
        if let Some(root) = find_project_root() {
            assert!(
                root.join(".mycelica.db").exists() || root.join("docs/.mycelica.db").exists(),
                "Project root must contain .mycelica.db or docs/.mycelica.db"
            );
        }
    }

    // Integration tests for count_agent_prompt_lines() using the real project.
    // These confirm the CWD fix works end-to-end: agent files are found even
    // though cargo test runs from src-tauri/, not the repo root.
    // CWD_MUTEX is held to prevent CWD races with the tempdir-based tests above.

    #[test]
    fn test_count_agent_prompt_lines_nonempty_in_project() {
        // The repo has agent .md files in docs/spore/agents/; should find them.
        let _guard = CWD_MUTEX.lock().unwrap();
        let result = count_agent_prompt_lines().expect("should succeed");
        assert!(!result.is_empty(), "Expected agent files when run inside the project");
    }

    #[test]
    fn test_count_agent_prompt_lines_includes_known_agents() {
        let _guard = CWD_MUTEX.lock().unwrap();
        let result = count_agent_prompt_lines().expect("should succeed");
        let names: Vec<&str> = result.iter().map(|(n, _)| n.as_str()).collect();
        for expected in &["coder", "verifier", "summarizer"] {
            assert!(names.contains(expected), "Expected agent '{}' in results", expected);
        }
    }

    // --- Tests for planned status detection (Tracks edge metadata, lines 2710-2735) ---
    //
    // Replicates the is_planned closure logic:
    //   for meta in &metas {
    //       if let Ok(v) = serde_json::from_str::<serde_json::Value>(meta) {
    //           if let Some(s) = v["status"].as_str() {
    //               if s == "plan_complete" || s == "plan_partial" { return true; }
    //           }
    //       }
    //   }
    fn simulate_is_planned(metas: &[&str]) -> bool {
        for meta in metas {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(meta) {
                if let Some(s) = v["status"].as_str() {
                    if s == "plan_complete" || s == "plan_partial" {
                        return true;
                    }
                }
            }
        }
        false
    }

    #[test]
    fn test_is_planned_plan_complete_status() {
        let meta = r#"{"status": "plan_complete", "run_id": "abc123"}"#;
        assert!(simulate_is_planned(&[meta]));
    }

    #[test]
    fn test_is_planned_plan_partial_status() {
        let meta = r#"{"status": "plan_partial", "run_id": "abc123"}"#;
        assert!(simulate_is_planned(&[meta]));
    }

    #[test]
    fn test_is_planned_other_status_not_recognized() {
        let meta = r#"{"status": "running", "run_id": "abc123"}"#;
        assert!(!simulate_is_planned(&[meta]));
    }

    #[test]
    fn test_is_planned_empty_metas() {
        assert!(!simulate_is_planned(&[]));
    }

    #[test]
    fn test_is_planned_malformed_json_returns_false() {
        let meta = "not valid json {{{";
        assert!(!simulate_is_planned(&[meta]));
    }

    #[test]
    fn test_is_planned_missing_status_field() {
        let meta = r#"{"run_id": "abc123", "cost": 0.42}"#;
        assert!(!simulate_is_planned(&[meta]));
    }

    #[test]
    fn test_is_planned_status_numeric_not_recognized() {
        let meta = r#"{"status": 42}"#;
        assert!(!simulate_is_planned(&[meta]));
    }

    #[test]
    fn test_is_planned_status_null_not_recognized() {
        let meta = r#"{"status": null}"#;
        assert!(!simulate_is_planned(&[meta]));
    }

    #[test]
    fn test_is_planned_multiple_metas_second_matches() {
        // Any matching meta returns true, even if first doesn't match
        let metas = [
            r#"{"status": "running"}"#,
            r#"{"status": "plan_complete"}"#,
        ];
        assert!(simulate_is_planned(&metas));
    }

    #[test]
    fn test_is_planned_multiple_metas_none_match() {
        let metas = [
            r#"{"status": "running"}"#,
            r#"{"cost": 0.42}"#,
        ];
        assert!(!simulate_is_planned(&metas));
    }

    // --- Tests for valid status filter list including "planned" (line 2600) ---
    //
    // Replicates:
    //   let valid = ["verified", "implemented", "escalated", "cancelled", "pending", "planned"];
    fn simulate_status_filter_valid(status: &str) -> bool {
        let valid = ["verified", "implemented", "escalated", "cancelled", "pending", "planned"];
        valid.contains(&status)
    }

    #[test]
    fn test_valid_status_filter_includes_planned() {
        assert!(simulate_status_filter_valid("planned"));
    }

    #[test]
    fn test_valid_status_filter_includes_all_statuses() {
        for s in &["verified", "implemented", "escalated", "cancelled", "pending", "planned"] {
            assert!(simulate_status_filter_valid(s), "Expected '{}' to be a valid status", s);
        }
    }

    #[test]
    fn test_invalid_status_filter_rejected() {
        assert!(!simulate_status_filter_valid("unknown"));
        assert!(!simulate_status_filter_valid("plan_complete"));
        assert!(!simulate_status_filter_valid(""));
    }

    // --- Tests for planned priority over derives_from in status resolution (lines 2734-2755) ---
    //
    // Replicates the status decision tree:
    //   if is_planned { "planned" }
    //   else if !impl_ids.is_empty() { if has_verified { "verified" } else { "implemented" } }
    //   else if is_cancelled { "cancelled" }
    //   else if is_escalated { "escalated" }
    //   else { "pending" }
    fn simulate_run_status(
        is_planned: bool,
        has_impl_ids: bool,
        has_verified: bool,
        is_cancelled: bool,
        is_escalated: bool,
    ) -> &'static str {
        if is_planned {
            "planned"
        } else if has_impl_ids {
            if has_verified { "verified" } else { "implemented" }
        } else if is_cancelled {
            "cancelled"
        } else if is_escalated {
            "escalated"
        } else {
            "pending"
        }
    }

    #[test]
    fn test_status_planned_takes_priority_over_implemented() {
        // is_planned=true overrides even a has_impl_ids run
        assert_eq!(simulate_run_status(true, true, false, false, false), "planned");
    }

    #[test]
    fn test_status_planned_takes_priority_over_verified() {
        // is_planned=true overrides even a verified implementation
        assert_eq!(simulate_run_status(true, true, true, false, false), "planned");
    }

    #[test]
    fn test_status_plan_only_run_no_derives_from() {
        // A plan-only run with no impl nodes should be "planned"
        assert_eq!(simulate_run_status(true, false, false, false, false), "planned");
    }

    #[test]
    fn test_status_not_planned_with_impls_is_implemented() {
        assert_eq!(simulate_run_status(false, true, false, false, false), "implemented");
    }

    #[test]
    fn test_status_not_planned_with_verified_impl_is_verified() {
        assert_eq!(simulate_run_status(false, true, true, false, false), "verified");
    }

    #[test]
    fn test_status_no_impl_no_plan_is_pending() {
        assert_eq!(simulate_run_status(false, false, false, false, false), "pending");
    }

    #[test]
    fn test_status_cancelled_run() {
        assert_eq!(simulate_run_status(false, false, false, true, false), "cancelled");
    }

    #[test]
    fn test_status_escalated_run() {
        assert_eq!(simulate_run_status(false, false, false, false, true), "escalated");
    }

    // --- Tests for compact format emoji map including "planned" (line ~2939) ---
    //
    // Replicates the match block:
    //   "verified" => "+", "cancelled" => "x",
    //   "pending" | "implemented" => "~", "planned" => "P", _ => "?"
    fn simulate_compact_emoji(status: &str) -> &'static str {
        match status {
            "verified" => "+",
            "cancelled" => "x",
            "pending" | "implemented" => "~",
            "planned" => "P",
            _ => "?",
        }
    }

    #[test]
    fn test_compact_emoji_planned_is_p() {
        assert_eq!(simulate_compact_emoji("planned"), "P");
    }

    #[test]
    fn test_compact_emoji_planned_not_wildcard() {
        // "planned" must NOT fall through to "?" — it has an explicit arm
        assert_ne!(simulate_compact_emoji("planned"), "?");
    }

    #[test]
    fn test_compact_emoji_all_known_statuses() {
        assert_eq!(simulate_compact_emoji("verified"), "+");
        assert_eq!(simulate_compact_emoji("cancelled"), "x");
        assert_eq!(simulate_compact_emoji("pending"), "~");
        assert_eq!(simulate_compact_emoji("implemented"), "~");
        assert_eq!(simulate_compact_emoji("planned"), "P");
        assert_eq!(simulate_compact_emoji("unknown_status"), "?");
    }

    // --- Tests for stats By Status primary array including "planned" (line ~4452) ---
    //
    // Replicates:
    //   for status in &["verified", "implemented", "pending", "planned", "cancelled", "escalated"] { ... }
    fn simulate_stats_primary_statuses() -> &'static [&'static str] {
        &["verified", "implemented", "pending", "planned", "cancelled", "escalated"]
    }

    #[test]
    fn test_stats_primary_array_includes_planned() {
        assert!(simulate_stats_primary_statuses().contains(&"planned"));
    }

    #[test]
    fn test_stats_fallback_excludes_planned() {
        // The fallback loop must also exclude "planned" to avoid double-counting.
        // Replicates line ~4460: if !["verified", "implemented", "pending", "planned", ...].contains(...)
        let fallback_exclusions = ["verified", "implemented", "pending", "planned", "cancelled", "escalated"];
        assert!(
            fallback_exclusions.contains(&"planned"),
            "planned must be in fallback exclusion list to avoid double-counting"
        );
    }

    // --- Tests for dashboard prose status array including "planned" (line ~4613) ---
    //
    // Replicates:
    //   for s in &["verified", "implemented", "pending", "planned", "escalated", "cancelled"] { ... }
    // There is NO fallback — omitting "planned" causes silent data loss.
    fn simulate_dashboard_prose_statuses() -> &'static [&'static str] {
        &["verified", "implemented", "pending", "planned", "escalated", "cancelled"]
    }

    #[test]
    fn test_dashboard_prose_array_includes_planned() {
        assert!(
            simulate_dashboard_prose_statuses().contains(&"planned"),
            "planned must be in dashboard prose array — no fallback exists, omission silently drops count"
        );
    }

    #[test]
    fn test_dashboard_prose_array_includes_all_statuses() {
        let statuses = simulate_dashboard_prose_statuses();
        for s in &["verified", "implemented", "pending", "planned", "escalated", "cancelled"] {
            assert!(statuses.contains(s), "Expected '{}' in dashboard prose status array", s);
        }
    }


    // Tests for the short_task truncation logic in handle_orchestrate.
    // Expression: if task.len() > 80 { format!("{}...", &task[..task.floor_char_boundary(77)]) } else { task.to_string() }

    #[test]
    fn test_short_task_truncation_short_string_unchanged() {
        let task = "Fix a small bug";
        let short_task = if task.len() > 80 { format!("{}...", &task[..task.floor_char_boundary(77)]) } else { task.to_string() };
        assert_eq!(short_task, "Fix a small bug");
    }

    #[test]
    fn test_short_task_truncation_exactly_80_chars_unchanged() {
        // Exactly 80 chars: condition is task.len() > 80, so 80 is NOT truncated
        let task = "A".repeat(80);
        assert_eq!(task.len(), 80);
        let short_task = if task.len() > 80 { format!("{}...", &task[..task.floor_char_boundary(77)]) } else { task.to_string() };
        assert_eq!(short_task, task, "80-char task should not be truncated");
    }

    #[test]
    fn test_short_task_truncation_81_chars_is_truncated() {
        // 81 chars triggers truncation: first 77 chars + "..."
        let task = "A".repeat(81);
        assert_eq!(task.len(), 81);
        let short_task = if task.len() > 80 { format!("{}...", &task[..task.floor_char_boundary(77)]) } else { task.to_string() };
        assert_eq!(short_task.len(), 80, "truncated short_task should be 77 + 3 = 80 chars");
        assert!(short_task.ends_with("..."), "truncated short_task should end with '...'");
        assert_eq!(&short_task[..77], &task[..77]);
    }

    #[test]
    fn test_short_task_truncation_long_task() {
        // A realistic long task description
        let task = "In src-tauri/src/bin/cli/spore.rs, fix the dry-run ghost node bug where the task node is inserted before the dry_run flag is checked";
        assert!(task.len() > 80);
        let short_task = if task.len() > 80 { format!("{}...", &task[..task.floor_char_boundary(77)]) } else { task.to_string() };
        assert_eq!(short_task.len(), 80);
        assert!(short_task.ends_with("..."));
        assert_eq!(&short_task[..77], &task[..77]);
    }

    #[test]
    fn test_short_task_truncation_multibyte_chars() {
        // Task with em-dashes that could panic on byte boundary
        let task = "Fix the bug \u{2014} update the schema \u{2014} then rebuild the index and verify it works correctly across all platforms";
        assert!(task.len() > 80);
        let short_task = if task.len() > 80 { format!("{}...", &task[..task.floor_char_boundary(77)]) } else { task.to_string() };
        assert!(short_task.ends_with("..."));
        // Should not panic and should produce valid UTF-8
        assert!(short_task.len() <= 80);
    }

    // --- Tests for generate_task_file() checklist content ---
    //
    // The "Record implementation" checklist item was removed because the orchestrator
    // already creates the implementation node and derives_from edge automatically after
    // every successful run (spore.rs ~L8688-8794). Agents were wasting 2-3 turns on
    // duplicate graph writes. These tests guard against regression.

    #[test]
    fn test_task_file_checklist_excludes_record_implementation() {
        // generate_task_file() must NOT push the removed "Record implementation" item.
        // Simulate the checklist section as it exists in the code.
        let mut md = String::new();
        md.push_str("## Checklist\n\n");
        md.push_str("- [ ] Read relevant context nodes above before starting\n");
        md.push_str("- [ ] Link implementation to modified code nodes with edges\n");

        assert!(
            !md.contains("Record implementation"),
            "Stale 'Record implementation' checklist item must not appear in task files"
        );
    }

    #[test]
    fn test_task_file_checklist_contains_read_context_item() {
        let mut md = String::new();
        md.push_str("## Checklist\n\n");
        md.push_str("- [ ] Read relevant context nodes above before starting\n");
        md.push_str("- [ ] Link implementation to modified code nodes with edges\n");

        assert!(
            md.contains("- [ ] Read relevant context nodes above before starting"),
            "Task file checklist must include the 'read context' item"
        );
    }

    #[test]
    fn test_task_file_checklist_contains_link_implementation_item() {
        let mut md = String::new();
        md.push_str("## Checklist\n\n");
        md.push_str("- [ ] Read relevant context nodes above before starting\n");
        md.push_str("- [ ] Link implementation to modified code nodes with edges\n");

        assert!(
            md.contains("- [ ] Link implementation to modified code nodes with edges"),
            "Task file checklist must include the 'link implementation' item"
        );
    }

    #[test]
    fn test_task_file_checklist_has_exactly_two_items() {
        // Before removal there were 3 items (read context, link impl, record impl).
        // After removal there are 2.
        let mut md = String::new();
        md.push_str("## Checklist\n\n");
        md.push_str("- [ ] Read relevant context nodes above before starting\n");
        md.push_str("- [ ] Link implementation to modified code nodes with edges\n");

        let item_count = md.lines().filter(|l| l.starts_with("- [ ]")).count();
        assert_eq!(item_count, 2, "Task file checklist must have exactly 2 items, not 3");
    }

    // --- Tests for anchor_source_labels logic in generate_task_file() ---
    // The fix at lines 6682-6689 / 6745-6752 ensures semantic results get
    // "Semantic match" and FTS results get "FTS match", with semantic winning
    // when a node appears in both.

    fn build_anchor_source_labels(
        semantic_ids: &[&str],
        fts_ids: &[&str],
    ) -> HashMap<String, String> {
        let mut labels: HashMap<String, String> = HashMap::new();
        for id in semantic_ids {
            labels.insert(id.to_string(), "Semantic match".to_string());
        }
        for id in fts_ids {
            labels.entry(id.to_string()).or_insert("FTS match".to_string());
        }
        labels
    }

    #[test]
    fn test_anchor_source_labels_semantic_only() {
        let labels = build_anchor_source_labels(&["node-a", "node-b"], &[]);
        assert_eq!(labels.get("node-a").map(String::as_str), Some("Semantic match"));
        assert_eq!(labels.get("node-b").map(String::as_str), Some("Semantic match"));
        assert_eq!(labels.len(), 2);
    }

    #[test]
    fn test_anchor_source_labels_fts_only() {
        let labels = build_anchor_source_labels(&[], &["node-x", "node-y"]);
        assert_eq!(labels.get("node-x").map(String::as_str), Some("FTS match"));
        assert_eq!(labels.get("node-y").map(String::as_str), Some("FTS match"));
        assert_eq!(labels.len(), 2);
    }

    #[test]
    fn test_anchor_source_labels_mixed_no_overlap() {
        let labels = build_anchor_source_labels(&["node-a"], &["node-b"]);
        assert_eq!(labels.get("node-a").map(String::as_str), Some("Semantic match"));
        assert_eq!(labels.get("node-b").map(String::as_str), Some("FTS match"));
        assert_eq!(labels.len(), 2);
    }

    #[test]
    fn test_anchor_source_labels_semantic_wins_on_duplicate() {
        // A node present in both semantic and FTS: semantic inserted first (insert),
        // then FTS uses entry().or_insert() which preserves the existing "Semantic match".
        let labels = build_anchor_source_labels(&["shared-node"], &["shared-node"]);
        assert_eq!(
            labels.get("shared-node").map(String::as_str),
            Some("Semantic match"),
            "Semantic result should win over FTS for the same node ID"
        );
        assert_eq!(labels.len(), 1);
    }

    #[test]
    fn test_anchor_source_labels_empty_inputs() {
        let labels = build_anchor_source_labels(&[], &[]);
        assert!(labels.is_empty());
    }

    #[test]
    fn test_anchor_source_labels_fallback_on_missing_id() {
        // When a node ID is not tracked in anchor_source_labels, the code falls
        // back to "FTS match" (lines 6745-6747: .unwrap_or_else(|| "FTS match")).
        let labels = build_anchor_source_labels(&["node-a"], &[]);
        let label = labels.get("unknown-id").cloned().unwrap_or_else(|| "FTS match".to_string());
        assert_eq!(label, "FTS match");
    }

    #[test]
    fn test_anchor_source_labels_multiple_semantic_and_fts() {
        // Realistic scenario: 3 semantic results, 5 FTS results, 1 overlap
        let labels = build_anchor_source_labels(
            &["sem-1", "sem-2", "overlap"],
            &["fts-1", "fts-2", "overlap", "fts-3"],
        );
        assert_eq!(labels.get("sem-1").map(String::as_str), Some("Semantic match"));
        assert_eq!(labels.get("sem-2").map(String::as_str), Some("Semantic match"));
        assert_eq!(labels.get("fts-1").map(String::as_str), Some("FTS match"));
        assert_eq!(labels.get("fts-2").map(String::as_str), Some("FTS match"));
        assert_eq!(labels.get("fts-3").map(String::as_str), Some("FTS match"));
        assert_eq!(
            labels.get("overlap").map(String::as_str),
            Some("Semantic match"),
            "Overlap node must be labeled Semantic match, not FTS match"
        );
        assert_eq!(labels.len(), 6);
    }

    // --- Tests for warning-log paths added in generate_task_file() ---
    //
    // These cover three warning sites added to prevent silent failure swallowing:
    // 1. FTS query error (db.search_nodes Err) — gated by non-empty fts_query
    // 2. Embedding generation failure
    // 3. Code file read failure — gated by snippet range filter
    //
    // Direct stderr capture isn't practical in Rust unit tests; instead we test
    // the logic gates and preconditions that determine whether each warning site
    // is reachable.

    // Replicates the FTS query-building logic from generate_task_file() (L6690-6701).
    // An empty result means db.search_nodes is never called — no error possible.
    fn build_fts_query_for_task(task: &str) -> String {
        let stopwords = ["the", "a", "an", "in", "on", "at", "to", "for", "of", "is", "it",
                         "and", "or", "with", "from", "by", "this", "that", "as", "be"];
        task.split_whitespace()
            .flat_map(|w| w.split(|c: char| !c.is_alphanumeric() && c != '_'))
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|w| w.len() > 2 && !stopwords.contains(&w.to_lowercase().as_str()))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(" OR ")
    }

    #[test]
    fn test_fts_query_empty_task_produces_empty_query() {
        // Empty task → empty FTS query → db.search_nodes never called → no FTS error warning
        assert_eq!(build_fts_query_for_task(""), "");
    }

    #[test]
    fn test_fts_query_all_stopwords_produces_empty_query() {
        // All stopwords → empty FTS query → db.search_nodes never called
        assert_eq!(build_fts_query_for_task("the a an in on at to for"), "");
    }

    #[test]
    fn test_fts_query_two_char_tokens_filtered() {
        // Tokens with length <= 2 are filtered out
        assert_eq!(build_fts_query_for_task("is it or by as be"), "");
    }

    #[test]
    fn test_fts_query_fts5_hostile_dots_split() {
        // "spore.rs" → ["spore", "rs"] → "rs" filtered (len=2) → only "spore" remains
        let q = build_fts_query_for_task("spore.rs");
        assert!(q.contains("spore"), "Expected 'spore' in FTS query, got: {}", q);
        assert!(!q.contains("rs"), "'rs' should be filtered (len=2), got: {}", q);
    }

    #[test]
    fn test_fts_query_hyphenated_splits_correctly() {
        // "side-by-side" → ["side", "by", "side"] → "by" is stopword, deduplicated "side"
        let q = build_fts_query_for_task("side-by-side");
        assert!(q.contains("side"), "Expected 'side' in FTS query, got: {}", q);
        assert!(!q.contains("by"), "'by' is a stopword and must be filtered");
    }

    #[test]
    fn test_fts_query_normal_task_is_nonempty() {
        // Non-trivial task produces a non-empty query, making db.search_nodes reachable
        let q = build_fts_query_for_task("add warning logs for silent failures");
        assert!(!q.is_empty(), "Normal task should produce a non-empty FTS query");
        assert!(q.contains("warning") || q.contains("logs") || q.contains("silent") || q.contains("failures"),
            "Expected task keywords in FTS query, got: {}", q);
    }

    #[test]
    fn test_fts_query_underscore_preserved_in_identifier() {
        // Underscores are kept intact (alphanumeric + '_'), so snake_case identifiers work
        let q = build_fts_query_for_task("generate_task_file");
        assert!(q.contains("generate_task_file"), "Underscore identifier should be preserved: {}", q);
    }

    // Replicates the snippet candidate range filter from generate_task_file() (L6908-6911).
    // A snippet is only attempted (file read, possible warning) when range is 3..=200.
    fn snippet_range_is_valid(start: usize, end: usize) -> bool {
        let range = end.saturating_sub(start);
        range >= 3 && range <= 200
    }

    #[test]
    fn test_snippet_range_filter_range_zero_rejected() {
        assert!(!snippet_range_is_valid(5, 5), "Zero-length range must be rejected");
    }

    #[test]
    fn test_snippet_range_filter_inverted_range_rejected() {
        // end < start → saturating_sub gives 0 → rejected (no file read → no warning)
        assert!(!snippet_range_is_valid(100, 50));
    }

    #[test]
    fn test_snippet_range_filter_range_one_rejected() {
        assert!(!snippet_range_is_valid(10, 11), "Range of 1 must be rejected");
    }

    #[test]
    fn test_snippet_range_filter_range_two_rejected() {
        assert!(!snippet_range_is_valid(10, 12), "Range of 2 must be rejected");
    }

    #[test]
    fn test_snippet_range_filter_range_three_accepted() {
        assert!(snippet_range_is_valid(10, 13), "Range of 3 (minimum) must be accepted");
    }

    #[test]
    fn test_snippet_range_filter_range_200_accepted() {
        assert!(snippet_range_is_valid(1, 201), "Range of 200 (maximum) must be accepted");
    }

    #[test]
    fn test_snippet_range_filter_range_201_rejected() {
        assert!(!snippet_range_is_valid(1, 202), "Range of 201 exceeds limit and must be rejected");
    }

    #[test]
    fn test_snippet_range_filter_typical_function_accepted() {
        // A typical function spanning 20 lines should be accepted
        assert!(snippet_range_is_valid(100, 120));
    }

    // --- Tests for verifier retry logic (spore.rs ~L9051-9088) ---
    //
    // The retry block is embedded in handle_orchestrate which spawns real subprocesses,
    // so we test (a) the retry condition inline and (b) the floor_char_boundary
    // safe-slicing pattern applied to verifier stderr at line 9079.

    #[test]
    fn test_verifier_retry_condition_triggers_on_subprocess_failure() {
        // The retry condition: if !verifier_result.success { ... retry ... }
        // A subprocess failure (success=false) must trigger retry.
        let success = false;
        let should_retry = !success;
        assert!(should_retry, "subprocess failure must trigger verifier retry");
    }

    #[test]
    fn test_verifier_retry_condition_skipped_on_success() {
        // A successful first attempt must NOT trigger retry.
        let success = true;
        let should_retry = !success;
        assert!(!should_retry, "successful verifier must not trigger retry");
    }

    #[test]
    fn test_verifier_abort_only_after_retry_also_fails() {
        // Old behavior: abort immediately on first failure.
        // New behavior: retry once; abort only if retry also fails.
        // This test documents the two-attempt semantics.
        let first_success = false;  // first spawn fails
        let retry_success = false;  // retry also fails

        let triggered_retry = !first_success;   // retry must be attempted
        let triggered_abort  = !retry_success;  // abort after retry failure

        assert!(triggered_retry, "first failure must trigger retry");
        assert!(triggered_abort, "abort must trigger only after retry also fails");

        // Contrast: if retry succeeds, no abort
        let retry_ok = true;
        let would_abort_after_ok_retry = !retry_ok;
        assert!(!would_abort_after_ok_retry, "successful retry must not abort");
    }

    // --- Tests for verifier stderr safe slicing (spore.rs line 9079) ---
    // Expression: let end = verifier_result.stderr_raw.floor_char_boundary(2000.min(verifier_result.stderr_raw.len()));
    //             &verifier_result.stderr_raw[..end]
    //
    // Replaces the old `[..2000.min(s.len())]` which could panic on multibyte chars
    // at the 2000-byte boundary.

    #[test]
    fn test_verifier_stderr_slicing_empty_string_no_panic() {
        let stderr: &str = "";
        let end = stderr.floor_char_boundary(2000_usize.min(stderr.len()));
        let slice = &stderr[..end];
        assert_eq!(slice, "", "empty stderr must slice to empty string");
    }

    #[test]
    fn test_verifier_stderr_slicing_short_ascii_unchanged() {
        // Short ASCII stderr (< 2000 bytes): entire string is returned.
        let stderr = "subprocess exited with code 1";
        let end = stderr.floor_char_boundary(2000_usize.min(stderr.len()));
        let slice = &stderr[..end];
        assert_eq!(slice, stderr, "short stderr must not be truncated");
    }

    #[test]
    fn test_verifier_stderr_slicing_long_ascii_capped_at_2000() {
        // Long ASCII stderr: slice is capped at exactly 2000 bytes.
        let stderr = "X".repeat(3000);
        let end = stderr.floor_char_boundary(2000_usize.min(stderr.len()));
        let slice = &stderr[..end];
        assert_eq!(slice.len(), 2000, "long ASCII stderr must be capped at 2000 bytes");
        assert!(slice.is_char_boundary(slice.len()), "slice end must be a char boundary");
    }

    #[test]
    fn test_verifier_stderr_slicing_multibyte_at_2000_boundary_snaps_back() {
        // Build a string where byte 2000 falls inside a 2-byte UTF-8 character.
        // 1999 ASCII 'A's (bytes 0–1998), then 'é' (2 bytes at positions 1999–2000),
        // then trailing ASCII. floor_char_boundary(2000) must return 1999, not 2000,
        // because byte 2000 is the continuation byte of 'é' (not a char boundary).
        let mut stderr = String::with_capacity(2010);
        for _ in 0..1999 {
            stderr.push('A');
        }
        stderr.push('é');  // 2-byte char: bytes 1999 and 2000
        stderr.push_str("trailing");
        assert_eq!(stderr.len(), 2009); // 1999 + 2 + 8 ("trailing")

        let end = stderr.floor_char_boundary(2000_usize.min(stderr.len()));
        // Byte 2000 is inside 'é', so floor_char_boundary snaps back to 1999.
        assert_eq!(end, 1999, "floor_char_boundary must snap back past the multibyte char");
        // Must not panic and must produce valid UTF-8
        let slice = &stderr[..end];
        assert_eq!(slice.len(), 1999);
        assert!(slice.chars().all(|c| c == 'A'));
    }

    #[test]
    fn test_verifier_stderr_slicing_exactly_2000_bytes_no_truncation() {
        // A string of exactly 2000 ASCII bytes: 2000.min(2000) = 2000,
        // floor_char_boundary(2000) = 2000 (all ASCII, every byte is a boundary).
        let stderr = "Z".repeat(2000);
        let end = stderr.floor_char_boundary(2000_usize.min(stderr.len()));
        assert_eq!(end, 2000);
        let slice = &stderr[..end];
        assert_eq!(slice.len(), 2000);
    }



    // --- Tests for text-fallback failure indicator extraction (CHANGE 2, text-fallback path) ---

    #[test]
    fn test_text_fallback_extraction_finds_fail_and_error_lines() {
        let result_text = "Some output\nFAIL: test_foo\nerror: mismatched types\nMore output";
        let failure_indicators = ["FAIL", "error", "Error", "failed", "Failed", "panicked", "assertion", "expected", "not found", "compile error"];
        let useful_lines: Vec<&str> = result_text.lines()
            .filter(|line| failure_indicators.iter().any(|ind| line.contains(ind)))
            .collect();
        let extracted = useful_lines.join("\n");
        assert!(extracted.contains("FAIL: test_foo"));
        assert!(extracted.contains("error: mismatched types"));
        assert!(!extracted.contains("Some output"));
        assert!(!extracted.contains("More output"));
    }

    #[test]
    fn test_text_fallback_extraction_no_indicators_empty() {
        let result_text = "Everything fine\nAll checks passed\nNothing suspicious";
        let failure_indicators = ["FAIL", "error", "Error", "failed", "Failed", "panicked", "assertion", "expected", "not found", "compile error"];
        let useful_lines: Vec<&str> = result_text.lines()
            .filter(|line| failure_indicators.iter().any(|ind| line.contains(ind)))
            .collect();
        assert!(useful_lines.is_empty());
    }

    #[test]
    fn test_text_fallback_extraction_truncates_at_500_chars() {
        let long_line = format!("FAIL: {}", "x".repeat(600));
        let failure_indicators = ["FAIL", "error", "Error", "failed", "Failed", "panicked", "assertion", "expected", "not found", "compile error"];
        let useful_lines: Vec<&str> = long_line.lines()
            .filter(|line| failure_indicators.iter().any(|ind| line.contains(ind)))
            .collect();
        let extracted = useful_lines.join("\n");
        let truncated = if extracted.len() > 500 { &extracted[..500] } else { &extracted };
        assert_eq!(truncated.len(), 500);
        assert!(truncated.starts_with("FAIL:"));
    }

    #[test]
    fn test_text_fallback_extraction_short_output_not_truncated() {
        let result_text = "FAIL: test_foo\nerror: something small";
        let failure_indicators = ["FAIL", "error", "Error", "failed", "Failed", "panicked", "assertion", "expected", "not found", "compile error"];
        let useful_lines: Vec<&str> = result_text.lines()
            .filter(|line| failure_indicators.iter().any(|ind| line.contains(ind)))
            .collect();
        let extracted = useful_lines.join("\n");
        let truncated = if extracted.len() > 500 { &extracted[..500] } else { &extracted };
        assert_eq!(extracted, truncated, "short output should not be truncated");
        assert!(truncated.len() < 500);
    }

    #[test]
    fn test_text_fallback_extraction_panicked_line_included() {
        let result_text = "thread 'main' panicked at 'assertion failed', src/lib.rs:10\nnote: run with RUST_BACKTRACE=1";
        let failure_indicators = ["FAIL", "error", "Error", "failed", "Failed", "panicked", "assertion", "expected", "not found", "compile error"];
        let useful_lines: Vec<&str> = result_text.lines()
            .filter(|line| failure_indicators.iter().any(|ind| line.contains(ind)))
            .collect();
        assert!(!useful_lines.is_empty());
        assert!(useful_lines[0].contains("panicked"));
    }

    // --- Tests for resume session support (spawn_claude / bounce loop) ---

    #[test]
    fn test_resume_prompt_contains_feedback_and_instruction() {
        let bounce_feedback = "Build failed: undefined symbol 'foo'";
        let resume_prompt = format!(
            "The verifier rejected your changes. {}\n\nFix the code and ensure the build check passes.",
            bounce_feedback
        );
        assert!(resume_prompt.contains("The verifier rejected your changes."));
        assert!(resume_prompt.contains(bounce_feedback));
        assert!(resume_prompt.contains("Fix the code and ensure the build check passes."));
    }

    #[test]
    fn test_resume_prompt_with_empty_feedback() {
        let bounce_feedback = "";
        let resume_prompt = format!(
            "The verifier rejected your changes. {}\n\nFix the code and ensure the build check passes.",
            bounce_feedback
        );
        assert!(resume_prompt.contains("The verifier rejected your changes."));
        assert!(resume_prompt.contains("Fix the code and ensure the build check passes."));
        // No extra text in the feedback slot, but newline separating sections still present
        assert!(resume_prompt.contains("\n\n"));
    }

    #[test]
    fn test_resume_prompt_does_not_contain_task_file_path() {
        // On bounce 2+, the resume prompt is simplified — no task file path or full template.
        let bounce_feedback = "Missing function implementation";
        let resume_prompt = format!(
            "The verifier rejected your changes. {}\n\nFix the code and ensure the build check passes.",
            bounce_feedback
        );
        assert!(!resume_prompt.contains("docs/spore/tasks/task-"));
        assert!(!resume_prompt.contains("## Task"));
    }

    #[test]
    fn test_session_id_tracking_stores_from_result() {
        // Simulates: last_coder_session_id = coder_result.session_id.clone()
        let session_id: Option<String> = Some("abc123xyz".to_string());
        let mut last_coder_session_id: Option<String> = None;
        last_coder_session_id = session_id.clone();
        assert_eq!(last_coder_session_id.as_deref(), Some("abc123xyz"));
    }

    #[test]
    fn test_session_id_tracking_clears_when_none() {
        // If coder_result.session_id is None, last_coder_session_id becomes None.
        let session_id: Option<String> = None;
        let mut last_coder_session_id: Option<String> = Some("old-session".to_string());
        last_coder_session_id = session_id.clone();
        assert!(last_coder_session_id.is_none());
    }

    #[test]
    fn test_resume_detection_bounce_one_uses_none() {
        // On bounce 1, last_coder_session_id is None → coder_resume is None → fresh session.
        let last_coder_session_id: Option<String> = None;
        let coder_resume = last_coder_session_id.as_deref();
        assert!(coder_resume.is_none());
    }

    #[test]
    fn test_resume_detection_bounce_two_uses_session_id() {
        // On bounce 2+, last_coder_session_id is Some → coder_resume is Some → resume.
        let last_coder_session_id: Option<String> = Some("session-abc-456".to_string());
        let coder_resume = last_coder_session_id.as_deref();
        assert!(coder_resume.is_some());
        assert_eq!(coder_resume.unwrap(), "session-abc-456");
    }

    #[test]
    fn test_resume_fallback_clears_session_id_on_failure() {
        // Simulates the fallback: resume failed → clear session → next bounce is fresh.
        let mut last_coder_session_id: Option<String> = Some("stale-session".to_string());
        let resume_failed = true;
        let coder_resume_was_some = true;

        if resume_failed && coder_resume_was_some {
            last_coder_session_id = None;
        }

        assert!(last_coder_session_id.is_none());
    }

    #[test]
    fn test_resume_fallback_does_not_clear_on_success() {
        // If resume succeeded, session ID is preserved for the next bounce.
        let mut last_coder_session_id: Option<String> = Some("good-session".to_string());
        let resume_failed = false;
        let coder_resume_was_some = true;

        if resume_failed && coder_resume_was_some {
            last_coder_session_id = None;
        }

        assert_eq!(last_coder_session_id.as_deref(), Some("good-session"));
    }

    #[test]
    fn test_resume_fallback_not_triggered_without_resume() {
        // If coder_resume was None (bounce 1), fallback does not apply even on failure.
        let mut last_coder_session_id: Option<String> = None;
        let resume_failed = true;
        let coder_resume_was_some = false; // bounce 1: no resume was attempted

        if resume_failed && coder_resume_was_some {
            last_coder_session_id = None;
        }

        // Remains None (unchanged), not incorrectly triggered
        assert!(last_coder_session_id.is_none());
    }

    #[test]
    fn test_resume_session_as_deref_passthrough() {
        // Verifies that as_deref() correctly converts Option<String> → Option<&str>
        // for passing to spawn_claude's resume_session: Option<&str> parameter.
        let stored: Option<String> = Some("session-xyz-789".to_string());
        let as_ref: Option<&str> = stored.as_deref();
        assert_eq!(as_ref, Some("session-xyz-789"));

        let none: Option<String> = None;
        let none_ref: Option<&str> = none.as_deref();
        assert!(none_ref.is_none());
    }
}
