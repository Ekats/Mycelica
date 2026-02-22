use super::*;
use mycelica_lib::graph_analysis::{GraphSnapshot, AnalyzerConfig, analyze, AnalysisReport};

pub(crate) async fn handle_analyze(
    db: &Database,
    json: bool,
    region: Option<String>,
    top_n: usize,
    stale_days: i64,
    hub_threshold: usize,
) -> Result<(), String> {
    let snapshot = GraphSnapshot::from_db(db)?;

    let snapshot = if let Some(ref region_id) = region {
        snapshot.filter_to_region(region_id)
    } else {
        snapshot
    };

    let config = AnalyzerConfig {
        hub_threshold,
        top_n,
        stale_days,
    };

    let report = analyze(&snapshot, &config);

    if json {
        println!("{}", serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
    } else {
        print_human_readable(&report, &snapshot);
    }

    Ok(())
}

fn print_human_readable(report: &AnalysisReport, snapshot: &GraphSnapshot) {
    // Header with health score
    let health_bar = "█".repeat((report.health_score * 20.0) as usize);
    let health_empty = "░".repeat(20 - (report.health_score * 20.0) as usize);
    println!("\n  Graph Health: {:.0}%  [{}{}]", report.health_score * 100.0, health_bar, health_empty);
    println!("  breakdown: connectivity={:.2} components={:.2} staleness={:.2} fragility={:.2}\n",
        report.health_breakdown.connectivity,
        report.health_breakdown.components,
        report.health_breakdown.staleness,
        report.health_breakdown.fragility);

    // Topology section
    let t = &report.topology;
    println!("  TOPOLOGY");
    println!("  ────────────────────────────────────────");
    println!("  Nodes: {}  Edges: {}  Components: {}",
        t.total_nodes, t.total_edges, t.num_components);
    println!("  Largest component: {}  Smallest: {}",
        t.largest_component, t.smallest_component);

    if t.orphan_count > 0 {
        println!("  Orphans: {} disconnected nodes", t.orphan_count);
        // Show first few orphan IDs
        for id in t.orphan_ids.iter().take(5) {
            let title = snapshot.nodes.get(id).map(|n| n.title.as_str()).unwrap_or("?");
            println!("    - {} ({})", truncate_id(id), truncate_title(title, 50));
        }
        if t.orphan_count > 5 {
            println!("    ... and {} more", t.orphan_count - 5);
        }
    }

    // Degree distribution
    println!("\n  Degree distribution:");
    for (label, count) in &t.degree_histogram {
        if *count > 0 {
            let bar = "=".repeat((*count as f64).log2().ceil() as usize + 1);
            println!("    {:>5}: {:>4}  {}", label, count, bar);
        }
    }

    // Hubs
    if !t.hubs.is_empty() {
        println!("\n  Top hubs (degree > threshold):");
        for hub in &t.hubs {
            println!("    {} degree={} (in={}, out={})  {}",
                truncate_id(&hub.id), hub.degree, hub.in_degree, hub.out_degree,
                truncate_title(&hub.title, 40));
        }
    }

    // Staleness section
    let s = &report.staleness;
    if s.stale_node_count > 0 || s.stale_summary_count > 0 {
        println!("\n  STALENESS");
        println!("  ────────────────────────────────────────");

        if s.stale_node_count > 0 {
            println!("  {} stale nodes (old but recently referenced):", s.stale_node_count);
            for node in s.stale_nodes.iter().take(10) {
                println!("    {} {}d old, {} recent refs  {}",
                    truncate_id(&node.id),
                    node.days_since_update,
                    node.recent_reference_count,
                    truncate_title(&node.title, 40));
            }
        }

        if s.stale_summary_count > 0 {
            println!("  {} stale summaries (target updated after summary):", s.stale_summary_count);
            for ss in s.stale_summaries.iter().take(10) {
                println!("    {} -> {} ({}d drift)",
                    truncate_title(&ss.summary_title, 25),
                    truncate_title(&ss.target_title, 25),
                    ss.drift_days);
            }
        }
    }

    // Bridges section
    let b = &report.bridges;
    if b.ap_count > 0 || b.bridge_count > 0 || !b.fragile_connections.is_empty() {
        println!("\n  STRUCTURAL FRAGILITY");
        println!("  ────────────────────────────────────────");

        if b.ap_count > 0 {
            println!("  {} articulation points (removal disconnects graph):", b.ap_count);
            for ap in b.articulation_points.iter().take(10) {
                println!("    {} (degree ~{})  {}",
                    truncate_id(&ap.id),
                    ap.components_if_removed,
                    truncate_title(&ap.title, 40));
            }
        }

        if b.bridge_count > 0 {
            println!("  {} bridge edges (removal disconnects graph):", b.bridge_count);
            for be in b.bridge_edges.iter().take(10) {
                println!("    {} -> {}",
                    truncate_title(&be.source_title, 30),
                    truncate_title(&be.target_title, 30));
            }
        }

        if !b.fragile_connections.is_empty() {
            println!("  {} fragile inter-region connections (<=2 edges):", b.fragile_connections.len());
            for fc in b.fragile_connections.iter().take(10) {
                let ra_title = snapshot.nodes.get(&fc.region_a).map(|n| n.title.as_str()).unwrap_or(&fc.region_a);
                let rb_title = snapshot.nodes.get(&fc.region_b).map(|n| n.title.as_str()).unwrap_or(&fc.region_b);
                println!("    {} <-> {} ({} edge{})",
                    truncate_title(ra_title, 25),
                    truncate_title(rb_title, 25),
                    fc.cross_edges,
                    if fc.cross_edges != 1 { "s" } else { "" });
            }
        }
    }

    println!();
}

/// Truncate a node ID to first 8 characters for display. Uses floor_char_boundary for UTF-8 safety.
fn truncate_id(id: &str) -> &str {
    &id[..id.floor_char_boundary(8.min(id.len()))]
}

/// Truncate a title to max chars with ellipsis. Uses floor_char_boundary for UTF-8 safety.
fn truncate_title(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let byte_pos = s.floor_char_boundary(max);
        format!("{}...", &s[..byte_pos])
    }
}
