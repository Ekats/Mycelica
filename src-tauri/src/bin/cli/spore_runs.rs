//! Spore analytics, reporting, and inspection commands.
//!
//! Extracted from spore.rs to keep the orchestration pipeline separate from
//! run inspection, dashboard, health checks, and experiment comparison.

use super::*;
use std::collections::HashMap;
use std::path::PathBuf;

// Re-import shared utilities from the spore module.
use super::spore::{parse_since_to_millis, truncate_middle, is_lesson_quality};

// ============================================================================
// Health check
// ===========================================================================

pub(crate) fn handle_health(db: &Database, json: bool) -> Result<(), String> {
    struct Check {
        name: &'static str,
        ok: bool,
        detail: String,
    }

    let mut checks: Vec<Check> = Vec::new();

    // 1. Database accessible with node count
    match db.count_db_stats() {
        Ok((nodes, edges, items, categories)) => {
            checks.push(Check {
                name: "database",
                ok: true,
                detail: format!("{} nodes, {} edges, {} items, {} categories", nodes, edges, items, categories),
            });
        }
        Err(e) => {
            checks.push(Check {
                name: "database",
                ok: false,
                detail: format!("query failed: {}", e),
            });
        }
    }

    // 2. Stale code nodes (files no longer on disk)
    match db.get_all_code_file_paths() {
        Ok(file_paths) => {
            let project_root = find_project_root();
            let mut stale_nodes: usize = 0;
            let mut stale_files: usize = 0;
            for (file_path, count) in &file_paths {
                let p = std::path::Path::new(file_path);
                let resolved = if p.is_relative() {
                    match &project_root {
                        Some(root) => root.join(p),
                        None => p.to_path_buf(),
                    }
                } else {
                    p.to_path_buf()
                };
                if !resolved.exists() {
                    stale_nodes += count;
                    stale_files += 1;
                }
            }
            checks.push(Check {
                name: "stale_code",
                ok: stale_nodes == 0,
                detail: if stale_nodes == 0 {
                    "no stale code nodes".to_string()
                } else {
                    format!("{} stale code nodes across {} files", stale_nodes, stale_files)
                },
            });
        }
        Err(e) => {
            checks.push(Check {
                name: "stale_code",
                ok: false,
                detail: format!("query failed: {}", e),
            });
        }
    }

    // 3. Orphan edges (edges referencing non-existent nodes)
    match db.count_dead_edges() {
        Ok(count) => {
            checks.push(Check {
                name: "orphan_edges",
                ok: count == 0,
                detail: if count == 0 {
                    "no orphan edges".to_string()
                } else {
                    format!("{} orphan edges (run `maintenance tidy` to clean)", count)
                },
            });
        }
        Err(e) => {
            checks.push(Check {
                name: "orphan_edges",
                ok: false,
                detail: format!("query failed: {}", e),
            });
        }
    }

    // 4. Embedding coverage
    match (db.count_nodes_with_embeddings(), db.get_stats()) {
        (Ok(with_embeddings), Ok(stats)) => {
            let total_items = stats.1;
            let pct = if total_items > 0 {
                (with_embeddings as f64 / total_items as f64 * 100.0) as u32
            } else {
                100
            };
            checks.push(Check {
                name: "embeddings",
                ok: pct >= 90,
                detail: format!("{}/{} nodes have embeddings ({}%)", with_embeddings, total_items, pct),
            });
        }
        (Err(e), _) | (_, Err(e)) => {
            checks.push(Check {
                name: "embeddings",
                ok: false,
                detail: format!("query failed: {}", e),
            });
        }
    }

    // 5. Prompt size (drift detection)
    {
        let prompt_lines: usize = count_agent_prompt_lines()
            .unwrap_or_default()
            .iter()
            .map(|(_, c)| c)
            .sum();
        let threshold = 1000usize;
        if prompt_lines == 0 {
            checks.push(Check {
                name: "prompt_size",
                ok: true,
                detail: "no agent files found (docs/spore/agents/ missing)".to_string(),
            });
        } else if prompt_lines > threshold {
            checks.push(Check {
                name: "prompt_size",
                ok: false,
                detail: format!("{} lines total (threshold: {})", prompt_lines, threshold),
            });
        } else {
            checks.push(Check {
                name: "prompt_size",
                ok: true,
                detail: format!("{} lines total", prompt_lines),
            });
        }
    }

    // Output
    let all_ok = checks.iter().all(|c| c.ok);

    if json {
        let items: Vec<serde_json::Value> = checks.iter().map(|c| {
            serde_json::json!({
                "check": c.name,
                "status": if c.ok { "OK" } else { "WARNING" },
                "detail": c.detail,
            })
        }).collect();
        let output = serde_json::json!({
            "healthy": all_ok,
            "checks": items,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
    } else {
        println!("{:<24} {:<9} {}", "CHECK", "STATUS", "DETAIL");
        println!("{}", "-".repeat(72));
        for c in &checks {
            println!("{:<24} {:<9} {}",
                c.name,
                if c.ok { "OK" } else { "WARNING" },
                c.detail,
            );
        }
        println!("{}", "-".repeat(72));
        if all_ok {
            println!("All checks OK.");
        } else {
            let warnings = checks.iter().filter(|c| !c.ok).count();
            println!("{} warning(s).", warnings);
        }
    }

    Ok(())
}

// ============================================================================
// Prompt stats command
// ============================================================================

/// Walk up from `start` to find the project root (directory containing .mycelica.db).
/// Returns None if no project root is found.
pub(crate) fn find_project_root_from(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".mycelica.db").exists() || dir.join("docs/.mycelica.db").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Walk up from CWD to find the project root.
pub(crate) fn find_project_root() -> Option<PathBuf> {
    find_project_root_from(&std::env::current_dir().ok()?)
}

/// Count lines in agent prompt files under the given start directory.
pub(crate) fn count_agent_prompt_lines_in(start: &Path) -> Result<Vec<(String, usize)>, String> {
    let project_root = match find_project_root_from(start) {
        Some(root) => root,
        None => return Ok(vec![]),
    };
    let agent_dir = project_root.join("docs/spore/agents");
    if !agent_dir.exists() {
        return Ok(vec![]);
    }
    let mut entries: Vec<_> = std::fs::read_dir(agent_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            s.ends_with(".md") && s != "README.md"
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());
    let mut results = vec![];
    for entry in entries {
        let name = entry.file_name().to_string_lossy()
            .trim_end_matches(".md").to_string();
        let content = std::fs::read_to_string(entry.path())
            .map_err(|e| e.to_string())?;
        results.push((name, content.lines().count()));
    }
    Ok(results)
}

/// Count lines in agent prompt files, starting from CWD.
pub(crate) fn count_agent_prompt_lines() -> Result<Vec<(String, usize)>, String> {
    count_agent_prompt_lines_in(&std::env::current_dir().map_err(|e| e.to_string())?)
}

pub(crate) fn handle_prompt_stats() -> Result<(), String> {
    let files = count_agent_prompt_lines()?;
    if files.is_empty() {
        println!("No agent files found in docs/spore/agents/");
        return Ok(());
    }
    let max_name = files.iter().map(|(n, _)| n.len()).max().unwrap_or(5).max(5);
    println!("{:<width$}  LINES", "AGENT", width = max_name);
    let total: usize = files.iter().map(|(_, c)| *c).sum();
    for (name, count) in &files {
        println!("{:<width$}  {}", name, count, width = max_name);
    }
    println!("{}", "-".repeat(max_name + 8));
    println!("{:<width$}  {}", "TOTAL", total, width = max_name);
    Ok(())
}

// ============================================================================
// Lessons
// ============================================================================

pub(crate) fn handle_spore_lessons(db: &Database, json: bool, compact: bool) -> Result<(), String> {
    let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, title, content, created_at FROM nodes \
         WHERE node_class = 'operational' AND title LIKE 'Lesson:%' \
         ORDER BY created_at DESC"
    ).map_err(|e| e.to_string())?;
    let lessons: Vec<(String, String, String, i64)> = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2).unwrap_or_default(),
            row.get::<_, i64>(3)?,
        ))
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();
    drop(stmt);
    drop(conn);

    if json {
        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
        let items: Vec<serde_json::Value> = lessons.iter().map(|(id, title, content, _created_at)| {
            let name = title.strip_prefix("Lesson: ").unwrap_or(title);
            let pattern = content.lines()
                .skip_while(|l| !l.starts_with("## Pattern"))
                .skip(1)
                .take_while(|l| !l.starts_with("## "))
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string();
            let pattern = if pattern.is_empty() {
                if content.chars().count() > 200 {
                    utils::safe_truncate(content, 200).to_string()
                } else {
                    content.clone()
                }
            } else {
                pattern
            };
            let run_count: i64 = conn.prepare(
                "SELECT COUNT(*) FROM edges WHERE (source_id = ?1 OR target_id = ?1) AND type = 'derives_from'"
            ).and_then(|mut s| s.query_row(rusqlite::params![id], |r| r.get(0)))
            .unwrap_or(1);
            serde_json::json!({
                "title": name,
                "pattern": pattern,
                "run_count": run_count,
            })
        }).collect();
        drop(conn);
        println!("{}", serde_json::to_string_pretty(&items).unwrap_or_default());
    } else if lessons.is_empty() {
        println!("No lessons found.");
    } else if compact {
        for (_, title, _, _) in &lessons {
            let name = title.strip_prefix("Lesson: ").unwrap_or(title);
            println!("{}", name);
        }
    } else {
        for (_id, title, content, created_at) in &lessons {
            let name = title.strip_prefix("Lesson: ").unwrap_or(title);
            let date = chrono::DateTime::from_timestamp_millis(*created_at)
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "?".to_string());
            let preview = if content.chars().count() > 100 {
                format!("{}...", utils::safe_truncate(content, 97))
            } else {
                content.clone()
            };
            println!("  {}  {}", date, name);
            if !preview.is_empty() {
                println!("    {}", preview);
            }
            println!();
        }
        println!("{} lesson(s)", lessons.len());
    }
    Ok(())
}

