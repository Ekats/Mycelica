use super::*;

// ============================================================================
// Graph Operations Commands
// ============================================================================

pub(crate) async fn handle_graph(cmd: GraphCommands, db: &Database, json: bool) -> Result<(), String> {
    match cmd {
        GraphCommands::QueryEdges { edge_type, agent, target_agent, confidence_min, since, not_superseded, limit, compact } => {
            let since_millis = since.as_deref()
                .map(spore::parse_since_to_millis)
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

        GraphCommands::ExplainEdge { id, depth } => {
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

        GraphCommands::PathBetween { from, to, max_hops, edge_types } => {
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

        GraphCommands::EdgesForContext { id, top, not_superseded } => {
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

        GraphCommands::CreateMeta { meta_type, title, content, agent, connects_to, edge_type } => {
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

        GraphCommands::UpdateMeta { id, content, title, agent, add_connects, edge_type } => {
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

        GraphCommands::Status { all, format } => {
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

        // Create edge between existing nodes (delegates to handle_link)
        GraphCommands::CreateEdge { from, to, edge_type, content, reason, agent, confidence, supersedes, metadata } => {
            handle_link(&from, &to, &edge_type, reason, content, &agent, confidence, supersedes, metadata, "spore", db, json).await
        }

        // Read full content of a node (no metadata noise)
        GraphCommands::ReadContent { id } => {
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

        // List descendants of a category
        GraphCommands::ListRegion { id, class, items_only, limit } => {
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

        // Check freshness of summary meta-nodes
        GraphCommands::CheckFreshness { id } => {
            let node = resolve_node(db, &id)?;

            // Find all summarizes edges where this node is the TARGET
            let edges = db.get_edges_for_node(&node.id).map_err(|e| e.to_string())?;
            let summary_edges: Vec<&Edge> = edges.iter()
                .filter(|e| e.edge_type == EdgeType::Summarizes && e.target == node.id && e.superseded_by.is_none())
                .collect();

            if summary_edges.is_empty() {
                // Check if this node is itself a summary -- look at outgoing summarizes edges
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
                    // This node IS a summary -- check if its targets have been updated since
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
                // Node is summarized BY other nodes -- show their freshness
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

        GraphCommands::Gc { days, dry_run, force } => {
            let cutoff_ms = chrono::Utc::now().timestamp_millis() - (days as i64) * 86_400_000;
            // By default, exclude Lesson: and Summary: nodes -- they're valuable even without
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
    }
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
    metadata: Option<String>,
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
        metadata,
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
