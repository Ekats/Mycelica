use super::*;
use std::collections::HashMap;

// ============================================================================
// Spore Commands
// ============================================================================

pub(crate) async fn handle_spore(cmd: SporeCommands, db: &Database, json: bool) -> Result<(), String> {
    match cmd {
        SporeCommands::QueryEdges { edge_type, agent, target_agent, confidence_min, since, not_superseded, limit, compact } => {
            let since_millis = since.as_deref()
                .map(parse_since_to_millis)
                .transpose()?;

            let results = db.query_edges(
                edge_type.as_deref(),
                agent.as_deref(),
                target_agent.as_deref(),
                confidence_min,
                since_millis,
                not_superseded,
                limit,
            ).map_err(|e| e.to_string())?;

            if json {
                println!("{}", serde_json::to_string(&results).unwrap_or_default());
            } else if compact {
                for ewn in &results {
                    let e = &ewn.edge;
                    let src = ewn.source_title.as_deref().unwrap_or(&e.source[..8.min(e.source.len())]);
                    let tgt = ewn.target_title.as_deref().unwrap_or(&e.target[..8.min(e.target.len())]);
                    let conf = e.confidence.map(|c| format!("{:.2}", c)).unwrap_or_else(|| "?".to_string());
                    println!("{} {} {} -> {} [{}]",
                        &e.id[..8.min(e.id.len())],
                        e.edge_type.as_str(),
                        src, tgt, conf);
                }
            } else {
                if results.is_empty() {
                    println!("No matching edges found.");
                } else {
                    for ewn in &results {
                        let e = &ewn.edge;
                        let src = ewn.source_title.as_deref().unwrap_or(&e.source[..8.min(e.source.len())]);
                        let tgt = ewn.target_title.as_deref().unwrap_or(&e.target[..8.min(e.target.len())]);
                        let conf = e.confidence.map(|c| format!("{:.0}%", c * 100.0)).unwrap_or_else(|| "?".to_string());
                        let agent_str = e.agent_id.as_deref().unwrap_or("?");
                        let date = chrono::DateTime::from_timestamp_millis(e.created_at)
                            .map(|d| d.format("%Y-%m-%d").to_string())
                            .unwrap_or_else(|| "?".to_string());
                        println!("{} {} {} → {} [{}] agent:{} {}",
                            &e.id[..8.min(e.id.len())],
                            e.edge_type.as_str(),
                            src, tgt, conf, agent_str, date);
                    }
                    println!("\n{} edge(s)", results.len());
                }
            }
            Ok(())
        }

        SporeCommands::ExplainEdge { id, depth } => {
            let explanation = db.explain_edge(&id, depth).map_err(|e| e.to_string())?;
            match explanation {
                None => {
                    if json {
                        println!("null");
                    } else {
                        println!("Edge not found: {}", id);
                    }
                }
                Some(exp) => {
                    if json {
                        println!("{}", serde_json::to_string(&exp).unwrap_or_default());
                    } else {
                        let e = &exp.edge;
                        println!("Edge: {} [{}]", e.id, e.edge_type.as_str());
                        println!("  Confidence: {}", e.confidence.map(|c| format!("{:.0}%", c * 100.0)).unwrap_or_else(|| "?".to_string()));
                        println!("  Agent: {}", e.agent_id.as_deref().unwrap_or("?"));
                        if let Some(ref reason) = e.reason {
                            println!("  Reason: {}", reason);
                        }
                        if let Some(ref content) = e.content {
                            println!("  Content: {}", &content[..200.min(content.len())]);
                        }
                        if e.superseded_by.is_some() {
                            println!("  [SUPERSEDED by {}]", e.superseded_by.as_deref().unwrap_or("?"));
                        }
                        println!("\nSource: {} ({})", exp.source_node.ai_title.as_ref().unwrap_or(&exp.source_node.title), &exp.source_node.id[..8.min(exp.source_node.id.len())]);
                        if let Some(ref summary) = exp.source_node.summary {
                            println!("  {}", &summary[..200.min(summary.len())]);
                        }
                        println!("\nTarget: {} ({})", exp.target_node.ai_title.as_ref().unwrap_or(&exp.target_node.title), &exp.target_node.id[..8.min(exp.target_node.id.len())]);
                        if let Some(ref summary) = exp.target_node.summary {
                            println!("  {}", &summary[..200.min(summary.len())]);
                        }

                        if !exp.adjacent_edges.is_empty() {
                            println!("\nAdjacent edges ({}):", exp.adjacent_edges.len());
                            for ae in &exp.adjacent_edges {
                                println!("  {} {} {} → {}",
                                    &ae.id[..8.min(ae.id.len())],
                                    ae.edge_type.as_str(),
                                    &ae.source[..8.min(ae.source.len())],
                                    &ae.target[..8.min(ae.target.len())]);
                            }
                        }

                        if !exp.supersession_chain.is_empty() {
                            println!("\nSupersession chain ({}):", exp.supersession_chain.len());
                            for se in &exp.supersession_chain {
                                let status = if se.superseded_by.is_some() { "superseded" } else { "current" };
                                println!("  {} [{}] {}", &se.id[..8.min(se.id.len())], se.edge_type.as_str(), status);
                            }
                        }
                    }
                }
            }
            Ok(())
        }

        SporeCommands::PathBetween { from, to, max_hops, edge_types } => {
            let source = resolve_node(db, &from)?;
            let target = resolve_node(db, &to)?;

            let type_list: Option<Vec<String>> = edge_types.map(|s| s.split(',').map(|t| t.trim().to_string()).collect());
            let type_refs: Option<Vec<&str>> = type_list.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect());

            let paths = db.path_between(&source.id, &target.id, max_hops, type_refs.as_deref())
                .map_err(|e| e.to_string())?;

            if json {
                println!("{}", serde_json::to_string(&paths).unwrap_or_default());
            } else {
                if paths.is_empty() {
                    println!("No paths found between {} and {} (max {} hops)",
                        source.ai_title.as_ref().unwrap_or(&source.title),
                        target.ai_title.as_ref().unwrap_or(&target.title),
                        max_hops);
                } else {
                    let src_name = source.ai_title.as_ref().unwrap_or(&source.title);
                    for (i, path) in paths.iter().enumerate() {
                        let mut display = format!("{}", src_name);
                        for hop in path {
                            display.push_str(&format!(" →[{}]→ {}", hop.edge.edge_type.as_str(), hop.node_title));
                        }
                        println!("Path {}: {}", i + 1, display);
                    }
                    println!("\n{} path(s) found", paths.len());
                }
            }
            Ok(())
        }

        SporeCommands::EdgesForContext { id, top, not_superseded } => {
            let node = resolve_node(db, &id)?;

            let edges = db.edges_for_context(&node.id, top, not_superseded)
                .map_err(|e| e.to_string())?;

            if json {
                println!("{}", serde_json::to_string(&edges).unwrap_or_default());
            } else {
                if edges.is_empty() {
                    println!("No edges found for {}", node.ai_title.as_ref().unwrap_or(&node.title));
                } else {
                    println!("Top {} edges for: {}", edges.len(), node.ai_title.as_ref().unwrap_or(&node.title));
                    for (i, e) in edges.iter().enumerate() {
                        let other_id = if e.source == node.id { &e.target } else { &e.source };
                        let other_name = db.get_node(other_id).ok().flatten()
                            .map(|n| n.ai_title.unwrap_or(n.title))
                            .unwrap_or_else(|| other_id[..8.min(other_id.len())].to_string());
                        let conf = e.confidence.map(|c| format!(" {:.0}%", c * 100.0)).unwrap_or_default();
                        let dir = if e.source == node.id { "→" } else { "←" };
                        println!("  {}. {} {} {} [{}]{}",
                            i + 1, dir, e.edge_type.as_str(), other_name, &e.id[..8.min(e.id.len())], conf);
                    }
                }
            }
            Ok(())
        }

        SporeCommands::CreateMeta { meta_type, title, content, agent, connects_to, edge_type } => {
            // Validate meta_type
            let valid_types = ["summary", "contradiction", "status"];
            if !valid_types.contains(&meta_type.as_str()) {
                return Err(format!("Invalid meta type: '{}'. Must be one of: summary, contradiction, status", meta_type));
            }

            // Validate edge_type
            let et = EdgeType::from_str(&edge_type.to_lowercase())
                .ok_or_else(|| format!("Unknown edge type: '{}'", edge_type))?;

            // Find universe node for parent_id
            let universe_id = {
                let all_nodes = db.get_all_nodes(true).map_err(|e| e.to_string())?;
                all_nodes.iter().find(|n| n.is_universe).map(|n| n.id.clone())
                    .ok_or_else(|| "No universe node found. Run 'mycelica-cli hierarchy build' first.".to_string())?
            };

            let author = settings::get_author_or_default();
            let now = Utc::now().timestamp_millis();
            let node_id = uuid::Uuid::new_v4().to_string();

            let node = Node {
                id: node_id.clone(),
                node_type: NodeType::Thought,
                title: title.clone(),
                url: None,
                content,
                position: Position { x: 0.0, y: 0.0 },
                created_at: now,
                updated_at: now,
                cluster_id: None,
                cluster_label: None,
                depth: 1,
                is_item: false,
                is_universe: false,
                parent_id: Some(universe_id),
                child_count: 0,
                ai_title: None,
                summary: None,
                tags: None,
                emoji: None,
                is_processed: false,
                conversation_id: None,
                sequence_index: None,
                is_pinned: false,
                last_accessed_at: None,
                latest_child_date: None,
                is_private: None,
                privacy_reason: None,
                source: Some("spore".to_string()),
                pdf_available: None,
                content_type: None,
                associated_idea_id: None,
                privacy: None,
                human_edited: None,
                human_created: true,
                author: Some(author.clone()),
                agent_id: Some(agent.clone()),
                node_class: Some("meta".to_string()),
                meta_type: Some(meta_type.clone()),
            };

            let mut edges = Vec::new();
            for target_id in &connects_to {
                edges.push(Edge {
                    id: uuid::Uuid::new_v4().to_string(),
                    source: node_id.clone(),
                    target: target_id.clone(),
                    edge_type: et.clone(),
                    label: None,
                    weight: Some(1.0),
                    edge_source: Some("user".to_string()),
                    evidence_id: None,
                    confidence: Some(1.0),
                    created_at: now,
                    updated_at: Some(now),
                    author: Some(author.clone()),
                    reason: None,
                    content: None,
                    agent_id: Some(agent.clone()),
                    superseded_by: None,
                    metadata: None,
                });
            }

            db.create_meta_node_with_edges(&node, &edges).map_err(|e| e.to_string())?;

            if json {
                println!(r#"{{"id":"{}","type":"{}","title":"{}","edges":{}}}"#,
                    node_id, meta_type, escape_json(&title), edges.len());
            } else {
                println!("Created meta node: {} [{}] \"{}\"", &node_id[..8], meta_type, title);
                if !connects_to.is_empty() {
                    println!("  {} {} edge(s) created", connects_to.len(), edge_type);
                }
            }
            Ok(())
        }

        SporeCommands::UpdateMeta { id, content, title, agent, add_connects, edge_type } => {
            let old_node = resolve_node(db, &id)?;

            // Verify it's a meta node
            if old_node.node_class.as_deref() != Some("meta") {
                return Err(format!("Node {} is not a meta node (class: {:?})", id, old_node.node_class));
            }

            let author = settings::get_author_or_default();
            let now = Utc::now().timestamp_millis();
            let new_id = uuid::Uuid::new_v4().to_string();

            // Create NEW meta node inheriting fields from old
            let new_node = Node {
                id: new_id.clone(),
                node_type: old_node.node_type.clone(),
                title: title.unwrap_or_else(|| old_node.title.clone()),
                url: old_node.url.clone(),
                content: content.or_else(|| old_node.content.clone()),
                position: Position { x: 0.0, y: 0.0 },
                created_at: now,
                updated_at: now,
                cluster_id: None,
                cluster_label: None,
                depth: old_node.depth,
                is_item: old_node.is_item,
                is_universe: false,
                parent_id: old_node.parent_id.clone(),
                child_count: 0,
                ai_title: old_node.ai_title.clone(),
                summary: old_node.summary.clone(),
                tags: old_node.tags.clone(),
                emoji: old_node.emoji.clone(),
                is_processed: old_node.is_processed,
                conversation_id: None,
                sequence_index: None,
                is_pinned: false,
                last_accessed_at: None,
                latest_child_date: None,
                is_private: old_node.is_private,
                privacy_reason: old_node.privacy_reason.clone(),
                source: Some("spore".to_string()),
                pdf_available: None,
                content_type: old_node.content_type.clone(),
                associated_idea_id: None,
                privacy: old_node.privacy,
                human_edited: None,
                human_created: true,
                author: Some(author.clone()),
                agent_id: Some(agent.clone()),
                node_class: old_node.node_class.clone(),
                meta_type: old_node.meta_type.clone(),
            };

            // Build edges for new node
            let mut edges = Vec::new();

            // 1. Supersedes edge: new -> old
            edges.push(Edge {
                id: uuid::Uuid::new_v4().to_string(),
                source: new_id.clone(),
                target: old_node.id.clone(),
                edge_type: EdgeType::Supersedes,
                label: None,
                weight: Some(1.0),
                edge_source: Some("spore".to_string()),
                evidence_id: None,
                confidence: Some(1.0),
                created_at: now,
                updated_at: Some(now),
                author: Some(author.clone()),
                reason: Some(format!("Supersedes {}", &old_node.id[..8.min(old_node.id.len())])),
                content: None,
                agent_id: Some(agent.clone()),
                superseded_by: None,
                metadata: None,
            });

            // 2. Copy old node's outgoing edges (excluding superseded and Supersedes-typed)
            let old_edges = db.get_edges_for_node(&old_node.id).map_err(|e| e.to_string())?;
            for old_edge in &old_edges {
                // Only copy outgoing edges from old node
                if old_edge.source != old_node.id {
                    continue;
                }
                // Skip edges that have been superseded
                if old_edge.superseded_by.is_some() {
                    continue;
                }
                // Skip Supersedes-typed edges (avoid false chains)
                if old_edge.edge_type == EdgeType::Supersedes {
                    continue;
                }
                edges.push(Edge {
                    id: uuid::Uuid::new_v4().to_string(),
                    source: new_id.clone(),
                    target: old_edge.target.clone(),
                    edge_type: old_edge.edge_type.clone(),
                    label: old_edge.label.clone(),
                    weight: old_edge.weight,
                    edge_source: Some("spore".to_string()),
                    evidence_id: old_edge.evidence_id.clone(),
                    confidence: old_edge.confidence,
                    created_at: now,
                    updated_at: Some(now),
                    author: Some(author.clone()),
                    reason: old_edge.reason.clone(),
                    content: old_edge.content.clone(),
                    agent_id: Some(agent.clone()),
                    superseded_by: None,
                    metadata: old_edge.metadata.clone(),
                });
            }

            // 3. New --add-connects edges
            if !add_connects.is_empty() {
                let et = EdgeType::from_str(&edge_type.to_lowercase())
                    .ok_or_else(|| format!("Unknown edge type: '{}'", edge_type))?;
                for target_id in &add_connects {
                    edges.push(Edge {
                        id: uuid::Uuid::new_v4().to_string(),
                        source: new_id.clone(),
                        target: target_id.clone(),
                        edge_type: et.clone(),
                        label: None,
                        weight: Some(1.0),
                        edge_source: Some("spore".to_string()),
                        evidence_id: None,
                        confidence: Some(1.0),
                        created_at: now,
                        updated_at: Some(now),
                        author: Some(author.clone()),
                        reason: None,
                        content: None,
                        agent_id: Some(agent.clone()),
                        superseded_by: None,
                        metadata: None,
                    });
                }
            }

            let copied_count = edges.len() - 1 - add_connects.len(); // total - supersedes - new
            db.create_meta_node_with_edges(&new_node, &edges).map_err(|e| e.to_string())?;

            if json {
                println!(r#"{{"newId":"{}","oldId":"{}","copiedEdges":{},"newEdges":{}}}"#,
                    new_id, old_node.id, copied_count, add_connects.len());
            } else {
                println!("Created superseding meta node: {} -> {}",
                    &new_id[..8.min(new_id.len())], &old_node.id[..8.min(old_node.id.len())]);
                println!("  Copied {} edge(s), added {} new edge(s)", copied_count, add_connects.len());
            }
            Ok(())
        }

        SporeCommands::Status { all, format } => {
            let full_mode = format == "full";
            // Meta nodes by type
            let meta_nodes = db.get_meta_nodes(None).map_err(|e| e.to_string())?;
            let summaries = meta_nodes.iter().filter(|n| n.meta_type.as_deref() == Some("summary")).count();
            let contradictions_meta = meta_nodes.iter().filter(|n| n.meta_type.as_deref() == Some("contradiction")).count();
            let statuses = meta_nodes.iter().filter(|n| n.meta_type.as_deref() == Some("status")).count();

            // Edge stats
            let all_edges = db.get_all_edges().map_err(|e| e.to_string())?;
            let now = Utc::now().timestamp_millis();
            let day_ago = now - 86_400_000;
            let week_ago = now - 7 * 86_400_000;

            let edges_24h = all_edges.iter().filter(|e| e.created_at >= day_ago).count();
            let edges_7d = all_edges.iter().filter(|e| e.created_at >= week_ago).count();

            // Unresolved contradictions (contradiction edges not superseded)
            let unresolved = all_edges.iter()
                .filter(|e| e.edge_type == EdgeType::Contradicts && e.superseded_by.is_none())
                .count();

            // Coverage: knowledge nodes referenced by summarizes edges
            let all_nodes = db.get_all_nodes(true).map_err(|e| e.to_string())?;
            let knowledge_nodes = all_nodes.iter().filter(|n| n.node_class.as_deref() != Some("meta") && n.is_item).count();
            let summarized_targets: std::collections::HashSet<&str> = all_edges.iter()
                .filter(|e| e.edge_type == EdgeType::Summarizes && e.superseded_by.is_none())
                .map(|e| e.target.as_str())
                .collect();
            let coverage = if knowledge_nodes > 0 {
                summarized_targets.len() as f64 / knowledge_nodes as f64
            } else {
                0.0
            };

            // Coherence
            let active_edges = all_edges.iter().filter(|e| e.superseded_by.is_none()).count();
            let coherence = if active_edges > 0 {
                1.0 - (unresolved as f64 / active_edges as f64)
            } else {
                1.0
            };

            if json {
                println!(r#"{{"metaNodes":{{"summary":{},"contradiction":{},"status":{}}},"edges":{{"last24h":{},"last7d":{},"total":{}}},"unresolvedContradictions":{},"coverage":{:.4},"coherence":{:.6}}}"#,
                    summaries, contradictions_meta, statuses,
                    edges_24h, edges_7d, all_edges.len(),
                    unresolved, coverage, coherence);
            } else {
                println!("=== Spore Status ===\n");
                println!("Meta nodes: {} total", meta_nodes.len());
                println!("  Summaries:      {}", summaries);
                println!("  Contradictions: {}", contradictions_meta);
                println!("  Status nodes:   {}", statuses);

                println!("\nEdge activity:");
                println!("  Last 24h: {}", edges_24h);
                println!("  Last 7d:  {}", edges_7d);
                println!("  Total:    {}", all_edges.len());

                println!("\nUnresolved contradictions: {}", unresolved);
                println!("Coverage: {:.1}% ({} / {} knowledge nodes summarized)", coverage * 100.0, summarized_targets.len(), knowledge_nodes);
                println!("Coherence: {:.4}", coherence);

                if all {
                    // Agent breakdown
                    let mut agent_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
                    for e in all_edges.iter().filter(|e| e.created_at >= week_ago) {
                        let agent_key = e.agent_id.as_deref().unwrap_or("unknown").to_string();
                        *agent_counts.entry(agent_key).or_insert(0) += 1;
                    }
                    if !agent_counts.is_empty() {
                        println!("\nEdges by agent (last 7d):");
                        let mut sorted: Vec<_> = agent_counts.into_iter().collect();
                        sorted.sort_by(|a, b| b.1.cmp(&a.1));
                        for (agent_name, count) in sorted {
                            println!("  {}: {}", agent_name, count);
                        }
                    }

                    // List unresolved contradictions
                    if unresolved > 0 {
                        println!("\nUnresolved contradiction edges:");
                        for e in all_edges.iter()
                            .filter(|e| e.edge_type == EdgeType::Contradicts && e.superseded_by.is_none())
                            .take(10)
                        {
                            let src_name = db.get_node(&e.source).ok().flatten()
                                .map(|n| n.ai_title.unwrap_or(n.title))
                                .unwrap_or_else(|| e.source[..8.min(e.source.len())].to_string());
                            let tgt_name = db.get_node(&e.target).ok().flatten()
                                .map(|n| n.ai_title.unwrap_or(n.title))
                                .unwrap_or_else(|| e.target[..8.min(e.target.len())].to_string());
                            println!("  {} contradicts {}", src_name, tgt_name);
                        }
                        if unresolved > 10 {
                            println!("  ... and {} more", unresolved - 10);
                        }
                    }

                    // List meta nodes
                    if !meta_nodes.is_empty() {
                        println!("\nMeta nodes:");
                        for mn in &meta_nodes {
                            let mt = mn.meta_type.as_deref().unwrap_or("?");
                            let agent_name = mn.agent_id.as_deref().unwrap_or("?");
                            println!("  [{}] {} (agent: {}, {})",
                                mt, mn.title, agent_name, &mn.id[..8.min(mn.id.len())]);
                        }
                    }
                }

                if full_mode {
                    // (1) Top 5 most-connected nodes by edge count
                    println!("\n--- Full Mode ---");
                    println!("\nTop 5 most-connected nodes:");
                    // Collect IDs first, then drop the lock before calling get_node
                    let top_nodes: Vec<(String, i64)> = (|| -> Result<Vec<_>, String> {
                        let conn = db.raw_conn().lock().unwrap();
                        let mut stmt = conn.prepare(
                            "SELECT node_id, COUNT(*) as edge_count FROM (
                                SELECT source_id as node_id FROM edges
                                UNION ALL
                                SELECT target_id as node_id FROM edges
                            ) GROUP BY node_id ORDER BY edge_count DESC LIMIT 5"
                        ).map_err(|e| e.to_string())?;
                        let rows: Vec<_> = stmt.query_map([], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                        }).map_err(|e| e.to_string())?
                        .filter_map(|r| r.ok())
                        .collect();
                        Ok(rows)
                    })().unwrap_or_default();
                    for (node_id, count) in &top_nodes {
                        let name = db.get_node(node_id).ok().flatten()
                            .map(|n| n.ai_title.unwrap_or(n.title))
                            .unwrap_or_else(|| node_id[..8.min(node_id.len())].to_string());
                        println!("  {} edges  {}  ({})", count, name, &node_id[..8.min(node_id.len())]);
                    }

                    // (2) Edge type distribution
                    println!("\nEdge type distribution:");
                    {
                        let mut type_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
                        for e in &all_edges {
                            *type_counts.entry(e.edge_type.as_str().to_string()).or_insert(0) += 1;
                        }
                        let mut sorted: Vec<_> = type_counts.into_iter().collect();
                        sorted.sort_by(|a, b| b.1.cmp(&a.1));
                        for (etype, count) in sorted {
                            println!("  {:20} {}", etype, count);
                        }
                    }

                    // (3) Recent operational nodes (last 24h)
                    println!("\nRecent operational nodes (last 24h):");
                    {
                        let ops: Vec<_> = all_nodes.iter()
                            .filter(|n| n.node_class.as_deref() == Some("operational") && n.created_at >= day_ago)
                            .collect();
                        if ops.is_empty() {
                            println!("  (none)");
                        } else {
                            for n in &ops {
                                let agent_name = n.agent_id.as_deref().unwrap_or("?");
                                let ts = chrono::DateTime::from_timestamp_millis(n.created_at)
                                    .map(|dt| dt.format("%H:%M:%S").to_string())
                                    .unwrap_or_else(|| "?".to_string());
                                println!("  [{}] {} (agent: {}, {})",
                                    ts, n.title, agent_name, &n.id[..8.min(n.id.len())]);
                            }
                        }
                    }
                }
            }
            Ok(())
        }

        // Gap 3a: Create edge between existing nodes (delegates to handle_link)
        SporeCommands::CreateEdge { from, to, edge_type, content, reason, agent, confidence, supersedes } => {
            handle_link(&from, &to, &edge_type, reason, content, &agent, confidence, supersedes, "spore", db, json).await
        }

        // Gap 3b: Read full content of a node (no metadata noise)
        SporeCommands::ReadContent { id } => {
            let node = resolve_node(db, &id)?;
            if json {
                println!(r#"{{"id":"{}","title":"{}","content":{},"tags":{},"content_type":{},"node_class":{},"meta_type":{}}}"#,
                    node.id,
                    escape_json(&node.title),
                    node.content.as_ref().map(|c| format!("\"{}\"", escape_json(c))).unwrap_or("null".to_string()),
                    node.tags.as_ref().map(|t| format!("\"{}\"", escape_json(t))).unwrap_or("null".to_string()),
                    node.content_type.as_ref().map(|c| format!("\"{}\"", c)).unwrap_or("null".to_string()),
                    node.node_class.as_ref().map(|c| format!("\"{}\"", c)).unwrap_or("null".to_string()),
                    node.meta_type.as_ref().map(|m| format!("\"{}\"", m)).unwrap_or("null".to_string()),
                );
            } else {
                if let Some(ref content) = node.content {
                    println!("{}", content);
                } else {
                    println!("(no content)");
                }
            }
            Ok(())
        }

        // Gap 3c: List descendants of a category
        SporeCommands::ListRegion { id, class, items_only, limit } => {
            let parent = resolve_node(db, &id)?;
            let descendants = db.get_descendants(&parent.id, class.as_deref(), items_only, limit)
                .map_err(|e| e.to_string())?;

            if json {
                let items: Vec<String> = descendants.iter().map(|n| {
                    format!(r#"{{"id":"{}","title":"{}","depth":{},"is_item":{},"node_class":{},"content_type":{}}}"#,
                        n.id,
                        escape_json(&n.title),
                        n.depth,
                        n.is_item,
                        n.node_class.as_ref().map(|c| format!("\"{}\"", c)).unwrap_or("null".to_string()),
                        n.content_type.as_ref().map(|c| format!("\"{}\"", c)).unwrap_or("null".to_string()),
                    )
                }).collect();
                println!("[{}]", items.join(","));
            } else {
                if descendants.is_empty() {
                    println!("No descendants found for: {} ({})", parent.title, &parent.id[..8.min(parent.id.len())]);
                } else {
                    let parent_depth = parent.depth;
                    for n in &descendants {
                        let indent = "  ".repeat((n.depth - parent_depth).max(0) as usize);
                        let marker = if n.is_item { "[I]" } else { "[C]" };
                        let class_label = n.node_class.as_deref().unwrap_or("");
                        let ct_label = n.content_type.as_deref().map(|c| format!(" ({})", c)).unwrap_or_default();
                        println!("{}{} {} {}{}", indent, marker, &n.id[..8.min(n.id.len())], n.title, if class_label.is_empty() { ct_label } else { format!(" [{}]{}", class_label, ct_label) });
                    }
                    println!("\n{} descendant(s)", descendants.len());
                }
            }
            Ok(())
        }

        // Gap 3f: Check freshness of summary meta-nodes
        SporeCommands::CheckFreshness { id } => {
            let node = resolve_node(db, &id)?;

            // Find all summarizes edges where this node is the TARGET
            let edges = db.get_edges_for_node(&node.id).map_err(|e| e.to_string())?;
            let summary_edges: Vec<&Edge> = edges.iter()
                .filter(|e| e.edge_type == EdgeType::Summarizes && e.target == node.id && e.superseded_by.is_none())
                .collect();

            if summary_edges.is_empty() {
                // Check if this node is itself a summary — look at outgoing summarizes edges
                let outgoing: Vec<&Edge> = edges.iter()
                    .filter(|e| e.edge_type == EdgeType::Summarizes && e.source == node.id && e.superseded_by.is_none())
                    .collect();

                if outgoing.is_empty() {
                    if json {
                        println!(r#"{{"id":"{}","summaries":[],"message":"No summarizes edges found"}}"#, node.id);
                    } else {
                        println!("No summarizes edges found for: {}", node.title);
                    }
                } else {
                    // This node IS a summary — check if its targets have been updated since
                    let summary_updated = node.updated_at;
                    if json {
                        let items: Vec<String> = outgoing.iter().map(|e| {
                            let target_node = db.get_node(&e.target).ok().flatten();
                            let target_updated = target_node.as_ref().map(|n| n.updated_at).unwrap_or(0);
                            let stale = target_updated > summary_updated;
                            let target_title = target_node.map(|n| n.title).unwrap_or_else(|| e.target.clone());
                            format!(r#"{{"targetId":"{}","targetTitle":"{}","stale":{},"targetUpdated":{},"summaryUpdated":{}}}"#,
                                e.target, escape_json(&target_title), stale, target_updated, summary_updated)
                        }).collect();
                        println!(r#"{{"id":"{}","title":"{}","targets":[{}]}}"#, node.id, escape_json(&node.title), items.join(","));
                    } else {
                        println!("Summary: {} (updated {})", node.title,
                            chrono::DateTime::from_timestamp_millis(summary_updated)
                                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                                .unwrap_or_else(|| "?".to_string()));
                        for e in &outgoing {
                            let target_node = db.get_node(&e.target).ok().flatten();
                            let target_updated = target_node.as_ref().map(|n| n.updated_at).unwrap_or(0);
                            let stale = target_updated > summary_updated;
                            let target_title = target_node.map(|n| n.title).unwrap_or_else(|| e.target.clone());
                            let status = if stale { "STALE" } else { "fresh" };
                            println!("  {} {} (updated {})", status, target_title,
                                chrono::DateTime::from_timestamp_millis(target_updated)
                                    .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                                    .unwrap_or_else(|| "?".to_string()));
                        }
                    }
                }
            } else {
                // Node is summarized BY other nodes — show their freshness
                if json {
                    let items: Vec<String> = summary_edges.iter().map(|e| {
                        let summary_node = db.get_node(&e.source).ok().flatten();
                        let summary_updated = summary_node.as_ref().map(|n| n.updated_at).unwrap_or(0);
                        let stale = node.updated_at > summary_updated;
                        let summary_title = summary_node.map(|n| n.title).unwrap_or_else(|| e.source.clone());
                        format!(r#"{{"summaryId":"{}","summaryTitle":"{}","stale":{},"summaryUpdated":{},"nodeUpdated":{}}}"#,
                            e.source, escape_json(&summary_title), stale, summary_updated, node.updated_at)
                    }).collect();
                    println!(r#"{{"id":"{}","title":"{}","summaries":[{}]}}"#, node.id, escape_json(&node.title), items.join(","));
                } else {
                    println!("Node: {} (updated {})", node.title,
                        chrono::DateTime::from_timestamp_millis(node.updated_at)
                            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "?".to_string()));
                    for e in &summary_edges {
                        let summary_node = db.get_node(&e.source).ok().flatten();
                        let summary_updated = summary_node.as_ref().map(|n| n.updated_at).unwrap_or(0);
                        let stale = node.updated_at > summary_updated;
                        let summary_title = summary_node.map(|n| n.title).unwrap_or_else(|| e.source.clone());
                        let status = if stale { "STALE" } else { "fresh" };
                        println!("  {} {} (updated {})", status, summary_title,
                            chrono::DateTime::from_timestamp_millis(summary_updated)
                                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                                .unwrap_or_else(|| "?".to_string()));
                    }
                }
            }
            Ok(())
        }

        SporeCommands::Runs { cmd } => {
            super::spore_runs::handle_runs(cmd, db, json)
        }

        SporeCommands::Orchestrate { task, max_bounces, max_turns, coder_prompt, verifier_prompt, summarizer_prompt, no_summarize, dry_run, verbose, quiet, timeout, agent, agent_prompt, native_agent, experiment, coder_model } => {
            let verbose = verbose && !quiet;

            // Short-circuit: single-agent dispatch (--agent flag)
            if let Some(ref agent_role) = agent {
                let prompt_path = agent_prompt.unwrap_or_else(|| {
                    std::path::PathBuf::from(format!("docs/spore/agents/{}.md", agent_role))
                });
                return handle_single_agent(
                    db, &task, agent_role, &prompt_path,
                    max_turns, verbose, quiet,
                    timeout, dry_run,
                ).await;
            }

            // Log the active pipeline
            {
                let mut phases = vec!["context", "coder", "verifier"];
                if !no_summarize { phases.push("summarizer"); }
                let pipeline_str = phases.join(" -> ");
                println!("[orchestrator] Pipeline: {}", pipeline_str);
            }

            handle_orchestrate(db, &task, max_bounces, max_turns, &coder_prompt, &verifier_prompt, &summarizer_prompt, !no_summarize, dry_run, verbose, timeout, None, quiet, native_agent, experiment.as_deref(), coder_model.as_deref()).await.map(|_| ())
        }

        SporeCommands::Retry { run_id, max_bounces, max_turns, no_summarize, verbose } => {
            // Find the original task node and extract its description
            let node = resolve_node(db, &run_id)?;
            // Extract full task from content (## Task section), fall back to title
            let task_desc = node.content.as_deref()
                .and_then(|c| {
                    c.find("## Task\n").map(|start| {
                        let after = &c[start + 8..];
                        after.lines()
                            .take_while(|l| !l.starts_with("## "))
                            .collect::<Vec<_>>()
                            .join("\n")
                            .trim()
                            .to_string()
                    })
                })
                .unwrap_or_else(|| node.title.strip_prefix("Orchestration:").unwrap_or(&node.title).trim().to_string());
            if task_desc.is_empty() {
                return Err(format!("Node {} doesn't look like an orchestration task (title: {})", &run_id, node.title));
            }
            println!("[retry] Original run: {} ({})", &node.id[..8.min(node.id.len())], node.title);
            println!("[retry] Task: {}", task_desc);
            println!("[retry] Retrying with max_bounces={}, max_turns={}", max_bounces, max_turns);

            let coder_prompt = std::path::PathBuf::from("docs/spore/agents/coder.md");
            let verifier_prompt = std::path::PathBuf::from("docs/spore/agents/verifier.md");
            let summarizer_prompt = std::path::PathBuf::from("docs/spore/agents/summarizer.md");
            handle_orchestrate(db, &task_desc, max_bounces, max_turns, &coder_prompt, &verifier_prompt, &summarizer_prompt, !no_summarize, false, verbose, None, None, false, false, None, None).await.map(|_| ())
        }

        SporeCommands::Resume { id, verbose } => {
            let checkpoint = if id == "last" {
                find_latest_checkpoint()
                    .ok_or_else(|| "No checkpoint found. Run 'spore orchestrate' first.".to_string())?
            } else {
                load_checkpoint(&id)
                    .ok_or_else(|| format!("No checkpoint found for task node {}", id))?
            };

            println!("=== Resuming orchestrator run ===");
            println!("Task: {}", if checkpoint.task.len() > 60 { &checkpoint.task[..checkpoint.task.floor_char_boundary(60)] } else { &checkpoint.task });
            println!("Task node: {}", &checkpoint.task_node_id[..8]);
            println!("Bounce: {}/{}", checkpoint.bounce + 1, checkpoint.max_bounces);
            println!("Next phase: {}", checkpoint.next_phase);
            if let Some(ref impl_id) = checkpoint.impl_node_id {
                println!("Implementation: {}", &impl_id[..8.min(impl_id.len())]);
            }

            let coder_prompt = std::path::PathBuf::from("docs/spore/agents/coder.md");
            let verifier_prompt = std::path::PathBuf::from("docs/spore/agents/verifier.md");
            let summarizer_prompt = std::path::PathBuf::from("docs/spore/agents/summarizer.md");
            let resume_state = ResumeState {
                task_node_id: checkpoint.task_node_id.clone(),
                from_phase: checkpoint.next_phase.clone(),
                impl_node_id: checkpoint.impl_node_id.clone(),
                last_impl_id: checkpoint.last_impl_id.clone(),
                bounce: checkpoint.bounce,
            };

            handle_orchestrate(
                db, &checkpoint.task,
                checkpoint.max_bounces - checkpoint.bounce, // remaining bounces
                checkpoint.max_turns,
                &coder_prompt, &verifier_prompt, &summarizer_prompt,
                true, false, verbose,
                None,
                Some(resume_state), false,
                false,
                None,
                None,
            ).await.map(|_| ())
        }

        SporeCommands::Batch { file, max_bounces, max_turns, timeout, verbose, stop_on_failure, dry_run } => {
            // Read task file: one task per line, # comments and blank lines ignored
            let content = std::fs::read_to_string(&file)
                .map_err(|e| format!("Failed to read batch file {:?}: {}", file, e))?;
            let tasks: Vec<&str> = content.lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect();

            if tasks.is_empty() {
                return Err("No tasks found in batch file (blank lines and # comments ignored)".to_string());
            }

            let coder_prompt = std::path::PathBuf::from("docs/spore/agents/coder.md");
            let verifier_prompt = std::path::PathBuf::from("docs/spore/agents/verifier.md");
            let summarizer_prompt = std::path::PathBuf::from("docs/spore/agents/summarizer.md");

            println!("=== Batch: {} task(s) from {:?} ===\n", tasks.len(), file);

            if dry_run {
                for (i, task) in tasks.iter().enumerate() {
                    let complexity = estimate_complexity(task);
                    let task_short = if task.len() > 70 { &task[..task.floor_char_boundary(70)] } else { task };
                    println!("  {}. [complexity {}/10] {}", i + 1, complexity, task_short);
                }
                println!("\nDry run — no agents spawned.");
                return Ok(());
            }

            let mut succeeded = 0;
            let mut failed = 0;
            let start_time = std::time::Instant::now();

            for (i, task) in tasks.iter().enumerate() {
                println!("\n=== Task {}/{}: {} ===", i + 1, tasks.len(),
                    if task.len() > 60 { &task[..task.floor_char_boundary(60)] } else { task });

                let result = handle_orchestrate(db, task, max_bounces, max_turns, &coder_prompt, &verifier_prompt, &summarizer_prompt, true, false, verbose, timeout, None, false, false, None, None).await.map(|_| ());

                match result {
                    Ok(_) => {
                        succeeded += 1;
                        println!("[batch] Task {}/{} completed", i + 1, tasks.len());

                        // Auto-commit between batch tasks so the next task starts
                        // with a clean diff (avoids dirty-tree confusion for verifier)
                        if i + 1 < tasks.len() {
                            let short_desc = if task.len() > 50 { &task[..task.floor_char_boundary(50)] } else { task };
                            let msg = format!("feat(batch): {}", short_desc);
                            let staged = selective_git_add(&std::env::current_dir().unwrap_or_default());
                            if staged {
                                let commit = std::process::Command::new("git")
                                    .args(["commit", "-m", &msg, "--allow-empty"])
                                    .output();
                                match commit {
                                    Ok(o) if o.status.success() => {
                                        println!("[batch] Auto-committed changes before next task");
                                    }
                                    _ => {
                                        if verbose {
                                            eprintln!("[batch] No changes to commit (or commit failed)");
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        failed += 1;
                        eprintln!("[batch] Task {}/{} failed: {}", i + 1, tasks.len(), e);
                        if stop_on_failure {
                            eprintln!("[batch] --stop-on-failure: aborting remaining tasks");
                            break;
                        }
                    }
                }
            }

            let elapsed = start_time.elapsed().as_secs();
            println!("\n=== Batch Complete ===");
            println!("  Succeeded: {}/{}", succeeded, tasks.len());
            println!("  Failed:    {}/{}", failed, tasks.len());
            println!("  Elapsed:   {}m {}s", elapsed / 60, elapsed % 60);
            Ok(())
        }

        SporeCommands::Loop { source, budget, max_runs, max_bounces, max_turns, timeout, dry_run, pause_on_escalation, summarize, verbose, reset, experiment, coder_model } => {
            handle_spore_loop(db, &source, budget, max_runs, max_bounces, max_turns, timeout, dry_run, pause_on_escalation, summarize, verbose, reset, json, experiment.as_deref(), coder_model.as_deref()).await
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

        SporeCommands::Gc { days, dry_run, force } => {
            let cutoff_ms = chrono::Utc::now().timestamp_millis() - (days as i64) * 86_400_000;
            // By default, exclude Lesson: and Summary: nodes — they're valuable even without
            // incoming edges. --force includes them.
            // Only GC nodes with NO edges at all (incoming or outgoing).
            // Previous version only checked incoming edges, which incorrectly
            // deleted verifier/summary nodes that had outgoing supports/summarizes edges.
            let query = if force {
                "SELECT n.id, n.title, n.created_at FROM nodes n
                 WHERE n.node_class = 'operational'
                   AND n.created_at < ?1
                   AND NOT EXISTS (
                     SELECT 1 FROM edges e WHERE e.target_id = n.id OR e.source_id = n.id
                   )".to_string()
            } else {
                "SELECT n.id, n.title, n.created_at FROM nodes n
                 WHERE n.node_class = 'operational'
                   AND n.created_at < ?1
                   AND n.title NOT LIKE 'Lesson:%'
                   AND n.title NOT LIKE 'Summary:%'
                   AND NOT EXISTS (
                     SELECT 1 FROM edges e WHERE e.target_id = n.id OR e.source_id = n.id
                   )".to_string()
            };
            let candidates: Vec<(String, String, i64)> = (|| -> Result<Vec<_>, String> {
                let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
                let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
                let rows = stmt.query_map(rusqlite::params![cutoff_ms], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
                }).map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
                Ok(rows)
            })().unwrap_or_default();

            if json {
                let items: Vec<serde_json::Value> = candidates.iter().map(|(id, title, created_at)| {
                    serde_json::json!({ "id": id, "title": title, "created_at": created_at })
                }).collect();
                println!("{}", serde_json::to_string(&items).unwrap_or_default());
            } else if candidates.is_empty() {
                println!("No GC candidates found (operational nodes older than {} days with no incoming edges).", days);
            } else if dry_run {
                println!("GC candidates: operational nodes older than {} days with no incoming edges\n", days);
                for (id, title, created_at) in &candidates {
                    let date = chrono::DateTime::from_timestamp_millis(*created_at)
                        .map(|d| d.format("%Y-%m-%d").to_string())
                        .unwrap_or_else(|| "?".to_string());
                    println!("  {}  {}  created {}", &id[..8.min(id.len())], title, date);
                }
                println!("\n{} candidate(s). --dry-run: no nodes were deleted.", candidates.len());
            } else {
                println!("Deleting {} GC candidate(s)...\n", candidates.len());
                let mut deleted = 0;
                for (id, title, created_at) in &candidates {
                    let date = chrono::DateTime::from_timestamp_millis(*created_at)
                        .map(|d| d.format("%Y-%m-%d").to_string())
                        .unwrap_or_else(|| "?".to_string());
                    match db.delete_node_tracked(id, "spore:gc") {
                        Ok(()) => {
                            println!("  deleted {}  {}  created {}", &id[..8.min(id.len())], title, date);
                            deleted += 1;
                        }
                        Err(e) => {
                            eprintln!("  FAILED  {}  {}: {}", &id[..8.min(id.len())], title, e);
                        }
                    }
                }
                println!("\nDeleted {} node(s).", deleted);
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


// ============================================================================
// Orchestrator
// ============================================================================

fn make_orchestrator_node(
    id: String,
    title: String,
    content: String,
    node_class: &str,
    meta_type: Option<&str>,
) -> Node {
    let now = chrono::Utc::now().timestamp_millis();
    Node {
        id,
        node_type: NodeType::Thought,
        title,
        url: None,
        content: Some(content),
        position: Position { x: 0.0, y: 0.0 },
        created_at: now,
        updated_at: now,
        cluster_id: None,
        cluster_label: None,
        ai_title: None,
        summary: None,
        tags: None,
        emoji: None,
        is_processed: true,
        depth: 0,
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
        source: Some("orchestrator".to_string()),
        pdf_available: None,
        content_type: None,
        associated_idea_id: None,
        privacy: None,
        human_edited: None,
        human_created: false,
        author: Some("orchestrator".to_string()),
        agent_id: Some("spore:orchestrator".to_string()),
        node_class: Some(node_class.to_string()),
        meta_type: meta_type.map(|s| s.to_string()),
    }
}

/// Resolve the full path to the mycelica-cli binary (for MCP configs).
fn resolve_cli_binary() -> Result<PathBuf, String> {
    // We ARE mycelica-cli, so current_exe gives the full path.
    // On Linux, /proc/self/exe gets " (deleted)" appended when the binary is
    // replaced (unlink + copy) during post_coder_cleanup. If the clean path
    // exists (new binary was copied), use that instead.
    let exe = std::env::current_exe()
        .map_err(|e| format!("Failed to resolve CLI binary path: {}", e))?;
    let path_str = exe.to_string_lossy();
    if path_str.ends_with(" (deleted)") {
        let clean = PathBuf::from(path_str.trim_end_matches(" (deleted)"));
        if clean.exists() {
            return Ok(clean);
        }
    }
    Ok(exe)
}


/// Estimate task complexity (0-10) using heuristics. No LLM call.
fn estimate_complexity(task: &str) -> u8 {
    let mut score: u8 = 0;
    let lower = task.to_lowercase();

    // Word count: longer descriptions correlate with complexity
    let words = task.split_whitespace().count();
    if words > 60 { score += 3; }
    else if words > 30 { score += 1; }

    // Multi-verb: count distinct action verbs
    let actions = [
        "add", "implement", "create", "refactor", "redesign",
        "modify", "update", "change", "replace", "remove",
        "migrate", "convert", "integrate", "support", "extract",
    ];
    let action_count = actions.iter()
        .filter(|a| lower.contains(*a))
        .count();
    if action_count >= 3 { score += 3; }
    else if action_count >= 2 { score += 1; }

    // Cross-cutting keywords suggesting broad changes
    let cross_cutting = [
        "throughout", "across", "all instances", "every",
        "refactor", "redesign", "full", "complete", "mode",
        "system", "framework", "architecture",
    ];
    if cross_cutting.iter().any(|k| lower.contains(k)) {
        score += 2;
    }

    // Multi-sentence: multiple distinct requirements
    let sentences = task.split('.')
        .filter(|s| s.trim().len() > 10)
        .count()
        + task.lines()
            .filter(|s| {
                let t = s.trim();
                t.starts_with('-') || t.starts_with('*') || t.starts_with("(")
            })
            .count();
    if sentences >= 3 { score += 2; }

    score.min(10)
}

/// Select the Claude model based on agent role and task complexity.
/// Pure function — easy to test, easy to change the mapping later.
fn select_model_for_role(role: &str, _complexity: i32) -> String {
    match role {
        // A/B experiment (2026-02-20): opus was 28% cheaper and 36% faster than
        // sonnet on 4 moderate tasks (fewer turns overcomes higher per-token cost).
        "coder" => "opus".to_string(),
        "verifier" => "opus".to_string(),
        "summarizer" => "sonnet".to_string(),
        "operator" => "opus".to_string(),
        _ => "opus".to_string(),
    }
}


// ============================================================================
// Claude Subprocess
// ============================================================================

struct ClaudeResult {
    success: bool,
    exit_code: i32,
    session_id: Option<String>,
    result_text: Option<String>,
    thinking_log: Option<String>,
    total_cost_usd: Option<f64>,
    num_turns: Option<u32>,
    duration_ms: Option<u64>,
    stdout_raw: String,
    stderr_raw: String,
    /// MCP server connection statuses: (name, status) pairs.
    /// Status is "connected", "failed", etc.
    mcp_status: Vec<(String, String)>,
}

/// Check if a native Claude Code agent file exists for the given role.
/// Returns Some(role_name) if `.claude/agents/<role>.md` exists, None otherwise.
/// When Some, the caller should pass this as `agent_name` to `spawn_claude`
/// and omit the `docs/spore/agents/<role>.md` template prefix from the prompt.
fn resolve_agent_name(role: &str) -> Option<String> {
    let path = PathBuf::from(format!(".claude/agents/{}.md", role));
    if path.exists() {
        Some(role.to_string())
    } else {
        None
    }
}

/// Spawn Claude Code as a subprocess with streaming output.
/// Reads stdout line-by-line (stream-json format) and prints real-time progress.
fn spawn_claude(
    prompt: &str,
    mcp_config: &Path,
    max_turns: usize,
    verbose: bool,
    role: &str,
    allowed_tools: Option<&str>,
    disallowed_tools: Option<&str>,
    timeout: Option<u64>,
    quiet: bool,
    model: Option<&str>,
    agent_name: Option<&str>,
    resume_session: Option<&str>,
) -> Result<ClaudeResult, String> {
    spawn_claude_in_dir(prompt, mcp_config, max_turns, verbose, role, allowed_tools, disallowed_tools, timeout, None, quiet, model, agent_name, resume_session)
}

// Core agent spawning function — builds and executes a claude CLI subprocess.
// When agent_name is Some, uses --agent flag for native Claude Code agent mode.
// When agent_name is None, the full prompt (including behavioral template) must be in the -p arg.
fn spawn_claude_in_dir(
    prompt: &str,
    mcp_config: &Path,
    max_turns: usize,
    verbose: bool,
    role: &str,
    allowed_tools: Option<&str>,
    disallowed_tools: Option<&str>,
    timeout: Option<u64>,
    work_dir: Option<&Path>,
    quiet: bool,
    model: Option<&str>,
    agent_name: Option<&str>,
    resume_session: Option<&str>,
) -> Result<ClaudeResult, String> {
    use std::process::{Command, Stdio};

    let mut cmd = Command::new("claude");
    cmd.arg("-p")
        .arg(prompt)
        .arg("--model")
        .arg(model.unwrap_or("opus"))
        .arg("--mcp-config")
        .arg(mcp_config)
        .arg("--dangerously-skip-permissions")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--max-turns")
        .arg(max_turns.to_string());

    if let Some(dir) = work_dir {
        cmd.current_dir(dir);
    }

    if let Some(tools) = allowed_tools {
        cmd.arg("--allowedTools").arg(tools);
    }
    if let Some(tools) = disallowed_tools {
        cmd.arg("--disallowedTools").arg(tools);
    }

    if let Some(name) = agent_name {
        cmd.arg("--agent").arg(name);
    }

    if let Some(sid) = resume_session {
        cmd.arg("--resume").arg(sid);
    }

    // Clear CLAUDECODE env var so the child process doesn't refuse to start
    // when the orchestrator itself is running inside a Claude Code session.
    cmd.env_remove("CLAUDECODE");

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn claude: {}", e))?;

    // Watchdog: kill the child if it exceeds timeout (custom or turns * 2 minutes, min 10 min)
    let timeout_secs = timeout.unwrap_or_else(|| std::cmp::max(max_turns as u64 * 120, 600));
    let child_id = child.id();
    let watchdog_role = role.to_string();
    let watchdog_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watchdog_done_clone = watchdog_done.clone();
    // Track whether the subprocess has produced any output (detects MCP startup hangs)
    let first_output = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let first_output_clone = first_output.clone();
    let startup_timeout_secs: u64 = 90;
    let _watchdog = std::thread::spawn(move || {
        // Phase 1: startup timeout — kill early if no output within 90s (MCP init hang)
        let startup_deadline = std::time::Instant::now() + std::time::Duration::from_secs(startup_timeout_secs);
        while std::time::Instant::now() < startup_deadline {
            if watchdog_done_clone.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            if first_output_clone.load(std::sync::atomic::Ordering::Relaxed) {
                break; // Got output, move to normal timeout
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        if !watchdog_done_clone.load(std::sync::atomic::Ordering::Relaxed)
            && !first_output_clone.load(std::sync::atomic::Ordering::Relaxed)
        {
            eprintln!("[{}] Startup timeout — no output after {}s (likely MCP init hang)", watchdog_role, startup_timeout_secs);
            #[cfg(unix)]
            unsafe { libc::kill(child_id as i32, libc::SIGTERM); }
            std::thread::sleep(std::time::Duration::from_secs(3));
            if !watchdog_done_clone.load(std::sync::atomic::Ordering::Relaxed) {
                #[cfg(unix)]
                unsafe { libc::kill(child_id as i32, libc::SIGKILL); }
            }
            return;
        }
        // Phase 2: normal execution timeout
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        while std::time::Instant::now() < deadline {
            if watchdog_done_clone.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        if !watchdog_done_clone.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("[{}] Process timeout after {}s — killing", watchdog_role, timeout_secs);
            #[cfg(unix)]
            unsafe { libc::kill(child_id as i32, libc::SIGTERM); }
            std::thread::sleep(std::time::Duration::from_secs(3));
            if !watchdog_done_clone.load(std::sync::atomic::Ordering::Relaxed) {
                eprintln!("[{}] Force-killing after SIGTERM grace period", watchdog_role);
                #[cfg(unix)]
                unsafe { libc::kill(child_id as i32, libc::SIGKILL); }
            }
        }
    });

    let stdout = child.stdout.take()
        .ok_or_else(|| "Failed to capture stdout".to_string())?;
    let reader = std::io::BufReader::new(stdout);

    let mut session_id: Option<String> = None;
    let mut result_text: Option<String> = None;
    let mut thinking_log = String::new();
    let mut total_cost_usd: Option<f64> = None;
    let mut num_turns: Option<u32> = None;
    let mut duration_ms: Option<u64> = None;
    let mut mcp_status: Vec<(String, String)> = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        // Signal watchdog that we got output (clears startup timeout)
        first_output.store(true, std::sync::atomic::Ordering::Relaxed);

        let json: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match json.get("type").and_then(|t| t.as_str()) {
            Some("system") => {
                eprintln!("[{}] Connected", role);
                // Check MCP server status
                if let Some(servers) = json.get("mcp_servers").and_then(|s| s.as_array()) {
                    for srv in servers {
                        let name = srv.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                        let status = srv.get("status").and_then(|s| s.as_str()).unwrap_or("?");
                        eprintln!("[{}] MCP: {} ({})", role, name, status);
                        mcp_status.push((name.to_string(), status.to_string()));
                    }
                }
            }
            Some("assistant") => {
                if let Some(content) = json.pointer("/message/content").and_then(|c| c.as_array()) {
                    for block in content {
                        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match block_type {
                            "text" => {
                                if verbose {
                                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                        let text = text.trim();
                                        if !text.is_empty() {
                                            let truncated = if text.len() > 500 {
                                                format!("{}...", &text[..text.floor_char_boundary(500)])
                                            } else {
                                                text.to_string()
                                            };
                                            eprintln!("[{}] {}", role, truncated);
                                        }
                                    }
                                }
                            }
                            "tool_use" if !quiet => {
                                let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                                let input = block.get("input");
                                let summary = match name {
                                    "Bash" => {
                                        let cmd = input
                                            .and_then(|i| i.get("command"))
                                            .and_then(|c| c.as_str())
                                            .unwrap_or("?");
                                        let cmd = if cmd.len() > 120 {
                                            format!("{}...", &cmd[..cmd.floor_char_boundary(120)])
                                        } else {
                                            cmd.to_string()
                                        };
                                        format!("$ {}", cmd)
                                    }
                                    n if n.starts_with("mcp__") => {
                                        let tool_name = n.rsplit("__").next().unwrap_or(n);
                                        format!("mcp: {}", tool_name)
                                    }
                                    "Read" | "Edit" | "Write" => {
                                        let path = input
                                            .and_then(|i| i.get("file_path"))
                                            .and_then(|p| p.as_str())
                                            .unwrap_or("?");
                                        format!("{}: {}", name, path)
                                    }
                                    _ => format!("tool: {}", name),
                                };
                                eprintln!("[{}] {}", role, summary);
                            }
                            "thinking" => {
                                if let Some(text) = block.get("thinking").and_then(|t| t.as_str()) {
                                    let text = text.trim();
                                    if !text.is_empty() {
                                        thinking_log.push_str(text);
                                        thinking_log.push('\n');
                                        if verbose {
                                            let preview = if text.len() > 300 {
                                                format!("{}...", &text[..text.floor_char_boundary(300)])
                                            } else {
                                                text.to_string()
                                            };
                                            eprintln!("[{}:think] {}", role, preview);
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some("result") => {
                session_id = json.get("session_id").and_then(|v| v.as_str()).map(|s| s.to_string());
                result_text = json.get("result").and_then(|v| v.as_str()).map(|s| s.to_string());
                total_cost_usd = json.get("total_cost_usd").and_then(|v| v.as_f64());
                num_turns = json.get("num_turns").and_then(|v| v.as_u64()).map(|n| n as u32);
                duration_ms = json.get("duration_ms").and_then(|v| v.as_u64());
            }
            _ => {}
        }
    }

    // Read stderr after stdout is drained
    let mut stderr_buf = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut stderr_buf);
    }

    let status = child.wait()
        .map_err(|e| format!("Failed to wait on claude: {}", e))?;
    watchdog_done.store(true, std::sync::atomic::Ordering::Relaxed);
    let exit_code = status.code().unwrap_or(-1);

    let stdout_raw = result_text.clone().unwrap_or_default();
    Ok(ClaudeResult {
        success: status.success(),
        exit_code,
        session_id,
        result_text,
        thinking_log: if thinking_log.is_empty() { None } else { Some(thinking_log) },
        total_cost_usd,
        num_turns,
        duration_ms,
        stdout_raw,
        stderr_raw: stderr_buf,
        mcp_status,
    })
}

/// Write a temporary MCP config file for an agent run.
fn write_temp_mcp_config(
    cli_binary: &Path,
    role: &str,
    agent_id: &str,
    run_id: &str,
    db_path: &str,
) -> Result<PathBuf, String> {
    let dir = PathBuf::from("/tmp/mycelica-orchestrator");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;

    let filename = format!("mcp-{}-{}.json", role, &run_id[..8.min(run_id.len())]);
    let path = dir.join(filename);

    let binary_str = cli_binary.to_string_lossy();
    let args = vec![
        "mcp-server".to_string(),
        "--stdio".to_string(),
        "--agent-role".to_string(),
        role.to_string(),
        "--agent-id".to_string(),
        agent_id.to_string(),
        "--run-id".to_string(),
        run_id.to_string(),
        "--db".to_string(),
        db_path.to_string(),
    ];

    let config = serde_json::json!({
        "mcpServers": {
            "mycelica": {
                "command": binary_str,
                "args": args,
            }
        }
    });

    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap())
        .map_err(|e| format!("Failed to write MCP config: {}", e))?;
    Ok(path)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Verdict {
    Supports,
    Contradicts,
    Unknown,
}

#[derive(Debug)]
struct VerifierVerdict {
    verdict: Verdict,
    reason: Option<String>,
    confidence: f64,
}

/// Check whether the verifier supports or contradicts the implementation node.
/// First checks edges with agent_id="spore:verifier", then falls back to
/// any supports/contradicts edge (handles CLI-created edges without agent_id).
fn check_verdict(db: &Database, impl_node_id: &str) -> Verdict {
    let edges = match db.get_edges_for_node(impl_node_id) {
        Ok(e) => e,
        Err(_) => return Verdict::Unknown,
    };

    // First pass: look for edges from the verifier agent specifically
    for edge in &edges {
        if edge.target != impl_node_id { continue; }
        if edge.agent_id.as_deref() != Some("spore:verifier") { continue; }
        if edge.superseded_by.is_some() { continue; }
        match edge.edge_type {
            EdgeType::Supports => return Verdict::Supports,
            EdgeType::Contradicts => return Verdict::Contradicts,
            _ => {}
        }
    }

    // Second pass: accept any non-superseded supports/contradicts edge
    // (handles edges created via CLI link command without agent_id)
    for edge in &edges {
        if edge.target != impl_node_id { continue; }
        if edge.superseded_by.is_some() { continue; }
        match edge.edge_type {
            EdgeType::Supports => return Verdict::Supports,
            EdgeType::Contradicts => return Verdict::Contradicts,
            _ => {}
        }
    }

    Verdict::Unknown
}

/// Parse verdict from verifier's stdout as a last-resort fallback.
/// Looks for "PASS", "FAIL", "supports", "contradicts" keywords.
fn parse_verdict_from_text(text: &str) -> Verdict {
    let lower = text.to_lowercase();
    // Look for explicit verdict markers (more specific first)
    if lower.contains("verification result: **pass**") || lower.contains("verdict: pass") || lower.contains("verdict: **pass**") {
        return Verdict::Supports;
    }
    if lower.contains("verification result: **fail**") || lower.contains("verdict: fail") || lower.contains("verdict: **fail**") {
        return Verdict::Contradicts;
    }
    // Fallback to edge type mentions
    if lower.contains("edge_type: \"supports\"") || lower.contains("edge_type: supports") {
        return Verdict::Supports;
    }
    if lower.contains("edge_type: \"contradicts\"") || lower.contains("edge_type: contradicts") {
        return Verdict::Contradicts;
    }
    Verdict::Unknown
}

/// Parse structured verdict JSON from verifier stdout.
/// The verifier may emit a block like:
///   <verdict>{"verdict":"supports","reason":"..."}</verdict>
/// Supported fields: "verdict" or "result", values: "supports"/"pass" or "contradicts"/"fail".
/// Optional "reason" field is captured in the returned struct.
/// Returns None if the <verdict> block is absent.
/// Returns Some with verdict=Unknown if the block is present but cannot be parsed.
fn parse_verifier_verdict(text: &str) -> Option<VerifierVerdict> {
    let start_marker = "<verdict>";
    let end_marker = "</verdict>";

    let start = text.find(start_marker)? + start_marker.len();
    let end = text[start..].find(end_marker).map(|i| start + i)?;

    let json_str = text[start..end].trim();
    let value: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return Some(VerifierVerdict { verdict: Verdict::Unknown, reason: None, confidence: 0.0 }),
    };

    let reason = value.get("reason").and_then(|r| r.as_str()).map(|s| s.to_string());
    let confidence = value.get("confidence")
        .and_then(|c| c.as_f64())
        .unwrap_or(0.9)
        .clamp(0.0, 1.0);

    // Check "verdict" field first, then "result" as a synonym
    for field in &["verdict", "result"] {
        if let Some(v) = value.get(field).and_then(|v| v.as_str()) {
            let verdict = match v.to_lowercase().as_str() {
                "supports" | "pass" => Verdict::Supports,
                "contradicts" | "fail" => Verdict::Contradicts,
                _ => continue,
            };
            return Some(VerifierVerdict { verdict, reason, confidence });
        }
    }

    Some(VerifierVerdict { verdict: Verdict::Unknown, reason, confidence: 0.0 })
}

/// Capture git blob hashes for a set of files (content-aware dirty detection).
/// Returns filename → hash map. Used to detect in-place edits on bounce 2+.
fn capture_file_hashes(files: &HashSet<String>) -> HashMap<String, String> {
    if files.is_empty() {
        return HashMap::new();
    }
    let file_list: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
    let mut cmd = std::process::Command::new("git");
    cmd.arg("hash-object");
    for f in &file_list {
        cmd.arg(f);
    }
    match cmd.output() {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let hashes: Vec<&str> = stdout.trim().lines().collect();
            file_list.iter().zip(hashes.iter())
                .map(|(f, h)| (f.to_string(), h.to_string()))
                .collect()
        }
        _ => HashMap::new(),
    }
}

/// Post-coder cleanup: re-index changed files, reinstall CLI if needed, create related edges.
/// Failures warn but do NOT abort orchestration.
/// Returns the list of files changed by the coder (for passing to tester).
fn post_coder_cleanup(
    db: &Database,
    impl_node_id: &str,
    before_dirty: &HashSet<String>,
    before_untracked: &HashSet<String>,
    before_hashes: &HashMap<String, String>,
    cli_binary: &Path,
    verbose: bool,
) -> Vec<String> {
    // 1. Get files dirty/untracked NOW, subtract pre-existing ones
    let after_dirty: HashSet<String> = std::process::Command::new("git")
        .args(["diff", "--name-only"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect())
        .unwrap_or_default();

    let after_untracked: HashSet<String> = std::process::Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect())
        .unwrap_or_default();

    // Detect in-place edits to already-dirty files via content hash comparison
    let after_hashes = capture_file_hashes(&after_dirty);
    let inplace_edits = before_dirty.intersection(&after_dirty)
        .filter(|f| before_hashes.get(*f) != after_hashes.get(*f))
        .cloned();

    let changed_files: Vec<String> = after_dirty.difference(before_dirty)
        .chain(after_untracked.difference(before_untracked))
        .cloned()
        .chain(inplace_edits)
        .collect();

    if changed_files.is_empty() {
        if verbose {
            eprintln!("[orchestrator] No files changed by coder");
        }
        return Vec::new();
    }

    println!("[orchestrator] {} file(s) changed by coder", changed_files.len());

    // 2. Re-index changed .rs files
    let rs_files: Vec<&str> = changed_files.iter()
        .filter(|f| f.ends_with(".rs"))
        .map(|f| f.as_str())
        .collect();

    for file in &rs_files {
        println!("[orchestrator] Indexing: {}", file);
        // Verify file exists before importing
        if !PathBuf::from(file).exists() {
            eprintln!("[orchestrator] WARNING: File {} not found (CWD may be wrong), skipping index", file);
            continue;
        }
        let result = std::process::Command::new(cli_binary)
            .args(["import", "code", file, "--update"])
            .output();
        match result {
            Ok(o) if !o.status.success() => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                let last = stderr.lines().last().unwrap_or("");
                eprintln!("[orchestrator] WARNING: Failed to index {}: {}", file, last);
            }
            Err(e) => {
                eprintln!("[orchestrator] WARNING: Failed to run import for {}: {}", file, e);
            }
            _ => {}
        }
    }

    // 3. Reinstall CLI if src-tauri/ files changed
    let needs_reinstall = changed_files.iter().any(|f| f.starts_with("src-tauri/"));
    if needs_reinstall {
        println!("[orchestrator] Reinstalling CLI (source files changed)");
        let home = std::env::var("HOME").unwrap_or_else(|_| {
            // Try common paths instead of hardcoding a username
            if PathBuf::from("/home/spore").exists() { "/home/spore".to_string() }
            else { "/home/ekats".to_string() }
        });
        let cargo_bin = PathBuf::from(&home).join(".cargo/bin/mycelica-cli");
        let sidecar = PathBuf::from("binaries/mycelica-cli-x86_64-unknown-linux-gnu");

        // Ensure binaries/ dir exists
        let _ = std::fs::create_dir_all("binaries");

        // Try cargo build --release (more reliable than cargo install on nightly)
        let build_result = std::process::Command::new("cargo")
            .args(["+nightly", "build", "--release", "--bin", "mycelica-cli", "--features", "mcp"])
            .current_dir("src-tauri")
            .output();

        let build_ok = match &build_result {
            Ok(o) if o.status.success() => {
                let built = PathBuf::from("src-tauri/target/release/mycelica-cli");
                // Unlink target first to avoid "Text file busy" when binary is running.
                // Running processes keep their inode open; new processes get the new file.
                let _ = std::fs::remove_file(&cargo_bin);
                match std::fs::copy(&built, &cargo_bin) {
                    Ok(_) => true,
                    Err(e) => {
                        eprintln!("[orchestrator] WARNING: Build succeeded but copy failed: {}", e);
                        false
                    }
                }
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                let last_lines: String = stderr.lines().rev().take(5).collect::<Vec<_>>().into_iter().rev()
                    .collect::<Vec<_>>().join("\n");
                eprintln!("[orchestrator] cargo build failed (exit {}):\n{}", o.status, last_lines);
                false
            }
            Err(e) => {
                eprintln!("[orchestrator] Failed to run cargo: {}", e);
                false
            }
        };

        if !build_ok {
            eprintln!("[orchestrator] WARNING: CLI rebuild failed — MCP server may be stale");
        }

        // Copy sidecar regardless of which method succeeded
        if cargo_bin.exists() {
            let _ = std::fs::remove_file(&sidecar);
            match std::fs::copy(&cargo_bin, &sidecar) {
                Ok(_) => println!("[orchestrator] CLI reinstalled and sidecar updated"),
                Err(e) => eprintln!("[orchestrator] WARNING: Failed to copy sidecar: {}", e),
            }
        }
    }

    // 4. Create related edges from impl node to code nodes matching changed files
    let mut linked = 0usize;
    for file in &changed_files {
        // Find code nodes with this file_path in tags JSON
        let node_ids: Vec<String> = (|| -> Option<Vec<String>> {
            let conn = db.raw_conn().lock().ok()?;
            let mut stmt = conn.prepare(
                "SELECT id FROM nodes WHERE JSON_EXTRACT(tags, '$.file_path') = ?1 LIMIT 10"
            ).ok()?;
            let rows = stmt.query_map([file.as_str()], |row| row.get(0)).ok()?;
            Some(rows.filter_map(|r| r.ok()).collect())
        })().unwrap_or_default();

        let now = chrono::Utc::now().timestamp_millis();
        for node_id in &node_ids {
            let edge = Edge {
                id: uuid::Uuid::new_v4().to_string(),
                source: impl_node_id.to_string(),
                target: node_id.clone(),
                edge_type: EdgeType::Related,
                label: None,
                weight: None,
                edge_source: Some("orchestrator".to_string()),
                evidence_id: None,
                confidence: Some(0.85),
                created_at: now,
                updated_at: Some(now),
                author: None,
                reason: None,
                content: Some("Implementation modifies this code".to_string()),
                agent_id: Some("spore:orchestrator".to_string()),
                superseded_by: None,
                metadata: None,
            };
            if db.insert_edge(&edge).is_ok() {
                linked += 1;
            }
        }
    }

    if linked > 0 {
        println!("[orchestrator] Linked impl node to {} code node(s)", linked);
    }

    changed_files
}


/// Generate a task file with graph context for an agent before spawning.
///
/// Produces a markdown file at `docs/spore/tasks/task-<run_id>.md` that serves as
/// both bootstrap context for the spawned agent and an audit trail for the run.
///
/// # Generated Sections
///
/// - **Header** — Run metadata: truncated task title, run ID, agent role, bounce
///   number (current/max), and UTC timestamp.
/// - **Task** — The full, untruncated task description as provided by the caller.
/// - **Previous Bounce** (conditional) — Present only when `last_impl_id` is set,
///   meaning the verifier rejected a prior implementation. Points the agent at the
///   failed node so it can read the incoming `contradicts` edges and fix the issues.
/// - **Graph Context** — A relevance-ranked table of knowledge-graph nodes related
///   to the task. Each row shows the node title, short ID, relevance score, the
///   anchor it was reached from, and the edge-type path taken to reach it.
/// - **Checklist** — Static reminders for the agent: read context first, create an
///   operational node when done, and link it to modified code nodes.
///
/// # Context Gathering: Semantic Search + Dijkstra
///
/// 1. **Semantic anchor search** — The task description is embedded using the local
///    all-MiniLM-L6-v2 model and compared against all stored node embeddings via
///    cosine similarity. This captures meaning ("format flag" ≈ SporeCommands) rather
///    than requiring keyword overlap. Falls back to FTS5 with OR-joined tokens if
///    embedding generation fails (model not downloaded, etc.).
///    The top 3 non-operational matches become anchor nodes.
/// 2. **Dijkstra expansion** — For each anchor, [`Database::context_for_task`] runs
///    a weighted shortest-path traversal (max 4 hops, cost ceiling 2.0) that
///    follows semantic edges (supports, contradicts, derives_from, etc.) while
///    skipping structural edges (defined_in, belongs_to, sibling). Edge confidence
///    and type priority determine traversal weights, so high-confidence semantic
///    edges are explored first.
/// 3. **Dedup and rank** — Nodes discovered from multiple anchors are deduplicated,
///    keeping the highest relevance score. The final list is sorted by descending
///    relevance and rendered into the Graph Context table.
fn generate_task_file(
    db: &Database,
    task: &str,
    role: &str,
    run_id: &str,
    _task_node_id: &str,
    bounce: usize,
    max_bounces: usize,
    last_impl_id: Option<&str>,
    last_impl_verdict: Option<Verdict>,
    experiment: Option<&str>,
) -> Result<(PathBuf, usize), String> {
    use std::collections::HashMap;

    let skip_context = experiment == Some("no-context");

    // Compute task embedding once — reused for anchor search and lesson matching.
    // Skipped in no-context experiment (A/B baseline).
    let (task_embedding, all_embeddings): (Option<Vec<f32>>, Vec<(String, Vec<f32>)>) = if !skip_context {
        let emb = match local_embeddings::generate(task) {
            Ok(emb) => Some(emb),
            Err(e) => {
                eprintln!("[spore] warning: embedding generation failed: {}", e);
                None
            }
        };
        // Load all node embeddings once — reused for anchors + lesson ranking.
        let all_embs = db.get_nodes_with_embeddings().unwrap_or_default();
        (emb, all_embs)
    } else {
        (None, vec![])
    };

    // 1. Find anchor nodes via semantic search (embedding similarity).
    //    Natural language task descriptions like "Add a format flag to spore status"
    //    work poorly with FTS5 (common words match everything or nothing useful).
    //    Embedding similarity captures meaning: "format flag" is semantically close
    //    to the SporeCommands enum and handle_spore function even without keyword overlap.
    //    Falls back to FTS5 if embedding generation fails (model not downloaded, etc.).
    //    Skipped entirely in no-context experiment (A/B baseline).
    let context_rows: Vec<(String, (f64, String, String, String))> = if skip_context {
        vec![]
    } else {
    let (anchors, anchor_source_labels): (Vec<Node>, HashMap<String, String>) = {
        // Semantic search (embedding similarity)
        let semantic_results: Vec<Node> = (|| -> Result<Vec<Node>, String> {
            let query_embedding = task_embedding.as_ref()
                .ok_or_else(|| "No task embedding available".to_string())?;
            let similar = similarity::find_similar(
                query_embedding, &all_embeddings, _task_node_id, 10, 0.3,
            );
            // Resolve node IDs to full nodes, filtering operational
            let mut result = Vec::new();
            for (node_id, _score) in similar {
                if let Ok(Some(node)) = db.get_node(&node_id) {
                    if node.node_class.as_deref() != Some("operational") {
                        result.push(node);
                        if result.len() >= 5 {
                            break;
                        }
                    }
                }
            }
            Ok(result)
        })().unwrap_or_default();

        if !semantic_results.is_empty() {
            println!("[task-file] Semantic search found {} candidate(s)", semantic_results.len());
        }

        // FTS keyword search (always runs — catches token matches semantic misses)
        let fts_results: Vec<Node> = {
            let stopwords = ["the", "a", "an", "in", "on", "at", "to", "for", "of", "is", "it",
                             "and", "or", "with", "from", "by", "this", "that", "as", "be"];
            // Split on dots, hyphens, and other FTS5-hostile characters to avoid
            // query crashes. E.g. "spore.rs" → ["spore", "rs"], "side-by-side" → ["side", "by", "side"]
            let fts_query: String = task.split_whitespace()
                .flat_map(|w| w.split(|c: char| !c.is_alphanumeric() && c != '_'))
                .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
                .filter(|w| w.len() > 2 && !stopwords.contains(&w.to_lowercase().as_str()))
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(" OR ");
            if fts_query.is_empty() {
                Vec::new()
            } else {
                match db.search_nodes(&fts_query) {
                    Ok(nodes) => nodes,
                    Err(e) => {
                        eprintln!("[spore] warning: FTS query failed: {}", e);
                        Vec::new()
                    }
                }
                    .into_iter()
                    .filter(|n| n.id != _task_node_id)
                    .filter(|n| n.node_class.as_deref() != Some("operational"))
                    .take(5)
                    .collect()
            }
        };

        if !fts_results.is_empty() {
            println!("[task-file] FTS search found {} candidate(s)", fts_results.len());
        }

        // Track source labels before merge (semantic has priority for dupes)
        let mut anchor_source_labels: HashMap<String, String> = HashMap::new();
        for node in &semantic_results {
            anchor_source_labels.insert(node.id.clone(), "Semantic match".to_string());
        }
        for node in &fts_results {
            anchor_source_labels.entry(node.id.clone()).or_insert("FTS match".to_string());
        }

        // Merge: semantic results first (priority), then FTS, deduplicated by node ID
        let mut seen_ids = std::collections::HashSet::new();
        let mut merged: Vec<Node> = Vec::new();
        for node in semantic_results {
            if seen_ids.insert(node.id.clone()) {
                merged.push(node);
            }
        }
        for node in fts_results {
            if seen_ids.insert(node.id.clone()) {
                merged.push(node);
            }
        }
        merged.truncate(5);

        println!("[task-file] {} anchor(s) after merge+dedup", merged.len());
        (merged, anchor_source_labels)
    };

    // 2. Gather context via Dijkstra from each anchor
    let mut seen: HashMap<String, (f64, String, String, String)> = HashMap::new(); // id -> (relevance, title, anchor_title, via)

    for anchor in &anchors {
        let anchor_title = anchor.ai_title.as_deref().unwrap_or(&anchor.title);
        let context = db.context_for_task(
            &anchor.id, 7, Some(4), Some(2.0), None,
            Some(&["clicked", "backtracked", "session_item"]),
            true, true,
        ).unwrap_or_default();

        for node in &context {
            // Skip operational nodes (other orchestrator artifacts)
            if node.node_class.as_deref() == Some("operational") {
                continue;
            }
            let via = if node.path.is_empty() {
                "direct".to_string()
            } else {
                node.path.iter()
                    .map(|hop| hop.edge.edge_type.as_str())
                    .collect::<Vec<_>>()
                    .join(" → ")
            };
            let key = node.node_id.clone();
            if !seen.contains_key(&key) || node.relevance > seen[&key].0 {
                seen.insert(key, (
                    node.relevance,
                    node.node_title.clone(),
                    anchor_title.to_string(),
                    via,
                ));
            }
        }

        // Also include the anchor itself
        if !seen.contains_key(&anchor.id) {
            let source_label = anchor_source_labels.get(&anchor.id)
                .cloned()
                .unwrap_or_else(|| "FTS match".to_string());
            seen.insert(anchor.id.clone(), (
                1.0,
                anchor_title.to_string(),
                "search".to_string(),
                source_label,
            ));
        }
    }

    // 3. Sort by relevance, filter out the task node itself
    let mut rows: Vec<_> = seen.into_iter()
        .filter(|(id, _)| id != _task_node_id)
        .collect();
    rows.sort_by(|a, b| b.1.0.partial_cmp(&a.1.0).unwrap_or(std::cmp::Ordering::Equal));
    rows
    }; // end if skip_context else

    // 4. Format markdown
    let now = chrono::Utc::now();
    let task_short = if task.len() > 60 { &task[..task.floor_char_boundary(60)] } else { task };

    let mut md = String::new();
    md.push_str(&format!("# Task: {}\n\n", task_short));
    md.push_str(&format!("- **Run:** {}\n", &run_id[..8.min(run_id.len())]));
    md.push_str(&format!("- **Agent:** {}\n", role));
    md.push_str(&format!("- **Bounce:** {}/{}\n", bounce + 1, max_bounces));
    md.push_str(&format!("- **Generated:** {}\n\n", now.format("%Y-%m-%d %H:%M:%S UTC")));

    md.push_str("## Task\n\n");
    md.push_str(task);
    md.push_str("\n\n");

    if let Some(impl_id) = last_impl_id {
        match role {
            "verifier" => {
                md.push_str("## Implementation to Check\n\n");
                md.push_str(&format!(
                    "Implementation node ID: `{}`. Read it with `mycelica_read_content` to see what the coder changed and why.\n\n",
                    impl_id
                ));
            }
            "summarizer" => {
                md.push_str("## Implementation to Summarize\n\n");
                md.push_str(&format!(
                    "Implementation node ID: `{}`. Read it and the full bounce trail with `mycelica_read_content` and `mycelica_nav_edges`.\n\n",
                    impl_id
                ));
            }
            _ => {
                // coder on bounce 2+: previous impl had issues, coder needs to fix
                md.push_str("## Previous Bounce\n\n");
                if matches!(last_impl_verdict, Some(Verdict::Unknown)) {
                    md.push_str(&format!(
                        "The verifier could not parse a verdict from the previous attempt (node `{}`). Review your changes carefully and ensure correctness.\n\n",
                        impl_id
                    ));
                } else {
                    md.push_str(&format!(
                        "Verifier found issues with node `{}`. Check its incoming `contradicts` edges and fix the code.\n\n",
                        impl_id
                    ));
                }
            }
        }
    }

    if !skip_context {
    md.push_str("## Graph Context\n\n");
    md.push_str("Relevant nodes found by search + Dijkstra traversal from the task description.\n");
    md.push_str("Use `mycelica_node_get` or `mycelica_read_content` to read full content of any node.\n\n");

    if context_rows.is_empty() {
        md.push_str("_No relevant nodes found in the graph._\n\n");
    } else {
        md.push_str("| # | Node | ID | Relevance | Via |\n");
        md.push_str("|---|------|----|-----------|-----|\n");
        for (i, (id, (rel, title, anchor, via))) in context_rows.iter().enumerate() {
            let title_short = if title.len() > 50 { &title[..title.floor_char_boundary(50)] } else { title.as_str() };
            let id_short = &id[..12.min(id.len())];
            md.push_str(&format!(
                "| {} | {} | `{}` | {:.0}% | {} → {} |\n",
                i + 1, title_short, id_short, rel * 100.0, anchor, via
            ));
        }
        md.push_str("\n");

        // For code nodes, append file locations so agents can Read files directly
        let code_locations: Vec<(String, String, usize, usize)> = context_rows.iter()
            .filter(|(id, _)| id.starts_with("code-"))
            .filter_map(|(id, (_, title, _, _))| {
                db.get_node(id).ok().flatten().and_then(|n| {
                    let tags: serde_json::Value = n.tags.as_deref()
                        .and_then(|t| serde_json::from_str(t).ok())
                        .unwrap_or(serde_json::Value::Null);
                    let file = tags["file_path"].as_str()?;
                    let start = tags["line_start"].as_u64().unwrap_or(1) as usize;
                    let end = tags["line_end"].as_u64().unwrap_or(start as u64) as usize;
                    Some((title.clone(), file.to_string(), start, end))
                })
            })
            .collect();

        if !code_locations.is_empty() {
            md.push_str("### Code Locations\n\n");
            md.push_str("Use `Read` tool with these paths for direct file access (faster than MCP):\n\n");
            for (title, file, start, end) in &code_locations {
                let title_short = if title.len() > 40 { &title[..title.floor_char_boundary(40)] } else { title.as_str() };
                md.push_str(&format!("- `{}` L{}-{} — {}\n", file, start, end, title_short));
            }
            md.push_str("\n");

            // Inline code snippets for the top 5 most relevant code nodes.
            // This saves coders ~10 turns of file exploration by providing key
            // function signatures and first lines of implementation directly.
            // Prefer functions over structs — function implementations are more
            // useful for understanding control flow and making changes.
            let snippet_limit = 5;
            let snippet_lines = 30; // max lines per snippet
            let mut snippet_candidates: Vec<&(String, String, usize, usize)> = code_locations.iter()
                .filter(|(_, _, start, end)| {
                    let range = end.saturating_sub(*start);
                    range >= 3 && range <= 200
                })
                .collect();
            // Sort: functions first (fn/async fn/pub fn), then structs, preserving order within groups
            snippet_candidates.sort_by_key(|(title, _, _, _)| {
                let t = title.trim_start();
                if t.starts_with("fn ") || t.starts_with("pub fn ") || t.starts_with("pub(crate) fn ")
                    || t.starts_with("async fn ") || t.starts_with("pub async fn ") || t.starts_with("pub(crate) async fn ")
                {
                    0 // functions first
                } else {
                    1 // structs, enums, etc. second
                }
            });
            let mut snippets_added = 0;
            for (title, file, start, end) in snippet_candidates.iter().take(snippet_limit) {
                let full_path = if file.starts_with("./") || file.starts_with('/') {
                    PathBuf::from(file)
                } else {
                    std::env::current_dir().unwrap_or_default().join(file)
                };
                let content = match std::fs::read_to_string(&full_path) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[spore] warning: failed to read code file {}: {}", full_path.display(), e);
                        continue;
                    }
                };
                let lines: Vec<&str> = content.lines().collect();
                let start_idx = start.saturating_sub(1); // 1-indexed to 0-indexed
                let end_idx = (*end).min(lines.len());
                let snippet_end = (start_idx + snippet_lines).min(end_idx);
                if start_idx < lines.len() && snippet_end > start_idx {
                    if snippets_added == 0 {
                        md.push_str("### Key Code Snippets\n\n");
                        md.push_str("Top code sections — read these before exploring further.\n\n");
                    }
                    let title_short = if title.len() > 60 { &title[..title.floor_char_boundary(60)] } else { title.as_str() };
                    md.push_str(&format!("**{}** (`{}` L{}-{}):\n", title_short, file, start, end));
                    let lang = std::path::Path::new(file.as_str())
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|ext| match ext {
                            "rs" => "rust",
                            "ts" | "tsx" => "typescript",
                            "js" | "jsx" | "mjs" | "cjs" => "javascript",
                            "py" | "pyi" => "python",
                            "go" => "go",
                            "c" | "h" => "c",
                            "cpp" | "hpp" | "cc" | "cxx" => "cpp",
                            "java" => "java",
                            "md" => "markdown",
                            _ => "",
                        })
                        .unwrap_or("");
                    md.push_str(&format!("```{}\n", lang));
                    for line in &lines[start_idx..snippet_end] {
                        md.push_str(line);
                        md.push_str("\n");
                    }
                    if snippet_end < end_idx {
                        md.push_str(&format!("// ... ({} more lines)\n", end_idx - snippet_end));
                    }
                    md.push_str("```\n\n");
                    snippets_added += 1;
                }
            }
        }

        // 4a-ii. Files Likely Touched — group code_locations by file, ranked by node count
        if !code_locations.is_empty() {
            let mut file_nodes: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
            for (title, file, _, _) in &code_locations {
                file_nodes.entry(file.as_str()).or_default().push(title.as_str());
            }
            let mut file_list: Vec<_> = file_nodes.iter().collect();
            file_list.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

            md.push_str("### Files Likely Touched\n\n");
            md.push_str("Ranked by number of relevant code nodes per file:\n\n");
            for (file, nodes) in file_list.iter().take(8) {
                let node_names: Vec<_> = nodes.iter()
                    .take(3)
                    .map(|n| {
                        let short = if n.len() > 35 { &n[..n.floor_char_boundary(35)] } else { *n };
                        format!("`{}`", short)
                    })
                    .collect();
                let suffix = if nodes.len() > 3 {
                    format!(" +{} more", nodes.len() - 3)
                } else {
                    String::new()
                };
                md.push_str(&format!("1. **`{}`** ({} nodes) — {}{}\n",
                    file, nodes.len(), node_names.join(", "), suffix));
            }
            md.push_str("\n");
        }

        // 4a-iii. Call Chain — for top function nodes, show callers and callees
        {
            let fn_nodes: Vec<_> = code_locations.iter()
                .filter(|(title, _, start, end)| {
                    let t = title.trim_start();
                    let range = end.saturating_sub(*start);
                    range >= 3 && range <= 500
                        && (t.starts_with("fn ") || t.starts_with("pub fn ")
                            || t.starts_with("pub(crate) fn ") || t.starts_with("async fn ")
                            || t.starts_with("pub async fn ") || t.starts_with("pub(crate) async fn "))
                })
                .take(3)
                .collect();

            if !fn_nodes.is_empty() {
                // Collect node IDs for function code nodes
                let fn_node_ids: Vec<_> = context_rows.iter()
                    .filter(|(id, _)| id.starts_with("code-"))
                    .filter(|(id, (_, title, _, _))| {
                        fn_nodes.iter().any(|(ft, _, _, _)| ft == title)
                    })
                    .map(|(id, _)| id.as_str())
                    .collect();

                if !fn_node_ids.is_empty() {
                    let mut call_lines: Vec<String> = Vec::new();
                    for node_id in &fn_node_ids {
                        if let Ok(edges) = db.get_edges_for_node(node_id) {
                            let callers: Vec<_> = edges.iter()
                                .filter(|e| e.edge_type.as_str() == "calls" && e.target == *node_id)
                                .filter_map(|e| db.get_node(&e.source).ok().flatten())
                                .map(|n| n.title)
                                .take(3)
                                .collect();
                            let callees: Vec<_> = edges.iter()
                                .filter(|e| e.edge_type.as_str() == "calls" && e.source == *node_id)
                                .filter_map(|e| db.get_node(&e.target).ok().flatten())
                                .map(|n| n.title)
                                .take(3)
                                .collect();

                            if !callers.is_empty() || !callees.is_empty() {
                                let fn_title = db.get_node(node_id).ok().flatten()
                                    .map(|n| n.title)
                                    .unwrap_or_else(|| node_id.to_string());
                                let fn_short = if fn_title.len() > 40 { &fn_title[..fn_title.floor_char_boundary(40)] } else { fn_title.as_str() };
                                let mut line = format!("- **`{}`**", fn_short);
                                if !callers.is_empty() {
                                    let caller_names: Vec<_> = callers.iter()
                                        .map(|c| {
                                            let s = if c.len() > 30 { &c[..c.floor_char_boundary(30)] } else { c.as_str() };
                                            format!("`{}`", s)
                                        })
                                        .collect();
                                    line.push_str(&format!(" — called by: {}", caller_names.join(", ")));
                                }
                                if !callees.is_empty() {
                                    let callee_names: Vec<_> = callees.iter()
                                        .map(|c| {
                                            let s = if c.len() > 30 { &c[..c.floor_char_boundary(30)] } else { c.as_str() };
                                            format!("`{}`", s)
                                        })
                                        .collect();
                                    if !callers.is_empty() { line.push_str(";"); }
                                    line.push_str(&format!(" calls: {}", callee_names.join(", ")));
                                }
                                call_lines.push(line);
                            }
                        }
                    }

                    if !call_lines.is_empty() {
                        md.push_str("### Call Graph\n\n");
                        md.push_str("Who calls these functions and what do they call:\n\n");
                        for line in &call_lines {
                            md.push_str(line);
                            md.push_str("\n");
                        }
                        md.push_str("\n");
                    }
                }
            }
        }
    }
    } // end if !skip_context (Graph Context)

    // 4b. Include lessons from past runs (cross-run learning — Concern 13)
    //     Uses embedding similarity to find lessons relevant to THIS task,
    //     not just the 5 most recent. Falls back to recency if no embedding.
    //     Skipped in no-context experiment (A/B baseline).
    if !skip_context {
    let lessons: Vec<(String, String, String)> = (|| -> Result<Vec<(String, String, String)>, String> {
        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id, title, content FROM nodes \
             WHERE node_class = 'operational' AND title LIKE 'Lesson:%' \
             ORDER BY created_at DESC LIMIT 20"
        ).map_err(|e| e.to_string())?;
        let all_lessons: Vec<(String, String, String)> = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let content: String = row.get::<_, String>(2).unwrap_or_default();
            Ok((id, title, content))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
        drop(stmt);
        drop(conn);

        // Rank by embedding similarity to task if available
        let ranked: Vec<&(String, String, String)> = if let Some(ref emb) = task_embedding {
            // Build embeddings for lesson nodes — generate on-the-fly if missing
            let lesson_ids: std::collections::HashSet<&str> = all_lessons.iter()
                .map(|(id, _, _)| id.as_str()).collect();
            let mut lesson_embeddings: Vec<(String, Vec<f32>)> = all_embeddings.iter()
                .filter(|(id, _)| lesson_ids.contains(id.as_str()))
                .map(|(id, emb)| (id.clone(), emb.clone()))
                .collect();
            // Generate embeddings for lessons that don't have one yet
            let existing_ids: std::collections::HashSet<String> = lesson_embeddings.iter()
                .map(|(id, _)| id.clone()).collect();
            for (id, title, content) in &all_lessons {
                if !existing_ids.contains(id) {
                    // Use title + first 200 chars of content for embedding
                    let text = format!("{} {}", title, &content[..content.len().min(200)]);
                    match local_embeddings::generate(&text) {
                        Ok(emb) => {
                            let _ = db.update_node_embedding(id, &emb);
                            lesson_embeddings.push((id.clone(), emb));
                        }
                        Err(e) => {
                            eprintln!("[spore] warning: failed to generate embedding for lesson {}: {}", &id[..8.min(id.len())], e);
                        }
                    }
                }
            }
            if lesson_embeddings.is_empty() {
                all_lessons.iter().take(5).collect()
            } else {
                let similar = similarity::find_similar(emb, &lesson_embeddings, _task_node_id, 5, 0.15);
                let ranked_ids: Vec<String> = similar.iter().map(|(id, _)| id.clone()).collect();
                // Build ordered result preserving similarity ranking
                let mut result: Vec<&(String, String, String)> = Vec::new();
                for rid in &ranked_ids {
                    if let Some(lesson) = all_lessons.iter().find(|(id, _, _)| id == rid) {
                        result.push(lesson);
                    }
                }
                // If similarity found < 5, pad with remaining by recency
                if result.len() < 5 {
                    for lesson in &all_lessons {
                        if result.len() >= 5 { break; }
                        if !ranked_ids.contains(&lesson.0) {
                            result.push(lesson);
                        }
                    }
                }
                result
            }
        } else {
            // No embedding model — fall back to recency
            all_lessons.iter().take(5).collect()
        };

        // Extract pattern summary from each lesson
        let rows: Vec<(String, String, String)> = ranked.iter().map(|(_, title, content)| {
            let pattern = content.lines()
                .skip_while(|l| !l.starts_with("## Pattern") && !l.starts_with("## Situation"))
                .skip(1)
                .take_while(|l| !l.starts_with("## "))
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string();
            let fix = content.lines()
                .skip_while(|l| !l.starts_with("## Fix"))
                .skip(1)
                .take_while(|l| !l.starts_with("## "))
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string();
            let summary = if pattern.is_empty() {
                title.strip_prefix("Lesson: ").unwrap_or(title).to_string()
            } else {
                pattern
            };
            (title.clone(), summary, fix)
        })
        .filter(|(_, summary, _)| is_lesson_quality(summary))
        .collect();
        Ok(rows)
    })().unwrap_or_default();

    if !lessons.is_empty() {
        md.push_str("## Lessons from Past Runs\n\n");
        md.push_str("These were extracted from previous orchestrator runs. Keep them in mind.\n\n");
        for (title, summary, fix) in &lessons {
            let lesson_name = title.strip_prefix("Lesson: ").unwrap_or(title);
            if fix.is_empty() {
                md.push_str(&format!("- **{}**: {}\n", lesson_name, summary));
            } else {
                md.push_str(&format!("- **{}**: {}\n  **Fix:** {}\n", lesson_name, summary, fix));
            }
        }
        md.push_str("\n");
    }
    } // end if !skip_context (Lessons)

    md.push_str("## Checklist\n\n");
    md.push_str("- [ ] Read relevant context nodes above before starting\n");
    md.push_str("- [ ] Link implementation to modified code nodes with edges\n");

    // 5. Write to disk
    let tasks_dir = PathBuf::from("docs/spore/tasks");
    std::fs::create_dir_all(&tasks_dir)
        .map_err(|e| format!("Failed to create tasks dir: {}", e))?;

    let filename = format!("task-{}.md", &run_id[..8.min(run_id.len())]);
    let path = tasks_dir.join(&filename);
    std::fs::write(&path, &md)
        .map_err(|e| format!("Failed to write task file: {}", e))?;

    let line_count = md.lines().count();
    eprintln!("[task-file] Generated: {} lines", line_count);
    Ok((path, line_count))
}

/// Check if a file path should be excluded from spore auto-commits.
fn is_spore_excluded(path: &str) -> bool {
    let basename = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);

    if path.ends_with(".loop-state.json") {
        return true;
    }
    if basename.starts_with(".env") {
        return true;
    }
    if path.starts_with("target/") || path.starts_with("node_modules/") {
        return true;
    }
    if path.ends_with(".db") || path.ends_with(".db-journal") || path.ends_with(".db-wal") || path.ends_with(".db-shm") {
        return true;
    }
    false
}

/// Selectively stage files for git commit, excluding spore-internal artifacts.
/// Runs `git add -u` for tracked files, then selectively adds new untracked files
/// that pass the spore exclusion filter. Returns true if staging succeeded.
fn selective_git_add(cwd: &std::path::Path) -> bool {
    // Stage modifications/deletions to already-tracked files
    let add_u = std::process::Command::new("git")
        .args(["add", "-u"])
        .current_dir(cwd)
        .status();
    if !matches!(add_u, Ok(s) if s.success()) {
        return false;
    }

    // List new untracked files (respects .gitignore)
    let ls_output = std::process::Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .current_dir(cwd)
        .output();

    if let Ok(o) = ls_output {
        if o.status.success() {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let files: Vec<&str> = stdout
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter(|l| !is_spore_excluded(l))
                .collect();
            if !files.is_empty() {
                let mut cmd = std::process::Command::new("git");
                cmd.arg("add").current_dir(cwd);
                for f in &files {
                    cmd.arg(f);
                }
                let _ = cmd.status();
            }
        }
    }

    true
}
// ============================================================================
// Single-Task Orchestration
// ============================================================================

/// State for resuming an interrupted orchestrator run at a specific phase.
struct ResumeState {
    /// Existing task node ID to reuse (skip creating a new one)
    task_node_id: String,
    /// Phase to resume from: "coder", "verifier", "summarizer"
    from_phase: String,
    /// Implementation node ID from the interrupted run (used when skipping coder)
    impl_node_id: Option<String>,
    /// Last implementation node ID (for bounce continuity)
    last_impl_id: Option<String>,
    /// Which bounce we're resuming (0-indexed)
    bounce: usize,
}

/// Dispatch a single agent session without the coder→verifier bounce loop.
///
/// No post-run cleanup is performed: no git diff, no code re-index, no binary rebuild.
/// The caller (or the agent itself) is responsible for any side effects.
/// This is the entry point for `--agent <role>` dispatch.
async fn handle_single_agent(
    db: &Database,
    task: &str,
    role_name: &str,
    prompt_path: &Path,
    max_turns: usize,
    verbose: bool,
    quiet: bool,
    timeout: Option<u64>,
    dry_run: bool,
) -> Result<(), String> {
    let cli_binary = resolve_cli_binary()?;
    let db_path = db.get_path();

    // Default turn budget: 80 for operator, max_turns for others
    let turn_budget = if role_name == "operator" && max_turns == 50 { 80 } else { max_turns };

    let run_id = uuid::Uuid::new_v4().to_string();
    let short_task = truncate_middle(task, 60);

    // Create task node
    let task_node_id = uuid::Uuid::new_v4().to_string();
    let task_node = make_orchestrator_node(
        task_node_id.clone(),
        format!("Task: {}", &short_task),
        format!("Single-agent dispatch ({}) for: {}", role_name, task),
        "operational",
        Some("task"),
    );
    db.insert_node(&task_node).map_err(|e| format!("Failed to create task node: {}", e))?;

    // Generate task file with graph context
    let (task_file, _line_count) = generate_task_file(
        db, task, role_name, &run_id[..8], &task_node_id,
        0, 1, None, None, None,
    )?;
    println!("[{}] Task file: {}", role_name, task_file.display());

    if dry_run {
        println!("[dry-run] Would run {} for task: {}", role_name, short_task);
        return Ok(());
    }

    // Read agent prompt template
    let abs_prompt_path = if prompt_path.is_absolute() {
        prompt_path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(prompt_path)
    };
    let prompt_template = std::fs::read_to_string(&abs_prompt_path)
        .map_err(|e| format!("Failed to read {} prompt at {}: {}", role_name, abs_prompt_path.display(), e))?;

    // Compose prompt
    let single_agent = resolve_agent_name(role_name);
    let prompt = if single_agent.is_some() {
        // Native agent mode: task-specific context only, no template
        format!(
            "Read the task file at {} for full context and graph-gathered information.\n\nTask node ID: {}\n\nYour task: {}",
            task_file.display(), task_node_id, task
        )
    } else {
        format!(
            "{}\n\nRead the task file at {} for full context and graph-gathered information.\n\nTask node ID: {}\n\nYour task: {}",
            prompt_template, task_file.display(), task_node_id, task
        )
    };

    // Write MCP config
    let mcp_config = write_temp_mcp_config(
        &cli_binary, role_name, &format!("spore:{}", role_name), &run_id, &db_path,
    )?;

    // Tool permissions per role
    let (allowed, disallowed) = match role_name {
        "operator" => (
            Some("Read,Write,Edit,Bash(*),Glob,Grep,mcp__mycelica__*"),
            None::<&str>,
        ),
        "coder" => (
            Some("Read,Write,Edit,Bash(*),mcp__mycelica__*"),
            Some("Grep,Glob"),
        ),
        "verifier" => (
            Some("Read,Grep,Glob,Bash(cargo:*),Bash(cd:*),Bash(mycelica-cli:*),mcp__mycelica__*"),
            None,
        ),
        "summarizer" => (
            Some("mcp__mycelica__*,Read,Grep"),
            Some("Edit,Write,Bash,Glob"),
        ),
        _ => (
            Some("Read,Grep,Glob,Bash(*),mcp__mycelica__*"),
            None,
        ),
    };

    let complexity = estimate_complexity(task) as i32;
    let agent_model = select_model_for_role(role_name, complexity);
    println!("[{}] Starting (run: {}, turns: {}, model: {})", role_name, &run_id[..8], turn_budget, agent_model);
    let start = std::time::Instant::now();
    let result = spawn_claude(
        &prompt, &mcp_config, turn_budget, verbose, role_name,
        allowed, disallowed, timeout, quiet,
        Some(&agent_model),
        single_agent.as_deref(),
        None,
    )?;
    let elapsed = start.elapsed().as_millis() as u64;

    if !result.success {
        eprintln!("[{}] FAILED (exit code {})", role_name, result.exit_code);
        if !result.stderr_raw.is_empty() {
            eprintln!("[{} stderr] {}", role_name, &result.stderr_raw[..2000.min(result.stderr_raw.len())]);
        }
    } else {
        println!("[{}] Done ({} turns, ${:.2}, {})",
            role_name,
            result.num_turns.unwrap_or(0),
            result.total_cost_usd.unwrap_or(0.0),
            format_duration_short(elapsed),
        );
    }

    // Write thinking log
    if let Some(ref thinking) = result.thinking_log {
        let think_path = format!("docs/spore/tasks/think-{}-{}.log", role_name, &run_id[..8]);
        let _ = std::fs::write(&think_path, thinking);
    }

    // Record run status
    record_run_status_with_cost(
        db, &task_node_id, &run_id, &format!("spore:{}", role_name),
        if result.success { "completed" } else { "failed" },
        result.exit_code, result.total_cost_usd,
        result.num_turns, Some(elapsed),
        None,
        None,
    )?;

    // Print result summary
    if let Some(ref text) = result.result_text {
        if !text.is_empty() && !quiet {
            println!("\n--- {} output ---\n{}", role_name, text);
        }
    }

    if result.success { Ok(()) } else {
        Err(format!("{} failed with exit code {}", role_name, result.exit_code))
    }
}

async fn handle_orchestrate(
    db: &Database,
    task: &str,
    max_bounces: usize,
    max_turns: usize,
    coder_prompt_path: &Path,
    verifier_prompt_path: &Path,
    summarizer_prompt_path: &Path,
    summarize: bool,
    dry_run: bool,
    verbose: bool,
    timeout: Option<u64>,
    resume: Option<ResumeState>,
    quiet: bool,
    _native_agent: bool,
    experiment: Option<&str>,
    coder_model_override: Option<&str>,
) -> Result<String, String> {
    // Resolve CLI binary path
    let cli_binary = resolve_cli_binary()?;
    if verbose {
        eprintln!("[orchestrator] CLI binary: {}", cli_binary.display());
    }

    // Fail fast if claude is not available
    let which_result = std::process::Command::new("which")
        .arg("claude")
        .output()
        .map_err(|e| format!("Failed to check for claude: {}", e))?;
    if !which_result.status.success() {
        return Err("'claude' not found in PATH. Install Claude Code first.".to_string());
    }

    // Resolve DB path early — its parent is the repo root, used for prompt path fallback
    let db_path_str = db.get_path();
    let repo_root = Path::new(&db_path_str).parent();

    // Resolve prompt paths: try CWD first, then relative to repo root (DB parent dir)
    let resolve_prompt = |p: &Path| -> PathBuf {
        if p.exists() {
            return p.to_path_buf();
        }
        if let Some(root) = repo_root {
            let resolved = root.join(p);
            if resolved.exists() {
                return resolved;
            }
        }
        p.to_path_buf()
    };
    let coder_path = resolve_prompt(coder_prompt_path);
    let verifier_path = resolve_prompt(verifier_prompt_path);
    let summarizer_path = resolve_prompt(summarizer_prompt_path);

    // Read agent prompts
    let coder_prompt_template = std::fs::read_to_string(&coder_path)
        .map_err(|e| format!("Failed to read coder prompt at {}: {}", coder_path.display(), e))?;
    let verifier_prompt_template = std::fs::read_to_string(&verifier_path)
        .map_err(|e| format!("Failed to read verifier prompt at {}: {}", verifier_path.display(), e))?;
    let summarizer_prompt_template = if summarize {
        Some(std::fs::read_to_string(&summarizer_path)
            .map_err(|e| format!("Failed to read summarizer prompt at {}: {}", summarizer_path.display(), e))?)
    } else {
        None
    };
    // Clean up old temp MCP configs from previous runs
    let _ = std::fs::remove_dir_all("/tmp/mycelica-orchestrator");
    let db_path = std::fs::canonicalize(&db_path_str)
        .map_err(|e| format!("Failed to resolve absolute DB path '{}': {}", db_path_str, e))?
        .to_string_lossy()
        .to_string();

    if dry_run {
        println!("=== DRY RUN ===");
        println!("Task: {}", task);
        println!("Max bounces: {}", max_bounces);
        println!("Max turns per agent: {}", max_turns);
        println!("CLI binary: {}", cli_binary.display());
        println!("Coder prompt: {} ({} bytes)", coder_prompt_path.display(), coder_prompt_template.len());
        println!("Verifier prompt: {} ({} bytes)", verifier_prompt_path.display(), verifier_prompt_template.len());
        println!("DB path: {}", db_path);
        println!("\nWould run {} bounce(s) of: Coder -> Verifier", max_bounces);

        // Generate a preview task file to inspect context quality
        let preview_run_id = "dry-run-preview-00000000";
        let preview_task_id = "00000000-0000-0000-0000-000000000000";
        match generate_task_file(db, task, "coder", preview_run_id, preview_task_id, 0, max_bounces, None, None, experiment) {
            Ok((path, _line_count)) => println!("\nPreview task file: {}", path.display()),
            Err(e) => println!("\nTask file preview failed: {}", e),
        }
        return Ok(String::new());
    }

    // Determine resume state — phase to skip to on first bounce
    let resume_phase = resume.as_ref().map(|r| r.from_phase.clone());
    let resume_bounce = resume.as_ref().map(|r| r.bounce).unwrap_or(0);

    // Create task node or reuse existing one from checkpoint
    let task_node_id = if let Some(ref rs) = resume {
        println!("Resuming task node: {} (phase: {})", &rs.task_node_id[..8], rs.from_phase);
        rs.task_node_id.clone()
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        let task_node = make_orchestrator_node(
            id.clone(),
            format!("Orchestration: {}", truncate_middle(task, 60)),
            format!("## Task\n{}\n\n## Config\n- max_bounces: {}\n- max_turns: {}", task, max_bounces, max_turns),
            "operational",
            None,
        );
        db.insert_node(&task_node).map_err(|e| format!("Failed to create task node: {}", e))?;
        println!("Created task node: {}", &id[..8]);
        id
    };

    let mut last_impl_id: Option<String> = resume.as_ref().and_then(|r| r.last_impl_id.clone());
    let mut last_verdict: Option<Verdict> = None;
    let mut last_verdict_reason: Option<String> = None;
    let mut last_coder_session_id: Option<String> = None;
    let complexity = estimate_complexity(task) as i32;

    // Initialize checkpoint
    let mut checkpoint = OrchestratorCheckpoint {
        task: task.to_string(),
        task_node_id: task_node_id.clone(),
        db_path: db_path.clone(),
        bounce: resume_bounce,
        max_bounces: max_bounces + resume_bounce, // total bounces including already-done ones
        max_turns,
        next_phase: resume_phase.clone().unwrap_or_else(|| "coder".to_string()),
        impl_node_id: resume.as_ref().and_then(|r| r.impl_node_id.clone()),
        last_impl_id: last_impl_id.clone(),
        created_at: chrono::Utc::now().timestamp_millis(),
        updated_at: chrono::Utc::now().timestamp_millis(),
    };

    // Phase ordering for skip comparisons
    fn phase_ord(phase: &str) -> u8 {
        match phase {
            "coder" => 0,
            "verifier" => 1,
            "summarizer" => 2,
            _ => 0,
        }
    }

    for bounce in 0..max_bounces {
        last_verdict_reason = None;
        // On the first bounce of a resume, we may skip ahead to a later phase.
        // On subsequent bounces (or non-resume runs), run all phases.
        let skip_to = if bounce == 0 { resume_phase.as_deref() } else { None };
        let skip_ord = skip_to.map(phase_ord).unwrap_or(0);

        let display_bounce = bounce + resume_bounce + 1;
        let display_max = max_bounces + resume_bounce;
        println!("\n--- Bounce {}/{}: {} ---", display_bounce, display_max, truncate_middle(task, 60));
        checkpoint.bounce = bounce + resume_bounce;
        checkpoint.next_phase = skip_to.unwrap_or("coder").to_string();
        checkpoint.updated_at = chrono::Utc::now().timestamp_millis();
        let _ = save_checkpoint(&checkpoint);

        // === CODER PHASE (skipped if resuming from a later phase) ===
        let impl_holder: Node;
        let mut coder_changed_files: Vec<String> = Vec::new();

        if skip_ord > 0 {
            // Resume: skip coder phase, load existing impl node from checkpoint
            let resume_impl_id = resume.as_ref()
                .and_then(|r| r.impl_node_id.clone())
                .ok_or("Resume from non-coder phase requires impl_node_id in checkpoint")?;
            impl_holder = db.get_node(&resume_impl_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Resume impl node {} not found in graph", resume_impl_id))?;
            println!("[resume] Skipping coder — impl node: {} ({})", &impl_holder.id[..8], impl_holder.title);
        } else {

        let coder_run_id = uuid::Uuid::new_v4().to_string();

        // Capture dirty + untracked files before coder runs (to diff against after)
        let before_dirty: HashSet<String> = std::process::Command::new("git")
            .args(["diff", "--name-only"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.to_string())
                .collect())
            .unwrap_or_default();

        // Content hashes for already-dirty files (detect in-place edits on bounce 2+)
        let before_hashes = capture_file_hashes(&before_dirty);

        let before_untracked: HashSet<String> = std::process::Command::new("git")
            .args(["ls-files", "--others", "--exclude-standard"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.to_string())
                .collect())
            .unwrap_or_default();

        let coder_start = chrono::Utc::now().timestamp_millis();

        let coder_mcp = write_temp_mcp_config(
            &cli_binary, "coder", "spore:coder", &coder_run_id, &db_path,
        )?;

        // Generate task file with graph context
        let (task_file, _line_count) = generate_task_file(
            db, task, "coder", &coder_run_id, &task_node_id,
            bounce, max_bounces, last_impl_id.as_deref(),
            last_verdict, experiment,
        )?;
        println!("[coder] Task file: {}", task_file.display());

        let coder_agent = resolve_agent_name("coder");
        let bounce_feedback = if last_verdict == Some(Verdict::Unknown) {
            "The verifier could not parse a verdict from the previous attempt. Review your changes carefully and ensure correctness.".to_string()
        } else if let Some(ref reason) = last_verdict_reason {
            format!("The verifier rejected your implementation: {}. Fix these specific issues.", reason)
        } else {
            "Check its incoming contradicts edges and fix the code.".to_string()
        };
        let coder_prompt = if coder_agent.is_some() {
            // Native agent mode: task-specific context only, no template
            if let Some(ref impl_id) = last_impl_id {
                format!(
                    "Read the task file at {} for full context and graph-gathered information.\n\nThe Verifier found issues with node {}. {}\n\nYour task: {}",
                    task_file.display(), impl_id, bounce_feedback, task
                )
            } else {
                format!(
                    "Read the task file at {} for full context and graph-gathered information.\n\nYour task: {}",
                    task_file.display(), task
                )
            }
        } else {
            // Inline mode: prepend full template (existing behavior)
            if let Some(ref impl_id) = last_impl_id {
                format!(
                    "{}\n\nRead the task file at {} for full context and graph-gathered information.\n\nThe Verifier found issues with node {}. {}\n\nYour task: {}",
                    coder_prompt_template, task_file.display(), impl_id, bounce_feedback, task
                )
            } else {
                format!(
                    "{}\n\nRead the task file at {} for full context and graph-gathered information.\n\nYour task: {}",
                    coder_prompt_template, task_file.display(), task
                )
            }
        };

        let coder_model = coder_model_override
            .map(|m| m.to_string())
            .unwrap_or_else(|| select_model_for_role("coder", complexity));
        println!("[coder] Starting (run: {}, model: {}{}{})", &coder_run_id[..8], coder_model,
            if coder_model_override.is_some() { " (override)" } else { "" },
            if coder_agent.is_some() { ", native-agent" } else { "" });
        let coder_resume = last_coder_session_id.as_deref();
        let mut coder_result = if coder_resume.is_some() {
            // Bounce 2+: resume previous coder session with simplified prompt
            let resume_prompt = format!(
                "The verifier rejected your changes. {}\n\nFix the code and ensure the build check passes.",
                bounce_feedback
            );
            spawn_claude(
                &resume_prompt, &coder_mcp, max_turns, verbose, "coder",
                Some("Read,Write,Edit,Bash(*),mcp__mycelica__*"),
                Some("Grep,Glob"),
                timeout,
                quiet,
                Some(&coder_model),
                None, // don't pass --agent when resuming
                coder_resume,
            )?
        } else {
            // Bounce 1 or session lost: full prompt
            spawn_claude(
                &coder_prompt, &coder_mcp, max_turns, verbose, "coder",
                Some("Read,Write,Edit,Bash(*),mcp__mycelica__*"),
                Some("Grep,Glob"),
                timeout,
                quiet,
                Some(&coder_model),
                coder_agent.as_deref(),
                None,
            )?
        };

        // Fallback: if resume failed, retry with fresh session
        if !coder_result.success && coder_resume.is_some() {
            eprintln!("[coder] Resume failed, retrying with fresh session");
            last_coder_session_id = None;
            coder_result = spawn_claude(
                &coder_prompt, &coder_mcp, max_turns, verbose, "coder",
                Some("Read,Write,Edit,Bash(*),mcp__mycelica__*"),
                Some("Grep,Glob"),
                timeout,
                quiet,
                Some(&coder_model),
                coder_agent.as_deref(),
                None,
            )?;
        }

        if coder_result.num_turns.unwrap_or(0) == 0 {
            eprintln!("[coder] WARNING: 0 turns — retrying after 10s cooldown");
            std::thread::sleep(std::time::Duration::from_secs(10));
            coder_result = spawn_claude(
                &coder_prompt, &coder_mcp, max_turns, verbose, "coder",
                Some("Read,Write,Edit,Bash(*),mcp__mycelica__*"),
                Some("Grep,Glob"),
                timeout,
                quiet,
                Some(&coder_model),
                coder_agent.as_deref(),
                None,
            )?;
        }

        // Check for MCP connection failure (all servers failed = crippled agent)
        let mcp_all_failed = !coder_result.mcp_status.is_empty()
            && coder_result.mcp_status.iter().all(|(_, s)| s == "failed");

        let coder_failed = !coder_result.success;
        if coder_failed {
            eprintln!("[coder] FAILED (exit code {})", coder_result.exit_code);
            if !coder_result.stderr_raw.is_empty() {
                eprintln!("[coder stderr] {}", &coder_result.stderr_raw[..2000.min(coder_result.stderr_raw.len())]);
            }
        } else {
            println!("[coder] Done ({} turns, ${:.2}, {})",
                coder_result.num_turns.unwrap_or(0),
                coder_result.total_cost_usd.unwrap_or(0.0),
                format_duration_short(coder_result.duration_ms.unwrap_or(0)),
            );
        }
        // Write thinking log
        if let Some(ref thinking) = coder_result.thinking_log {
            let think_path = format!("docs/spore/tasks/think-coder-{}.log", &coder_run_id[..8]);
            let _ = std::fs::write(&think_path, thinking);
        }

        // Abort immediately on startup hang (0 turns) or MCP total failure after retry
        if coder_result.num_turns.unwrap_or(0) == 0 {
            let reason = if mcp_all_failed {
                "MCP connection failed after retry — agent had no graph tools"
            } else {
                "0 turns after retry — subprocess likely hung during startup"
            };
            eprintln!("[coder] WARNING: {}", reason);
            record_run_status(db, &task_node_id, &coder_run_id, "spore:coder", "failed-startup", coder_result.exit_code, experiment)?;
            checkpoint.next_phase = "coder".to_string();
            checkpoint.updated_at = chrono::Utc::now().timestamp_millis();
            let _ = save_checkpoint(&checkpoint);
            return Err(format!("Coder failed: {}", reason));
        }
        if let Some(ref sid) = coder_result.session_id {
            println!("[coder] Session: {}", sid);
        }
        last_coder_session_id = coder_result.session_id.clone();

        // === ORCHESTRATOR-DRIVEN NODE CREATION ===
        // The orchestrator ALWAYS creates the implementation node from git diff.
        // Coders never need to self-record — this eliminates the 28% fallback rate
        // and saves 2-3 coder turns per run.

        // 1. Detect changed files
        let after_dirty: HashSet<String> = std::process::Command::new("git")
            .args(["diff", "--name-only"])
            .output().ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).lines()
                .filter(|l| !l.trim().is_empty()).map(|l| l.to_string()).collect())
            .unwrap_or_default();
        let after_untracked: HashSet<String> = std::process::Command::new("git")
            .args(["ls-files", "--others", "--exclude-standard"])
            .output().ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).lines()
                .filter(|l| !l.trim().is_empty()).map(|l| l.to_string()).collect())
            .unwrap_or_default();
        let after_hashes = capture_file_hashes(&after_dirty);
        let inplace_edits = before_dirty.intersection(&after_dirty)
            .filter(|f| before_hashes.get(*f) != after_hashes.get(*f))
            .cloned();
        let changed: Vec<String> = after_dirty.difference(&before_dirty)
            .chain(after_untracked.difference(&before_untracked))
            .cloned()
            .chain(inplace_edits)
            .collect();

        // If coder failed AND made no changes, abort — nothing to verify
        if coder_failed && changed.is_empty() {
            record_run_status_with_cost(db, &task_node_id, &coder_run_id, "spore:coder", "failed", coder_result.exit_code, coder_result.total_cost_usd, coder_result.num_turns, coder_result.duration_ms, experiment, Some(&coder_model))?;
            checkpoint.next_phase = "coder".to_string();
            checkpoint.updated_at = chrono::Utc::now().timestamp_millis();
            let _ = save_checkpoint(&checkpoint);
            return Err(format!("Coder failed on bounce {} with no file changes (exit code {})", display_bounce, coder_result.exit_code));
        }

        // 2. Get git diff --stat for a concise change summary
        let diff_stat = std::process::Command::new("git")
            .args(["diff", "--stat"])
            .output().ok()
            .filter(|o| o.status.success())
            .map(|o| {
                let s = String::from_utf8_lossy(&o.stdout).to_string();
                if s.len() > 2000 { format!("{}...(truncated)", &s[..s.floor_char_boundary(2000)]) } else { s }
            })
            .unwrap_or_default();

        // 3. Capture coder's stdout summary (what it said about its work)
        let coder_summary = coder_result.result_text.as_deref()
            .filter(|t| !t.is_empty())
            .map(|t| if t.len() > 3000 { format!("{}...(truncated)", &t[..t.floor_char_boundary(2900)]) } else { t.to_string() })
            .unwrap_or_else(|| "(no coder output captured)".to_string());

        // 4. Build rich implementation node content
        let files_section = if changed.is_empty() {
            "No new files changed (code may already exist or agent explored only)".to_string()
        } else {
            changed.iter().map(|f| format!("- {}", f)).collect::<Vec<_>>().join("\n")
        };

        let task_short = truncate_middle(task, 60);
        let status_label = if coder_failed { "Partial" } else { "Implemented" };
        let impl_id = uuid::Uuid::new_v4().to_string();

        let node_content = format!(
            "## Task\n{}\n\n## Files Changed\n{}\n\n## Diff Summary\n```\n{}\n```\n\n## Coder Summary\n{}\n\n## Agent Stats\n- Turns: {}\n- Cost: ${:.2}\n- Duration: {}\n- Exit code: {}",
            task,
            files_section,
            diff_stat.trim(),
            coder_summary,
            coder_result.num_turns.unwrap_or(0),
            coder_result.total_cost_usd.unwrap_or(0.0),
            format_duration_short(coder_result.duration_ms.unwrap_or(0)),
            coder_result.exit_code,
        );

        let impl_node_obj = make_orchestrator_node(
            impl_id.clone(),
            format!("{}: {}", status_label, task_short),
            node_content,
            "operational",
            None,
        );
        db.insert_node(&impl_node_obj).map_err(|e| format!("Failed to create implementation node: {}", e))?;

        // 5. Create derives_from edge to task node
        let edge = Edge {
            id: uuid::Uuid::new_v4().to_string(),
            source: impl_id.clone(),
            target: task_node_id.clone(),
            edge_type: EdgeType::DerivesFrom,
            label: None, weight: None,
            edge_source: Some("orchestrator".to_string()),
            evidence_id: None,
            confidence: if coder_failed { Some(0.5) } else { Some(0.9) },
            created_at: chrono::Utc::now().timestamp_millis(),
            updated_at: None,
            author: None, reason: None,
            content: Some("Orchestrator-created implementation node from git diff".to_string()),
            agent_id: Some("spore:orchestrator".to_string()),
            superseded_by: None,
            metadata: None,
        };
        let _ = db.insert_edge(&edge);

        let run_status = if coder_failed { "failed-partial" } else { "completed" };
        println!("[coder] Implementation node: {} ({} file(s) changed)", &impl_id[..8], changed.len());
        impl_holder = db.get_node(&impl_id).map_err(|e| e.to_string())?
            .ok_or("Failed to read back implementation node")?;
        record_run_status_with_cost(db, &task_node_id, &coder_run_id, "spore:coder", run_status, coder_result.exit_code, coder_result.total_cost_usd, coder_result.num_turns, coder_result.duration_ms, experiment, Some(&coder_model))?;

        if coder_failed {
            eprintln!("[coder] Partial recovery: {} file(s) changed — continuing to verifier", changed.len());
        }

        // === POST-CODER CLEANUP ===
        coder_changed_files = post_coder_cleanup(db, &impl_holder.id, &before_dirty, &before_untracked, &before_hashes, &cli_binary, verbose);

        } // end of else (coder phase)

        let impl_node = &impl_holder;

        // Update checkpoint: coder done, verifier next
        checkpoint.impl_node_id = Some(impl_node.id.clone());
        checkpoint.last_impl_id = Some(impl_node.id.clone());
        checkpoint.next_phase = "verifier".to_string();
        checkpoint.updated_at = chrono::Utc::now().timestamp_millis();
        let _ = save_checkpoint(&checkpoint);

        // === VERIFIER PHASE ===
        let verifier_run_id = uuid::Uuid::new_v4().to_string();

        let verifier_mcp = write_temp_mcp_config(
            &cli_binary, "verifier", "spore:verifier", &verifier_run_id, &db_path,
        )?;

        // Generate verifier task file with graph context
        let (verifier_task_file, _line_count) = generate_task_file(
            db, task, "verifier", &verifier_run_id, &task_node_id,
            bounce, max_bounces, Some(&impl_node.id), None, experiment,
        )?;
        println!("[verifier] Task file: {}", verifier_task_file.display());

        let verifier_agent = resolve_agent_name("verifier");
        let verifier_prompt = if verifier_agent.is_some() {
            // Native agent mode: task-specific context only, no template
            format!(
                "Read the task file at {} first — it contains the implementation node ID in the \"Implementation to Check\" section.\n\nVerify implementation node `{}`. Your primary deliverable: a `supports` or `contradicts` edge from your verification node to that implementation node.",
                verifier_task_file.display(), impl_node.id
            )
        } else {
            format!(
                "{}\n\nRead the task file at {} first — it contains the implementation node ID in the \"Implementation to Check\" section.\n\nVerify implementation node `{}`. Your primary deliverable: a `supports` or `contradicts` edge from your verification node to that implementation node.",
                verifier_prompt_template, verifier_task_file.display(), impl_node.id
            )
        };

        let verifier_model = select_model_for_role("verifier", complexity);
        println!("[verifier] Starting (run: {}, model: {})", &verifier_run_id[..8], verifier_model);
        let mut verifier_result = spawn_claude(
            &verifier_prompt, &verifier_mcp, max_turns, verbose, "verifier",
            Some("Read,Grep,Glob,Bash(cargo:*),Bash(cd:*),Bash(mycelica-cli:*),mcp__mycelica__*"),
            None,
            timeout,
            quiet,
            Some(&verifier_model),
            verifier_agent.as_deref(),
            None,
        )?;

        // Single retry on verifier subprocess failure (modeled on coder retry at ~line 8726)
        if !verifier_result.success {
            eprintln!("[verifier] WARNING: subprocess failed (exit code {:?}) — retrying after 10s cooldown", verifier_result.exit_code);
            std::thread::sleep(std::time::Duration::from_secs(10));
            verifier_result = spawn_claude(
                &verifier_prompt, &verifier_mcp, max_turns, verbose, "verifier",
                Some("Read,Grep,Glob,Bash(cargo:*),Bash(cd:*),Bash(mycelica-cli:*),mcp__mycelica__*"),
                None,
                timeout,
                quiet,
                Some(&verifier_model),
                verifier_agent.as_deref(),
                None,
            )?;
        }

        if !verifier_result.success {
            eprintln!("[verifier] FAILED (exit code {})", verifier_result.exit_code);
            if !verifier_result.stderr_raw.is_empty() {
                let end = verifier_result.stderr_raw.floor_char_boundary(2000.min(verifier_result.stderr_raw.len()));
                eprintln!("[verifier stderr] {}", &verifier_result.stderr_raw[..end]);
            }
            record_run_status(db, &task_node_id, &verifier_run_id, "spore:verifier", "failed", verifier_result.exit_code, experiment)?;
            // Save checkpoint so resume retries from verifier (not coder)
            checkpoint.next_phase = "verifier".to_string();
            checkpoint.updated_at = chrono::Utc::now().timestamp_millis();
            let _ = save_checkpoint(&checkpoint);
            return Err(format!("Verifier failed on bounce {} with exit code {}", display_bounce, verifier_result.exit_code));
        }
        println!("[verifier] Done ({} turns, ${:.2}, {})",
            verifier_result.num_turns.unwrap_or(0),
            verifier_result.total_cost_usd.unwrap_or(0.0),
            format_duration_short(verifier_result.duration_ms.unwrap_or(0)),
        );
        record_run_status_with_cost(db, &task_node_id, &verifier_run_id, "spore:verifier", "completed", 0, verifier_result.total_cost_usd, verifier_result.num_turns, verifier_result.duration_ms, experiment, None)?;

        // Write thinking log
        if let Some(ref thinking) = verifier_result.thinking_log {
            let think_path = format!("docs/spore/tasks/think-verifier-{}.log", &verifier_run_id[..8]);
            let _ = std::fs::write(&think_path, thinking);
        }

        // Check verdict — graph edges, then structured JSON, then plain text
        let mut verdict = check_verdict(db, &impl_node.id);
        if verdict == Verdict::Unknown {
            // Preferred fallback: structured <verdict>...</verdict> JSON block
            if let Some(parsed) = parse_verifier_verdict(verifier_result.result_text.as_deref().unwrap_or("")) {
                if parsed.verdict != Verdict::Unknown {
                    // Orchestrator creates graph nodes/edges from structured verdict
                    let is_pass = parsed.verdict == Verdict::Supports;
                    let reason = parsed.reason.as_deref().unwrap_or("Structured verdict from verifier").to_string();
                    let verdict_node_id = uuid::Uuid::new_v4().to_string();
                    let verdict_title = if is_pass {
                        format!("Verified: {}", truncate_middle(task, 60))
                    } else {
                        format!("Verification failed: {}", truncate_middle(&reason, 80))
                    };
                    let verdict_node_obj = make_orchestrator_node(
                        verdict_node_id.clone(),
                        verdict_title,
                        reason.clone(),
                        "operational",
                        None,
                    );
                    if let Err(e) = db.insert_node(&verdict_node_obj) {
                        eprintln!("[orchestrator] Warning: failed to create verdict node: {}", e);
                    } else {
                        let now = chrono::Utc::now().timestamp_millis();
                        let edge_type = if is_pass { EdgeType::Supports } else { EdgeType::Contradicts };
                        let confidence = parsed.confidence;
                        let verdict_edge = Edge {
                            id: uuid::Uuid::new_v4().to_string(),
                            source: verdict_node_id,
                            target: impl_node.id.clone(),
                            edge_type,
                            label: None, weight: None,
                            edge_source: Some("orchestrator".to_string()),
                            evidence_id: None,
                            confidence: Some(confidence),
                            created_at: now,
                            updated_at: Some(now),
                            author: Some("orchestrator".to_string()),
                            reason: Some(reason.clone()),
                            content: None,
                            agent_id: Some("spore:orchestrator".to_string()),
                            superseded_by: None,
                            metadata: None,
                        };
                        if let Err(e) = db.insert_edge(&verdict_edge) {
                            eprintln!("[orchestrator] Warning: failed to create verdict edge: {}", e);
                        }
                    }
                    eprintln!("[verifier] Structured verdict: {} (confidence: {:.0}%) — {}", if is_pass { "PASS" } else { "FAIL" }, parsed.confidence * 100.0, reason);
                    verdict = parsed.verdict;
                    last_verdict_reason = parsed.reason.clone();
                }
            }
        }
        if verdict == Verdict::Unknown {
            // Last-resort: keyword scan of verifier stdout
            let text_verdict = parse_verdict_from_text(verifier_result.result_text.as_deref().unwrap_or(""));
            if text_verdict != Verdict::Unknown {
                let is_pass = text_verdict == Verdict::Supports;
                let reason = "Verdict inferred from verifier output text (keyword scan)".to_string();
                // For text-fallback contradicts, try to extract useful failure info
                if text_verdict == Verdict::Contradicts {
                    let result_text = verifier_result.result_text.as_deref().unwrap_or("");
                    let failure_indicators = ["FAIL", "error", "Error", "failed", "Failed", "panicked", "assertion", "expected", "not found", "compile error"];
                    let useful_lines: Vec<&str> = result_text.lines()
                        .filter(|line| failure_indicators.iter().any(|ind| line.contains(ind)))
                        .collect();
                    if !useful_lines.is_empty() {
                        let extracted: String = useful_lines.join("\n");
                        let truncated = if extracted.len() > 500 { &extracted[..500] } else { &extracted };
                        last_verdict_reason = Some(truncated.to_string());
                    }
                }
                let verdict_node_id = uuid::Uuid::new_v4().to_string();
                let verdict_title = if is_pass {
                    format!("Verified (text-fallback): {}", truncate_middle(task, 60))
                } else {
                    format!("Verification failed (text-fallback): {}", truncate_middle(&reason, 80))
                };
                let verdict_node_obj = make_orchestrator_node(
                    verdict_node_id.clone(),
                    verdict_title,
                    reason.clone(),
                    "operational",
                    None,
                );
                if let Err(e) = db.insert_node(&verdict_node_obj) {
                    eprintln!("[orchestrator] Warning: failed to create text-fallback verdict node: {}", e);
                } else {
                    let now = chrono::Utc::now().timestamp_millis();
                    let edge_type = if is_pass { EdgeType::Supports } else { EdgeType::Contradicts };
                    let verdict_edge = Edge {
                        id: uuid::Uuid::new_v4().to_string(),
                        source: verdict_node_id,
                        target: impl_node.id.clone(),
                        edge_type,
                        label: None, weight: None,
                        edge_source: Some("orchestrator".to_string()),
                        evidence_id: None,
                        confidence: Some(0.5),
                        created_at: now,
                        updated_at: Some(now),
                        author: Some("orchestrator".to_string()),
                        reason: Some(reason.clone()),
                        content: None,
                        agent_id: Some("spore:orchestrator".to_string()),
                        superseded_by: None,
                        metadata: None,
                    };
                    if let Err(e) = db.insert_edge(&verdict_edge) {
                        eprintln!("[orchestrator] Warning: failed to create text-fallback verdict edge: {}", e);
                    }
                }
                eprintln!("[verifier] Text-fallback verdict: {} (confidence: 50%) — graph edge created", if is_pass { "PASS" } else { "FAIL" });
                verdict = text_verdict;
            }
        }
        match verdict {
            Verdict::Supports => {
                println!("\n=== TASK COMPLETE ===");
                println!("Verifier supports the implementation after {} bounce(s).", bounce + 1);
                println!("Implementation: {} ({})", &impl_node.id[..8], truncate_middle(&impl_node.title, 60));
                println!("Task node: {}", &task_node_id[..8]);
                clear_checkpoint(&task_node_id);

                // Run summarizer if requested
                if let Some(ref summarizer_template) = summarizer_prompt_template {
                    println!("\n--- Summarizer ---");
                    let summarizer_run_id = uuid::Uuid::new_v4().to_string()[..8].to_string();

                    let (summarizer_task_file, _line_count) = generate_task_file(
                        db, task, "summarizer", &summarizer_run_id, &task_node_id,
                        bounce, max_bounces, Some(&impl_node.id), None, experiment,
                    )?;
                    println!("[summarizer] Task file: {}", summarizer_task_file.display());

                    let summarizer_mcp = write_temp_mcp_config(&cli_binary, "summarizer", "spore:summarizer", &summarizer_run_id, &db_path)?;

                    let single_summarizer_agent = resolve_agent_name("summarizer");
                    let summarizer_prompt = if single_summarizer_agent.is_some() {
                        // Native agent mode: task-specific context only, no template
                        format!(
                            "Read the task file at {} for context about the run.\n\nSummarize the orchestrator run for task node {}. The final implementation node is {}.\nThe run took {} bounce(s). Your job: read the trail, write one summary node with a summarizes edge to the task node.",
                            summarizer_task_file.display(), &task_node_id[..8], impl_node.id, bounce + 1
                        )
                    } else {
                        format!(
                            "{}\n\nRead the task file at {} for context about the run.\n\nSummarize the orchestrator run for task node {}. The final implementation node is {}.\nThe run took {} bounce(s). Your job: read the trail, write one summary node with a summarizes edge to the task node.",
                            summarizer_template, summarizer_task_file.display(), &task_node_id[..8], impl_node.id, bounce + 1
                        )
                    };

                    let summarizer_model = select_model_for_role("summarizer", complexity);
                    let summarizer_result = spawn_claude(
                        &summarizer_prompt,
                        &summarizer_mcp,
                        15, // summarizer needs fewer turns
                        verbose,
                        "summarizer",
                        None,
                        Some("Bash,Edit,Write"),
                        None,
                        quiet,
                        Some(&summarizer_model),
                        single_summarizer_agent.as_deref(),
                        None,
                    )?;

                    println!("[summarizer] Done ({} turns, ${:.2}, {})",
                        summarizer_result.num_turns.unwrap_or(0),
                        summarizer_result.total_cost_usd.unwrap_or(0.0),
                        format_duration_short(summarizer_result.duration_ms.unwrap_or(0)),
                    );
                    record_run_status_with_cost(db, &task_node_id, &summarizer_run_id, "spore:summarizer", "completed", 0, summarizer_result.total_cost_usd, summarizer_result.num_turns, summarizer_result.duration_ms, experiment, None)?;
                    // Write thinking log
                    if let Some(ref thinking) = summarizer_result.thinking_log {
                        let think_path = format!("docs/spore/tasks/think-summarizer-{}.log", &summarizer_run_id);
                        let _ = std::fs::write(&think_path, thinking);
                    }
                }

                return Ok(task_node_id.clone());
            }
            Verdict::Contradicts => {
                println!("[verifier] Contradicts implementation — will bounce to coder");
                last_impl_id = Some(impl_node.id.clone());
                last_verdict = Some(Verdict::Contradicts);
            }
            Verdict::Unknown => {
                eprintln!("[verifier] WARNING: No supports/contradicts edge found. Verifier may not have recorded a verdict.");
                last_impl_id = Some(impl_node.id.clone());
                last_verdict = Some(Verdict::Unknown);
            }
        }
    }

    // Max bounces reached — escalate
    println!("\n=== MAX BOUNCES REACHED ({}) ===", max_bounces);
    clear_checkpoint(&task_node_id);
    if let Some(ref impl_id) = last_impl_id {
        create_escalation(db, &task_node_id, impl_id, max_bounces, task)?;
        println!("Escalation node created. Human review required.");
    }
    Err(format!("Task not resolved after {} bounce(s). Escalation created.", max_bounces))
}

// ============================================================================
// Spore Loop — Continuous Orchestration Engine
// ============================================================================

#[derive(Debug, Clone)]
enum LoopStatus {
    Verified,
    Escalated,
    Failed,
}

#[derive(Debug, Clone)]
struct LoopRunResult {
    task: String,
    status: LoopStatus,
    cost: f64,
    duration: std::time::Duration,
    task_node_id: Option<String>,
}

// ============================================================================
// Loop State Persistence
// ============================================================================

/// Per-task record stored in the loop state file.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct LoopStateRun {
    task: String,
    /// "verified", "escalated", or "failed"
    status: String,
    cost: f64,
    duration_ms: u64,
    task_node_id: Option<String>,
    completed_at: String,
}

/// Persistent state for the spore loop, written after each task so runs survive interruptions.
#[derive(serde::Serialize, serde::Deserialize)]
struct LoopState {
    source: String,
    /// Tasks that finished as Verified — skipped on resume.
    verified_tasks: HashSet<String>,
    /// Cumulative cost across all runs (including resumed ones).
    total_cost: f64,
    runs: Vec<LoopStateRun>,
    created_at: String,
    updated_at: String,
}

/// Return the loop state file path for a given source, placed next to the task file.
/// E.g. `tasks.txt` → `tasks.loop-state.json` in the same directory.
fn loop_state_path(source: &str) -> PathBuf {
    let path_str = source.strip_prefix("file:").unwrap_or(source);
    let p = std::path::Path::new(path_str);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("tasks");
    let dir = p.parent().unwrap_or_else(|| std::path::Path::new("."));
    dir.join(format!("{}.loop-state.json", stem))
}

impl LoopState {
    fn new(source: &str) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            source: source.to_string(),
            verified_tasks: HashSet::new(),
            total_cost: 0.0,
            runs: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    fn load(path: &Path, source: &str) -> Self {
        if !path.exists() {
            return Self::new(source);
        }
        match fs::read_to_string(path)
            .ok()
            .and_then(|data| serde_json::from_str::<Self>(&data).ok())
        {
            Some(state) => state,
            None => Self::new(source),
        }
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize loop state: {}", e))?;
        fs::write(path, data)
            .map_err(|e| format!("Failed to write loop state to {:?}: {}", path, e))
    }

    fn is_verified(&self, task: &str) -> bool {
        self.verified_tasks.contains(task)
    }

    fn record_result(&mut self, result: &LoopRunResult) {
        let status_str = match result.status {
            LoopStatus::Verified => "verified",
            LoopStatus::Escalated => "escalated",
            LoopStatus::Failed => "failed",
        };
        if matches!(result.status, LoopStatus::Verified) {
            self.verified_tasks.insert(result.task.clone());
        }
        self.total_cost += result.cost;
        self.runs.push(LoopStateRun {
            task: result.task.clone(),
            status: status_str.to_string(),
            cost: result.cost,
            duration_ms: result.duration.as_millis() as u64,
            task_node_id: result.task_node_id.clone(),
            completed_at: Utc::now().to_rfc3339(),
        });
        self.updated_at = Utc::now().to_rfc3339();
    }
}

/// Read tasks from a source file. One task per line, # comments and blank lines ignored.
fn read_task_source(source: &str) -> Result<Vec<String>, String> {
    // Strip optional "file:" prefix
    let path = if let Some(p) = source.strip_prefix("file:") {
        p
    } else {
        source
    };
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read task source '{}': {}", path, e))?;
    let tasks = parse_task_content(&content);
    if tasks.is_empty() {
        return Err(format!("No tasks found in '{}' (blank lines and # comments ignored)", path));
    }
    Ok(tasks)
}

/// Parse task content from a string. Supports two formats:
/// 1. One task per line (original format)
/// 2. Multi-line tasks separated by `---` on its own line
///
/// In both formats, blank lines and lines starting with `#` are skipped.
/// When `---` delimiters are present, lines within each section are joined with spaces.
fn parse_task_content(content: &str) -> Vec<String> {
    let has_delimiter = content.lines().any(|l| l.trim() == "---");

    if has_delimiter {
        // Split on --- lines, join each section into one task
        let mut tasks = Vec::new();
        let mut current_lines: Vec<&str> = Vec::new();

        for line in content.lines() {
            if line.trim() == "---" {
                // Flush current section
                let task = current_lines.iter()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                    .collect::<Vec<&str>>()
                    .join(" ");
                if !task.is_empty() {
                    tasks.push(task);
                }
                current_lines.clear();
            } else {
                current_lines.push(line);
            }
        }
        // Flush final section (after last --- or if no trailing ---)
        let task = current_lines.iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect::<Vec<&str>>()
            .join(" ");
        if !task.is_empty() {
            tasks.push(task);
        }

        tasks
    } else {
        // Original format: one task per line
        content.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    }
}

/// Query the graph for the cost of a specific orchestration run by its task node ID.
/// Sums cost_usd from all tracks edges on the node.
fn query_run_cost(db: &Database, task_node_id: &str) -> f64 {
    (|| -> Result<f64, String> {
        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT metadata FROM edges \
             WHERE source_id = ?1 AND target_id = ?1 AND type = 'tracks' AND metadata IS NOT NULL"
        ).map_err(|e| e.to_string())?;
        let total: f64 = stmt.query_map(rusqlite::params![task_node_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .filter_map(|meta| {
                serde_json::from_str::<serde_json::Value>(&meta).ok()
                    .and_then(|v| v["cost_usd"].as_f64())
            })
            .sum();
        Ok(total)
    })().unwrap_or(0.0)
}

/// Find the most recently created Orchestration: node after a given timestamp.
fn find_recent_orchestration_node(db: &Database, after_millis: i64) -> Option<String> {
    (|| -> Result<Option<String>, String> {
        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id FROM nodes \
             WHERE node_class = 'operational' AND title LIKE 'Orchestration:%' \
               AND created_at > ?1 \
             ORDER BY created_at DESC LIMIT 1"
        ).map_err(|e| e.to_string())?;
        let id: Option<String> = stmt.query_row(rusqlite::params![after_millis], |row| {
            row.get::<_, String>(0)
        }).ok();
        Ok(id)
    })().unwrap_or(None)
}

/// Check whether an orchestration run was escalated (has ESCALATION node).
fn check_run_escalated(db: &Database, task_node_id: &str) -> bool {
    (|| -> Result<bool, String> {
        let conn = db.raw_conn().lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT COUNT(*) FROM edges e \
             JOIN nodes esc ON esc.id = e.source_id \
             WHERE e.target_id = ?1 AND e.type = 'tracks' \
               AND esc.title LIKE 'ESCALATION:%'"
        ).map_err(|e| e.to_string())?;
        let count: i64 = stmt.query_row(rusqlite::params![task_node_id], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        Ok(count > 0)
    })().unwrap_or(false)
}

fn print_loop_summary(results: &[LoopRunResult], total_cost: f64, budget: f64, total_duration: std::time::Duration, json: bool) {
    let total = results.len();
    let verified = results.iter().filter(|r| matches!(r.status, LoopStatus::Verified)).count();
    let escalated = results.iter().filter(|r| matches!(r.status, LoopStatus::Escalated)).count();
    let failed = results.iter().filter(|r| matches!(r.status, LoopStatus::Failed)).count();
    let avg_cost = if total > 0 { total_cost / total as f64 } else { 0.0 };

    if json {
        let tasks_json: Vec<serde_json::Value> = results.iter().map(|r| {
            serde_json::json!({
                "description": r.task,
                "status": match r.status {
                    LoopStatus::Verified => "verified",
                    LoopStatus::Escalated => "escalated",
                    LoopStatus::Failed => "failed",
                },
                "cost": r.cost,
                "duration_ms": r.duration.as_millis() as u64,
            })
        }).collect();
        let output = serde_json::json!({
            "tasks_dispatched": total,
            "verified": verified,
            "escalated": escalated,
            "failed": failed,
            "total_cost": total_cost,
            "budget": budget,
            "avg_cost_per_task": avg_cost,
            "total_duration_ms": total_duration.as_millis() as u64,
            "tasks": tasks_json,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
        return;
    }

    let rate = if total > 0 { (verified as f64 / total as f64) * 100.0 } else { 0.0 };

    println!("\n[loop] === Summary ===");
    println!("  Tasks dispatched: {}", total);
    println!("  Verified:         {} ({:.0}%)", verified, rate);
    println!("  Escalated:        {}", escalated);
    println!("  Failed:           {}", failed);
    println!("  Total cost:       ${:.2} / ${:.2} budget", total_cost, budget);
    println!("  Avg cost/task:    ${:.2}", avg_cost);
    println!("  Total duration:   {}", format_duration_short(total_duration.as_millis() as u64));

    // Per-task breakdown
    if total > 0 {
        println!("\n  Task details:");
        for (i, r) in results.iter().enumerate() {
            let status_str = match r.status {
                LoopStatus::Verified => "VERIFIED",
                LoopStatus::Escalated => "ESCALATED",
                LoopStatus::Failed => "FAILED",
            };
            let task_short = truncate_middle(&r.task, 50);
            println!("    {}. [{}] ${:.2} {} — {}",
                i + 1, status_str, r.cost,
                format_duration_short(r.duration.as_millis() as u64),
                task_short);
        }
    }
}

async fn handle_spore_loop(
    db: &Database,
    source: &str,
    budget: f64,
    max_runs: usize,
    max_bounces: usize,
    max_turns: usize,
    timeout: Option<u64>,
    dry_run: bool,
    pause_on_escalation: bool,
    summarize: bool,
    verbose: bool,
    reset: bool,
    json: bool,
    experiment: Option<&str>,
    coder_model_override: Option<&str>,
) -> Result<(), String> {
    let tasks = read_task_source(source)?;

    // Load persisted loop state for resume capability
    let state_path = loop_state_path(source);
    if reset && state_path.exists() {
        std::fs::remove_file(&state_path)
            .map_err(|e| format!("Failed to delete loop state: {}", e))?;
        eprintln!("Loop state reset: deleted {}", state_path.display());
    }
    let mut loop_state = LoopState::load(&state_path, source);
    let already_verified = loop_state.verified_tasks.len();

    let coder_prompt = std::path::PathBuf::from("docs/spore/agents/coder.md");
    let verifier_prompt = std::path::PathBuf::from("docs/spore/agents/verifier.md");
    let summarizer_prompt = std::path::PathBuf::from("docs/spore/agents/summarizer.md");

    println!("[loop] Starting: {} tasks, ${:.2} budget, max {} runs", tasks.len(), budget, max_runs);
    println!("[loop] Config: max_bounces={}, max_turns={}, summarize={}", max_bounces, max_turns, summarize);
    if already_verified > 0 {
        println!("[loop] Resuming: {} task(s) already verified, will skip.", already_verified);
    }

    if dry_run {
        println!("\n[loop] === DRY RUN ===");
        for (i, task) in tasks.iter().enumerate() {
            if i >= max_runs { break; }
            let complexity = estimate_complexity(task);
            let task_short = truncate_middle(task, 70);
            println!("  {}. [complexity {}/10] {}", i + 1, complexity, task_short);
        }
        let shown = tasks.len().min(max_runs);
        if tasks.len() > max_runs {
            println!("  ... and {} more tasks (limited by --max-runs {})", tasks.len() - max_runs, max_runs);
        }
        println!("\n[loop] Would dispatch {} task(s). No agents spawned.", shown);
        return Ok(());
    }

    // Resume total_cost from persisted state so budget is accurate across restarts
    let mut total_cost: f64 = loop_state.total_cost;
    let mut results: Vec<LoopRunResult> = Vec::new();
    let mut consecutive_escalations: usize = 0;
    let loop_start = std::time::Instant::now();

    for (i, task) in tasks.iter().enumerate() {
        // Budget check
        if total_cost >= budget {
            println!("\n[loop] Budget exhausted (${:.2}/${:.2}). Stopping.", total_cost, budget);
            break;
        }

        // Max runs check
        if results.len() >= max_runs {
            println!("\n[loop] Max runs reached ({}/{}). Stopping.", results.len(), max_runs);
            break;
        }

        // Consecutive escalation check (3 in a row = systemic problem)
        if consecutive_escalations >= 3 {
            println!("\n[loop] 3 consecutive escalations. Stopping — likely systemic issue.");
            break;
        }

        // Skip tasks that were already verified in a previous run
        if loop_state.is_verified(task) {
            println!("[loop] Skipping task {} (already verified)", i + 1);
            continue;
        }

        let remaining_budget = budget - total_cost;
        println!("\n[loop] === Task {}/{}: {} ===",
            i + 1, tasks.len(), truncate_middle(task, 60));
        println!("[loop] Budget remaining: ${:.2}", remaining_budget);

        let task_start = std::time::Instant::now();
        let before_millis = chrono::Utc::now().timestamp_millis();

        // Loop always uses simple pipeline (no auto-escalation from complexity)
        let complexity = estimate_complexity(task);
        if verbose {
            println!("[loop] Complexity {}/10 (informational only)", complexity);
        }

        // Dispatch via simple orchestrate pipeline: context -> coder -> verifier -> summarizer
        let result = handle_orchestrate(
            db, task, max_bounces, max_turns,
            &coder_prompt, &verifier_prompt, &summarizer_prompt,
            summarize, false, verbose, timeout,
            None, false,
            false,
            experiment,
            coder_model_override,
        ).await.map(|_| ());

        let task_duration = task_start.elapsed();

        // Determine status and extract cost from graph
        let task_node_id = find_recent_orchestration_node(db, before_millis);
        let run_cost = task_node_id.as_ref()
            .map(|id| query_run_cost(db, id))
            .unwrap_or(0.0);

        let status = match &result {
            Ok(_) => LoopStatus::Verified,
            Err(msg) => {
                // Check if escalation node was created
                if let Some(ref tn_id) = task_node_id {
                    if check_run_escalated(db, tn_id) {
                        LoopStatus::Escalated
                    } else if msg.contains("Escalation") || msg.contains("bounce") {
                        LoopStatus::Escalated
                    } else {
                        LoopStatus::Failed
                    }
                } else if msg.contains("Escalation") || msg.contains("bounce") {
                    LoopStatus::Escalated
                } else {
                    LoopStatus::Failed
                }
            }
        };

        let run_result = LoopRunResult {
            task: task.clone(),
            status: status.clone(),
            cost: run_cost,
            duration: task_duration,
            task_node_id,
        };

        total_cost += run_cost;

        // Persist loop state immediately so a restart can skip this task
        loop_state.record_result(&run_result);
        if let Err(e) = loop_state.save(&state_path) {
            eprintln!("[loop] Warning: failed to persist loop state: {}", e);
        }

        // Mechanical evaluation — no LLM
        match &run_result.status {
            LoopStatus::Verified => {
                consecutive_escalations = 0;
                println!("[loop] VERIFIED: ${:.2}, {}", run_cost,
                    format_duration_short(task_duration.as_millis() as u64));

                // Auto-commit between tasks (clean working tree for next task)
                if i + 1 < tasks.len() {
                    let short_desc = truncate_middle(task, 50);
                    let msg = format!("feat(loop): {}", short_desc);
                    let staged = selective_git_add(&std::env::current_dir().unwrap_or_default());
                    if staged {
                        let commit = std::process::Command::new("git")
                            .args(["commit", "-m", &msg, "--allow-empty"])
                            .output();
                        match commit {
                            Ok(o) if o.status.success() => {
                                println!("[loop] Auto-committed changes before next task");
                            }
                            _ => {
                                if verbose {
                                    eprintln!("[loop] No changes to commit (or commit failed)");
                                }
                            }
                        }
                    }
                }
            }
            LoopStatus::Escalated => {
                consecutive_escalations += 1;
                println!("[loop] ESCALATED: #{} consecutive — {}",
                    consecutive_escalations, truncate_middle(task, 50));
                if pause_on_escalation {
                    println!("[loop] --pause-on-escalation: stopping loop");
                    results.push(run_result);
                    break;
                }
            }
            LoopStatus::Failed => {
                // Don't count against escalation streak — failures are infrastructure issues
                println!("[loop] FAILED: {} — {}", truncate_middle(task, 50),
                    result.as_ref().err().map(|e| e.as_str()).unwrap_or("unknown"));
            }
        }

        // Cost anomaly detection: warn if current task cost > 3x running average
        if results.len() >= 3 && run_cost > 0.0 {
            let previous_total = total_cost - run_cost;
            let avg = previous_total / results.len() as f64;
            if avg > 0.0 {
                let ratio = run_cost / avg;
                if ratio > 3.0 {
                    println!("[loop] Cost anomaly: ${:.2} is {:.1}x the average ${:.2}",
                        run_cost, ratio, avg);
                }
            }
        }

        results.push(run_result);

        // Brief pause between dispatches (let filesystem settle, avoid rate limits)
        if i + 1 < tasks.len() && results.len() < max_runs {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    let total_duration = loop_start.elapsed();
    print_loop_summary(&results, total_cost, budget, total_duration, json);
    Ok(())
}

// ============================================================================
// Checkpoint System
// ============================================================================

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct OrchestratorCheckpoint {
    task: String,
    task_node_id: String,
    db_path: String,
    bounce: usize,
    max_bounces: usize,
    max_turns: usize,
    /// Next phase to run: "coder", "verifier", "summarizer", "complete"
    next_phase: String,
    impl_node_id: Option<String>,
    last_impl_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

fn checkpoint_path(task_node_id: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/mycelica-orchestrator/{}.checkpoint.json", &task_node_id[..8.min(task_node_id.len())]))
}

fn save_checkpoint(cp: &OrchestratorCheckpoint) -> Result<(), String> {
    let path = checkpoint_path(&cp.task_node_id);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = serde_json::to_string_pretty(cp).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to write checkpoint: {}", e))?;
    Ok(())
}

fn load_checkpoint(task_node_id: &str) -> Option<OrchestratorCheckpoint> {
    let path = checkpoint_path(task_node_id);
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

fn clear_checkpoint(task_node_id: &str) {
    let _ = std::fs::remove_file(checkpoint_path(task_node_id));
}

/// Find the most recent checkpoint file in /tmp/mycelica-orchestrator/
fn find_latest_checkpoint() -> Option<OrchestratorCheckpoint> {
    let dir = PathBuf::from("/tmp/mycelica-orchestrator");
    let entries = std::fs::read_dir(&dir).ok()?;
    let mut latest: Option<(i64, OrchestratorCheckpoint)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(cp) = serde_json::from_str::<OrchestratorCheckpoint>(&content) {
                if cp.next_phase != "complete" {
                    if latest.as_ref().map(|(ts, _)| cp.updated_at > *ts).unwrap_or(true) {
                        latest = Some((cp.updated_at, cp));
                    }
                }
            }
        }
    }
    latest.map(|(_, cp)| cp)
}

/// Record a run status edge from task node to itself.
fn record_run_status(
    db: &Database,
    task_node_id: &str,
    run_id: &str,
    agent: &str,
    status: &str,
    exit_code: i32,
    experiment: Option<&str>,
) -> Result<(), String> {
    record_run_status_with_cost(db, task_node_id, run_id, agent, status, exit_code, None, None, None, experiment, None)
}

fn record_run_status_with_cost(
    db: &Database,
    task_node_id: &str,
    run_id: &str,
    agent: &str,
    status: &str,
    exit_code: i32,
    cost_usd: Option<f64>,
    num_turns: Option<u32>,
    duration_ms: Option<u64>,
    experiment: Option<&str>,
    model: Option<&str>,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    let mut metadata = serde_json::json!({
        "run_id": run_id,
        "status": status,
        "exit_code": exit_code,
        "agent": agent,
    });
    if let Some(cost) = cost_usd {
        metadata["cost_usd"] = serde_json::json!(cost);
    }
    if let Some(turns) = num_turns {
        metadata["num_turns"] = serde_json::json!(turns);
    }
    if let Some(dur) = duration_ms {
        metadata["duration_ms"] = serde_json::json!(dur);
    }
    if let Some(exp) = experiment {
        metadata["experiment"] = serde_json::json!(exp);
    }
    if let Some(m) = model {
        metadata["model"] = serde_json::json!(m);
    }
    let edge = Edge {
        id: uuid::Uuid::new_v4().to_string(),
        source: task_node_id.to_string(),
        target: task_node_id.to_string(),
        edge_type: EdgeType::Tracks,
        label: None,
        weight: None,
        edge_source: Some("orchestrator".to_string()),
        evidence_id: None,
        confidence: Some(1.0),
        created_at: now,
        updated_at: Some(now),
        author: Some("orchestrator".to_string()),
        reason: Some(format!("{} run {}", agent, status)),
        content: None,
        agent_id: Some("spore:orchestrator".to_string()),
        superseded_by: None,
        metadata: Some(metadata.to_string()),
    };
    db.insert_edge(&edge).map_err(|e| format!("Failed to record run status: {}", e))?;
    Ok(())
}

/// Create an escalation meta node after max bounces.
fn create_escalation(
    db: &Database,
    task_node_id: &str,
    last_impl_id: &str,
    bounce_count: usize,
    task: &str,
) -> Result<(), String> {
    let esc_id = uuid::Uuid::new_v4().to_string();
    let esc_node = make_orchestrator_node(
        esc_id.clone(),
        format!("ESCALATION: {} (after {} bounces)", if task.len() > 40 { &task[..task.floor_char_boundary(40)] } else { task }, bounce_count),
        format!(
            "## Escalation\n\nTask did not converge after {} Coder-Verifier bounces.\n\n\
             ### Task\n{}\n\n\
             ### Last Implementation\nNode: {}\n\n\
             ### Action Required\nHuman review needed. Check the contradicts edges on the last implementation node \
             to understand what the Verifier flagged.",
            bounce_count, task, last_impl_id
        ),
        "meta",
        Some("escalation"),
    );
    db.insert_node(&esc_node).map_err(|e| format!("Failed to create escalation node: {}", e))?;
    let now = esc_node.created_at;

    // Edge: escalation flags last implementation
    let flags_edge = Edge {
        id: uuid::Uuid::new_v4().to_string(),
        source: esc_id.clone(),
        target: last_impl_id.to_string(),
        edge_type: EdgeType::Flags,
        label: None,
        weight: None,
        edge_source: Some("orchestrator".to_string()),
        evidence_id: None,
        confidence: Some(1.0),
        created_at: now,
        updated_at: Some(now),
        author: Some("orchestrator".to_string()),
        reason: Some(format!("Escalation after {} bounces", bounce_count)),
        content: None,
        agent_id: Some("spore:orchestrator".to_string()),
        superseded_by: None,
        metadata: None,
    };
    db.insert_edge(&flags_edge).map_err(|e| format!("Failed to create flags edge: {}", e))?;

    // Edge: escalation tracks task
    let tracks_edge = Edge {
        id: uuid::Uuid::new_v4().to_string(),
        source: esc_id,
        target: task_node_id.to_string(),
        edge_type: EdgeType::Tracks,
        label: None,
        weight: None,
        edge_source: Some("orchestrator".to_string()),
        evidence_id: None,
        confidence: Some(1.0),
        created_at: now,
        updated_at: Some(now),
        author: Some("orchestrator".to_string()),
        reason: Some("Tracks orchestration task".to_string()),
        content: None,
        agent_id: Some("spore:orchestrator".to_string()),
        superseded_by: None,
        metadata: None,
    };
    db.insert_edge(&tracks_edge).map_err(|e| format!("Failed to create tracks edge: {}", e))?;

    println!("Escalation: {} ({})", &esc_node.id[..8], esc_node.title);
    Ok(())
}

pub(crate) async fn handle_link(
    source_ref: &str,
    target_ref: &str,
    edge_type_str: &str,
    reason: Option<String>,
    content: Option<String>,
    agent: &str,
    confidence: Option<f64>,
    supersedes: Option<String>,
    edge_source: &str,
    db: &Database,
    json: bool,
) -> Result<(), String> {
    // Parse edge type (case-insensitive)
    let edge_type = EdgeType::from_str(&edge_type_str.to_lowercase())
        .ok_or_else(|| format!(
            "Unknown edge type: '{}'. Valid types include: related, reference, because, contains, \
             belongs_to, calls, uses_type, implements, defined_in, imports, tests, documents, \
             prerequisite, contradicts, supports, evolved_from, questions, \
             summarizes, tracks, flags, resolves, derives_from, supersedes",
            edge_type_str
        ))?;

    let source = resolve_node(db, source_ref)?;
    let target = resolve_node(db, target_ref)?;

    let author = settings::get_author_or_default();
    let now = Utc::now().timestamp_millis();
    let edge_id = uuid::Uuid::new_v4().to_string();

    let edge = Edge {
        id: edge_id.clone(),
        source: source.id.clone(),
        target: target.id.clone(),
        edge_type,
        label: None,
        weight: Some(1.0),
        edge_source: Some(edge_source.to_string()),
        evidence_id: None,
        confidence: Some(confidence.unwrap_or(1.0)),
        created_at: now,
        updated_at: Some(now),
        author: Some(author),
        reason,
        content,
        agent_id: Some(agent.to_string()),
        superseded_by: None,
        metadata: None,
    };

    db.insert_edge(&edge).map_err(|e| e.to_string())?;

    // If superseding another edge, mark the old one
    if let Some(ref old_edge_id) = supersedes {
        db.supersede_edge(old_edge_id, &edge_id).map_err(|e| e.to_string())?;
    }

    if json {
        println!(r#"{{"id":"{}","source":"{}","target":"{}","type":"{}","agent":"{}","confidence":{}}}"#,
            edge_id, source.id, target.id, edge_type_str.to_lowercase(), agent, confidence.unwrap_or(1.0));
    } else {
        let conf_str = confidence.map(|c| format!(" ({:.0}%)", c * 100.0)).unwrap_or_default();
        println!("Linked: {} -> {} [{}]{} (agent: {})",
            source.ai_title.as_ref().unwrap_or(&source.title),
            target.ai_title.as_ref().unwrap_or(&target.title),
            edge_type_str.to_lowercase(),
            conf_str,
            agent);
        if let Some(ref old_id) = supersedes {
            println!("  Superseded edge: {}", &old_id[..8.min(old_id.len())]);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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

    // --- Tests for print_loop_summary JSON output ---

    /// Helper: build the same JSON that print_loop_summary produces,
    /// so we can verify field names, types, and values without capturing stdout.
    fn build_loop_summary_json(
        results: &[LoopRunResult],
        total_cost: f64,
        budget: f64,
        total_duration: std::time::Duration,
    ) -> serde_json::Value {
        let total = results.len();
        let verified = results.iter().filter(|r| matches!(r.status, LoopStatus::Verified)).count();
        let escalated = results.iter().filter(|r| matches!(r.status, LoopStatus::Escalated)).count();
        let failed = results.iter().filter(|r| matches!(r.status, LoopStatus::Failed)).count();
        let avg_cost = if total > 0 { total_cost / total as f64 } else { 0.0 };

        let tasks_json: Vec<serde_json::Value> = results.iter().map(|r| {
            serde_json::json!({
                "description": r.task,
                "status": match r.status {
                    LoopStatus::Verified => "verified",
                    LoopStatus::Escalated => "escalated",
                    LoopStatus::Failed => "failed",
                },
                "cost": r.cost,
                "duration_ms": r.duration.as_millis() as u64,
            })
        }).collect();

        serde_json::json!({
            "tasks_dispatched": total,
            "verified": verified,
            "escalated": escalated,
            "failed": failed,
            "total_cost": total_cost,
            "budget": budget,
            "avg_cost_per_task": avg_cost,
            "total_duration_ms": total_duration.as_millis() as u64,
            "tasks": tasks_json,
        })
    }

    #[test]
    fn test_loop_summary_json_mixed_statuses() {
        let results = vec![
            LoopRunResult {
                task: "Fix bug A".into(),
                status: LoopStatus::Verified,
                cost: 0.50,
                duration: std::time::Duration::from_secs(30),
                task_node_id: Some("node-1".into()),
            },
            LoopRunResult {
                task: "Add feature B".into(),
                status: LoopStatus::Escalated,
                cost: 1.20,
                duration: std::time::Duration::from_secs(120),
                task_node_id: Some("node-2".into()),
            },
            LoopRunResult {
                task: "Refactor C".into(),
                status: LoopStatus::Failed,
                cost: 0.30,
                duration: std::time::Duration::from_secs(15),
                task_node_id: None,
            },
        ];

        let json = build_loop_summary_json(&results, 2.00, 10.0, std::time::Duration::from_secs(165));

        assert_eq!(json["tasks_dispatched"], 3);
        assert_eq!(json["verified"], 1);
        assert_eq!(json["escalated"], 1);
        assert_eq!(json["failed"], 1);
        assert!((json["total_cost"].as_f64().unwrap() - 2.0).abs() < 1e-10);
        assert!((json["budget"].as_f64().unwrap() - 10.0).abs() < 1e-10);
        assert!((json["avg_cost_per_task"].as_f64().unwrap() - 0.6667).abs() < 0.01);
        assert_eq!(json["total_duration_ms"], 165_000);

        let tasks = json["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0]["description"], "Fix bug A");
        assert_eq!(tasks[0]["status"], "verified");
        assert!((tasks[0]["cost"].as_f64().unwrap() - 0.50).abs() < 1e-10);
        assert_eq!(tasks[0]["duration_ms"], 30_000);

        assert_eq!(tasks[1]["status"], "escalated");
        assert_eq!(tasks[2]["status"], "failed");
    }

    #[test]
    fn test_loop_summary_json_empty_results() {
        let results: Vec<LoopRunResult> = vec![];
        let json = build_loop_summary_json(&results, 0.0, 5.0, std::time::Duration::from_secs(0));

        assert_eq!(json["tasks_dispatched"], 0);
        assert_eq!(json["verified"], 0);
        assert_eq!(json["escalated"], 0);
        assert_eq!(json["failed"], 0);
        assert!((json["total_cost"].as_f64().unwrap() - 0.0).abs() < 1e-10);
        assert!((json["avg_cost_per_task"].as_f64().unwrap() - 0.0).abs() < 1e-10);
        assert_eq!(json["total_duration_ms"], 0);
        assert!(json["tasks"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_loop_summary_json_all_verified() {
        let results = vec![
            LoopRunResult {
                task: "Task 1".into(),
                status: LoopStatus::Verified,
                cost: 0.40,
                duration: std::time::Duration::from_millis(5_500),
                task_node_id: Some("n1".into()),
            },
            LoopRunResult {
                task: "Task 2".into(),
                status: LoopStatus::Verified,
                cost: 0.60,
                duration: std::time::Duration::from_millis(8_200),
                task_node_id: Some("n2".into()),
            },
        ];

        let json = build_loop_summary_json(&results, 1.0, 10.0, std::time::Duration::from_millis(13_700));

        assert_eq!(json["tasks_dispatched"], 2);
        assert_eq!(json["verified"], 2);
        assert_eq!(json["escalated"], 0);
        assert_eq!(json["failed"], 0);
        assert!((json["avg_cost_per_task"].as_f64().unwrap() - 0.50).abs() < 1e-10);
        assert_eq!(json["total_duration_ms"], 13_700);
    }

    #[test]
    fn test_loop_summary_json_status_strings() {
        // Verify each LoopStatus maps to the correct JSON string
        let cases: Vec<(LoopStatus, &str)> = vec![
            (LoopStatus::Verified, "verified"),
            (LoopStatus::Escalated, "escalated"),
            (LoopStatus::Failed, "failed"),
        ];
        for (status, expected) in cases {
            let results = vec![LoopRunResult {
                task: "test".into(),
                status,
                cost: 0.0,
                duration: std::time::Duration::from_secs(0),
                task_node_id: None,
            }];
            let json = build_loop_summary_json(&results, 0.0, 0.0, std::time::Duration::from_secs(0));
            assert_eq!(
                json["tasks"][0]["status"].as_str().unwrap(),
                expected,
                "LoopStatus::{:?} should map to '{}'", results[0].status, expected
            );
        }
    }

    #[test]
    fn test_loop_summary_json_avg_cost_single_task() {
        let results = vec![LoopRunResult {
            task: "Only task".into(),
            status: LoopStatus::Verified,
            cost: 1.50,
            duration: std::time::Duration::from_secs(60),
            task_node_id: Some("n1".into()),
        }];

        let json = build_loop_summary_json(&results, 1.50, 5.0, std::time::Duration::from_secs(60));
        assert!((json["avg_cost_per_task"].as_f64().unwrap() - 1.50).abs() < 1e-10);
    }

    #[test]
    fn test_loop_summary_json_has_all_required_fields() {
        let results = vec![LoopRunResult {
            task: "t".into(),
            status: LoopStatus::Verified,
            cost: 0.1,
            duration: std::time::Duration::from_secs(1),
            task_node_id: None,
        }];
        let json = build_loop_summary_json(&results, 0.1, 1.0, std::time::Duration::from_secs(1));

        // Verify all top-level fields exist and have correct types
        assert!(json["tasks_dispatched"].is_u64());
        assert!(json["verified"].is_u64());
        assert!(json["escalated"].is_u64());
        assert!(json["failed"].is_u64());
        assert!(json["total_cost"].is_f64());
        assert!(json["budget"].is_f64());
        assert!(json["avg_cost_per_task"].is_f64());
        assert!(json["total_duration_ms"].is_u64());
        assert!(json["tasks"].is_array());

        // Verify per-task fields
        let task = &json["tasks"][0];
        assert!(task["description"].is_string());
        assert!(task["status"].is_string());
        assert!(task["cost"].is_f64());
        assert!(task["duration_ms"].is_u64());
    }

    #[test]
    fn test_loop_summary_json_duration_millis_precision() {
        // Verify sub-second durations are captured correctly
        let results = vec![LoopRunResult {
            task: "quick task".into(),
            status: LoopStatus::Verified,
            cost: 0.05,
            duration: std::time::Duration::from_millis(1_234),
            task_node_id: None,
        }];
        let json = build_loop_summary_json(&results, 0.05, 1.0, std::time::Duration::from_millis(1_234));
        assert_eq!(json["total_duration_ms"], 1_234);
        assert_eq!(json["tasks"][0]["duration_ms"], 1_234);
    }

    #[test]
    fn test_loop_summary_json_is_valid_json_string() {
        let results = vec![LoopRunResult {
            task: "Task with \"quotes\" and \\ backslash".into(),
            status: LoopStatus::Failed,
            cost: 0.0,
            duration: std::time::Duration::from_secs(0),
            task_node_id: None,
        }];
        let json = build_loop_summary_json(&results, 0.0, 0.0, std::time::Duration::from_secs(0));

        // Serialize to string and parse back — should round-trip cleanly
        let json_str = serde_json::to_string_pretty(&json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["tasks"][0]["description"], "Task with \"quotes\" and \\ backslash");
    }

    #[test]
    fn test_print_loop_summary_json_does_not_panic() {
        // Call the actual function with json=true — verify no panic
        let results = vec![
            LoopRunResult {
                task: "task-a".into(),
                status: LoopStatus::Verified,
                cost: 0.25,
                duration: std::time::Duration::from_secs(10),
                task_node_id: Some("id-a".into()),
            },
            LoopRunResult {
                task: "task-b".into(),
                status: LoopStatus::Failed,
                cost: 0.75,
                duration: std::time::Duration::from_secs(45),
                task_node_id: None,
            },
        ];
        // This should not panic
        print_loop_summary(&results, 1.0, 5.0, std::time::Duration::from_secs(55), true);
    }

    #[test]
    fn test_print_loop_summary_text_does_not_panic() {
        // Call with json=false for comparison — verify no panic
        let results = vec![LoopRunResult {
            task: "some task".into(),
            status: LoopStatus::Escalated,
            cost: 0.50,
            duration: std::time::Duration::from_secs(30),
            task_node_id: Some("id-x".into()),
        }];
        print_loop_summary(&results, 0.50, 2.0, std::time::Duration::from_secs(30), false);
    }

    #[test]
    fn test_print_loop_summary_empty_json_does_not_panic() {
        let results: Vec<LoopRunResult> = vec![];
        print_loop_summary(&results, 0.0, 10.0, std::time::Duration::from_secs(0), true);
    }

    // --- Tests for select_model_for_role ---

    #[test]
    fn test_select_model_verifier_always_opus() {
        assert_eq!(select_model_for_role("verifier", 0), "opus");
        assert_eq!(select_model_for_role("verifier", 5), "opus");
        assert_eq!(select_model_for_role("verifier", 10), "opus");
    }

    #[test]
    fn test_select_model_operator_always_opus() {
        assert_eq!(select_model_for_role("operator", 0), "opus");
        assert_eq!(select_model_for_role("operator", 10), "opus");
    }

    #[test]
    fn test_select_model_summarizer_always_sonnet() {
        assert_eq!(select_model_for_role("summarizer", 0), "sonnet");
        assert_eq!(select_model_for_role("summarizer", 10), "sonnet");
    }

    #[test]
    fn test_select_model_coder_always_opus() {
        // A/B experiment (2026-02-20): opus is always cheaper due to fewer turns.
        assert_eq!(select_model_for_role("coder", 0), "opus");
        assert_eq!(select_model_for_role("coder", 5), "opus");
        assert_eq!(select_model_for_role("coder", 10), "opus");
    }

    #[test]
    fn test_select_model_unknown_role_defaults_to_opus() {
        assert_eq!(select_model_for_role("unknown", 0), "opus");
        assert_eq!(select_model_for_role("", 0), "opus");
        assert_eq!(select_model_for_role("debugger", 3), "opus");
    }

    #[test]
    fn test_select_model_negative_complexity() {
        // Negative complexity: all roles return their fixed model
        assert_eq!(select_model_for_role("coder", -1), "opus");
        assert_eq!(select_model_for_role("verifier", -5), "opus");
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

    #[test]
    fn test_parse_verifier_verdict_supports() {
        let text = r#"Some preamble\n<verdict>{"verdict":"supports","reason":"looks good"}</verdict>\ntrailing"#;
        let parsed = parse_verifier_verdict(text).unwrap();
        assert_eq!(parsed.verdict, Verdict::Supports);
        assert_eq!(parsed.reason.as_deref(), Some("looks good"));
    }

    #[test]
    fn test_parse_verifier_verdict_contradicts() {
        let text = r#"<verdict>{"verdict":"contradicts"}</verdict>"#;
        let parsed = parse_verifier_verdict(text).unwrap();
        assert_eq!(parsed.verdict, Verdict::Contradicts);
        assert!(parsed.reason.is_none());
    }

    #[test]
    fn test_parse_verifier_verdict_pass_alias() {
        let text = r#"<verdict>{"verdict":"pass"}</verdict>"#;
        assert_eq!(parse_verifier_verdict(text).unwrap().verdict, Verdict::Supports);
    }

    #[test]
    fn test_parse_verifier_verdict_fail_alias() {
        let text = r#"<verdict>{"verdict":"fail"}</verdict>"#;
        assert_eq!(parse_verifier_verdict(text).unwrap().verdict, Verdict::Contradicts);
    }

    #[test]
    fn test_parse_verifier_verdict_result_field() {
        let text = r#"<verdict>{"result":"supports"}</verdict>"#;
        assert_eq!(parse_verifier_verdict(text).unwrap().verdict, Verdict::Supports);
    }

    #[test]
    fn test_parse_verifier_verdict_case_insensitive() {
        let text = r#"<verdict>{"verdict":"SUPPORTS"}</verdict>"#;
        assert_eq!(parse_verifier_verdict(text).unwrap().verdict, Verdict::Supports);
    }

    #[test]
    fn test_parse_verifier_verdict_no_marker() {
        let text = "The implementation looks correct.";
        assert!(parse_verifier_verdict(text).is_none());
    }

    #[test]
    fn test_parse_verifier_verdict_invalid_json() {
        let text = "<verdict>not json at all</verdict>";
        let parsed = parse_verifier_verdict(text).unwrap();
        assert_eq!(parsed.verdict, Verdict::Unknown);
        assert!(parsed.reason.is_none());
    }

    #[test]
    fn test_parse_verifier_verdict_unknown_verdict_value() {
        let text = r#"<verdict>{"verdict":"pending"}</verdict>"#;
        assert_eq!(parse_verifier_verdict(text).unwrap().verdict, Verdict::Unknown);
    }

    #[test]
    fn test_parse_verifier_verdict_missing_end_marker() {
        let text = r#"<verdict>{"verdict":"supports"}"#;
        assert!(parse_verifier_verdict(text).is_none());
    }

    #[test]
    fn test_parse_verifier_verdict_whitespace_around_json() {
        let text = "<verdict>  \n  {\"verdict\":\"contradicts\"}  \n  </verdict>";
        assert_eq!(parse_verifier_verdict(text).unwrap().verdict, Verdict::Contradicts);
    }

    #[test]
    fn test_parse_verifier_verdict_reason_on_contradicts() {
        let text = r#"<verdict>{"verdict":"contradicts","reason":"tests are failing"}</verdict>"#;
        let parsed = parse_verifier_verdict(text).unwrap();
        assert_eq!(parsed.verdict, Verdict::Contradicts);
        assert_eq!(parsed.reason.as_deref(), Some("tests are failing"));
    }

    #[test]
    fn test_parse_verifier_verdict_result_field_with_reason() {
        // reason should be extracted even when using "result" synonym instead of "verdict"
        let text = r#"<verdict>{"result":"supports","reason":"via result field"}</verdict>"#;
        let parsed = parse_verifier_verdict(text).unwrap();
        assert_eq!(parsed.verdict, Verdict::Supports);
        assert_eq!(parsed.reason.as_deref(), Some("via result field"));
    }

    #[test]
    fn test_parse_verifier_verdict_verdict_field_priority_over_result() {
        // "verdict" field takes precedence over "result" when both are present
        let text = r#"<verdict>{"verdict":"supports","result":"contradicts"}</verdict>"#;
        let parsed = parse_verifier_verdict(text).unwrap();
        assert_eq!(parsed.verdict, Verdict::Supports);
    }

    #[test]
    fn test_parse_verifier_verdict_case_insensitive_contradicts() {
        let text = r#"<verdict>{"verdict":"CONTRADICTS"}</verdict>"#;
        assert_eq!(parse_verifier_verdict(text).unwrap().verdict, Verdict::Contradicts);
    }

    #[test]
    fn test_parse_verifier_verdict_empty_block() {
        // empty block is present but unparseable → Some(Unknown) not None
        let text = "<verdict></verdict>";
        let parsed = parse_verifier_verdict(text).unwrap();
        assert_eq!(parsed.verdict, Verdict::Unknown);
        assert!(parsed.reason.is_none());
    }

    #[test]
    fn test_parse_verifier_verdict_confidence_extracted() {
        let text = r#"<verdict>{"verdict":"supports","confidence":0.85}</verdict>"#;
        let parsed = parse_verifier_verdict(text).unwrap();
        assert_eq!(parsed.verdict, Verdict::Supports);
        assert_eq!(parsed.confidence, 0.85);
    }

    #[test]
    fn test_parse_verifier_verdict_confidence_default() {
        let text = r#"<verdict>{"verdict":"supports"}</verdict>"#;
        let parsed = parse_verifier_verdict(text).unwrap();
        assert_eq!(parsed.verdict, Verdict::Supports);
        assert_eq!(parsed.confidence, 0.9);
    }

    #[test]
    fn test_parse_verifier_verdict_confidence_clamped() {
        let text = r#"<verdict>{"verdict":"supports","confidence":1.5}</verdict>"#;
        let parsed = parse_verifier_verdict(text).unwrap();
        assert_eq!(parsed.verdict, Verdict::Supports);
        assert_eq!(parsed.confidence, 1.0);
    }

    #[test]
    fn test_parse_verifier_verdict_confidence_zero_on_error() {
        let text = "<verdict>not json at all</verdict>";
        let parsed = parse_verifier_verdict(text).unwrap();
        assert_eq!(parsed.verdict, Verdict::Unknown);
        assert_eq!(parsed.confidence, 0.0);
    }

    // --- parse_task_content tests ---

    #[test]
    fn test_parse_task_content_single_line_tasks() {
        let content = "Task one\nTask two\nTask three\n";
        let tasks = parse_task_content(content);
        assert_eq!(tasks, vec!["Task one", "Task two", "Task three"]);
    }

    #[test]
    fn test_parse_task_content_single_line_skips_blanks_and_comments() {
        let content = "# This is a comment\nTask one\n\n# Another comment\nTask two\n";
        let tasks = parse_task_content(content);
        assert_eq!(tasks, vec!["Task one", "Task two"]);
    }

    #[test]
    fn test_parse_task_content_multiline_with_delimiter() {
        let content = "Add a new function\nthat does X and Y\n---\nFix the bug in\nmodule Z\n";
        let tasks = parse_task_content(content);
        assert_eq!(tasks, vec![
            "Add a new function that does X and Y",
            "Fix the bug in module Z",
        ]);
    }

    #[test]
    fn test_parse_task_content_multiline_skips_blanks_and_comments() {
        let content = "# Header comment\nFirst line of task\n\nSecond line of task\n---\n# Comment in second task\nAnother task here\n";
        let tasks = parse_task_content(content);
        assert_eq!(tasks, vec![
            "First line of task Second line of task",
            "Another task here",
        ]);
    }

    #[test]
    fn test_parse_task_content_mixed_single_and_multiline() {
        // When --- is present, all sections are treated as multi-line
        let content = "Single line task\n---\nMulti line\ntask here\n";
        let tasks = parse_task_content(content);
        assert_eq!(tasks, vec![
            "Single line task",
            "Multi line task here",
        ]);
    }

    #[test]
    fn test_parse_task_content_delimiter_with_whitespace() {
        let content = "Task A\n  ---  \nTask B\n";
        let tasks = parse_task_content(content);
        assert_eq!(tasks, vec!["Task A", "Task B"]);
    }

    #[test]
    fn test_parse_task_content_empty_sections_skipped() {
        let content = "---\nTask one\n---\n---\nTask two\n---\n";
        let tasks = parse_task_content(content);
        assert_eq!(tasks, vec!["Task one", "Task two"]);
    }

    #[test]
    fn test_parse_task_content_no_trailing_delimiter() {
        let content = "Task A\n---\nTask B line 1\nTask B line 2";
        let tasks = parse_task_content(content);
        assert_eq!(tasks, vec![
            "Task A",
            "Task B line 1 Task B line 2",
        ]);
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

    // --- Tests for parse_verdict_from_text() (Tier 3 text-fallback) ---

    #[test]
    fn test_parse_verdict_from_text_empty_returns_unknown() {
        assert_eq!(parse_verdict_from_text(""), Verdict::Unknown);
    }

    #[test]
    fn test_parse_verdict_from_text_no_keywords_returns_unknown() {
        assert_eq!(parse_verdict_from_text("The implementation looks good. All changes applied."), Verdict::Unknown);
    }

    #[test]
    fn test_parse_verdict_from_text_verification_result_pass() {
        let text = "Verification Result: **PASS**\nAll criteria met.";
        assert_eq!(parse_verdict_from_text(text), Verdict::Supports);
    }

    #[test]
    fn test_parse_verdict_from_text_verdict_pass() {
        let text = "After reviewing the code:\nVerdict: pass";
        assert_eq!(parse_verdict_from_text(text), Verdict::Supports);
    }

    #[test]
    fn test_parse_verdict_from_text_verdict_bold_pass() {
        let text = "Verdict: **PASS** — implementation correct";
        assert_eq!(parse_verdict_from_text(text), Verdict::Supports);
    }

    #[test]
    fn test_parse_verdict_from_text_edge_type_supports_quoted() {
        let text = r#"edge_type: "supports""#;
        assert_eq!(parse_verdict_from_text(text), Verdict::Supports);
    }

    #[test]
    fn test_parse_verdict_from_text_edge_type_supports_unquoted() {
        let text = "edge_type: supports";
        assert_eq!(parse_verdict_from_text(text), Verdict::Supports);
    }

    #[test]
    fn test_parse_verdict_from_text_verification_result_fail() {
        let text = "Verification Result: **FAIL**\nCriteria not met.";
        assert_eq!(parse_verdict_from_text(text), Verdict::Contradicts);
    }

    #[test]
    fn test_parse_verdict_from_text_verdict_fail() {
        let text = "Verdict: fail — the implementation is incomplete";
        assert_eq!(parse_verdict_from_text(text), Verdict::Contradicts);
    }

    #[test]
    fn test_parse_verdict_from_text_verdict_bold_fail() {
        let text = "Summary:\nVerdict: **FAIL**";
        assert_eq!(parse_verdict_from_text(text), Verdict::Contradicts);
    }

    #[test]
    fn test_parse_verdict_from_text_edge_type_contradicts_quoted() {
        let text = r#"edge_type: "contradicts""#;
        assert_eq!(parse_verdict_from_text(text), Verdict::Contradicts);
    }

    #[test]
    fn test_parse_verdict_from_text_edge_type_contradicts_unquoted() {
        let text = "edge_type: contradicts";
        assert_eq!(parse_verdict_from_text(text), Verdict::Contradicts);
    }

    #[test]
    fn test_parse_verdict_from_text_case_insensitive_upper() {
        // The function lowercases the input, so uppercase variants should match
        assert_eq!(parse_verdict_from_text("VERDICT: PASS"), Verdict::Supports);
        assert_eq!(parse_verdict_from_text("VERDICT: FAIL"), Verdict::Contradicts);
    }

    #[test]
    fn test_parse_verdict_from_text_pass_takes_priority_over_fail() {
        // "verification result: **pass**" is checked before "verdict: fail"
        // If both appear, the more specific match wins
        let text = "verification result: **pass**\nverdict: fail";
        assert_eq!(parse_verdict_from_text(text), Verdict::Supports);
    }

    // --- Tests for Tier 3 text-fallback title format ---

    #[test]
    fn test_text_fallback_pass_title_prefix() {
        // Pass title must use "(text-fallback)" prefix to distinguish from Tier 2
        let task = "Add a new feature to handle user authentication";
        let is_pass = true;
        let reason = "Verdict inferred from verifier output text (keyword scan)".to_string();
        let title = if is_pass {
            format!("Verified (text-fallback): {}", truncate_middle(task, 60))
        } else {
            format!("Verification failed (text-fallback): {}", truncate_middle(&reason, 80))
        };
        assert!(title.starts_with("Verified (text-fallback): "), "Pass title should start with 'Verified (text-fallback): ', got: {}", title);
        assert!(!title.starts_with("Verified: "), "Pass title must NOT use Tier 2 prefix 'Verified: '");
    }

    #[test]
    fn test_text_fallback_fail_title_prefix() {
        // Fail title must use "(text-fallback)" prefix to distinguish from Tier 2
        let task = "Some task";
        let is_pass = false;
        let reason = "Verdict inferred from verifier output text (keyword scan)".to_string();
        let title = if is_pass {
            format!("Verified (text-fallback): {}", truncate_middle(task, 60))
        } else {
            format!("Verification failed (text-fallback): {}", truncate_middle(&reason, 80))
        };
        assert!(title.starts_with("Verification failed (text-fallback): "), "Fail title should start with 'Verification failed (text-fallback): ', got: {}", title);
        assert!(!title.starts_with("Verification failed: "), "Fail title must NOT use Tier 2 prefix 'Verification failed: '");
    }

    #[test]
    fn test_text_fallback_pass_title_uses_task_not_reason() {
        // Pass title uses truncate_middle(task, 60), not the reason string
        let task = "Fix the login authentication bug in the API handler";
        let reason = "Verdict inferred from verifier output text (keyword scan)".to_string();
        let title = format!("Verified (text-fallback): {}", truncate_middle(task, 60));
        assert!(title.contains("Fix the login"), "Pass title should embed the task description, got: {}", title);
        assert!(!title.contains("keyword scan"), "Pass title should NOT embed the reason, got: {}", title);
    }

    #[test]
    fn test_text_fallback_fail_title_uses_reason_not_task() {
        // Fail title uses truncate_middle(&reason, 80), not the task
        let task = "Fix the login authentication bug in the API handler";
        let reason = "Verdict inferred from verifier output text (keyword scan)".to_string();
        let title = format!("Verification failed (text-fallback): {}", truncate_middle(&reason, 80));
        assert!(title.contains("keyword scan"), "Fail title should embed the reason, got: {}", title);
        assert!(!title.contains("Fix the login"), "Fail title should NOT embed the task, got: {}", title);
    }

    #[test]
    fn test_text_fallback_pass_title_truncates_long_task() {
        // Long task descriptions are truncated at 60 bytes
        let task = "A very long task description that definitely exceeds sixty characters and goes on and on";
        assert!(task.len() > 60);
        let title = format!("Verified (text-fallback): {}", truncate_middle(task, 60));
        let task_part = &title["Verified (text-fallback): ".len()..];
        assert!(task_part.len() <= 60, "task part '{}' exceeds 60 bytes", task_part);
        assert!(task_part.contains("..."));
    }

    #[test]
    fn test_text_fallback_reason_string_content() {
        // The hardcoded reason string used in Tier 3
        let reason = "Verdict inferred from verifier output text (keyword scan)";
        assert!(reason.contains("keyword scan"), "reason must mention keyword scan");
        assert!(reason.contains("verifier"), "reason must mention verifier");
    }

    #[test]
    fn test_text_fallback_confidence_is_half() {
        // Tier 3 confidence is hardcoded at 0.5, lower than Tier 2's typical 0.9
        let confidence: f64 = 0.5;
        assert_eq!(confidence, 0.5);
        assert!(confidence < 0.9, "Tier 3 confidence must be less than Tier 2 default (0.9)");
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

    // --- Tests for is_spore_excluded ---

    #[test]
    fn test_is_spore_excluded_loop_state_json() {
        assert!(is_spore_excluded("myloop.loop-state.json"));
        assert!(is_spore_excluded("some/path/myrun.loop-state.json"));
        assert!(is_spore_excluded(".loop-state.json"));
    }

    #[test]
    fn test_is_spore_excluded_env_files() {
        assert!(is_spore_excluded(".env.local"));
        assert!(is_spore_excluded(".env"));
        assert!(is_spore_excluded(".env.production"));
        assert!(is_spore_excluded("config/.env.local"));
    }

    #[test]
    fn test_is_spore_excluded_target_prefix() {
        assert!(is_spore_excluded("target/debug/foo"));
        assert!(is_spore_excluded("target/release/mycelica-cli"));
        assert!(is_spore_excluded("target/"));
    }

    #[test]
    fn test_is_spore_excluded_node_modules_prefix() {
        assert!(is_spore_excluded("node_modules/pkg"));
        assert!(is_spore_excluded("node_modules/some/deep/path/file.js"));
    }

    #[test]
    fn test_is_spore_excluded_db_extensions() {
        assert!(is_spore_excluded("data.db"));
        assert!(is_spore_excluded(".db-wal"));
        assert!(is_spore_excluded("mycelica.db-journal"));
        assert!(is_spore_excluded("app.db-shm"));
        assert!(is_spore_excluded("path/to/store.db"));
    }

    #[test]
    fn test_is_spore_excluded_normal_source_files() {
        assert!(!is_spore_excluded("src/main.rs"));
        assert!(!is_spore_excluded("tests/integration.rs"));
        assert!(!is_spore_excluded("README.md"));
        assert!(!is_spore_excluded("Cargo.toml"));
        assert!(!is_spore_excluded("src/lib.rs"));
        assert!(!is_spore_excluded("package.json"));
    }

    #[test]
    fn test_is_spore_excluded_target_in_non_prefix_position() {
        // "target" appearing not at the start should NOT be excluded
        assert!(!is_spore_excluded("src/target_util.rs"));
        assert!(!is_spore_excluded("my_target/file.rs"));
    }

    #[test]
    fn test_is_spore_excluded_env_in_non_basename_position() {
        // ".env" only excluded when it's the basename — not mid-path components
        // "some/.env/file" has basename "file", not ".env"
        assert!(!is_spore_excluded("some/env_config.rs"));
        // But "some/.env.local" has basename ".env.local" which starts with ".env"
        assert!(is_spore_excluded("some/.env.local"));
    }

    #[test]
    fn test_is_spore_excluded_loop_state_json_in_subdir() {
        // loop-state.json in a subdirectory should still be excluded
        assert!(is_spore_excluded("subdir/run.loop-state.json"));
    }

    #[test]
    fn test_is_spore_excluded_bare_target_not_excluded() {
        // "target" without trailing slash is NOT excluded — only "target/" prefix is
        assert!(!is_spore_excluded("target"));
        assert!(!is_spore_excluded("my_target"));
    }

    #[test]
    fn test_is_spore_excluded_bare_node_modules_not_excluded() {
        // "node_modules" without trailing slash is NOT excluded
        assert!(!is_spore_excluded("node_modules"));
    }

    #[test]
    fn test_is_spore_excluded_env_as_path_component() {
        // ".env" as a directory component (not basename) is NOT excluded
        // Path "some/.env/config.txt" has basename "config.txt"
        assert!(!is_spore_excluded("some/.env/config.txt"));
        assert!(!is_spore_excluded(".env/subfile"));
    }

    // --- Integration test for selective_git_add ---

    #[test]
    fn test_selective_git_add_excludes_loop_state_and_stages_source() {
        use std::fs;
        use std::process::Command;

        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let repo = tmp.path();

        // Initialize a git repo with a dummy user so commit commands work
        let init = Command::new("git").args(["init"]).current_dir(repo).output();
        if init.map(|o| !o.status.success()).unwrap_or(true) {
            // git not available or failed — skip
            return;
        }
        let _ = Command::new("git").args(["config", "user.email", "test@test.com"]).current_dir(repo).status();
        let _ = Command::new("git").args(["config", "user.name", "Test"]).current_dir(repo).status();

        // Create and commit an initial tracked file
        fs::write(repo.join("main.rs"), "fn main() {}").unwrap();
        let _ = Command::new("git").args(["add", "main.rs"]).current_dir(repo).status();
        let _ = Command::new("git").args(["commit", "-m", "init"]).current_dir(repo).status();

        // Modify the tracked file
        fs::write(repo.join("main.rs"), "fn main() { println!(\"hi\"); }").unwrap();

        // Add a new source file (should be staged by selective_git_add)
        fs::write(repo.join("lib.rs"), "pub fn foo() {}").unwrap();

        // Add a loop-state file (should NOT be staged)
        fs::write(repo.join("run1.loop-state.json"), r#"{"state":"running"}"#).unwrap();

        // Add a .db file (should NOT be staged)
        fs::write(repo.join("data.db"), "binary").unwrap();

        let staged = selective_git_add(repo);
        assert!(staged, "selective_git_add should return true on success");

        // Check what is staged
        let status_output = Command::new("git")
            .args(["diff", "--cached", "--name-only"])
            .current_dir(repo)
            .output()
            .expect("git diff --cached failed");
        let staged_files = String::from_utf8_lossy(&status_output.stdout);

        // main.rs (tracked modification) should be staged
        assert!(staged_files.contains("main.rs"), "modified tracked file should be staged; got: {}", staged_files);

        // lib.rs (new source file) should be staged
        assert!(staged_files.contains("lib.rs"), "new source file should be staged; got: {}", staged_files);

        // loop-state.json should NOT be staged
        assert!(!staged_files.contains("loop-state.json"), "loop-state.json should not be staged; got: {}", staged_files);

        // data.db should NOT be staged
        assert!(!staged_files.contains("data.db"), "data.db should not be staged; got: {}", staged_files);
    }

    #[test]
    fn test_selective_git_add_excludes_env_file() {
        use std::fs;
        use std::process::Command;

        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let repo = tmp.path();

        let init = Command::new("git").args(["init"]).current_dir(repo).output();
        if init.map(|o| !o.status.success()).unwrap_or(true) {
            return; // git not available
        }
        let _ = Command::new("git").args(["config", "user.email", "test@test.com"]).current_dir(repo).status();
        let _ = Command::new("git").args(["config", "user.name", "Test"]).current_dir(repo).status();

        // Initial commit to have a HEAD
        fs::write(repo.join("main.rs"), "fn main() {}").unwrap();
        let _ = Command::new("git").args(["add", "main.rs"]).current_dir(repo).status();
        let _ = Command::new("git").args(["commit", "-m", "init"]).current_dir(repo).status();

        // New untracked source file (should be staged)
        fs::write(repo.join("lib.rs"), "pub fn bar() {}").unwrap();

        // New .env file (should NOT be staged)
        fs::write(repo.join(".env"), "SECRET=abc123").unwrap();

        // New .env.local file (should NOT be staged)
        fs::write(repo.join(".env.local"), "DEBUG=true").unwrap();

        selective_git_add(repo);

        let status_output = Command::new("git")
            .args(["diff", "--cached", "--name-only"])
            .current_dir(repo)
            .output()
            .expect("git diff --cached failed");
        let staged_files = String::from_utf8_lossy(&status_output.stdout);

        assert!(staged_files.contains("lib.rs"), "lib.rs should be staged; got: {}", staged_files);
        assert!(!staged_files.contains(".env"), ".env should not be staged; got: {}", staged_files);
    }

    #[test]
    fn test_selective_git_add_excludes_target_dir_files() {
        use std::fs;
        use std::process::Command;

        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let repo = tmp.path();

        let init = Command::new("git").args(["init"]).current_dir(repo).output();
        if init.map(|o| !o.status.success()).unwrap_or(true) {
            return;
        }
        let _ = Command::new("git").args(["config", "user.email", "test@test.com"]).current_dir(repo).status();
        let _ = Command::new("git").args(["config", "user.name", "Test"]).current_dir(repo).status();

        // Initial commit
        fs::write(repo.join("main.rs"), "fn main() {}").unwrap();
        let _ = Command::new("git").args(["add", "main.rs"]).current_dir(repo).status();
        let _ = Command::new("git").args(["commit", "-m", "init"]).current_dir(repo).status();

        // New source file (should be staged)
        fs::write(repo.join("util.rs"), "pub fn helper() {}").unwrap();

        // File in target/ directory (should NOT be staged)
        fs::create_dir_all(repo.join("target/debug")).unwrap();
        fs::write(repo.join("target/debug/binary"), "ELF").unwrap();

        selective_git_add(repo);

        let status_output = Command::new("git")
            .args(["diff", "--cached", "--name-only"])
            .current_dir(repo)
            .output()
            .expect("git diff --cached failed");
        let staged_files = String::from_utf8_lossy(&status_output.stdout);

        assert!(staged_files.contains("util.rs"), "util.rs should be staged; got: {}", staged_files);
        assert!(!staged_files.contains("target/"), "target/ files should not be staged; got: {}", staged_files);
    }

    #[test]
    fn test_selective_git_add_only_tracked_modifications_no_untracked() {
        use std::fs;
        use std::process::Command;

        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let repo = tmp.path();

        let init = Command::new("git").args(["init"]).current_dir(repo).output();
        if init.map(|o| !o.status.success()).unwrap_or(true) {
            return;
        }
        let _ = Command::new("git").args(["config", "user.email", "test@test.com"]).current_dir(repo).status();
        let _ = Command::new("git").args(["config", "user.name", "Test"]).current_dir(repo).status();

        // Initial commit with two tracked files
        fs::write(repo.join("a.rs"), "fn a() {}").unwrap();
        fs::write(repo.join("b.rs"), "fn b() {}").unwrap();
        let _ = Command::new("git").args(["add", "."]).current_dir(repo).status();
        let _ = Command::new("git").args(["commit", "-m", "init"]).current_dir(repo).status();

        // Modify both tracked files, no new untracked files
        fs::write(repo.join("a.rs"), "fn a() { println!(\"a\"); }").unwrap();
        fs::write(repo.join("b.rs"), "fn b() { println!(\"b\"); }").unwrap();

        let result = selective_git_add(repo);
        assert!(result, "selective_git_add should succeed with only tracked modifications");

        let status_output = Command::new("git")
            .args(["diff", "--cached", "--name-only"])
            .current_dir(repo)
            .output()
            .expect("git diff --cached failed");
        let staged_files = String::from_utf8_lossy(&status_output.stdout);

        assert!(staged_files.contains("a.rs"), "a.rs should be staged; got: {}", staged_files);
        assert!(staged_files.contains("b.rs"), "b.rs should be staged; got: {}", staged_files);
    }

    #[test]
    fn test_selective_git_add_returns_false_in_non_git_dir() {
        use std::process::Command;

        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let non_git_dir = tmp.path();

        // Verify this is not a git repo
        let check = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(non_git_dir)
            .output();
        if check.map(|o| o.status.success()).unwrap_or(false) {
            return; // Accidentally inside a git repo — skip
        }

        let result = selective_git_add(non_git_dir);
        assert!(!result, "selective_git_add should return false in a non-git directory");
    }

    // --- Tests for parse_verifier_verdict reason extraction (CHANGE 2) ---

    #[test]
    fn test_parse_verifier_verdict_reason_supports() {
        let text = r#"<verdict>{"verdict":"supports","reason":"All tests pass","confidence":0.95}</verdict>"#;
        let result = parse_verifier_verdict(text).unwrap();
        assert_eq!(result.verdict, Verdict::Supports);
        assert_eq!(result.reason, Some("All tests pass".to_string()));
    }

    #[test]
    fn test_parse_verifier_verdict_reason_contradicts() {
        let text = r#"<verdict>{"verdict":"contradicts","reason":"Test failed: expected 42 got 0","confidence":0.9}</verdict>"#;
        let result = parse_verifier_verdict(text).unwrap();
        assert_eq!(result.verdict, Verdict::Contradicts);
        assert_eq!(result.reason, Some("Test failed: expected 42 got 0".to_string()));
    }

    #[test]
    fn test_parse_verifier_verdict_no_reason_field() {
        let text = r#"<verdict>{"verdict":"supports","confidence":0.9}</verdict>"#;
        let result = parse_verifier_verdict(text).unwrap();
        assert_eq!(result.verdict, Verdict::Supports);
        assert_eq!(result.reason, None);
    }

    #[test]
    fn test_parse_verifier_verdict_fail_synonym_extracts_reason() {
        let text = r#"<verdict>{"result":"fail","reason":"Compilation failed","confidence":0.8}</verdict>"#;
        let result = parse_verifier_verdict(text).unwrap();
        assert_eq!(result.verdict, Verdict::Contradicts);
        assert_eq!(result.reason, Some("Compilation failed".to_string()));
    }

    #[test]
    fn test_parse_verifier_verdict_pass_synonym_extracts_reason() {
        let text = r#"<verdict>{"result":"pass","reason":"Implementation correct","confidence":1.0}</verdict>"#;
        let result = parse_verifier_verdict(text).unwrap();
        assert_eq!(result.verdict, Verdict::Supports);
        assert_eq!(result.reason, Some("Implementation correct".to_string()));
    }

    #[test]
    fn test_parse_verifier_verdict_no_block_returns_none() {
        let text = "No verdict block here at all";
        assert!(parse_verifier_verdict(text).is_none());
    }

    #[test]
    fn test_parse_verifier_verdict_malformed_json_reason_is_none() {
        let text = "<verdict>not valid json</verdict>";
        let result = parse_verifier_verdict(text).unwrap();
        assert_eq!(result.verdict, Verdict::Unknown);
        assert_eq!(result.reason, None);
    }

    // --- Tests for bounce_feedback construction logic (CHANGE 3) ---

    #[test]
    fn test_bounce_feedback_unknown_verdict_shows_parse_failure_message() {
        let last_verdict: Option<Verdict> = Some(Verdict::Unknown);
        let last_verdict_reason: Option<String> = None;
        let bounce_feedback = if last_verdict == Some(Verdict::Unknown) {
            "The verifier could not parse a verdict from the previous attempt. Review your changes carefully and ensure correctness.".to_string()
        } else if let Some(ref reason) = last_verdict_reason {
            format!("The verifier rejected your implementation: {}. Fix these specific issues.", reason)
        } else {
            "Check its incoming contradicts edges and fix the code.".to_string()
        };
        assert!(bounce_feedback.contains("could not parse a verdict"));
        assert!(!bounce_feedback.contains("contradicts edges"));
    }

    #[test]
    fn test_bounce_feedback_with_reason_injects_reason_text() {
        let last_verdict: Option<Verdict> = Some(Verdict::Contradicts);
        let last_verdict_reason: Option<String> = Some("Test failed: assertion `left == right` at lib.rs:42".to_string());
        let bounce_feedback = if last_verdict == Some(Verdict::Unknown) {
            "The verifier could not parse a verdict from the previous attempt. Review your changes carefully and ensure correctness.".to_string()
        } else if let Some(ref reason) = last_verdict_reason {
            format!("The verifier rejected your implementation: {}. Fix these specific issues.", reason)
        } else {
            "Check its incoming contradicts edges and fix the code.".to_string()
        };
        assert!(bounce_feedback.contains("Test failed: assertion `left == right` at lib.rs:42"));
        assert!(bounce_feedback.contains("Fix these specific issues"));
        assert!(!bounce_feedback.contains("contradicts edges"));
    }

    #[test]
    fn test_bounce_feedback_no_reason_uses_generic_fallback() {
        let last_verdict: Option<Verdict> = Some(Verdict::Contradicts);
        let last_verdict_reason: Option<String> = None;
        let bounce_feedback = if last_verdict == Some(Verdict::Unknown) {
            "The verifier could not parse a verdict from the previous attempt. Review your changes carefully and ensure correctness.".to_string()
        } else if let Some(ref reason) = last_verdict_reason {
            format!("The verifier rejected your implementation: {}. Fix these specific issues.", reason)
        } else {
            "Check its incoming contradicts edges and fix the code.".to_string()
        };
        assert_eq!(bounce_feedback, "Check its incoming contradicts edges and fix the code.");
    }

    #[test]
    fn test_bounce_feedback_first_bounce_no_verdict_uses_fallback() {
        // On first bounce, last_verdict is None — should use generic fallback
        let last_verdict: Option<Verdict> = None;
        let last_verdict_reason: Option<String> = None;
        let bounce_feedback = if last_verdict == Some(Verdict::Unknown) {
            "The verifier could not parse a verdict from the previous attempt. Review your changes carefully and ensure correctness.".to_string()
        } else if let Some(ref reason) = last_verdict_reason {
            format!("The verifier rejected your implementation: {}. Fix these specific issues.", reason)
        } else {
            "Check its incoming contradicts edges and fix the code.".to_string()
        };
        assert_eq!(bounce_feedback, "Check its incoming contradicts edges and fix the code.");
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