pub(crate) fn handle_dashboard(db: &Database, json: bool, limit: usize, format: DashboardFormat, count: bool, cost: bool, stale: bool) -> Result<(), String> {
    // --format flag takes precedence; fall back to legacy --json flag
    let format = if json { DashboardFormat::Json } else { format };
    let now = chrono::Utc::now().timestamp_millis();
    let week_ago = now - 7 * 86_400_000;

    // Count stale code nodes (files no longer on disk)
    let stale_count: usize = if stale {
        db.get_all_code_file_paths()
            .unwrap_or_default()
            .iter()
            .filter(|(file_path, _)| {
                let p = std::path::Path::new(file_path);
                let resolved = if p.is_relative() {
                    std::path::Path::new(".").join(p)
                } else {
                    p.to_path_buf()
                };
                !resolved.exists()
            })
            .map(|(_, count)| count)
            .sum()
    } else {
        0
    };

    // 1. Recent runs (last 5 orchestrator runs with cost + status)
    let recent_runs: Vec<(String, String, i64, String, Option<f64>)> = if count { Vec::new() } else { (|| -> Result<Vec<_>, String> {
        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id, title, created_at FROM nodes \
             WHERE node_class = 'operational' AND title LIKE 'Orchestration:%' \
             ORDER BY created_at DESC LIMIT ?1"
        ).map_err(|e| e.to_string())?;
        let rows: Vec<(String, String, i64)> = stmt.query_map(rusqlite::params![limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
        drop(stmt);

        let mut result = Vec::new();
        for (task_id, title, created_at) in rows {
            // Status from edges
            let has_impl = conn.prepare(
                "SELECT COUNT(*) FROM edges WHERE target_id = ?1 AND type = 'derives_from'"
            ).and_then(|mut s| s.query_row(rusqlite::params![&task_id], |r| r.get::<_, i64>(0)))
            .unwrap_or(0) > 0;
            let has_verified = if has_impl {
                conn.prepare(
                    "SELECT COUNT(*) FROM edges e1 \
                     JOIN edges e2 ON e2.target_id = e1.source_id \
                     WHERE e1.target_id = ?1 AND e1.type = 'derives_from' AND e2.type = 'supports'"
                ).and_then(|mut s| s.query_row(rusqlite::params![&task_id], |r| r.get::<_, i64>(0)))
                .unwrap_or(0) > 0
            } else { false };
            let status = if has_verified { "verified" } else if has_impl { "implemented" } else { "pending" };

            // Cost from tracks edges
            let mut cost_stmt = conn.prepare(
                "SELECT metadata FROM edges WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks'"
            ).map_err(|e| e.to_string())?;
            let costs: Vec<f64> = cost_stmt.query_map(rusqlite::params![&task_id], |row| {
                row.get::<_, String>(0)
            }).map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .filter_map(|meta| {
                serde_json::from_str::<serde_json::Value>(&meta).ok()
                    .and_then(|v| v["cost_usd"].as_f64())
            })
            .collect();
            let total_cost = if costs.is_empty() { None } else { Some(costs.iter().sum()) };

            let desc = title.strip_prefix("Orchestration:").unwrap_or(&title).trim().to_string();
            result.push((task_id, desc, created_at, status.to_string(), total_cost));
        }
        Ok(result)
    })().unwrap_or_default() };

    // 2. Lessons
    let lessons: Vec<(String, String)> = if count { Vec::new() } else { (|| -> Result<Vec<_>, String> {
        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT title, content FROM nodes \
             WHERE node_class = 'operational' AND title LIKE 'Lesson:%' \
             ORDER BY created_at DESC LIMIT 5"
        ).map_err(|e| e.to_string())?;
        let rows: Vec<(String, String)> = stmt.query_map([], |row| {
            let title: String = row.get(0)?;
            let content: String = row.get::<_, String>(1).unwrap_or_default();
            let pattern = content.lines()
                .skip_while(|l| !l.starts_with("## Pattern"))
                .skip(1)
                .take_while(|l| !l.starts_with("## "))
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string();
            let summary = if pattern.is_empty() {
                title.strip_prefix("Lesson: ").unwrap_or(&title).to_string()
            } else { pattern };
            Ok((title.strip_prefix("Lesson: ").unwrap_or(&title).to_string(), summary))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .filter(|(_, summary)| is_lesson_quality(summary))
        .collect();
        Ok(rows)
    })().unwrap_or_default() };

    // 3. Graph health stats (lightweight — just counts)
    let (node_count, edge_count, edge_7d, unresolved_contradictions): (i64, i64, i64, i64) = (|| -> Result<_, String> {
        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
        let nodes: i64 = conn.prepare("SELECT COUNT(*) FROM nodes")
            .and_then(|mut s| s.query_row([], |r| r.get(0)))
            .unwrap_or(0);
        let edges: i64 = conn.prepare("SELECT COUNT(*) FROM edges")
            .and_then(|mut s| s.query_row([], |r| r.get(0)))
            .unwrap_or(0);
        let recent: i64 = conn.prepare("SELECT COUNT(*) FROM edges WHERE created_at >= ?1")
            .and_then(|mut s| s.query_row(rusqlite::params![week_ago], |r| r.get(0)))
            .unwrap_or(0);
        let contras: i64 = conn.prepare(
            "SELECT COUNT(*) FROM edges WHERE type = 'contradicts' AND superseded_by IS NULL"
        ).and_then(|mut s| s.query_row([], |r| r.get(0)))
        .unwrap_or(0);
        Ok((nodes, edges, recent, contras))
    })().unwrap_or((0, 0, 0, 0));

    // 4. Unresolved contradiction details (if any)
    let contradiction_details: Vec<(String, String, String, String)> = if count { Vec::new() } else if unresolved_contradictions > 0 {
        (|| -> Result<Vec<_>, String> {
            let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
            let mut stmt = conn.prepare(
                "SELECT e.source_id, e.target_id, \
                 COALESCE(s.title, e.source_id), COALESCE(t.title, e.target_id) \
                 FROM edges e \
                 LEFT JOIN nodes s ON s.id = e.source_id \
                 LEFT JOIN nodes t ON t.id = e.target_id \
                 WHERE e.type = 'contradicts' AND e.superseded_by IS NULL \
                 LIMIT 5"
            ).map_err(|e| e.to_string())?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?, row.get::<_, String>(3)?))
            }).map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
            Ok(rows)
        })().unwrap_or_default()
    } else {
        Vec::new()
    };

    // 5. Total cost across all tracked runs
    let all_time_spend: f64 = (|| -> Result<f64, String> {
        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT metadata FROM edges WHERE type = 'tracks' AND metadata IS NOT NULL"
        ).map_err(|e| e.to_string())?;
        let total: f64 = stmt.query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .filter_map(|meta| {
                serde_json::from_str::<serde_json::Value>(&meta).ok()
                    .and_then(|v| v["cost_usd"].as_f64())
            })
            .sum();
        Ok(total)
    })().unwrap_or(0.0);

    // 5b. Today's cost (only when --cost flag is set)
    let (today_spend, today_run_count): (f64, i64) = if cost {
        (|| -> Result<(f64, i64), String> {
            let start_of_today = chrono::Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
            let start_of_today_millis = start_of_today.and_utc().timestamp_millis();
            let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
            let mut stmt = conn.prepare(
                "SELECT COALESCE(SUM(json_extract(e.metadata, '$.cost_usd')), 0.0), \
                        COUNT(DISTINCT n.id) \
                 FROM nodes n \
                 JOIN edges e ON e.source_id = n.id AND e.target_id = n.id AND e.type = 'tracks' \
                 WHERE n.node_class = 'operational' AND n.title LIKE 'Orchestration:%' \
                   AND n.created_at >= ?1 \
                   AND e.metadata IS NOT NULL \
                   AND json_extract(e.metadata, '$.cost_usd') IS NOT NULL"
            ).map_err(|e| e.to_string())?;
            let (total, count): (f64, i64) = stmt.query_row(
                rusqlite::params![start_of_today_millis],
                |row| Ok((row.get(0)?, row.get(1)?))
            ).map_err(|e| e.to_string())?;
            Ok((total, count))
        })().unwrap_or((0.0, 0))
    } else {
        (0.0, 0)
    };

    // 6. Success rate across ALL runs (not just recent)
    let (total_runs, verified_runs, implemented_runs, escalated_runs, cancelled_runs): (i64, i64, i64, i64, i64) = (|| -> Result<_, String> {
        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
        let total: i64 = conn.prepare(
            "SELECT COUNT(*) FROM nodes WHERE node_class = 'operational' AND title LIKE 'Orchestration:%'"
        ).and_then(|mut s| s.query_row([], |r| r.get(0))).unwrap_or(0);

        let verified: i64 = conn.prepare(
            "SELECT COUNT(DISTINCT n.id) FROM nodes n \
             JOIN edges e1 ON e1.target_id = n.id AND e1.type = 'derives_from' \
             JOIN edges e2 ON e2.target_id = e1.source_id AND e2.type = 'supports' \
             WHERE n.node_class = 'operational' AND n.title LIKE 'Orchestration:%'"
        ).and_then(|mut s| s.query_row([], |r| r.get(0))).unwrap_or(0);

        let implemented: i64 = conn.prepare(
            "SELECT COUNT(DISTINCT n.id) FROM nodes n \
             JOIN edges e1 ON e1.target_id = n.id AND e1.type = 'derives_from' \
             WHERE n.node_class = 'operational' AND n.title LIKE 'Orchestration:%' \
             AND n.id NOT IN ( \
               SELECT n2.id FROM nodes n2 \
               JOIN edges e3 ON e3.target_id = n2.id AND e3.type = 'derives_from' \
               JOIN edges e4 ON e4.target_id = e3.source_id AND e4.type = 'supports' \
               WHERE n2.node_class = 'operational' AND n2.title LIKE 'Orchestration:%' \
             )"
        ).and_then(|mut s| s.query_row([], |r| r.get(0))).unwrap_or(0);

        let escalated: i64 = conn.prepare(
            "SELECT COUNT(*) FROM nodes WHERE title LIKE 'ESCALATION:%'"
        ).and_then(|mut s| s.query_row([], |r| r.get(0))).unwrap_or(0);

        let cancelled: i64 = conn.prepare(
            "SELECT COUNT(DISTINCT n.id) FROM nodes n \
             JOIN edges e ON e.source_id = n.id AND e.target_id = n.id AND e.type = 'tracks' \
             AND e.content = 'Cancelled by user' \
             WHERE n.node_class = 'operational' AND n.title LIKE 'Orchestration:%'"
        ).and_then(|mut s| s.query_row([], |r| r.get(0))).unwrap_or(0);

        Ok((total, verified, implemented, escalated, cancelled))
    })().unwrap_or((0, 0, 0, 0, 0));

    match format {
        DashboardFormat::Json => {
            if count {
                let mut output = serde_json::json!({
                    "total_runs": total_runs,
                    "verified": verified_runs,
                    "implemented": implemented_runs,
                    "escalated": escalated_runs,
                    "cancelled": cancelled_runs,
                    "total_spend_usd": all_time_spend,
                });
                if stale && stale_count > 0 {
                    output["stale_nodes"] = serde_json::json!(stale_count);
                }
                if cost {
                    output["today_spend_usd"] = serde_json::json!(today_spend);
                    output["today_run_count"] = serde_json::json!(today_run_count);
                }
                println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
            } else {
                let runs_json: Vec<serde_json::Value> = recent_runs.iter().map(|(id, desc, ts, status, cost)| {
                    let mut v = serde_json::json!({
                        "run_id": &id[..8.min(id.len())],
                        "description": desc,
                        "created_at": chrono::DateTime::from_timestamp_millis(*ts)
                            .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
                            .unwrap_or_default(),
                        "status": status,
                    });
                    if let Some(c) = cost { v["cost_usd"] = serde_json::json!(c); }
                    v
                }).collect();
                let mut output = serde_json::json!({
                    "recent_runs": runs_json,
                    "lessons": lessons.iter().map(|(t, s)| serde_json::json!({"title": t, "summary": s})).collect::<Vec<_>>(),
                    "graph": { "nodes": node_count, "edges": edge_count, "edges_7d": edge_7d, "unresolved_contradictions": unresolved_contradictions },
                    "total_spend_usd": all_time_spend,
                    "success_rate": {
                        "total": total_runs,
                        "verified": verified_runs,
                        "implemented": implemented_runs,
                        "escalated": escalated_runs,
                    },
                });
                if stale && stale_count > 0 {
                    output["graph"]["stale_nodes"] = serde_json::json!(stale_count);
                }
                if cost {
                    output["today_spend_usd"] = serde_json::json!(today_spend);
                    output["today_run_count"] = serde_json::json!(today_run_count);
                }
                println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
            }
        }
        DashboardFormat::Csv => {
            if count {
                if cost {
                    println!("total_runs,verified,implemented,escalated,cancelled,total_spend_usd,today_spend_usd,today_run_count");
                    println!("{},{},{},{},{},{:.2},{:.2},{}", total_runs, verified_runs, implemented_runs, escalated_runs, cancelled_runs, all_time_spend, today_spend, today_run_count);
                } else {
                    println!("total_runs,verified,implemented,escalated,cancelled,total_spend_usd");
                    println!("{},{},{},{},{},{:.2}", total_runs, verified_runs, implemented_runs, escalated_runs, cancelled_runs, all_time_spend);
                }
            } else {
                // Runs section
                println!("section,run_id,description,created_at,status,cost_usd");
                for (id, desc, ts, status, cost) in &recent_runs {
                    let date = chrono::DateTime::from_timestamp_millis(*ts)
                        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
                        .unwrap_or_default();
                    let cost_str = cost.map(|c| format!("{:.4}", c)).unwrap_or_default();
                    // Escape quotes in description for CSV
                    let desc_escaped = desc.replace('"', "\"\"");
                    println!("run,{},\"{}\",{},{},{}", &id[..8.min(id.len())], desc_escaped, date, status, cost_str);
                }
                // Lessons section
                for (title, summary) in &lessons {
                    let title_escaped = title.replace('"', "\"\"");
                    let summary_escaped = summary.replace('"', "\"\"");
                    println!("lesson,,\"{}\",,,\"{}\"", title_escaped, summary_escaped);
                }
                // Graph stats as a single row
                if stale && stale_count > 0 {
                    println!("graph,nodes={},edges={},edges_7d={},contradictions={},stale={}", node_count, edge_count, edge_7d, unresolved_contradictions, stale_count);
                } else {
                    println!("graph,nodes={},edges={},edges_7d={},contradictions={},", node_count, edge_count, edge_7d, unresolved_contradictions);
                }
                // Success rate
                if cost {
                    println!("stats,total={},verified={},implemented={},escalated={},spend={:.2},today={:.2},today_runs={}", total_runs, verified_runs, implemented_runs, escalated_runs, all_time_spend, today_spend, today_run_count);
                } else {
                    println!("stats,total={},verified={},implemented={},escalated={},spend={:.2}", total_runs, verified_runs, implemented_runs, escalated_runs, all_time_spend);
                }
            }
        }
        DashboardFormat::Compact | DashboardFormat::Text => {
            if count {
                let rate = if total_runs > 0 { verified_runs as f64 / total_runs as f64 * 100.0 } else { 0.0 };
                if cost && today_run_count > 0 {
                    println!("Runs: {} total, {} verified ({:.0}%), cost: ${:.2}, today: ${:.2} ({} runs)", total_runs, verified_runs, rate, all_time_spend, today_spend, today_run_count);
                } else {
                    println!("Runs: {} total, {} verified ({:.0}%), cost: ${:.2}", total_runs, verified_runs, rate, all_time_spend);
                }
                if stale && stale_count > 0 {
                    println!("{} stale node(s)", stale_count);
                }
            } else {
                println!("=== Spore Dashboard ===\n");

                // Recent runs
                if recent_runs.is_empty() {
                    println!("No orchestrator runs yet.\n");
                } else {
                    println!("Recent runs:");
                    for (id, desc, ts, status, cost) in &recent_runs {
                        let date = chrono::DateTime::from_timestamp_millis(*ts)
                            .map(|d| d.format("%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "?".to_string());
                        let cost_str = cost.map(|c| format!(" ${:.2}", c)).unwrap_or_default();
                        let desc_short = if desc.chars().count() > 45 {
                            format!("{}...", utils::safe_truncate(desc, 42))
                        } else {
                            desc.clone()
                        };
                        let status_marker = match status.as_str() {
                            "verified" => "+",
                            "implemented" => "~",
                            _ => " ",
                        };
                        println!("  {} {} {:<47} {} {}{}", &id[..8.min(id.len())], status_marker, desc_short, date, status, cost_str);
                    }
                    println!();
                }

                // Lessons
                if !lessons.is_empty() {
                    println!("Lessons learned:");
                    for (title, summary) in &lessons {
                        let summary_short = if summary.chars().count() > 80 {
                            format!("{}...", utils::safe_truncate(summary, 77))
                        } else {
                            summary.clone()
                        };
                        println!("  * {}: {}", title, summary_short);
                    }
                    println!();
                }

                // Graph health
                if stale && stale_count > 0 {
                    println!("Graph: {} nodes, {} edges ({} this week), {} stale node(s)", node_count, edge_count, edge_7d, stale_count);
                } else {
                    println!("Graph: {} nodes, {} edges ({} this week)", node_count, edge_count, edge_7d);
                }
                if !contradiction_details.is_empty() {
                    println!("  {} unresolved contradiction(s):", unresolved_contradictions);
                    for (_src_id, _tgt_id, src_title, tgt_title) in &contradiction_details {
                        let src_short = if src_title.chars().count() > 40 {
                            format!("{}...", utils::safe_truncate(src_title, 37))
                        } else { src_title.clone() };
                        let tgt_short = if tgt_title.chars().count() > 40 {
                            format!("{}...", utils::safe_truncate(tgt_title, 37))
                        } else { tgt_title.clone() };
                        println!("    {} -> {}", src_short, tgt_short);
                    }
                }
                // Success rate
                if total_runs > 0 {
                    let rate = if total_runs > 0 { verified_runs as f64 / total_runs as f64 * 100.0 } else { 0.0 };
                    let pending = total_runs - verified_runs - implemented_runs - cancelled_runs;
                    println!("Runs: {} total, {} verified ({:.0}%), {} implemented, {} escalated, {} cancelled, {} pending",
                        total_runs, verified_runs, rate, implemented_runs, escalated_runs, cancelled_runs, pending);
                }
                if all_time_spend > 0.0 {
                    println!("Total spend: ${:.2}", all_time_spend);
                }
                if cost && today_run_count > 0 {
                    println!("Today: ${:.2} ({} runs)", today_spend, today_run_count);
                }
            }
        }
    }
    Ok(())
}

// ============================================================================
// Runs
// ============================================================================

pub(crate) fn handle_runs(cmd: RunCommands, db: &Database, json: bool) -> Result<(), String> {
    match cmd {
        RunCommands::List { all, cost: show_cost, escalated, since, limit, verbose, format, status: status_filter, duration: min_duration_secs, agent: agent_filter } => {
            let since_millis = since.as_deref().map(parse_since_to_millis).transpose()?;

            // Parse and validate --status filter
            let status_filters: Option<Vec<String>> = if let Some(ref s) = status_filter {
                let valid = ["verified", "implemented", "escalated", "cancelled", "pending", "planned"];
                let filters: Vec<String> = s.split(',').map(|v| v.trim().to_lowercase()).collect();
                for f in &filters {
                    if !valid.contains(&f.as_str()) {
                        return Err(format!(
                            "Invalid status '{}'. Valid values: {}", f, valid.join(", ")
                        ));
                    }
                }
                Some(filters)
            } else {
                None
            };

            // Query task nodes: Orchestration: prefixed, plus (with --all) any operational node with tracks edges
            let task_rows: Vec<(String, String, i64)> = (|| -> Result<Vec<_>, String> {
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                let query = if escalated {
                    // Only runs that have an ESCALATION node tracking them
                    "SELECT DISTINCT n.id, n.title, n.created_at FROM nodes n \
                     WHERE n.node_class = 'operational' AND n.title LIKE 'Orchestration:%' \
                       AND EXISTS ( \
                         SELECT 1 FROM edges e \
                         JOIN nodes esc ON esc.id = e.source_id \
                         WHERE e.target_id = n.id AND e.type = 'tracks' \
                           AND esc.title LIKE 'ESCALATION:%' \
                       ) \
                     ORDER BY n.created_at DESC"
                } else if all {
                    "SELECT DISTINCT n.id, n.title, n.created_at FROM nodes n \
                     WHERE n.node_class = 'operational' AND ( \
                       n.title LIKE 'Orchestration:%' \
                       OR EXISTS (SELECT 1 FROM edges e WHERE e.source_id = n.id AND e.type = 'tracks') \
                     ) \
                     ORDER BY n.created_at DESC"
                } else {
                    "SELECT id, title, created_at FROM nodes \
                     WHERE node_class = 'operational' AND title LIKE 'Orchestration:%' \
                     ORDER BY created_at DESC"
                };
                let mut stmt = conn.prepare(query).map_err(|e| e.to_string())?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
                }).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
                Ok(rows)
            })().map_err(|e| e.to_string())?;

            // Filter by --since if provided
            let task_rows: Vec<(String, String, i64)> = if let Some(since_ms) = since_millis {
                task_rows.into_iter().filter(|(_, _, created_at)| *created_at >= since_ms).collect()
            } else {
                task_rows
            };

            // Apply --limit (0 = no limit)
            let task_rows: Vec<(String, String, i64)> = if limit > 0 {
                task_rows.into_iter().take(limit).collect()
            } else {
                task_rows
            };

            if task_rows.is_empty() {
                if json {
                    println!("[]");
                } else {
                    println!("No {} found.", if escalated { "escalated runs" } else if all { "runs" } else { "orchestrator runs" });
                }
                return Ok(());
            }

            // For each task node, determine status from edges
            struct RunInfo {
                id: String,
                description: String,
                created_at: i64,
                status: String,
                kind: &'static str,
                total_cost: Option<f64>,
                total_duration_ms: Option<u64>,
                escalation_reason: Option<String>,
                agents: Vec<String>,
            }

            let mut runs = Vec::new();
            for (task_id, title, created_at) in &task_rows {
                let description = title.strip_prefix("Orchestration:").unwrap_or(title).trim();
                let description = if verbose {
                    description.to_string()
                } else {
                    truncate_middle(description, 50)
                };
                let is_orchestration = title.starts_with("Orchestration:");

                // Check for derives_from edges where this task is the target
                let impl_ids: Vec<String> = (|| -> Result<Vec<String>, String> {
                    let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                    let mut stmt = conn.prepare(
                        "SELECT source_id FROM edges \
                         WHERE target_id = ?1 AND type = 'derives_from'"
                    ).map_err(|e| e.to_string())?;
                    let ids = stmt.query_map(rusqlite::params![task_id], |row| {
                        row.get::<_, String>(0)
                    }).map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();
                    Ok(ids)
                })().map_err(|e| format!("derives_from query failed: {}", e))?;

                // Check Tracks edge metadata for plan status (takes priority over derives_from)
                let is_planned = (|| -> Result<bool, String> {
                    let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                    let mut stmt = conn.prepare(
                        "SELECT metadata FROM edges \
                         WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks'"
                    ).map_err(|e| e.to_string())?;
                    let metas: Vec<String> = stmt.query_map(rusqlite::params![task_id], |row| {
                        row.get::<_, String>(0)
                    }).map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();
                    for meta in &metas {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(meta) {
                            if let Some(s) = v["status"].as_str() {
                                if s == "plan_complete" || s == "plan_partial" {
                                    return Ok(true);
                                }
                            }
                        }
                    }
                    Ok(false)
                })().unwrap_or(false);

                let status = if is_planned {
                    "planned"
                } else if !impl_ids.is_empty() {
                    // Check if any implementation has a supports edge from a verifier
                    let has_verified = (|| -> Result<bool, String> {
                        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                        let placeholders: Vec<String> = impl_ids.iter().enumerate()
                            .map(|(i, _)| format!("?{}", i + 1))
                            .collect();
                        let query = format!(
                            "SELECT COUNT(*) FROM edges \
                             WHERE target_id IN ({}) AND type = 'supports'",
                            placeholders.join(",")
                        );
                        let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
                        let count: i64 = stmt.query_row(
                            rusqlite::params_from_iter(&impl_ids),
                            |row| row.get(0),
                        ).map_err(|e| e.to_string())?;
                        Ok(count > 0)
                    })().map_err(|e| format!("supports query failed: {}", e))?;
                    if has_verified { "verified" } else { "implemented" }
                } else {
                    // Check for cancellation tracks edge
                    let is_cancelled = (|| -> Result<bool, String> {
                        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                        let mut stmt = conn.prepare(
                            "SELECT COUNT(*) FROM edges \
                             WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks' \
                             AND content = 'Cancelled by user'"
                        ).map_err(|e| e.to_string())?;
                        let count: i64 = stmt.query_row(
                            rusqlite::params![task_id],
                            |row| row.get(0),
                        ).map_err(|e| e.to_string())?;
                        Ok(count > 0)
                    })().map_err(|e| format!("cancelled check failed: {}", e))?;
                    if is_cancelled {
                        "cancelled"
                    } else {
                        // Check for escalation
                        let is_escalated = (|| -> Result<bool, String> {
                            let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                            let mut stmt = conn.prepare(
                                "SELECT COUNT(*) FROM edges e \
                                 JOIN nodes esc ON esc.id = e.source_id \
                                 WHERE e.target_id = ?1 AND e.type = 'tracks' \
                                   AND esc.title LIKE 'ESCALATION:%'"
                            ).map_err(|e| e.to_string())?;
                            let count: i64 = stmt.query_row(rusqlite::params![task_id], |row| row.get(0))
                                .map_err(|e| e.to_string())?;
                            Ok(count > 0)
                        })().unwrap_or(false);
                        if is_escalated { "escalated" } else { "pending" }
                    }
                };

                // Compute cost, duration, and agents from tracks edges metadata
                let (total_cost, total_duration_ms, run_agents): (Option<f64>, Option<u64>, Vec<String>) = (|| -> Result<(Option<f64>, Option<u64>, Vec<String>), String> {
                    let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                    let mut stmt = conn.prepare(
                        "SELECT metadata FROM edges \
                         WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks'"
                    ).map_err(|e| e.to_string())?;
                    let metas: Vec<String> = stmt.query_map(rusqlite::params![task_id], |row| {
                        let meta: String = row.get(0)?;
                        Ok(meta)
                    }).map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();
                    let mut costs = Vec::new();
                    let mut durations = Vec::new();
                    let mut agents = Vec::new();
                    for meta in &metas {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(meta) {
                            if let Some(c) = v["cost_usd"].as_f64() {
                                costs.push(c);
                            }
                            if let Some(d) = v["duration_ms"].as_u64() {
                                durations.push(d);
                            }
                            if let Some(a) = v["agent"].as_str() {
                                agents.push(a.to_string());
                            }
                        }
                    }
                    let cost = if costs.is_empty() { None } else { Some(costs.iter().sum()) };
                    let dur = if durations.is_empty() { None } else { Some(durations.iter().sum()) };
                    Ok((cost, dur, agents))
                })().unwrap_or((None, None, Vec::new()));

                // Look up escalation reason if --escalated flag is set
                let escalation_reason = if escalated {
                    // Get the last verifier contradicts edge content — that's the actual reason
                    (|| -> Result<Option<String>, String> {
                        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                        // Find the last contradicts edge targeting any impl node of this task
                        let mut stmt = conn.prepare(
                            "SELECT e2.content FROM edges e1 \
                             JOIN edges e2 ON e2.target_id = e1.source_id AND e2.type = 'contradicts' \
                             WHERE e1.target_id = ?1 AND e1.type = 'derives_from' \
                             ORDER BY e2.created_at DESC LIMIT 1"
                        ).map_err(|e| e.to_string())?;
                        let reason: Option<String> = stmt.query_row(rusqlite::params![task_id], |row| {
                            row.get::<_, Option<String>>(0)
                        }).ok().flatten();
                        // Fall back to ESCALATION node content if no contradicts edge found
                        if reason.is_some() {
                            return Ok(reason);
                        }
                        let mut stmt2 = conn.prepare(
                            "SELECT esc.content FROM edges e \
                             JOIN nodes esc ON esc.id = e.source_id \
                             WHERE e.target_id = ?1 AND e.type = 'tracks' \
                               AND esc.title LIKE 'ESCALATION:%' \
                             LIMIT 1"
                        ).map_err(|e| e.to_string())?;
                        let content: Option<String> = stmt2.query_row(rusqlite::params![task_id], |row| {
                            row.get::<_, Option<String>>(0)
                        }).ok().flatten();
                        Ok(content.and_then(|c| {
                            // Extract the first meaningful line after "## Escalation"
                            c.lines()
                                .skip_while(|l| l.starts_with('#') || l.trim().is_empty())
                                .next()
                                .map(|l| l.trim().to_string())
                        }))
                    })().unwrap_or(None)
                } else {
                    None
                };

                runs.push(RunInfo {
                    id: if verbose { task_id.clone() } else { task_id[..8.min(task_id.len())].to_string() },
                    description,
                    created_at: *created_at,
                    status: status.to_string(),
                    kind: if is_orchestration { "orchestration" } else { "tracked" },
                    total_cost,
                    total_duration_ms,
                    escalation_reason,
                    agents: run_agents,
                });
            }

            // Apply --status filter
            let runs: Vec<RunInfo> = if let Some(ref filters) = status_filters {
                runs.into_iter().filter(|r| filters.contains(&r.status)).collect()
            } else {
                runs
            };

            // Apply --duration filter (minimum duration in seconds)
            let runs: Vec<RunInfo> = if let Some(min_secs) = min_duration_secs {
                let min_ms = min_secs * 1000;
                runs.into_iter().filter(|r| r.total_duration_ms.unwrap_or(0) >= min_ms).collect()
            } else {
                runs
            };

            // Apply --agent filter: keep runs where at least one tracks edge has a matching agent
            let runs: Vec<RunInfo> = if let Some(ref agent_name) = agent_filter {
                // Normalize: support both "coder" and "spore:coder"
                let needle = if agent_name.contains(':') {
                    agent_name.to_lowercase()
                } else {
                    format!("spore:{}", agent_name.to_lowercase())
                };
                runs.into_iter().filter(|r| {
                    r.agents.iter().any(|a| a.to_lowercase() == needle)
                }).collect()
            } else {
                runs
            };

            // Sort by cost (most expensive first) if --cost flag is set
            let runs: Vec<RunInfo> = if show_cost {
                let mut sorted = runs;
                sorted.sort_by(|a, b| {
                    let cost_a = a.total_cost.unwrap_or(0.0);
                    let cost_b = b.total_cost.unwrap_or(0.0);
                    cost_b.partial_cmp(&cost_a).unwrap_or(std::cmp::Ordering::Equal)
                });
                sorted
            } else {
                runs
            };

            if runs.is_empty() {
                if json {
                    println!("[]");
                } else if status_filters.is_some() {
                    println!("No runs matching status filter found.");
                } else {
                    println!("No {} found.", if escalated { "escalated runs" } else if all { "runs" } else { "orchestrator runs" });
                }
                return Ok(());
            }

            if matches!(format, DashboardFormat::Compact) {
                for r in &runs {
                    let emoji = match r.status.as_str() {
                        "verified" => "+",
                        "cancelled" => "x",
                        "pending" | "implemented" => "~",
                        "planned" => "P",
                        _ => "?",
                    };
                    let title = if verbose { r.description.clone() } else { truncate_middle(&r.description, 40) };
                    let cost_str = r.total_cost
                        .map(|c| format!("${:.2}", c))
                        .unwrap_or_else(|| "-".to_string());
                    let date = chrono::DateTime::from_timestamp_millis(r.created_at)
                        .map(|d| d.format("%Y-%m-%d").to_string())
                        .unwrap_or_else(|| "?".to_string());
                    println!("{} {} {} {} {}", r.id, emoji, title, cost_str, date);
                }
            } else if json {
                let items: Vec<serde_json::Value> = runs.iter().map(|r| {
                    let mut obj = serde_json::json!({
                        "run_id": r.id,
                        "description": r.description,
                        "created_at": chrono::DateTime::from_timestamp_millis(r.created_at)
                            .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
                            .unwrap_or_else(|| "?".to_string()),
                        "status": r.status,
                    });
                    if all {
                        obj["kind"] = serde_json::json!(r.kind);
                    }
                    obj["cost_usd"] = r.total_cost
                        .map(|c| serde_json::json!(format!("{:.2}", c)))
                        .unwrap_or(serde_json::json!(null));
                    obj["duration_ms"] = r.total_duration_ms
                        .map(|d| serde_json::json!(d))
                        .unwrap_or(serde_json::json!(null));
                    if let Some(ref reason) = r.escalation_reason {
                        obj["escalation_reason"] = serde_json::json!(reason);
                    }
                    obj
                }).collect();
                println!("{}", serde_json::to_string_pretty(&items).unwrap_or_default());
            } else if escalated {
                println!("{:<10} {:<52} {:<20} {:<10} {:>8}  {}", "RUN ID", "DESCRIPTION", "CREATED", "STATUS", "COST", "ESCALATION REASON");
                println!("{}", "-".repeat(140));
                for r in &runs {
                    let created = chrono::DateTime::from_timestamp_millis(r.created_at)
                        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let cost_str = r.total_cost
                        .map(|c| format!("${:.2}", c))
                        .unwrap_or_else(|| "-".to_string());
                    let reason = r.escalation_reason.as_deref().unwrap_or("-");
                    let reason = if reason.chars().count() > 40 {
                        format!("{}...", utils::safe_truncate(reason, 37))
                    } else {
                        reason.to_string()
                    };
                    println!("{:<10} {:<52} {:<20} {:<10} {:>8}  {}", r.id, r.description, created, r.status, cost_str, reason);
                }
                println!("\n{} escalated run(s)", runs.len());
            } else if all {
                println!("{:<10} {:<14} {:<52} {:<20} {:<12} {:>8}", "RUN ID", "KIND", "DESCRIPTION", "CREATED", "STATUS", "COST");
                println!("{}", "-".repeat(122));
                for r in &runs {
                    let created = chrono::DateTime::from_timestamp_millis(r.created_at)
                        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let cost_str = r.total_cost
                        .map(|c| format!("${:.2}", c))
                        .unwrap_or_else(|| "-".to_string());
                    println!("{:<10} {:<14} {:<52} {:<20} {:<12} {:>8}", r.id, r.kind, r.description, created, r.status, cost_str);
                }
                println!("\n{} run(s)", runs.len());
            } else {
                println!("{:<10} {:<52} {:<20} {:<12} {:>8}", "RUN ID", "DESCRIPTION", "CREATED", "STATUS", "COST");
                println!("{}", "-".repeat(108));
                for r in &runs {
                    let created = chrono::DateTime::from_timestamp_millis(r.created_at)
                        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let cost_str = r.total_cost
                        .map(|c| format!("${:.2}", c))
                        .unwrap_or_else(|| "-".to_string());
                    println!("{:<10} {:<52} {:<20} {:<12} {:>8}", r.id, r.description, created, r.status, cost_str);
                }
                let total: f64 = runs.iter().filter_map(|r| r.total_cost).sum();
                if show_cost || total > 0.0 {
                    println!("\n{} run(s), total cost: ${:.2}", runs.len(), total);
                } else {
                    println!("\n{} run(s)", runs.len());
                }
            }
            Ok(())
        }

        RunCommands::Get { run_id, json: local_json } => {
            let edges = db.get_run_edges(&run_id)
                .map_err(|e| format!("Failed to get run edges: {}", e))?;

            if json || local_json {
                println!("{}", serde_json::to_string_pretty(&edges).unwrap_or_default());
            } else if edges.is_empty() {
                println!("No edges found for run: {}", run_id);
            } else {
                println!("Run: {}", run_id);
                println!("{} edge(s):\n", edges.len());
                for e in &edges {
                    let created = chrono::DateTime::from_timestamp_millis(e.created_at)
                        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "?".to_string());
                    println!("  {} {:?} {} -> {} [{}]",
                        &e.id[..8.min(e.id.len())],
                        e.edge_type,
                        &e.source[..8.min(e.source.len())],
                        &e.target[..8.min(e.target.len())],
                        created,
                    );
                    if let Some(ref reason) = e.reason {
                        println!("    reason: {}", reason);
                    }
                }
            }
            Ok(())
        }

        RunCommands::Compare { run_a, run_b } => {
            // Resolve both run task nodes
            let node_a = resolve_node(db, &run_a)?;
            let node_b = resolve_node(db, &run_b)?;

            // Gather run metrics from tracks edge metadata
            struct RunMetrics {
                id: String,
                status: String,
                total_cost: f64,
                total_turns: u64,
                bounces: u64,
                num_agents: usize,
                duration_secs: u64,
            }

            let gather = |task_id: &str| -> Result<RunMetrics, String> {
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;

                // Get tracks edges (self-referencing: source_id = target_id = task_id)
                let mut stmt = conn.prepare(
                    "SELECT metadata, created_at FROM edges \
                     WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks' \
                     ORDER BY created_at"
                ).map_err(|e| e.to_string())?;
                let phases: Vec<(serde_json::Value, i64)> = stmt.query_map(
                    rusqlite::params![task_id], |row| {
                        let meta: String = row.get(0)?;
                        let ts: i64 = row.get(1)?;
                        Ok((meta, ts))
                    }
                ).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .filter_map(|(meta, ts)| serde_json::from_str::<serde_json::Value>(&meta).ok().map(|v| (v, ts)))
                .collect();
                drop(stmt);

                let mut total_cost = 0.0_f64;
                let mut total_turns = 0_u64;
                let mut max_bounce = 0_u64;
                let mut agents = std::collections::HashSet::new();
                let mut total_duration_ms = 0_u64;

                for (meta, _) in &phases {
                    if let Some(c) = meta["cost_usd"].as_f64() { total_cost += c; }
                    if let Some(t) = meta["turns"].as_u64().or_else(|| meta["num_turns"].as_u64()) {
                        total_turns += t;
                    }
                    if let Some(b) = meta["bounce"].as_u64() {
                        if b > max_bounce { max_bounce = b; }
                    }
                    if let Some(a) = meta["agent"].as_str() { agents.insert(a.to_string()); }
                    if let Some(d) = meta["duration_ms"].as_u64() { total_duration_ms += d; }
                }

                // Wall-clock duration: span from first to last phase + last phase duration
                let duration_secs = if phases.len() >= 2 {
                    let first_ts = phases.first().map(|(_, ts)| *ts).unwrap_or(0);
                    let last_ts = phases.last().map(|(_, ts)| *ts).unwrap_or(0);
                    let last_dur = phases.last()
                        .and_then(|(m, _)| m["duration_ms"].as_u64())
                        .unwrap_or(0);
                    ((last_ts - first_ts) as u64 + last_dur) / 1000
                } else {
                    total_duration_ms / 1000
                };

                // Check verification status
                let has_verified: bool = conn.prepare(
                    "SELECT COUNT(*) FROM edges e1 \
                     JOIN edges e2 ON e2.target_id = e1.source_id \
                     WHERE e1.target_id = ?1 AND e1.type = 'derives_from' AND e2.type = 'supports'"
                ).and_then(|mut s| s.query_row(rusqlite::params![task_id], |r| r.get::<_, i64>(0)))
                .unwrap_or(0) > 0;

                let has_impl: bool = conn.prepare(
                    "SELECT COUNT(*) FROM edges WHERE target_id = ?1 AND type = 'derives_from'"
                ).and_then(|mut s| s.query_row(rusqlite::params![task_id], |r| r.get::<_, i64>(0)))
                .unwrap_or(0) > 0;

                let status = if has_verified { "verified" } else if has_impl { "implemented" } else { "pending" };

                Ok(RunMetrics {
                    id: task_id[..8.min(task_id.len())].to_string(),
                    status: status.to_string(),
                    total_cost,
                    total_turns,
                    bounces: max_bounce,
                    num_agents: agents.len(),
                    duration_secs,
                })
            };

            let a = gather(&node_a.id)?;
            let b = gather(&node_b.id)?;

            let fmt_duration = |secs: u64| -> String {
                if secs >= 60 { format!("{}m {}s", secs / 60, secs % 60) } else { format!("{}s", secs) }
            };

            if json {
                let to_json = |r: &RunMetrics| -> serde_json::Value {
                    serde_json::json!({
                        "run_id": r.id,
                        "status": r.status,
                        "total_cost": r.total_cost,
                        "total_turns": r.total_turns,
                        "bounces": r.bounces,
                        "num_agents": r.num_agents,
                        "duration_secs": r.duration_secs,
                    })
                };
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "run_a": to_json(&a), "run_b": to_json(&b),
                })).unwrap_or_default());
            } else {
                // Table: Metric | Run 1 | Run 2
                let col0 = 14;
                let col1 = 20;
                let col2 = 20;
                let width = col0 + 3 + col1 + 3 + col2;

                println!("{:<col0$} | {:<col1$} | {:<col2$}",
                    "Metric",
                    format!("Run 1 ({})", a.id),
                    format!("Run 2 ({})", b.id),
                    col0 = col0, col1 = col1, col2 = col2);
                println!("{}", "-".repeat(width));
                println!("{:<col0$} | {:<col1$} | {:<col2$}",
                    "Status", a.status, b.status,
                    col0 = col0, col1 = col1, col2 = col2);
                println!("{:<col0$} | {:<col1$} | {:<col2$}",
                    "Total Cost", format!("${:.2}", a.total_cost), format!("${:.2}", b.total_cost),
                    col0 = col0, col1 = col1, col2 = col2);
                println!("{:<col0$} | {:<col1$} | {:<col2$}",
                    "Total Turns", a.total_turns.to_string(), b.total_turns.to_string(),
                    col0 = col0, col1 = col1, col2 = col2);
                println!("{:<col0$} | {:<col1$} | {:<col2$}",
                    "Bounces", a.bounces.to_string(), b.bounces.to_string(),
                    col0 = col0, col1 = col1, col2 = col2);
                println!("{:<col0$} | {:<col1$} | {:<col2$}",
                    "Agents", a.num_agents.to_string(), b.num_agents.to_string(),
                    col0 = col0, col1 = col1, col2 = col2);
                println!("{:<col0$} | {:<col1$} | {:<col2$}",
                    "Duration", fmt_duration(a.duration_secs), fmt_duration(b.duration_secs),
                    col0 = col0, col1 = col1, col2 = col2);
            }
            Ok(())
        }

        RunCommands::CompareExperiments { experiment } => {
            handle_runs_compare_experiments(db, &experiment[0], &experiment[1])
        }

        RunCommands::Show { run_id } |
        RunCommands::History { run_id } => {
            let task_node = resolve_node(db, &run_id)?;
            let task_id = &task_node.id;
            let task_desc = task_node.title.strip_prefix("Orchestration:").unwrap_or(&task_node.title).trim();
            let task_created = chrono::DateTime::from_timestamp_millis(task_node.created_at)
                .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "?".to_string());

            if json {
                // Collect all timeline events as JSON
                let mut events: Vec<serde_json::Value> = Vec::new();

                // Task creation event
                events.push(serde_json::json!({
                    "timestamp": task_node.created_at,
                    "time": &task_created,
                    "type": "task_created",
                    "description": task_desc,
                    "node_id": &task_id[..8.min(task_id.len())],
                }));

                // Get tracks edges (self-referencing: source=target=task_id for agent metadata)
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                let mut stmt = conn.prepare(
                    "SELECT metadata, created_at FROM edges WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks' ORDER BY created_at"
                ).map_err(|e| e.to_string())?;
                let agent_runs: Vec<(serde_json::Value, i64)> = stmt.query_map(rusqlite::params![task_id], |row| {
                    let meta: String = row.get(0)?;
                    let ts: i64 = row.get(1)?;
                    Ok((meta, ts))
                }).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .filter_map(|(meta, ts)| serde_json::from_str::<serde_json::Value>(&meta).ok().map(|v| (v, ts)))
                .collect();
                drop(stmt);

                for (meta, ts) in &agent_runs {
                    let time_str = chrono::DateTime::from_timestamp_millis(*ts)
                        .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                        .unwrap_or_else(|| "?".to_string());
                    events.push(serde_json::json!({
                        "timestamp": ts,
                        "time": time_str,
                        "type": "agent_run",
                        "agent": meta["agent"],
                        "status": meta["status"],
                        "cost_usd": meta["cost_usd"],
                        "turns": meta["turns"],
                        "duration_ms": meta["duration_ms"],
                        "bounce": meta["bounce"],
                    }));
                }

                // Get workflow edges by walking the task node's neighborhood
                drop(conn);
                let run_edges: Vec<Edge> = {
                    let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                    let mut all_edges = Vec::new();
                    let mut stmt = conn.prepare(
                        "SELECT source_id, target_id, type, created_at, confidence, content, agent_id, superseded_by \
                         FROM edges WHERE target_id = ?1 AND type = 'derives_from'"
                    ).map_err(|e| e.to_string())?;
                    let df_edges: Vec<(String, i64)> = stmt.query_map(rusqlite::params![task_id], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(3)?))
                    }).map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();
                    drop(stmt);
                    let impl_ids: Vec<String> = df_edges.iter().map(|(id, _)| id.clone()).collect();
                    for impl_id in &impl_ids {
                        let mut stmt2 = conn.prepare(
                            "SELECT source_id, target_id, type, created_at, confidence, content, agent_id, superseded_by \
                             FROM edges WHERE (source_id = ?1 OR target_id = ?1) AND type IN ('supports', 'contradicts', 'supersedes', 'flags')"
                        ).map_err(|e| e.to_string())?;
                        let edges: Vec<Edge> = stmt2.query_map(rusqlite::params![impl_id], |row| {
                            Ok(Edge {
                                id: String::new(), source: row.get(0)?, target: row.get(1)?,
                                edge_type: EdgeType::from_str(&row.get::<_, String>(2)?).unwrap_or(EdgeType::Related),
                                label: None, weight: None, edge_source: None, evidence_id: None,
                                confidence: row.get(4)?, created_at: row.get(3)?,
                                updated_at: None, author: None, reason: None,
                                content: row.get(5)?, agent_id: row.get(6)?,
                                superseded_by: row.get(7)?, metadata: None,
                            })
                        }).map_err(|e| e.to_string())?
                        .filter_map(|r| r.ok())
                        .collect();
                        all_edges.extend(edges);
                    }
                    for (src, ts) in &df_edges {
                        all_edges.push(Edge {
                            id: String::new(), source: src.clone(), target: task_id.clone(),
                            edge_type: EdgeType::DerivesFrom, label: None, weight: None,
                            edge_source: None, evidence_id: None, confidence: Some(1.0),
                            created_at: *ts, updated_at: None, author: None, reason: None,
                            content: None, agent_id: Some("spore:orchestrator".to_string()),
                            superseded_by: None, metadata: None,
                        });
                    }
                    drop(conn);
                    let mut seen = std::collections::HashSet::new();
                    all_edges.retain(|e| seen.insert((e.source.clone(), e.target.clone(), e.edge_type.as_str().to_string())));
                    all_edges
                };

                for e in &run_edges {
                    let time_str = chrono::DateTime::from_timestamp_millis(e.created_at)
                        .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let source_title = db.get_node(&e.source).ok().flatten()
                        .map(|n| n.title.clone()).unwrap_or_else(|| e.source[..8.min(e.source.len())].to_string());
                    let target_title = db.get_node(&e.target).ok().flatten()
                        .map(|n| n.title.clone()).unwrap_or_else(|| e.target[..8.min(e.target.len())].to_string());
                    events.push(serde_json::json!({
                        "timestamp": e.created_at,
                        "time": time_str,
                        "type": "edge",
                        "edge_type": e.edge_type.as_str(),
                        "edge_id": &e.id[..8.min(e.id.len())],
                        "source": &e.source[..8.min(e.source.len())],
                        "source_title": source_title,
                        "target": &e.target[..8.min(e.target.len())],
                        "target_title": target_title,
                        "confidence": e.confidence,
                        "content": e.content,
                        "agent_id": e.agent_id,
                    }));
                }

                // Sort by timestamp
                events.sort_by_key(|e| e["timestamp"].as_i64().unwrap_or(0));

                // Determine outcome for JSON
                let has_supports = run_edges.iter().any(|e| e.edge_type == EdgeType::Supports);
                let has_contradicts = run_edges.iter().any(|e| e.edge_type == EdgeType::Contradicts && e.superseded_by.is_none());
                let has_escalation = (|| -> Result<bool, String> {
                    let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                    let mut stmt = conn.prepare(
                        "SELECT 1 FROM edges e \
                         JOIN nodes esc ON esc.id = e.source_id \
                         WHERE e.target_id = ?1 AND e.type = 'tracks' \
                           AND esc.title LIKE 'ESCALATION:%' \
                         LIMIT 1"
                    ).map_err(|e| e.to_string())?;
                    Ok(stmt.query_row(rusqlite::params![task_id], |_| Ok(())).is_ok())
                })().unwrap_or(false);
                let outcome = if has_escalation {
                    "ESCALATED"
                } else if has_supports && !has_contradicts {
                    "VERIFIED"
                } else if has_contradicts {
                    "UNRESOLVED"
                } else if run_edges.iter().any(|e| e.edge_type == EdgeType::DerivesFrom) {
                    "IMPLEMENTED"
                } else {
                    "PENDING"
                };

                let output = serde_json::json!({
                    "run_id": &task_id[..8.min(task_id.len())],
                    "description": task_desc,
                    "created": task_created,
                    "outcome": outcome,
                    "events": events,
                });
                println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
            } else {
                // Human-readable chronological timeline
                println!("Run: {} ({})", &task_id[..8.min(task_id.len())], task_desc);
                println!("{}", "=".repeat(80));
                println!();

                // Collect all timeline events: (timestamp, indent_level, lines)
                struct TimelineEvent {
                    timestamp: i64,
                    lines: Vec<(usize, String)>, // (indent_level, text)
                }
                let mut events: Vec<TimelineEvent> = Vec::new();

                // Task creation
                events.push(TimelineEvent {
                    timestamp: task_node.created_at,
                    lines: vec![
                        (0, format!("[{}] Task created", task_created)),
                        (1, format!("\"{}\"", task_desc)),
                    ],
                });

                // Get tracks edges for agent run metadata
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                let mut stmt = conn.prepare(
                    "SELECT metadata, created_at FROM edges WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks' ORDER BY created_at"
                ).map_err(|e| e.to_string())?;
                let agent_runs: Vec<(serde_json::Value, i64)> = stmt.query_map(rusqlite::params![task_id], |row| {
                    let meta: String = row.get(0)?;
                    let ts: i64 = row.get(1)?;
                    Ok((meta, ts))
                }).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .filter_map(|(meta, ts)| serde_json::from_str::<serde_json::Value>(&meta).ok().map(|v| (v, ts)))
                .collect();
                drop(stmt);

                for (meta, ts) in &agent_runs {
                    let time_str = chrono::DateTime::from_timestamp_millis(*ts)
                        .map(|d| d.format("%H:%M:%S").to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let agent = meta["agent"].as_str().unwrap_or("?");
                    let short_agent = agent.strip_prefix("spore:").unwrap_or(agent);
                    let status = meta["status"].as_str().unwrap_or("?");
                    let bounce = meta["bounce"].as_u64();
                    let turns = meta["turns"].as_u64();
                    let duration_ms = meta["duration_ms"].as_u64();
                    let cost = meta["cost_usd"].as_f64();

                    let mut lines = Vec::new();
                    let bounce_str = bounce.map(|b| format!(" (bounce {})", b)).unwrap_or_default();
                    lines.push((0, format!("[{}] {} {}{}", time_str, short_agent, status, bounce_str)));

                    let mut details = Vec::new();
                    if let Some(t) = turns { details.push(format!("{} turns", t)); }
                    if let Some(d) = duration_ms {
                        let secs = d / 1000;
                        if secs >= 60 {
                            details.push(format!("{}m {}s", secs / 60, secs % 60));
                        } else {
                            details.push(format!("{}s", secs));
                        }
                    }
                    if let Some(c) = cost { details.push(format!("${:.2}", c)); }
                    if !details.is_empty() {
                        lines.push((1, details.join(" | ")));
                    }

                    events.push(TimelineEvent { timestamp: *ts, lines });
                }

                // Get workflow edges by walking the task node's neighborhood.
                // get_run_edges() only finds edges with run_id in metadata,
                // but derives_from/supports/contradicts don't have that.
                drop(conn);
                let run_edges: Vec<Edge> = {
                    let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                    // Find impl nodes (derives_from → task), then verification edges on those
                    let mut all_edges = Vec::new();
                    // 1. derives_from edges pointing to task node
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {} FROM edges WHERE target_id = ?1 AND type = 'derives_from'",
                        "id, source_id, target_id, type, label, weight, edge_source, evidence_id, confidence, created_at, updated_at, author, reason, content, agent_id, superseded_by, metadata"
                    )).map_err(|e| e.to_string())?;
                    let df_edges: Vec<(String, i64)> = stmt.query_map(rusqlite::params![task_id], |row| {
                        Ok((row.get::<_, String>(1)?, row.get::<_, i64>(9)?)) // source_id, created_at
                    }).map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();
                    drop(stmt);

                    // Collect impl node IDs
                    let impl_ids: Vec<String> = df_edges.iter().map(|(id, _)| id.clone()).collect();

                    // 2. For each impl node, get supports/contradicts/supersedes edges
                    for impl_id in &impl_ids {
                        let mut stmt2 = conn.prepare(
                            "SELECT source_id, target_id, type, created_at, confidence, content, agent_id, superseded_by \
                             FROM edges WHERE (source_id = ?1 OR target_id = ?1) AND type IN ('supports', 'contradicts', 'supersedes', 'flags')"
                        ).map_err(|e| e.to_string())?;
                        let edges: Vec<Edge> = stmt2.query_map(rusqlite::params![impl_id], |row| {
                            Ok(Edge {
                                id: String::new(),
                                source: row.get::<_, String>(0)?,
                                target: row.get::<_, String>(1)?,
                                edge_type: EdgeType::from_str(&row.get::<_, String>(2)?).unwrap_or(EdgeType::Related),
                                label: None, weight: None,
                                edge_source: None, evidence_id: None,
                                confidence: row.get(4)?,
                                created_at: row.get(3)?,
                                updated_at: None, author: None, reason: None,
                                content: row.get(5)?,
                                agent_id: row.get(6)?,
                                superseded_by: row.get(7)?,
                                metadata: None,
                            })
                        }).map_err(|e| e.to_string())?
                        .filter_map(|r| r.ok())
                        .collect();
                        all_edges.extend(edges);
                    }

                    // Also add derives_from edges as Edge objects
                    for (src, ts) in &df_edges {
                        all_edges.push(Edge {
                            id: String::new(),
                            source: src.clone(),
                            target: task_id.clone(),
                            edge_type: EdgeType::DerivesFrom,
                            label: None, weight: None,
                            edge_source: None, evidence_id: None,
                            confidence: Some(1.0),
                            created_at: *ts,
                            updated_at: None, author: None, reason: None,
                            content: None,
                            agent_id: Some("spore:orchestrator".to_string()),
                            superseded_by: None,
                            metadata: None,
                        });
                    }
                    drop(conn);
                    // Deduplicate edges by (source, target, type) — an edge between two impl nodes
                    // gets picked up once for each impl, so we need to remove duplicates
                    let mut seen = std::collections::HashSet::new();
                    all_edges.retain(|e| seen.insert((e.source.clone(), e.target.clone(), e.edge_type.as_str().to_string())));
                    all_edges
                };

                // Workflow edges: derives_from, supports, contradicts, supersedes
                let workflow_types = [
                    EdgeType::DerivesFrom, EdgeType::Supports, EdgeType::Contradicts,
                    EdgeType::Supersedes, EdgeType::Flags, EdgeType::Resolves, EdgeType::Questions,
                ];
                for e in &run_edges {
                    // Skip self-referencing tracks edges (already shown as agent runs)
                    if e.edge_type == EdgeType::Tracks && e.source == *task_id && e.target == *task_id {
                        continue;
                    }
                    if !workflow_types.contains(&e.edge_type) && e.edge_type != EdgeType::Tracks {
                        continue;
                    }
                    let time_str = chrono::DateTime::from_timestamp_millis(e.created_at)
                        .map(|d| d.format("%H:%M:%S").to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let source_title = db.get_node(&e.source).ok().flatten()
                        .map(|n| {
                            let t = n.title.clone();
                            if t.chars().count() > 50 { format!("{}...", utils::safe_truncate(&t, 47)) } else { t }
                        })
                        .unwrap_or_else(|| e.source[..8.min(e.source.len())].to_string());
                    let target_title = db.get_node(&e.target).ok().flatten()
                        .map(|n| {
                            let t = n.title.clone();
                            if t.chars().count() > 50 { format!("{}...", utils::safe_truncate(&t, 47)) } else { t }
                        })
                        .unwrap_or_else(|| e.target[..8.min(e.target.len())].to_string());

                    let edge_label = match e.edge_type {
                        EdgeType::DerivesFrom => "derives_from",
                        EdgeType::Supports => "SUPPORTS",
                        EdgeType::Contradicts => "CONTRADICTS",
                        EdgeType::Supersedes => "supersedes",
                        EdgeType::Flags => "flags",
                        EdgeType::Resolves => "resolves",
                        EdgeType::Questions => "questions",
                        EdgeType::Tracks => "tracks",
                        _ => e.edge_type.as_str(),
                    };

                    let agent_str = e.agent_id.as_deref()
                        .map(|a| a.strip_prefix("spore:").unwrap_or(a))
                        .unwrap_or("?");
                    let conf_str = e.confidence
                        .map(|c| format!(" [{:.0}%]", c * 100.0))
                        .unwrap_or_default();

                    let mut lines = Vec::new();
                    lines.push((0, format!("[{}] {} {} -> {}{}", time_str, edge_label, source_title, target_title, conf_str)));
                    lines.push((1, format!("by {} ({})", agent_str, &e.id[..8.min(e.id.len())])));
                    if let Some(ref content) = e.content {
                        let short = if content.chars().count() > 70 {
                            format!("{}...", utils::safe_truncate(content, 67))
                        } else {
                            content.clone()
                        };
                        lines.push((1, format!("\"{}\"", short)));
                    }

                    events.push(TimelineEvent { timestamp: e.created_at, lines });
                }

                // Sort all events by timestamp
                events.sort_by_key(|e| e.timestamp);

                // Render
                for event in &events {
                    for (indent, line) in &event.lines {
                        let prefix = "  ".repeat(*indent);
                        println!("{}{}", prefix, line);
                    }
                    println!();
                }

                // Final outcome summary
                let has_supports = run_edges.iter().any(|e| e.edge_type == EdgeType::Supports);
                let has_contradicts = run_edges.iter().any(|e| e.edge_type == EdgeType::Contradicts && e.superseded_by.is_none());
                let has_escalation = (|| -> Result<bool, String> {
                    let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                    let mut stmt = conn.prepare(
                        "SELECT 1 FROM edges e \
                         JOIN nodes esc ON esc.id = e.source_id \
                         WHERE e.target_id = ?1 AND e.type = 'tracks' \
                           AND esc.title LIKE 'ESCALATION:%' \
                         LIMIT 1"
                    ).map_err(|e| e.to_string())?;
                    Ok(stmt.query_row(rusqlite::params![task_id], |_| Ok(())).is_ok())
                })().unwrap_or(false);

                println!("{}", "-".repeat(80));
                let outcome = if has_escalation {
                    "ESCALATED"
                } else if has_supports && !has_contradicts {
                    "VERIFIED"
                } else if has_contradicts {
                    "UNRESOLVED (open contradictions)"
                } else if run_edges.iter().any(|e| e.edge_type == EdgeType::DerivesFrom) {
                    "IMPLEMENTED (not yet verified)"
                } else {
                    "PENDING"
                };

                let total_edges = run_edges.len();
                let total_cost: f64 = agent_runs.iter()
                    .filter_map(|(m, _)| m["cost_usd"].as_f64())
                    .sum();
                // Duration = span from task creation to last tracks edge
                let last_ts = agent_runs.iter().map(|(_, ts)| *ts).max().unwrap_or(task_node.created_at);
                let total_secs = ((last_ts - task_node.created_at) / 1000) as u64;

                println!("Outcome: {}", outcome);
                println!("Agents: {} invocations | Edges: {} | Cost: ${:.2} | Duration: {}m {}s",
                    agent_runs.len(), total_edges, total_cost, total_secs / 60, total_secs % 60);
            }
            Ok(())
        }

        RunCommands::Diff { run_id } => {
            let task_node = resolve_node(db, &run_id)?;
            let task_id = &task_node.id;

            // Walk derives_from edges to find implementation nodes
            let files_changed: Vec<String> = {
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                let mut stmt = conn.prepare(
                    "SELECT n.content FROM edges e \
                     JOIN nodes n ON n.id = e.source_id \
                     WHERE e.target_id = ?1 AND e.type = 'derives_from' AND n.content IS NOT NULL"
                ).map_err(|e| e.to_string())?;
                let contents: Vec<String> = stmt.query_map(
                    rusqlite::params![task_id], |row| row.get::<_, String>(0)
                ).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
                drop(stmt);
                drop(conn);

                let mut files = Vec::new();
                for content in &contents {
                    // Parse "## Files Changed" section from markdown content
                    if let Some(start) = content.find("## Files Changed") {
                        let section = &content[start..];
                        for line in section.lines().skip(1) {
                            if line.starts_with("## ") { break; }
                            let trimmed = line.trim();
                            if let Some(file) = trimmed.strip_prefix("- ") {
                                let file = file.split(':').next().unwrap_or(file).trim();
                                let file = file.trim_matches('`');
                                if !file.is_empty() && (file.contains('/') || file.contains('.')) && !files.contains(&file.to_string()) {
                                    files.push(file.to_string());
                                }
                            }
                        }
                    }
                }
                files
            };

            if files_changed.is_empty() {
                if json {
                    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                        "run_id": &task_id[..8.min(task_id.len())],
                        "files_changed": serde_json::Value::Array(vec![]),
                    })).unwrap_or_default());
                } else {
                    println!("No file changes recorded.");
                }
            } else if json {
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "run_id": &task_id[..8.min(task_id.len())],
                    "files_changed": files_changed,
                })).unwrap_or_default());
            } else {
                for f in &files_changed {
                    println!("{}", f);
                }
            }
            Ok(())
        }

        RunCommands::Rollback { run_id, delete_nodes, force, dry_run } => {
            if dry_run {
                let (edges, nodes) = db.preview_rollback_run(&run_id, delete_nodes)
                    .map_err(|e| format!("Failed to preview rollback: {}", e))?;

                if json {
                    let obj = serde_json::json!({
                        "runId": run_id,
                        "dryRun": true,
                        "edgesCount": edges.len(),
                        "nodesCount": nodes.len(),
                        "edges": edges,
                        "nodes": nodes.iter().map(|n| serde_json::json!({
                            "id": n.id,
                            "title": n.title,
                            "nodeClass": n.node_class,
                            "agentId": n.agent_id,
                        })).collect::<Vec<_>>(),
                    });
                    println!("{}", serde_json::to_string_pretty(&obj).unwrap_or_default());
                } else if edges.is_empty() {
                    println!("No edges found for run: {}", run_id);
                } else {
                    println!("[dry-run] Run: {}", run_id);
                    println!("\nEdges that would be deleted ({}):", edges.len());
                    for e in &edges {
                        let created = chrono::DateTime::from_timestamp_millis(e.created_at)
                            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or_else(|| "?".to_string());
                        println!("  {} {:?} {} -> {} [{}]",
                            &e.id[..8.min(e.id.len())],
                            e.edge_type,
                            &e.source[..8.min(e.source.len())],
                            &e.target[..8.min(e.target.len())],
                            created,
                        );
                        if let Some(ref reason) = e.reason {
                            println!("    reason: {}", reason);
                        }
                    }
                    if delete_nodes {
                        if nodes.is_empty() {
                            println!("\nNo operational nodes would be deleted.");
                        } else {
                            println!("\nNodes that would be deleted ({}):", nodes.len());
                            for n in &nodes {
                                println!("  {} {}",
                                    &n.id[..8.min(n.id.len())],
                                    n.title,
                                );
                                if let Some(ref agent) = n.agent_id {
                                    println!("    agent: {}", agent);
                                }
                            }
                        }
                    }
                    println!("\nNo changes made. Remove --dry-run and use --force to execute.");
                }
                return Ok(());
            }

            // Show what will be deleted first
            let edges = db.get_run_edges(&run_id)
                .map_err(|e| format!("Failed to get run edges: {}", e))?;
            if edges.is_empty() {
                println!("No edges found for run: {}", run_id);
                return Ok(());
            }

            if !force {
                println!("Will delete {} edge(s) from run {}", edges.len(), run_id);
                if delete_nodes {
                    println!("Will also delete operational nodes created during this run.");
                }
                println!("Use --force to confirm, or --dry-run to preview details.");
                return Ok(());
            }

            let (edges_deleted, nodes_deleted) = db.rollback_run(&run_id, delete_nodes)
                .map_err(|e| format!("Rollback failed: {}", e))?;

            if json {
                println!(r#"{{"runId":"{}","edgesDeleted":{},"nodesDeleted":{}}}"#,
                    run_id, edges_deleted, nodes_deleted);
            } else {
                println!("Rolled back run {}: {} edge(s) deleted, {} node(s) deleted",
                    run_id, edges_deleted, nodes_deleted);
            }
            Ok(())
        }

        RunCommands::Cancel { run_id, json: local_json } => {
            let node = resolve_node(db, &run_id)?;

            // Verify it's an operational/task node
            if node.node_class.as_deref() != Some("operational") {
                return Err(format!("Node {} is not an operational node", &node.id[..8.min(node.id.len())]));
            }

            // Check for derives_from edges (would mean it's already implemented)
            let has_impl = (|| -> Result<bool, String> {
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                let mut stmt = conn.prepare(
                    "SELECT COUNT(*) FROM edges \
                     WHERE target_id = ?1 AND type = 'derives_from'"
                ).map_err(|e| e.to_string())?;
                let count: i64 = stmt.query_row(
                    rusqlite::params![node.id],
                    |row| row.get(0),
                ).map_err(|e| e.to_string())?;
                Ok(count > 0)
            })().map_err(|e| format!("Failed to check status: {}", e))?;

            if has_impl {
                return Err(format!("Run {} is not pending (has derives_from edges — already implemented)", &node.id[..8.min(node.id.len())]));
            }

            // Check if already cancelled (has a tracks edge with content 'Cancelled by user')
            let already_cancelled = (|| -> Result<bool, String> {
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                let mut stmt = conn.prepare(
                    "SELECT COUNT(*) FROM edges \
                     WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks' \
                     AND content = 'Cancelled by user'"
                ).map_err(|e| e.to_string())?;
                let count: i64 = stmt.query_row(
                    rusqlite::params![node.id],
                    |row| row.get(0),
                ).map_err(|e| e.to_string())?;
                Ok(count > 0)
            })().map_err(|e| format!("Failed to check cancellation: {}", e))?;

            if already_cancelled {
                return Err(format!("Run {} is already cancelled", &node.id[..8.min(node.id.len())]));
            }

            // Insert tracks edge with content 'Cancelled by user' and agent_id 'spore:user'
            let now = chrono::Utc::now().timestamp_millis();
            let edge = Edge {
                id: uuid::Uuid::new_v4().to_string(),
                source: node.id.clone(),
                target: node.id.clone(),
                edge_type: EdgeType::Tracks,
                label: None,
                weight: None,
                edge_source: Some("user".to_string()),
                evidence_id: None,
                confidence: Some(1.0),
                created_at: now,
                updated_at: Some(now),
                author: Some(settings::get_author_or_default()),
                reason: Some("cancelled by user".to_string()),
                content: Some("Cancelled by user".to_string()),
                agent_id: Some("spore:user".to_string()),
                superseded_by: None,
                metadata: Some(serde_json::json!({
                    "status": "cancelled",
                }).to_string()),
            };
            db.insert_edge(&edge).map_err(|e| format!("Failed to insert cancellation edge: {}", e))?;

            // Update the task node's tags metadata with cancelled status
            let cancelled_at = chrono::DateTime::from_timestamp_millis(now)
                .map(|d| d.to_rfc3339())
                .unwrap_or_default();
            let new_tags = if let Some(ref existing) = node.tags {
                if let Ok(mut obj) = serde_json::from_str::<serde_json::Value>(existing) {
                    if let Some(map) = obj.as_object_mut() {
                        map.insert("status".to_string(), serde_json::json!("cancelled"));
                        map.insert("cancelled_at".to_string(), serde_json::json!(cancelled_at));
                        serde_json::to_string(&obj).unwrap_or_default()
                    } else {
                        // tags was not an object — replace with status object
                        serde_json::json!({"status": "cancelled", "cancelled_at": cancelled_at}).to_string()
                    }
                } else {
                    serde_json::json!({"status": "cancelled", "cancelled_at": cancelled_at}).to_string()
                }
            } else {
                serde_json::json!({"status": "cancelled", "cancelled_at": cancelled_at}).to_string()
            };
            db.update_node_tags(&node.id, &new_tags)
                .map_err(|e| format!("Failed to update task node metadata: {}", e))?;

            let short_id = &node.id[..8.min(node.id.len())];
            if json || local_json {
                println!("{}", serde_json::json!({
                    "cancelled": true,
                    "node_id": node.id,
                    "title": node.title,
                    "edge_id": edge.id,
                }));
            } else {
                println!("Cancelled run {} ({})", short_id, node.title);
            }
            Ok(())
        }

        RunCommands::Cost { since, json: local_json } => {
            let since_millis = since.as_deref().map(parse_since_to_millis).transpose()?;
            let use_json = json || local_json;

            // Query all orchestration task nodes
            let task_rows: Vec<(String, String, i64)> = (|| -> Result<Vec<_>, String> {
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                let mut stmt = conn.prepare(
                    "SELECT id, title, created_at FROM nodes \
                     WHERE node_class = 'operational' AND title LIKE 'Orchestration:%' \
                     ORDER BY created_at DESC"
                ).map_err(|e| e.to_string())?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
                }).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
                Ok(rows)
            })()?;

            // Filter by --since if provided
            let task_rows: Vec<_> = if let Some(since_ms) = since_millis {
                task_rows.into_iter().filter(|(_, _, created_at)| *created_at >= since_ms).collect()
            } else {
                task_rows
            };

            // Compute start-of-today in millis (UTC)
            let today_start = {
                let now = chrono::Utc::now();
                now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis()
            };

            // For each task, get status + cost
            struct CostEntry {
                status: String,
                cost: f64,
                created_at: i64,
            }

            let mut entries: Vec<CostEntry> = Vec::new();
            for (task_id, _title, created_at) in &task_rows {
                // Determine status
                let impl_ids: Vec<String> = (|| -> Result<Vec<String>, String> {
                    let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                    let mut stmt = conn.prepare(
                        "SELECT source_id FROM edges \
                         WHERE target_id = ?1 AND type = 'derives_from'"
                    ).map_err(|e| e.to_string())?;
                    let ids = stmt.query_map(rusqlite::params![task_id], |row| {
                        row.get::<_, String>(0)
                    }).map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();
                    Ok(ids)
                })()?;

                let status = if !impl_ids.is_empty() {
                    let has_verified = (|| -> Result<bool, String> {
                        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                        let placeholders: Vec<String> = impl_ids.iter().enumerate()
                            .map(|(i, _)| format!("?{}", i + 1))
                            .collect();
                        let query = format!(
                            "SELECT COUNT(*) FROM edges \
                             WHERE target_id IN ({}) AND type = 'supports'",
                            placeholders.join(",")
                        );
                        let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
                        let count: i64 = stmt.query_row(
                            rusqlite::params_from_iter(&impl_ids),
                            |row| row.get(0),
                        ).map_err(|e| e.to_string())?;
                        Ok(count > 0)
                    })()?;
                    if has_verified { "verified" } else { "implemented" }
                } else {
                    let is_cancelled = (|| -> Result<bool, String> {
                        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                        let mut stmt = conn.prepare(
                            "SELECT COUNT(*) FROM edges \
                             WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks' \
                             AND content = 'Cancelled by user'"
                        ).map_err(|e| e.to_string())?;
                        let count: i64 = stmt.query_row(
                            rusqlite::params![task_id],
                            |row| row.get(0),
                        ).map_err(|e| e.to_string())?;
                        Ok(count > 0)
                    })()?;
                    if is_cancelled { "cancelled" } else { "pending" }
                };

                // Get cost from tracks edges metadata
                let total_cost: f64 = (|| -> Result<f64, String> {
                    let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                    let mut stmt = conn.prepare(
                        "SELECT metadata FROM edges \
                         WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks'"
                    ).map_err(|e| e.to_string())?;
                    let costs: Vec<f64> = stmt.query_map(rusqlite::params![task_id], |row| {
                        let meta: String = row.get(0)?;
                        Ok(meta)
                    }).map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .filter_map(|meta| {
                        serde_json::from_str::<serde_json::Value>(&meta).ok()
                            .and_then(|v| v["cost_usd"].as_f64())
                    })
                    .collect();
                    Ok(costs.iter().sum())
                })().unwrap_or(0.0);

                entries.push(CostEntry {
                    status: status.to_string(),
                    cost: total_cost,
                    created_at: *created_at,
                });
            }

            // Aggregate
            let total_runs = entries.len();
            let total_cost: f64 = entries.iter().map(|e| e.cost).sum();
            let avg_cost = if total_runs > 0 { total_cost / total_runs as f64 } else { 0.0 };

            // Cost per status
            let mut status_costs: std::collections::BTreeMap<String, (usize, f64)> = std::collections::BTreeMap::new();
            for e in &entries {
                let entry = status_costs.entry(e.status.clone()).or_insert((0, 0.0));
                entry.0 += 1;
                entry.1 += e.cost;
            }

            // Today's cost
            let today_entries: Vec<&CostEntry> = entries.iter().filter(|e| e.created_at >= today_start).collect();
            let today_cost: f64 = today_entries.iter().map(|e| e.cost).sum();
            let today_runs = today_entries.len();

            if use_json {
                let status_obj: serde_json::Value = status_costs.iter().map(|(k, (count, cost))| {
                    (k.clone(), serde_json::json!({"runs": count, "cost_usd": format!("{:.2}", cost)}))
                }).collect::<serde_json::Map<String, serde_json::Value>>().into();

                println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "total_runs": total_runs,
                    "total_cost_usd": format!("{:.2}", total_cost),
                    "avg_cost_usd": format!("{:.2}", avg_cost),
                    "by_status": status_obj,
                    "today": {
                        "runs": today_runs,
                        "cost_usd": format!("{:.2}", today_cost),
                    },
                })).unwrap_or_default());
            } else {
                println!("Cost Breakdown");
                println!("{}", "=".repeat(42));
                println!();

                // Summary table
                println!("{:<24} {:>8} {:>8}", "METRIC", "RUNS", "COST");
                println!("{}", "-".repeat(42));
                println!("{:<24} {:>8} {:>8}", "Total", total_runs, format!("${:.2}", total_cost));
                println!("{:<24} {:>8} {:>8}", "Average per run", "-", format!("${:.2}", avg_cost));
                println!();

                // By status
                println!("By Status");
                println!("{}", "-".repeat(42));
                for status in &["verified", "implemented", "pending", "cancelled", "escalated"] {
                    if let Some((count, cost)) = status_costs.get(*status) {
                        println!("{:<24} {:>8} {:>8}", status, count, format!("${:.2}", cost));
                    }
                }
                for (status, (count, cost)) in &status_costs {
                    if !["verified", "implemented", "pending", "cancelled", "escalated"].contains(&status.as_str()) {
                        println!("{:<24} {:>8} {:>8}", status, count, format!("${:.2}", cost));
                    }
                }
                println!();

                // Today
                println!("Today");
                println!("{}", "-".repeat(42));
                println!("{:<24} {:>8} {:>8}", "Today's runs", today_runs, format!("${:.2}", today_cost));
            }
            Ok(())
        }

        RunCommands::Top { limit } => {
            // Query all orchestration task nodes
            let task_rows: Vec<(String, String, i64)> = (|| -> Result<Vec<_>, String> {
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                let mut stmt = conn.prepare(
                    "SELECT id, title, created_at FROM nodes \
                     WHERE node_class = 'operational' AND title LIKE 'Orchestration:%' \
                     ORDER BY created_at DESC"
                ).map_err(|e| e.to_string())?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
                }).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
                Ok(rows)
            })()?;

            struct TopEntry {
                id: String,
                title: String,
                cost: f64,
                status: String,
                created_at: i64,
            }

            let mut entries: Vec<TopEntry> = Vec::new();
            for (task_id, title, created_at) in &task_rows {
                let description = title.strip_prefix("Orchestration:").unwrap_or(title).trim().to_string();

                // Determine status
                let impl_ids: Vec<String> = (|| -> Result<Vec<String>, String> {
                    let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                    let mut stmt = conn.prepare(
                        "SELECT source_id FROM edges \
                         WHERE target_id = ?1 AND type = 'derives_from'"
                    ).map_err(|e| e.to_string())?;
                    let ids = stmt.query_map(rusqlite::params![task_id], |row| {
                        row.get::<_, String>(0)
                    }).map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();
                    Ok(ids)
                })()?;

                let status = if !impl_ids.is_empty() {
                    let has_verified = (|| -> Result<bool, String> {
                        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                        let placeholders: Vec<String> = impl_ids.iter().enumerate()
                            .map(|(i, _)| format!("?{}", i + 1))
                            .collect();
                        let query = format!(
                            "SELECT COUNT(*) FROM edges \
                             WHERE target_id IN ({}) AND type = 'supports'",
                            placeholders.join(",")
                        );
                        let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
                        let count: i64 = stmt.query_row(
                            rusqlite::params_from_iter(&impl_ids),
                            |row| row.get(0),
                        ).map_err(|e| e.to_string())?;
                        Ok(count > 0)
                    })()?;
                    if has_verified { "verified" } else { "implemented" }
                } else {
                    let is_cancelled = (|| -> Result<bool, String> {
                        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                        let mut stmt = conn.prepare(
                            "SELECT COUNT(*) FROM edges \
                             WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks' \
                             AND content = 'Cancelled by user'"
                        ).map_err(|e| e.to_string())?;
                        let count: i64 = stmt.query_row(
                            rusqlite::params![task_id],
                            |row| row.get(0),
                        ).map_err(|e| e.to_string())?;
                        Ok(count > 0)
                    })()?;
                    if is_cancelled { "cancelled" } else { "pending" }
                };

                // Get cost from tracks edges metadata
                let total_cost: f64 = (|| -> Result<f64, String> {
                    let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                    let mut stmt = conn.prepare(
                        "SELECT metadata FROM edges \
                         WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks'"
                    ).map_err(|e| e.to_string())?;
                    let costs: Vec<f64> = stmt.query_map(rusqlite::params![task_id], |row| {
                        let meta: String = row.get(0)?;
                        Ok(meta)
                    }).map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .filter_map(|meta| {
                        serde_json::from_str::<serde_json::Value>(&meta).ok()
                            .and_then(|v| v["cost_usd"].as_f64())
                    })
                    .collect();
                    Ok(costs.iter().sum())
                })().unwrap_or(0.0);

                entries.push(TopEntry {
                    id: task_id[..8.min(task_id.len())].to_string(),
                    title: description,
                    cost: total_cost,
                    status: status.to_string(),
                    created_at: *created_at,
                });
            }

            // Sort by cost descending, take top N
            entries.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));
            entries.truncate(limit);

            if entries.is_empty() {
                println!("No orchestrator runs found.");
                return Ok(());
            }

            println!("{:<10} {:<42} {:>8} {:<12} {}", "RUN ID", "TASK", "COST", "STATUS", "DATE");
            println!("{}", "-".repeat(86));
            for e in &entries {
                let title = truncate_middle(&e.title, 40);
                let cost_str = format!("${:.2}", e.cost);
                let date = chrono::DateTime::from_timestamp_millis(e.created_at)
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| "?".to_string());
                println!("{:<10} {:<42} {:>8} {:<12} {}", e.id, title, cost_str, e.status, date);
            }
            Ok(())
        }

        RunCommands::Stats { experiment } => {
            // Query all orchestration task nodes
            let task_rows: Vec<(String, i64)> = (|| -> Result<Vec<_>, String> {
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                let mut stmt = conn.prepare(
                    "SELECT id, created_at FROM nodes \
                     WHERE node_class = 'operational' AND title LIKE 'Orchestration:%' \
                     ORDER BY created_at DESC"
                ).map_err(|e| e.to_string())?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                }).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
                Ok(rows)
            })()?;

            // If --experiment is provided, filter to only runs with matching experiment label
            let task_rows: Vec<(String, i64)> = if let Some(ref exp_label) = experiment {
                task_rows.into_iter().filter(|(task_id, _)| {
                    let conn = db.raw_conn().lock().ok();
                    conn.map(|c| {
                        let mut stmt = c.prepare(
                            "SELECT metadata FROM edges \
                             WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks'"
                        ).ok();
                        stmt.as_mut().map(|s| {
                            s.query_map(rusqlite::params![task_id], |row| {
                                row.get::<_, String>(0)
                            }).ok()
                            .map(|rows| {
                                rows.filter_map(|r| r.ok())
                                    .any(|meta| {
                                        serde_json::from_str::<serde_json::Value>(&meta).ok()
                                            .and_then(|v| v["experiment"].as_str().map(|e| e == exp_label))
                                            .unwrap_or(false)
                                    })
                            }).unwrap_or(false)
                        }).unwrap_or(false)
                    }).unwrap_or(false)
                }).collect()
            } else {
                task_rows
            };

            let total_runs = task_rows.len();
            if total_runs == 0 {
                if experiment.is_some() {
                    println!("No orchestrator runs found with experiment label '{}'.", experiment.as_ref().unwrap());
                } else {
                    println!("No orchestrator runs found.");
                }
                return Ok(());
            }

            let mut total_cost = 0.0_f64;
            let mut costs: Vec<f64> = Vec::new();
            let mut total_turns = 0_u64;
            let mut runs_with_turns = 0_usize;
            let mut verified_bounce_counts: Vec<usize> = Vec::new();
            let mut status_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();

            for (task_id, _created_at) in &task_rows {
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;

                // Get tracks edges metadata for this run
                let mut stmt = conn.prepare(
                    "SELECT metadata FROM edges \
                     WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks'"
                ).map_err(|e| e.to_string())?;
                let metas: Vec<serde_json::Value> = stmt.query_map(rusqlite::params![task_id], |row| {
                    let meta: String = row.get(0)?;
                    Ok(meta)
                }).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .filter_map(|meta| serde_json::from_str::<serde_json::Value>(&meta).ok())
                .collect();
                drop(stmt);

                // Sum cost for this run
                let run_cost: f64 = metas.iter()
                    .filter_map(|v| v["cost_usd"].as_f64())
                    .sum();
                total_cost += run_cost;
                costs.push(run_cost);

                // Sum turns for this run
                let run_turns: u64 = metas.iter()
                    .filter_map(|v| v["num_turns"].as_u64())
                    .sum();
                if run_turns > 0 {
                    total_turns += run_turns;
                    runs_with_turns += 1;
                }

                // Count bounces (number of coder agent entries)
                let bounce_count = metas.iter()
                    .filter(|v| v["agent"].as_str().map(|a| a.contains("coder")).unwrap_or(false))
                    .count();

                // Check for planned status first (plan_complete/plan_partial in Tracks metadata)
                let is_planned = metas.iter().any(|v| {
                    matches!(v["status"].as_str(), Some("plan_complete") | Some("plan_partial"))
                });

                // Determine status
                let mut stmt2 = conn.prepare(
                    "SELECT source_id FROM edges \
                     WHERE target_id = ?1 AND type = 'derives_from'"
                ).map_err(|e| e.to_string())?;
                let impl_ids: Vec<String> = stmt2.query_map(rusqlite::params![task_id], |row| {
                    row.get::<_, String>(0)
                }).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
                drop(stmt2);

                let status = if is_planned {
                    "planned"
                } else if !impl_ids.is_empty() {
                    let placeholders: Vec<String> = impl_ids.iter().enumerate()
                        .map(|(i, _)| format!("?{}", i + 1))
                        .collect();
                    let query = format!(
                        "SELECT COUNT(*) FROM edges \
                         WHERE target_id IN ({}) AND type = 'supports'",
                        placeholders.join(",")
                    );
                    let mut stmt3 = conn.prepare(&query).map_err(|e| e.to_string())?;
                    let count: i64 = stmt3.query_row(
                        rusqlite::params_from_iter(&impl_ids),
                        |row| row.get(0),
                    ).map_err(|e| e.to_string())?;
                    if count > 0 { "verified" } else { "implemented" }
                } else {
                    let mut stmt4 = conn.prepare(
                        "SELECT COUNT(*) FROM edges \
                         WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks' \
                         AND content = 'Cancelled by user'"
                    ).map_err(|e| e.to_string())?;
                    let count: i64 = stmt4.query_row(
                        rusqlite::params![task_id],
                        |row| row.get(0),
                    ).map_err(|e| e.to_string())?;
                    if count > 0 {
                        "cancelled"
                    } else {
                        let mut stmt5 = conn.prepare(
                            "SELECT COUNT(*) FROM edges e \
                             JOIN nodes esc ON esc.id = e.source_id \
                             WHERE e.target_id = ?1 AND e.type = 'tracks' \
                               AND esc.title LIKE 'ESCALATION:%'"
                        ).map_err(|e| e.to_string())?;
                        let esc_count: i64 = stmt5.query_row(rusqlite::params![task_id], |row| row.get(0))
                            .map_err(|e| e.to_string())?;
                        if esc_count > 0 { "escalated" } else { "pending" }
                    }
                };

                *status_counts.entry(status.to_string()).or_insert(0) += 1;

                // Track bounces for verified runs
                if status == "verified" && bounce_count > 0 {
                    verified_bounce_counts.push(bounce_count);
                }
            }

            // Median cost
            costs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median_cost = if costs.is_empty() {
                0.0
            } else if costs.len() % 2 == 0 {
                (costs[costs.len() / 2 - 1] + costs[costs.len() / 2]) / 2.0
            } else {
                costs[costs.len() / 2]
            };

            // Average turns
            let avg_turns = if runs_with_turns > 0 {
                total_turns as f64 / runs_with_turns as f64
            } else {
                0.0
            };

            // Average bounces for verified runs
            let avg_bounces = if !verified_bounce_counts.is_empty() {
                verified_bounce_counts.iter().sum::<usize>() as f64 / verified_bounce_counts.len() as f64
            } else {
                0.0
            };

            // Most common failure reason from escalation nodes
            let failure_reason: Option<String> = (|| -> Result<Option<String>, String> {
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                // Get the last contradicts edge content for each escalated run
                let mut stmt = conn.prepare(
                    "SELECT e2.content FROM nodes n \
                     JOIN edges e1 ON e1.source_id = n.id AND e1.type = 'tracks' \
                     JOIN edges e_df ON e_df.target_id = e1.target_id AND e_df.type = 'derives_from' \
                     JOIN edges e2 ON e2.target_id = e_df.source_id AND e2.type = 'contradicts' \
                     WHERE n.title LIKE 'ESCALATION:%' \
                     ORDER BY e2.created_at DESC"
                ).map_err(|e| e.to_string())?;
                let reasons: Vec<String> = stmt.query_map([], |row| {
                    row.get::<_, Option<String>>(0)
                }).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .flatten()
                .collect();
                drop(stmt);

                if reasons.is_empty() {
                    // Fall back to escalation node titles
                    let mut stmt2 = conn.prepare(
                        "SELECT title FROM nodes WHERE title LIKE 'ESCALATION:%'"
                    ).map_err(|e| e.to_string())?;
                    let titles: Vec<String> = stmt2.query_map([], |row| {
                        row.get::<_, String>(0)
                    }).map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();

                    if titles.is_empty() {
                        return Ok(None);
                    }

                    // Count occurrences to find most common
                    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
                    for t in &titles {
                        // Normalize: strip prefix and bounce count suffix
                        let reason = t.strip_prefix("ESCALATION:").unwrap_or(t).trim();
                        // Remove " (after N bounces)" suffix
                        let reason = if let Some(pos) = reason.rfind(" (after ") {
                            reason[..pos].trim()
                        } else {
                            reason
                        };
                        *counts.entry(reason.to_string()).or_insert(0) += 1;
                    }
                    return Ok(counts.into_iter().max_by_key(|(_, c)| *c).map(|(r, _)| r));
                }

                // Count occurrences of each reason to find most common
                // Normalize by taking first line only
                let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
                for r in &reasons {
                    let first_line = r.lines().next().unwrap_or(r).trim().to_string();
                    if !first_line.is_empty() {
                        *counts.entry(first_line).or_insert(0) += 1;
                    }
                }
                Ok(counts.into_iter().max_by_key(|(_, c)| *c).map(|(r, _)| r))
            })().unwrap_or(None);

            // Display
            if let Some(ref exp_label) = experiment {
                println!("Run Statistics (experiment: {})", exp_label);
            } else {
                println!("Run Statistics");
            }
            println!("{}", "=".repeat(42));
            println!();
            println!("{:<30} {}", "Total runs:", total_runs);
            println!("{:<30} ${:.2}", "Total cost:", total_cost);
            println!("{:<30} ${:.2}", "Median cost per run:", median_cost);
            println!("{:<30} {:.1}", "Avg turns per run:", avg_turns);
            println!("{:<30} {:.1}", "Avg bounces (verified):", avg_bounces);
            println!();

            // Status breakdown
            println!("By Status");
            println!("{}", "-".repeat(42));
            for status in &["verified", "implemented", "pending", "planned", "cancelled", "escalated"] {
                if let Some(count) = status_counts.get(*status) {
                    let pct = (*count as f64 / total_runs as f64) * 100.0;
                    println!("{:<30} {:>4} ({:.0}%)", status, count, pct);
                }
            }
            // Any other statuses
            for (status, count) in &status_counts {
                if !["verified", "implemented", "pending", "planned", "cancelled", "escalated"].contains(&status.as_str()) {
                    let pct = (*count as f64 / total_runs as f64) * 100.0;
                    println!("{:<30} {:>4} ({:.0}%)", status, count, pct);
                }
            }
            println!();

            // Failure reason
            println!("Most common failure reason:");
            match failure_reason {
                Some(ref reason) => {
                    let display = if reason.chars().count() > 60 {
                        format!("{}...", &reason.chars().take(57).collect::<String>())
                    } else {
                        reason.clone()
                    };
                    println!("  {}", display);
                }
                None => println!("  (none)"),
            }

            Ok(())
        }

        RunCommands::Summary => {
            // Query all orchestration task nodes
            let task_rows: Vec<(String, String, i64)> = (|| -> Result<Vec<_>, String> {
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                let mut stmt = conn.prepare(
                    "SELECT id, title, created_at FROM nodes \
                     WHERE node_class = 'operational' AND title LIKE 'Orchestration:%' \
                     ORDER BY created_at DESC"
                ).map_err(|e| e.to_string())?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
                }).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
                Ok(rows)
            })()?;

            if task_rows.is_empty() {
                println!("No orchestrator runs found.");
                return Ok(());
            }

            // Compute start-of-today in millis (UTC)
            let today_start = {
                let now = chrono::Utc::now();
                now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis()
            };

            let mut status_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
            let mut verified_titles: Vec<String> = Vec::new();
            let mut escalated_titles: Vec<String> = Vec::new();
            let mut today_cost = 0.0_f64;

            for (task_id, title, created_at) in &task_rows {
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;

                // Get cost from tracks edges metadata
                let mut stmt = conn.prepare(
                    "SELECT metadata FROM edges \
                     WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks'"
                ).map_err(|e| e.to_string())?;
                let metas: Vec<serde_json::Value> = stmt.query_map(rusqlite::params![task_id], |row| {
                    let meta: String = row.get(0)?;
                    Ok(meta)
                }).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .filter_map(|meta| serde_json::from_str::<serde_json::Value>(&meta).ok())
                .collect();
                drop(stmt);

                let run_cost: f64 = metas.iter()
                    .filter_map(|v| v["cost_usd"].as_f64())
                    .sum();

                if *created_at >= today_start {
                    today_cost += run_cost;
                }

                // Determine status
                let mut stmt2 = conn.prepare(
                    "SELECT source_id FROM edges \
                     WHERE target_id = ?1 AND type = 'derives_from'"
                ).map_err(|e| e.to_string())?;
                let impl_ids: Vec<String> = stmt2.query_map(rusqlite::params![task_id], |row| {
                    row.get::<_, String>(0)
                }).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
                drop(stmt2);

                let status = if !impl_ids.is_empty() {
                    let placeholders: Vec<String> = impl_ids.iter().enumerate()
                        .map(|(i, _)| format!("?{}", i + 1))
                        .collect();
                    let query = format!(
                        "SELECT COUNT(*) FROM edges \
                         WHERE target_id IN ({}) AND type = 'supports'",
                        placeholders.join(",")
                    );
                    let mut stmt3 = conn.prepare(&query).map_err(|e| e.to_string())?;
                    let count: i64 = stmt3.query_row(
                        rusqlite::params_from_iter(&impl_ids),
                        |row| row.get(0),
                    ).map_err(|e| e.to_string())?;
                    if count > 0 { "verified" } else { "implemented" }
                } else {
                    let mut stmt4 = conn.prepare(
                        "SELECT COUNT(*) FROM edges \
                         WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks' \
                         AND content = 'Cancelled by user'"
                    ).map_err(|e| e.to_string())?;
                    let count: i64 = stmt4.query_row(
                        rusqlite::params![task_id],
                        |row| row.get(0),
                    ).map_err(|e| e.to_string())?;
                    if count > 0 {
                        "cancelled"
                    } else {
                        let mut stmt5 = conn.prepare(
                            "SELECT COUNT(*) FROM edges e \
                             JOIN nodes esc ON esc.id = e.source_id \
                             WHERE e.target_id = ?1 AND e.type = 'tracks' \
                               AND esc.title LIKE 'ESCALATION:%'"
                        ).map_err(|e| e.to_string())?;
                        let esc_count: i64 = stmt5.query_row(rusqlite::params![task_id], |row| row.get(0))
                            .map_err(|e| e.to_string())?;
                        if esc_count > 0 { "escalated" } else { "pending" }
                    }
                };

                *status_counts.entry(status.to_string()).or_insert(0) += 1;

                let short_title = title.strip_prefix("Orchestration:").unwrap_or(title).trim().to_string();

                if status == "verified" && verified_titles.len() < 5 {
                    // task_rows are ordered DESC, so first verified titles are the most recent
                    verified_titles.push(short_title.clone());
                }
                if status == "escalated" && escalated_titles.len() < 3 {
                    escalated_titles.push(short_title);
                }
            }

            // Build prose
            let total = task_rows.len();
            let mut parts: Vec<String> = Vec::new();

            // Opening: total runs and status breakdown
            let mut status_parts: Vec<String> = Vec::new();
            for s in &["verified", "implemented", "pending", "planned", "escalated", "cancelled"] {
                if let Some(c) = status_counts.get(*s) {
                    status_parts.push(format!("{} {}", c, s));
                }
            }
            parts.push(format!(
                "Across {} orchestrator runs, {} ({})",
                total,
                status_parts.join(", "),
                {
                    let verified = *status_counts.get("verified").unwrap_or(&0);
                    let pct = if total > 0 { (verified as f64 / total as f64) * 100.0 } else { 0.0 };
                    format!("{:.0}% success rate", pct)
                }
            ));

            // Recently verified features
            if !verified_titles.is_empty() {
                let titles_str = if verified_titles.len() == 1 {
                    format!("\"{}\"", verified_titles[0])
                } else {
                    let all: Vec<String> = verified_titles.iter().map(|t| {
                        let display = if t.chars().count() > 50 {
                            format!("{}...", t.chars().take(47).collect::<String>())
                        } else {
                            t.clone()
                        };
                        format!("\"{}\"", display)
                    }).collect();
                    all.join(", ")
                };
                parts.push(format!(
                    "The most recently verified features are: {}",
                    titles_str
                ));
            }

            // Escalated tasks
            if !escalated_titles.is_empty() {
                let titles_str: Vec<String> = escalated_titles.iter().map(|t| {
                    let display = if t.chars().count() > 50 {
                        format!("{}...", t.chars().take(47).collect::<String>())
                    } else {
                        t.clone()
                    };
                    format!("\"{}\"", display)
                }).collect();
                parts.push(format!(
                    "{} task{} escalated: {}",
                    escalated_titles.len(),
                    if escalated_titles.len() == 1 { " was" } else { "s were" },
                    titles_str.join(", ")
                ));
            }

            // Today's spend
            parts.push(format!("Today's spend is ${:.2}", today_cost));

            // Join as a paragraph
            println!("{}", parts.join(". ") + ".");

            Ok(())
        }

        RunCommands::Timeline { run_id } => {
            let task_node = resolve_node(db, &run_id)?;
            let task_id = &task_node.id;
            let task_desc = task_node.title.strip_prefix("Orchestration:").unwrap_or(&task_node.title).trim();

            // Query self-referencing tracks edges (agent phase metadata)
            let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
            let mut stmt = conn.prepare(
                "SELECT metadata, created_at FROM edges \
                 WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks' \
                 ORDER BY created_at"
            ).map_err(|e| e.to_string())?;
            let phases: Vec<(serde_json::Value, i64)> = stmt.query_map(rusqlite::params![task_id], |row| {
                let meta: String = row.get(0)?;
                let ts: i64 = row.get(1)?;
                Ok((meta, ts))
            }).map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .filter_map(|(meta, ts)| serde_json::from_str::<serde_json::Value>(&meta).ok().map(|v| (v, ts)))
            .collect();
            drop(stmt);
            drop(conn);

            if phases.is_empty() {
                if json {
                    println!("{{\"phases\":[],\"summary\":null}}");
                } else {
                    println!("No agent phases found for run {}.", &task_id[..8.min(task_id.len())]);
                }
                return Ok(());
            }

            let mut total_cost = 0.0_f64;
            let mut total_turns = 0_u64;
            let mut total_duration_ms = 0_u64;
            let phase_count = phases.len();

            if json {
                let mut json_phases = Vec::new();
                for (meta, ts) in &phases {
                    let agent = meta["agent"].as_str().unwrap_or("?").to_string();
                    let status = meta["status"].as_str().unwrap_or("?").to_string();
                    let turns = meta["turns"].as_u64().or_else(|| meta["num_turns"].as_u64()).unwrap_or(0);
                    let duration_ms = meta["duration_ms"].as_u64().unwrap_or(0);
                    let cost = meta["cost_usd"].as_f64().unwrap_or(0.0);

                    total_cost += cost;
                    total_turns += turns;
                    total_duration_ms += duration_ms;

                    json_phases.push(serde_json::json!({
                        "agent": agent,
                        "status": status,
                        "num_turns": turns,
                        "duration_ms": duration_ms,
                        "cost_usd": cost,
                        "created_at": ts,
                    }));
                }

                // Wall-clock duration in ms
                let wall_duration_ms = if phases.len() >= 2 {
                    let first_ts = phases.first().map(|(_, ts)| *ts).unwrap_or(0);
                    let last_ts = phases.last().map(|(_, ts)| *ts).unwrap_or(0);
                    let last_dur = phases.last()
                        .and_then(|(m, _)| m["duration_ms"].as_u64())
                        .unwrap_or(0);
                    (last_ts - first_ts) as u64 + last_dur
                } else {
                    total_duration_ms
                };

                let output = serde_json::json!({
                    "phases": json_phases,
                    "summary": {
                        "total_phases": phase_count,
                        "total_turns": total_turns,
                        "total_duration_ms": wall_duration_ms,
                        "total_cost": total_cost,
                    }
                });
                println!("{}", serde_json::to_string(&output).unwrap_or_default());
                return Ok(());
            }

            // Header
            let short_desc = if task_desc.chars().count() > 60 {
                format!("{}...", utils::safe_truncate(task_desc, 57))
            } else {
                task_desc.to_string()
            };
            println!("Run: {} -- {}", &task_id[..8.min(task_id.len())], short_desc);
            println!("{}", "=".repeat(64));
            println!();

            for (i, (meta, ts)) in phases.iter().enumerate() {
                let is_last = i == phase_count - 1;
                let time_str = chrono::DateTime::from_timestamp_millis(*ts)
                    .map(|d| d.format("%H:%M:%S").to_string())
                    .unwrap_or_else(|| "??:??:??".to_string());

                let agent = meta["agent"].as_str().unwrap_or("?");
                let short_agent = agent.strip_prefix("spore:").unwrap_or(agent);
                let status = meta["status"].as_str().unwrap_or("?");
                let bounce = meta["bounce"].as_u64();
                let turns = meta["turns"].as_u64().or_else(|| meta["num_turns"].as_u64());
                let duration_ms = meta["duration_ms"].as_u64();
                let cost = meta["cost_usd"].as_f64();

                if let Some(c) = cost { total_cost += c; }
                if let Some(t) = turns { total_turns += t; }
                if let Some(d) = duration_ms { total_duration_ms += d; }

                // Connector characters
                let (branch, _pipe) = if is_last { ("+-", "  ") } else { ("+-", "| ") };
                let _ = branch; // we use custom chars below

                let bounce_str = bounce.map(|b| format!(" (bounce {})", b)).unwrap_or_default();
                println!("  {}  {}{} {}{}", time_str,
                    if is_last { "\\-" } else { "+-" },
                    "-", short_agent, bounce_str);

                // Status line
                println!("  {:>8}  {}  Status: {}", "",
                    if is_last { " " } else { "|" }, status);

                // Details line
                let mut details = Vec::new();
                if let Some(t) = turns { details.push(format!("Turns: {}", t)); }
                if let Some(d) = duration_ms {
                    let secs = d / 1000;
                    if secs >= 60 {
                        details.push(format!("Duration: {}m {}s", secs / 60, secs % 60));
                    } else {
                        details.push(format!("Duration: {}s", secs));
                    }
                }
                if let Some(c) = cost { details.push(format!("Cost: ${:.2}", c)); }
                if !details.is_empty() {
                    println!("  {:>8}  {}  {}", "",
                        if is_last { " " } else { "|" }, details.join(" | "));
                }

                // Blank separator between phases
                if !is_last {
                    println!("  {:>8}  |", "");
                }
            }

            // Footer totals
            println!();
            println!("{}", "-".repeat(64));

            // Total duration: use wall-clock span from first to last phase creation
            let wall_duration_secs = if phases.len() >= 2 {
                let first_ts = phases.first().map(|(_, ts)| *ts).unwrap_or(0);
                let last_ts = phases.last().map(|(_, ts)| *ts).unwrap_or(0);
                // Add last phase's duration if available
                let last_dur = phases.last()
                    .and_then(|(m, _)| m["duration_ms"].as_u64())
                    .unwrap_or(0);
                ((last_ts - first_ts) as u64 + last_dur) / 1000
            } else {
                total_duration_ms / 1000
            };

            println!("Total: {} phases | {} turns | {}m {}s | ${:.2}",
                phase_count, total_turns,
                wall_duration_secs / 60, wall_duration_secs % 60,
                total_cost);

            Ok(())
        }
    }
}

// ============================================================================
// Experiment Comparison
// ============================================================================

// Tech debt: this duplicates some patterns from RunCommands::Compare and Stats.
// Consider extracting shared metric-gathering helpers if more comparison modes are added.

#[allow(dead_code)]
struct ExperimentRunMetrics {
    task_desc: String,
    status: String,
    total_cost: f64,
    coder_turns: u64,
    duration_secs: u64,
}

pub(crate) fn handle_runs_compare_experiments(db: &Database, exp_a: &str, exp_b: &str) -> Result<(), String> {
    let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;

    // Step 1: Find all task nodes with experiment labels A or B
    let mut stmt = conn.prepare(
        "SELECT DISTINCT e.source_id, json_extract(e.metadata, '$.experiment') as exp_label \
         FROM edges e \
         JOIN nodes n ON n.id = e.source_id \
         WHERE e.source_id = e.target_id \
           AND e.type = 'tracks' \
           AND n.node_class = 'operational' \
           AND n.title LIKE 'Orchestration:%' \
           AND json_extract(e.metadata, '$.experiment') IN (?1, ?2)"
    ).map_err(|e| e.to_string())?;

    let task_exp_rows: Vec<(String, String)> = stmt.query_map(
        rusqlite::params![exp_a, exp_b],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    ).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();
    drop(stmt);

    // Check that both experiments have at least one run
    let has_a = task_exp_rows.iter().any(|(_, label)| label == exp_a);
    let has_b = task_exp_rows.iter().any(|(_, label)| label == exp_b);
    if !has_a {
        return Err(format!("No runs found with experiment label '{}'.", exp_a));
    }
    if !has_b {
        return Err(format!("No runs found with experiment label '{}'.", exp_b));
    }

    // Step 2: For each task node, get title and strip prefix
    // Group by (task_desc, experiment_label)
    let mut task_metrics: HashMap<String, (Option<ExperimentRunMetrics>, Option<ExperimentRunMetrics>)> = HashMap::new();

    for (task_id, exp_label) in &task_exp_rows {
        // Get node title
        let title: String = conn.prepare("SELECT title FROM nodes WHERE id = ?1")
            .and_then(|mut s| s.query_row(rusqlite::params![task_id], |r| r.get(0)))
            .map_err(|e| format!("Failed to get node title for {}: {}", task_id, e))?;

        let task_desc = title.strip_prefix("Orchestration:")
            .unwrap_or(&title)
            .trim()
            .to_string();

        // Step 3: Gather per-run metrics from tracks edges
        let mut phase_stmt = conn.prepare(
            "SELECT metadata, created_at FROM edges \
             WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks' \
             ORDER BY created_at"
        ).map_err(|e| e.to_string())?;

        let phases: Vec<(serde_json::Value, i64)> = phase_stmt.query_map(
            rusqlite::params![task_id],
            |row| {
                let meta: String = row.get(0)?;
                let ts: i64 = row.get(1)?;
                Ok((meta, ts))
            }
        ).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .filter_map(|(meta, ts)| serde_json::from_str::<serde_json::Value>(&meta).ok().map(|v| (v, ts)))
        .collect();
        drop(phase_stmt);

        let mut total_cost = 0.0_f64;
        let mut coder_turns = 0_u64;

        for (meta, _) in &phases {
            if let Some(c) = meta["cost_usd"].as_f64() { total_cost += c; }
            let is_coder = meta["agent"].as_str()
                .map(|a| a.contains("coder"))
                .unwrap_or(false);
            if is_coder {
                if let Some(t) = meta["turns"].as_u64().or_else(|| meta["num_turns"].as_u64()) {
                    coder_turns += t;
                }
            }
        }

        // Duration: span from first to last phase
        let duration_secs = if phases.len() >= 2 {
            let first_ts = phases.first().map(|(_, ts)| *ts).unwrap_or(0);
            let last_ts = phases.last().map(|(_, ts)| *ts).unwrap_or(0);
            let last_dur = phases.last()
                .and_then(|(m, _)| m["duration_ms"].as_u64())
                .unwrap_or(0);
            ((last_ts - first_ts) as u64 + last_dur) / 1000
        } else {
            phases.iter()
                .filter_map(|(m, _)| m["duration_ms"].as_u64())
                .sum::<u64>() / 1000
        };

        // Status: check for verified via graph edges
        let has_verified: bool = conn.prepare(
            "SELECT COUNT(*) FROM edges e1 \
             JOIN edges e2 ON e2.target_id = e1.source_id \
             WHERE e1.target_id = ?1 AND e1.type = 'derives_from' AND e2.type = 'supports'"
        ).and_then(|mut s| s.query_row(rusqlite::params![task_id], |r| r.get::<_, i64>(0)))
        .unwrap_or(0) > 0;

        let has_impl: bool = conn.prepare(
            "SELECT COUNT(*) FROM edges WHERE target_id = ?1 AND type = 'derives_from'"
        ).and_then(|mut s| s.query_row(rusqlite::params![task_id], |r| r.get::<_, i64>(0)))
        .unwrap_or(0) > 0;

        let status = if has_verified { "verified" } else if has_impl { "implemented" } else { "pending" };

        let metrics = ExperimentRunMetrics {
            task_desc: task_desc.clone(),
            status: status.to_string(),
            total_cost,
            coder_turns,
            duration_secs,
        };

        // Step 4: Insert into paired map, taking most recent if duplicates
        let entry = task_metrics.entry(task_desc).or_insert((None, None));
        if exp_label == exp_a {
            entry.0 = Some(metrics);
        } else {
            entry.1 = Some(metrics);
        }
    }
    drop(conn);

    // Step 5: Render comparison table
    println!("Experiment Comparison: {} vs {}", exp_a, exp_b);
    println!();

    let col_task = 28;
    let col_cost = 8;
    let col_turns = 8;
    let col_dur = 8;
    let col_status = 11;

    // Header
    println!("{:<width_t$} | {:>w_c$} {:>w_c$} | {:>w_tu$} {:>w_tu$} | {:>w_d$} {:>w_d$} | {:<w_s$}",
        "Task",
        format!("Cost {}", "A"), format!("Cost {}", "B"),
        format!("Trn {}", "A"), format!("Trn {}", "B"),
        format!("Dur {}", "A"), format!("Dur {}", "B"),
        "Status",
        width_t = col_task, w_c = col_cost, w_tu = col_turns, w_d = col_dur, w_s = col_status);

    let total_width = col_task + 3 + col_cost * 2 + 1 + 3 + col_turns * 2 + 1 + 3 + col_dur * 2 + 1 + 3 + col_status;
    println!("{}", "-".repeat(total_width));

    // Sort tasks alphabetically for consistent output
    let mut task_descs: Vec<String> = task_metrics.keys().cloned().collect();
    task_descs.sort();

    let mut sum_cost_a = 0.0_f64;
    let mut sum_cost_b = 0.0_f64;
    let mut sum_turns_a = 0_u64;
    let mut sum_turns_b = 0_u64;
    let mut sum_dur_a = 0_u64;
    let mut sum_dur_b = 0_u64;
    let mut paired_count = 0_usize;

    let fmt_dur = |secs: u64| -> String {
        if secs >= 60 { format!("{}m{}s", secs / 60, secs % 60) } else { format!("{}s", secs) }
    };

    for desc in &task_descs {
        let (ref a, ref b) = task_metrics[desc];

        let truncated = if desc.chars().count() > col_task {
            format!("{}...", utils::safe_truncate(desc, col_task - 3))
        } else {
            desc.clone()
        };

        let (cost_a_str, cost_b_str) = match (a, b) {
            (Some(a), Some(b)) => (format!("${:.2}", a.total_cost), format!("${:.2}", b.total_cost)),
            (Some(a), None)    => (format!("${:.2}", a.total_cost), "---".to_string()),
            (None, Some(b))    => ("---".to_string(), format!("${:.2}", b.total_cost)),
            (None, None)       => ("---".to_string(), "---".to_string()),
        };

        let (turns_a_str, turns_b_str) = match (a, b) {
            (Some(a), Some(b)) => (a.coder_turns.to_string(), b.coder_turns.to_string()),
            (Some(a), None)    => (a.coder_turns.to_string(), "---".to_string()),
            (None, Some(b))    => ("---".to_string(), b.coder_turns.to_string()),
            (None, None)       => ("---".to_string(), "---".to_string()),
        };

        let (dur_a_str, dur_b_str) = match (a, b) {
            (Some(a), Some(b)) => (fmt_dur(a.duration_secs), fmt_dur(b.duration_secs)),
            (Some(a), None)    => (fmt_dur(a.duration_secs), "---".to_string()),
            (None, Some(b))    => ("---".to_string(), fmt_dur(b.duration_secs)),
            (None, None)       => ("---".to_string(), "---".to_string()),
        };

        let status_str = match (a, b) {
            (Some(a), Some(b)) => {
                let sa = if a.status == "verified" { "\u{2713}" } else { "\u{2717}" };
                let sb = if b.status == "verified" { "\u{2713}" } else { "\u{2717}" };
                format!("{} / {}", sa, sb)
            }
            (Some(a), None) => {
                let sa = if a.status == "verified" { "\u{2713}" } else { "\u{2717}" };
                format!("{} / -", sa)
            }
            (None, Some(b)) => {
                let sb = if b.status == "verified" { "\u{2713}" } else { "\u{2717}" };
                format!("- / {}", sb)
            }
            (None, None) => "- / -".to_string(),
        };

        // Accumulate totals for paired tasks only
        if let (Some(a), Some(b)) = (a, b) {
            sum_cost_a += a.total_cost;
            sum_cost_b += b.total_cost;
            sum_turns_a += a.coder_turns;
            sum_turns_b += b.coder_turns;
            sum_dur_a += a.duration_secs;
            sum_dur_b += b.duration_secs;
            paired_count += 1;
        }

        println!("{:<width_t$} | {:>w_c$} {:>w_c$} | {:>w_tu$} {:>w_tu$} | {:>w_d$} {:>w_d$} | {:<w_s$}",
            truncated,
            cost_a_str, cost_b_str,
            turns_a_str, turns_b_str,
            dur_a_str, dur_b_str,
            status_str,
            width_t = col_task, w_c = col_cost, w_tu = col_turns, w_d = col_dur, w_s = col_status);
    }

    // TOTAL row
    println!("{}", "-".repeat(total_width));
    if paired_count > 0 {
        println!("{:<width_t$} | {:>w_c$} {:>w_c$} | {:>w_tu$} {:>w_tu$} | {:>w_d$} {:>w_d$} |",
            format!("TOTAL ({} paired)", paired_count),
            format!("${:.2}", sum_cost_a), format!("${:.2}", sum_cost_b),
            sum_turns_a.to_string(), sum_turns_b.to_string(),
            fmt_dur(sum_dur_a), fmt_dur(sum_dur_b),
            width_t = col_task, w_c = col_cost, w_tu = col_turns, w_d = col_dur);

        // DELTA row: (B - A) / A * 100
        let cost_delta = if sum_cost_a > 0.0 {
            format!("{:+.0}%", (sum_cost_b - sum_cost_a) / sum_cost_a * 100.0)
        } else {
            "N/A".to_string()
        };
        let turns_delta = if sum_turns_a > 0 {
            format!("{:+.0}%", (sum_turns_b as f64 - sum_turns_a as f64) / sum_turns_a as f64 * 100.0)
        } else {
            "N/A".to_string()
        };
        let dur_delta = if sum_dur_a > 0 {
            format!("{:+.0}%", (sum_dur_b as f64 - sum_dur_a as f64) / sum_dur_a as f64 * 100.0)
        } else {
            "N/A".to_string()
        };

        println!("{:<width_t$} | {:>w_c$} {:>w_c$} | {:>w_tu$} {:>w_tu$} | {:>w_d$} {:>w_d$} |",
            "DELTA (B-A)/A",
            "", cost_delta,
            "", turns_delta,
            "", dur_delta,
            width_t = col_task, w_c = col_cost, w_tu = col_turns, w_d = col_dur);
    } else {
        println!("No paired tasks found (no tasks appear in both experiments).");
    }

    Ok(())
}

// ============================================================================
// Distill
// ============================================================================

pub(crate) async fn handle_distill(db: &Database, run_ref: &str, json: bool, compact: bool) -> Result<(), String> {
    use std::collections::{HashMap, VecDeque};

    // 1. Find the task node
    let task_node = if run_ref == "latest" {
        // Find most recent orchestration node
        let conn = db.raw_conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id FROM nodes WHERE node_class = 'operational' AND title LIKE 'Orchestration:%' ORDER BY created_at DESC LIMIT 1"
        ).map_err(|e| e.to_string())?;
        let id: Option<String> = stmt.query_row([], |row| row.get(0)).ok();
        drop(stmt);
        drop(conn);
        match id {
            Some(id) => db.get_node(&id).map_err(|e| e.to_string())?.ok_or("No orchestration runs found")?,
            None => return Err("No orchestration runs found".to_string()),
        }
    } else {
        resolve_node(db, run_ref)?
    };

    let task_title = task_node.ai_title.as_deref().unwrap_or(&task_node.title);
    println!("=== Distill: {} ===\n", &task_title[..task_title.len().min(60)]);

    // 2. BFS through connected operational/meta nodes to build the trail
    struct TrailNode {
        id: String,
        title: String,
        content: String,
        class: String,
        meta_type: Option<String>,
        agent: Option<String>,
        created_at: i64,
        edges_out: Vec<(String, String)>,  // (edge_type, target_id)
        edges_in: Vec<(String, String)>,   // (edge_type, source_id)
    }

    let mut trail: HashMap<String, TrailNode> = HashMap::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(task_node.id.clone());

    while let Some(nid) = queue.pop_front() {
        if trail.contains_key(&nid) {
            continue;
        }

        let node = match db.get_node(&nid).map_err(|e| e.to_string())? {
            Some(n) => n,
            None => continue,
        };

        // Only follow operational and meta nodes
        match node.node_class.as_deref() {
            Some("operational") | Some("meta") => {},
            _ => continue,
        }

        let edges = db.get_edges_for_node(&nid).map_err(|e| e.to_string())?;
        let mut edges_out = Vec::new();
        let mut edges_in = Vec::new();

        for edge in &edges {
            if edge.source == nid {
                edges_out.push((edge.edge_type.as_str().to_string(), edge.target.clone()));
                queue.push_back(edge.target.clone());
            } else {
                edges_in.push((edge.edge_type.as_str().to_string(), edge.source.clone()));
                queue.push_back(edge.source.clone());
            }
        }

        trail.insert(nid, TrailNode {
            id: node.id,
            title: node.ai_title.unwrap_or(node.title),
            content: node.content.unwrap_or_default(),
            class: node.node_class.unwrap_or_default(),
            meta_type: node.meta_type,
            agent: node.agent_id,
            created_at: node.created_at,
            edges_out,
            edges_in,
        });
    }

    // 3. Sort by timestamp and print timeline
    let mut sorted: Vec<&TrailNode> = trail.values().collect();
    sorted.sort_by_key(|n| n.created_at);

    if json {
        let timeline: Vec<_> = sorted.iter().map(|n| {
            serde_json::json!({
                "id": &n.id[..8.min(n.id.len())],
                "title": n.title,
                "class": n.class,
                "meta_type": n.meta_type,
                "agent": n.agent,
                "created_at": n.created_at,
                "edges_out": n.edges_out.iter().map(|(t, id)| format!("{} → {}", t, &id[..8.min(id.len())])).collect::<Vec<_>>(),
                "edges_in": n.edges_in.iter().map(|(t, id)| format!("{} ← {}", t, &id[..8.min(id.len())])).collect::<Vec<_>>(),
            })
        }).collect();
        println!("{}", serde_json::to_string_pretty(&timeline).unwrap_or_default());
        return Ok(());
    }

    // Classify nodes
    let mut task_nodes = Vec::new();
    let mut impl_nodes = Vec::new();
    let mut verify_nodes = Vec::new();
    let mut escalation_nodes = Vec::new();

    for n in &sorted {
        if n.title.starts_with("Orchestration:") {
            task_nodes.push(n);
        } else if n.title.starts_with("Implemented:") || n.title.starts_with("Fixed:") {
            impl_nodes.push(n);
        } else if n.title.starts_with("Verified:") || n.title.starts_with("Verification failed:") {
            verify_nodes.push(n);
        } else if n.meta_type.as_deref() == Some("escalation") {
            escalation_nodes.push(n);
        }
    }

    // Compute outcome
    let outcome = if !escalation_nodes.is_empty() {
        "Escalated"
    } else if !verify_nodes.is_empty() && verify_nodes.iter().any(|n| n.title.starts_with("Verified:")) {
        "Verified"
    } else if !impl_nodes.is_empty() {
        "Implemented (unverified)"
    } else {
        "Incomplete"
    };

    // Compute duration string
    let duration_str = if sorted.len() >= 2 {
        let first = sorted.first().unwrap().created_at;
        let last = sorted.last().unwrap().created_at;
        let duration_s = (last - first) / 1000;
        let mins = duration_s / 60;
        let secs = duration_s % 60;
        format!("{}m {}s", mins, secs)
    } else {
        "n/a".to_string()
    };

    // Bounce count: number of impl nodes (each represents a coder bounce)
    let bounces = impl_nodes.len();

    if compact {
        // One-line summary: outcome | duration | bounces | task title
        println!("{} | {} | {} bounce{} | {}",
            outcome, duration_str,
            bounces, if bounces == 1 { "" } else { "s" },
            &task_title[..task_title.len().min(60)]);
        return Ok(());
    }

    // --- Full output ---

    println!("Trail: {} node(s)\n", trail.len());

    for n in &sorted {
        let ts = chrono::DateTime::from_timestamp_millis(n.created_at)
            .map(|dt| dt.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| "?".to_string());
        let id_short = &n.id[..8.min(n.id.len())];

        if n.title.starts_with("Orchestration:") {
            println!("[{}] {} TASK {} ({})", ts, "📋", &n.title[..n.title.len().min(60)], id_short);
        } else if n.title.starts_with("Implemented:") || n.title.starts_with("Fixed:") {
            println!("[{}] {} IMPL {} ({})", ts, "🔨", &n.title[..n.title.len().min(60)], id_short);
        } else if n.title.starts_with("Verified:") || n.title.starts_with("Verification failed:") {
            println!("[{}] {} VERIFY {} ({})", ts, "✅", &n.title[..n.title.len().min(60)], id_short);
        } else if n.meta_type.as_deref() == Some("escalation") {
            println!("[{}] {} ESCALATION {} ({})", ts, "⚠️", &n.title[..n.title.len().min(60)], id_short);
        } else {
            println!("[{}] {} {} {} ({})", ts, "  ", n.class, &n.title[..n.title.len().min(60)], id_short);
        }

        // Show edges
        for (etype, tid) in &n.edges_out {
            if trail.contains_key(tid) {
                let target_title = &trail[tid].title;
                println!("         → {} → {}",
                    etype, &target_title[..target_title.len().min(40)]);
            }
        }
        for (etype, sid) in &n.edges_in {
            if trail.contains_key(sid) {
                let source_title = &trail[sid].title;
                println!("         ← {} ← {}",
                    etype, &source_title[..source_title.len().min(40)]);
            }
        }
        println!();
    }

    // 4. Generate summary
    println!("--- Summary ---\n");

    let task_desc = if let Some(tn) = task_nodes.first() {
        // Extract task from content (after "## Task\n")
        if let Some(idx) = tn.content.find("## Task\n") {
            let rest = &tn.content[idx + 8..];
            let end = rest.find("\n\n## ").unwrap_or(rest.len());
            rest[..end].trim().to_string()
        } else {
            tn.title.replace("Orchestration: ", "")
        }
    } else {
        "Unknown".to_string()
    };

    println!("Task: {}", &task_desc[..task_desc.len().min(80)]);
    println!("Agents: {}", sorted.iter()
        .filter_map(|n| n.agent.as_deref())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", "));

    println!("Outcome: {}", outcome);
    println!("Duration: {}", duration_str);

    // Key content from impl nodes
    if !impl_nodes.is_empty() {
        println!("\nImplementation highlights:");
        for imp in &impl_nodes {
            // Extract "## What" section
            if let Some(idx) = imp.content.find("## What\n") {
                let rest = &imp.content[idx + 8..];
                let end = rest.find("\n\n## ").unwrap_or(rest.len());
                let what = rest[..end].trim();
                for line in what.lines().take(3) {
                    println!("  {}", line);
                }
            }
        }
    }

    if !escalation_nodes.is_empty() {
        println!("\nEscalation reason: max bounces reached without verifier verdict");
    }

    println!("\nNodes in trail: {} task, {} impl, {} verify, {} escalation",
        task_nodes.len(), impl_nodes.len(), verify_nodes.len(), escalation_nodes.len());

    Ok(())
}
