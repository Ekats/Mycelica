use super::*;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};

// ============================================================================
// TUI Mode
// ============================================================================

/// TUI operating mode
#[derive(Clone, Copy, PartialEq)]
enum TuiMode {
    Navigation,  // Browsing hierarchy (tree view)
    LeafView,    // Viewing item content (full screen)
    Edit,        // Editing content
    Search,      // Search mode
    Maintenance, // Maintenance menu
    Settings,    // Settings screen
    Jobs,        // Job status popup
}

/// Focus state for Navigation mode (3-column layout)
#[derive(Clone, Copy, PartialEq)]
enum NavFocus {
    Tree,
    Pins,
    Recents,
}

/// Focus state for Leaf View mode
#[derive(Clone, Copy, PartialEq)]
enum LeafFocus {
    Content,
    Similar,
    Calls,  // Only shown for code nodes
}

/// Tree node for TUI display
#[derive(Clone)]
struct TreeNode {
    id: String,
    parent_id: Option<String>,
    title: String,
    emoji: Option<String>,
    depth: i32,
    child_count: i32,
    is_item: bool,
    is_expanded: bool,
    is_universe: bool,
    children_loaded: bool,
    created_at: i64,
    latest_child_date: Option<i64>,
}

/// Similar node for leaf view sidebar
#[derive(Clone)]
struct SimilarNodeInfo {
    id: String,
    title: String,
    emoji: Option<String>,
    similarity: f32,
    parent_title: Option<String>,
}

/// TUI Application state
struct TuiApp {
    // Mode
    mode: TuiMode,

    // Tree data
    nodes: Vec<TreeNode>,
    visible_nodes: Vec<usize>,  // Indices into nodes that are currently visible
    list_state: ListState,

    // CD-style navigation
    current_root_id: String,        // Current "directory" being viewed
    breadcrumb_path: Vec<(String, String)>,  // (id, title) pairs from Universe to current

    // Navigation mode focus (which pane is active)
    nav_focus: NavFocus,
    pins_selected: usize,
    recents_selected: usize,

    // Selected node in navigation
    selected_node: Option<Node>,

    // Leaf view state
    leaf_node_id: Option<String>,
    leaf_content: Option<String>,
    leaf_scroll_offset: u16,
    leaf_focus: LeafFocus,  // Which section is focused: Content, Similar, or Edges

    // Similar nodes (loaded on leaf view entry)
    similar_nodes: Vec<SimilarNodeInfo>,
    similar_selected: usize,

    // Calls for current leaf (only for code nodes)
    calls_for_node: Vec<(String, String, bool)>,  // (target_id, title, is_outgoing)
    calls_selected: usize,
    is_code_node: bool,  // Whether current leaf is a code node

    // Pins and recents
    pinned_nodes: Vec<Node>,
    recent_nodes: Vec<Node>,

    // Search
    search_mode: bool,
    search_query: String,
    search_results: Vec<Node>,

    // Edit mode
    edit_buffer: String,
    edit_cursor_line: usize,
    edit_cursor_col: usize,
    edit_scroll_offset: usize,
    edit_dirty: bool,  // Track if content has been modified

    // Status
    status_message: String,

    // Date range for color gradient
    date_min: i64,
    date_max: i64,
}

impl TuiApp {
    fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            mode: TuiMode::Navigation,
            nodes: Vec::new(),
            visible_nodes: Vec::new(),
            list_state,
            current_root_id: String::new(),
            breadcrumb_path: Vec::new(),
            nav_focus: NavFocus::Tree,
            pins_selected: 0,
            recents_selected: 0,
            selected_node: None,
            leaf_node_id: None,
            leaf_content: None,
            leaf_scroll_offset: 0,
            leaf_focus: LeafFocus::Content,
            similar_nodes: Vec::new(),
            similar_selected: 0,
            calls_for_node: Vec::new(),
            calls_selected: 0,
            is_code_node: false,
            pinned_nodes: Vec::new(),
            recent_nodes: Vec::new(),
            search_mode: false,
            search_query: String::new(),
            search_results: Vec::new(),
            edit_buffer: String::new(),
            edit_cursor_line: 0,
            edit_cursor_col: 0,
            edit_scroll_offset: 0,
            edit_dirty: false,
            status_message: String::new(),
            date_min: 0,
            date_max: i64::MAX,
        }
    }

    fn load_tree(&mut self, db: &Database) -> Result<(), String> {
        self.nodes.clear();
        self.visible_nodes.clear();

        // Get universe as root
        if let Some(universe) = db.get_universe().map_err(|e| e.to_string())? {
            // If no current root, start at Universe
            if self.current_root_id.is_empty() {
                self.current_root_id = universe.id.clone();
                self.breadcrumb_path = vec![(universe.id.clone(), "Universe".to_string())];
            }

            // Load children of current root directly (flat list, not tree)
            let children = db.get_children(&self.current_root_id).map_err(|e| e.to_string())?;

            // Calculate date range for color gradient (use effective dates)
            // Exclude Recent Notes from date range calculation (it skews the gradient)
            let recent_notes_id = settings::RECENT_NOTES_CONTAINER_ID;
            if !children.is_empty() {
                self.date_min = children.iter()
                    .filter(|n| n.id != recent_notes_id)
                    .map(|n| n.latest_child_date.unwrap_or(n.created_at))
                    .min()
                    .unwrap_or(0);
                self.date_max = children.iter()
                    .filter(|n| n.id != recent_notes_id)
                    .map(|n| n.latest_child_date.unwrap_or(n.created_at))
                    .max()
                    .unwrap_or(i64::MAX);
            }

            for child in children {
                self.nodes.push(TreeNode {
                    id: child.id.clone(),
                    parent_id: Some(self.current_root_id.clone()),
                    title: child.ai_title.clone().unwrap_or(child.title.clone()),
                    emoji: child.emoji.clone(),
                    depth: 0,  // Relative depth from current root
                    child_count: child.child_count,
                    is_item: child.is_item,
                    is_expanded: false,
                    is_universe: false,
                    children_loaded: false,
                    created_at: child.created_at,
                    latest_child_date: child.latest_child_date,
                });
            }
        }

        self.update_visible_nodes();

        // Load pins and recents
        self.pinned_nodes = db.get_pinned_nodes().unwrap_or_default();
        self.recent_nodes = db.get_recent_nodes(10).unwrap_or_default();

        Ok(())
    }

    /// CD into a cluster (make it the new root)
    fn cd_into(&mut self, db: &Database, node_id: &str) -> Result<(), String> {
        let node = db.get_node(node_id)
            .map_err(|e| e.to_string())?
            .ok_or("Node not found")?;

        // Add to breadcrumb
        let title = node.ai_title.clone().unwrap_or(node.title.clone());
        self.breadcrumb_path.push((node_id.to_string(), title));

        // Set as new root
        self.current_root_id = node_id.to_string();

        // Reload tree from new root
        self.nodes.clear();
        self.list_state.select(Some(0));

        let children = db.get_children(node_id).map_err(|e| e.to_string())?;

        // Update date range (use effective dates)
        // Exclude Recent Notes from date range calculation (it skews the gradient)
        let recent_notes_id = settings::RECENT_NOTES_CONTAINER_ID;
        if !children.is_empty() {
            self.date_min = children.iter()
                .filter(|n| n.id != recent_notes_id)
                .map(|n| n.latest_child_date.unwrap_or(n.created_at))
                .min()
                .unwrap_or(0);
            self.date_max = children.iter()
                .filter(|n| n.id != recent_notes_id)
                .map(|n| n.latest_child_date.unwrap_or(n.created_at))
                .max()
                .unwrap_or(i64::MAX);
        }

        for child in children {
            self.nodes.push(TreeNode {
                id: child.id.clone(),
                parent_id: Some(node_id.to_string()),
                title: child.ai_title.clone().unwrap_or(child.title.clone()),
                emoji: child.emoji.clone(),
                depth: 0,
                child_count: child.child_count,
                is_item: child.is_item,
                is_expanded: false,
                is_universe: false,
                children_loaded: false,
                created_at: child.created_at,
                latest_child_date: child.latest_child_date,
            });
        }

        self.update_visible_nodes();
        self.status_message = format!("Entered {} ({} items)",
            self.breadcrumb_path.last().map(|(_, t)| t.as_str()).unwrap_or("?"),
            self.nodes.len()
        );
        Ok(())
    }

    /// Go up one level (cd ..)
    fn cd_up(&mut self, db: &Database) -> Result<(), String> {
        if self.breadcrumb_path.len() <= 1 {
            self.status_message = "Already at root".to_string();
            return Ok(());
        }

        // Remove current from breadcrumb
        self.breadcrumb_path.pop();

        // Get parent ID
        let parent_id = self.breadcrumb_path.last()
            .map(|(id, _)| id.clone())
            .unwrap_or_default();

        self.current_root_id = parent_id.clone();

        // Reload tree from parent
        self.nodes.clear();
        self.list_state.select(Some(0));

        let children = db.get_children(&parent_id).map_err(|e| e.to_string())?;

        for child in children {
            self.nodes.push(TreeNode {
                id: child.id.clone(),
                parent_id: Some(parent_id.clone()),
                title: child.ai_title.clone().unwrap_or(child.title.clone()),
                emoji: child.emoji.clone(),
                depth: 0,
                child_count: child.child_count,
                is_item: child.is_item,
                is_expanded: false,
                is_universe: false,
                children_loaded: false,
                created_at: child.created_at,
                latest_child_date: child.latest_child_date,
            });
        }

        self.update_visible_nodes();
        self.status_message = format!("Back to {} ({} items)",
            self.breadcrumb_path.last().map(|(_, t)| t.as_str()).unwrap_or("Universe"),
            self.nodes.len()
        );
        Ok(())
    }

    /// Enter leaf view mode for an item
    fn enter_leaf_view(&mut self, db: &Database, node_id: &str) -> Result<(), String> {
        let node = db.get_node(node_id)
            .map_err(|e| e.to_string())?
            .ok_or("Node not found")?;

        self.mode = TuiMode::LeafView;
        self.leaf_node_id = Some(node_id.to_string());
        self.leaf_content = node.content.clone();
        self.leaf_scroll_offset = 0;
        self.leaf_focus = LeafFocus::Content;
        self.calls_selected = 0;
        self.similar_nodes.clear();
        self.similar_selected = 0;

        // Check if this is a code node
        self.is_code_node = node.content_type.as_ref().map(|ct| ct.starts_with("code_")).unwrap_or(false);

        // Store selected node for header display
        self.selected_node = Some(node.clone());

        // Load similar nodes using embeddings
        if let Some(target_emb) = db.get_node_embedding(node_id).ok().flatten() {
            if let Ok(all_embeddings) = db.get_nodes_with_embeddings() {
                let similar = similarity::find_similar(&target_emb, &all_embeddings, node_id, 15, 0.5);

                for (sim_id, score) in similar {
                    if let Ok(Some(sim_node)) = db.get_node(&sim_id) {
                        // Get parent title for grouping display
                        let parent_title = if let Some(ref pid) = sim_node.parent_id {
                            db.get_node(pid).ok().flatten().map(|p| p.ai_title.unwrap_or(p.title))
                        } else {
                            None
                        };

                        self.similar_nodes.push(SimilarNodeInfo {
                            id: sim_id,
                            title: sim_node.ai_title.unwrap_or(sim_node.title),
                            emoji: sim_node.emoji,
                            similarity: score,
                            parent_title,
                        });
                    }
                }
            }
        }

        // Load Calls edges for code nodes only
        self.calls_for_node.clear();
        if self.is_code_node {
            if let Ok(edges) = db.get_edges_for_node(node_id) {
                for edge in edges {
                    // Only process Calls edges
                    if edge.edge_type != EdgeType::Calls {
                        continue;
                    }
                    // Determine direction: outgoing if this node is source
                    let is_outgoing = edge.source == node_id;
                    let other_id = if is_outgoing { &edge.target } else { &edge.source };
                    if let Ok(Some(other_node)) = db.get_node(other_id) {
                        let title = other_node.ai_title.unwrap_or(other_node.title);
                        self.calls_for_node.push((other_id.to_string(), title, is_outgoing));
                    }
                }
            }
        }

        // Touch node to update recent
        let _ = db.touch_node(node_id);

        // Reload recents
        self.recent_nodes = db.get_recent_nodes(10).unwrap_or_default();

        self.status_message = format!("Viewing: {} ({} similar) [q/Esc to go back]",
            node.ai_title.unwrap_or(node.title),
            self.similar_nodes.len());
        Ok(())
    }

    /// Exit leaf view, return to navigation
    fn exit_leaf_view(&mut self) {
        self.mode = TuiMode::Navigation;
        self.leaf_node_id = None;
        self.leaf_content = None;
        self.similar_nodes.clear();
        self.calls_for_node.clear();
        self.is_code_node = false;
        self.status_message = "Back to navigation".to_string();
    }

    /// Enter edit mode from leaf view
    fn enter_edit_mode(&mut self) {
        if let Some(ref content) = self.leaf_content {
            self.edit_buffer = content.clone();
        } else {
            self.edit_buffer = String::new();
        }
        self.edit_cursor_line = 0;
        self.edit_cursor_col = 0;
        self.edit_scroll_offset = 0;
        self.edit_dirty = false;
        self.mode = TuiMode::Edit;
        self.status_message = "Edit mode: Ctrl+S save, Esc cancel".to_string();
    }

    /// Save edited content and return to leaf view
    fn save_edit(&mut self, db: &Database) -> Result<(), String> {
        if let Some(ref node_id) = self.leaf_node_id {
            db.update_node_content(node_id, &self.edit_buffer)
                .map_err(|e| e.to_string())?;

            // Update the leaf content with saved buffer
            self.leaf_content = Some(self.edit_buffer.clone());
            self.leaf_scroll_offset = 0;
            self.mode = TuiMode::LeafView;
            self.edit_dirty = false;
            self.status_message = "Content saved".to_string();
            Ok(())
        } else {
            Err("No node to save".to_string())
        }
    }

    /// Cancel edit and return to leaf view
    fn cancel_edit(&mut self) {
        let was_dirty = self.edit_dirty;
        self.mode = TuiMode::LeafView;
        self.edit_buffer.clear();
        self.edit_dirty = false;
        self.status_message = if was_dirty {
            "Edit cancelled (changes discarded)".to_string()
        } else {
            "Edit cancelled".to_string()
        };
    }

    /// Get the lines of the edit buffer
    fn edit_lines(&self) -> Vec<&str> {
        self.edit_buffer.lines().collect()
    }

    /// Get total line count in edit buffer
    fn edit_line_count(&self) -> usize {
        self.edit_buffer.lines().count().max(1)
    }

    /// Get the current line content
    fn current_edit_line(&self) -> &str {
        self.edit_buffer.lines().nth(self.edit_cursor_line).unwrap_or("")
    }

    /// Insert a character at cursor position
    fn edit_insert_char(&mut self, c: char) {
        let byte_pos = self.cursor_byte_position();
        self.edit_buffer.insert(byte_pos, c);
        if c == '\n' {
            self.edit_cursor_line += 1;
            self.edit_cursor_col = 0;
        } else {
            self.edit_cursor_col += 1;
        }
        self.edit_dirty = true;
    }

    /// Delete character before cursor (backspace)
    fn edit_backspace(&mut self) {
        if self.edit_cursor_col > 0 {
            // Delete character before cursor on current line
            let byte_pos = self.cursor_byte_position();
            if byte_pos > 0 {
                // Find the byte position of the previous character
                let prev_char_start = self.edit_buffer[..byte_pos]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.edit_buffer.remove(prev_char_start);
                self.edit_cursor_col = self.edit_cursor_col.saturating_sub(1);
                self.edit_dirty = true;
            }
        } else if self.edit_cursor_line > 0 {
            // At start of line, merge with previous line
            let byte_pos = self.cursor_byte_position();
            if byte_pos > 0 {
                // Remove the newline before current position
                self.edit_buffer.remove(byte_pos - 1);
                self.edit_cursor_line -= 1;
                // Set cursor to end of the now-merged line
                self.edit_cursor_col = self.edit_buffer
                    .lines()
                    .nth(self.edit_cursor_line)
                    .map(|l| l.chars().count())
                    .unwrap_or(0);
                self.edit_dirty = true;
            }
        }
    }

    /// Delete character at cursor (delete key)
    fn edit_delete(&mut self) {
        let byte_pos = self.cursor_byte_position();
        if byte_pos < self.edit_buffer.len() {
            self.edit_buffer.remove(byte_pos);
            self.edit_dirty = true;
        }
    }

    /// Move cursor left
    fn edit_cursor_left(&mut self) {
        if self.edit_cursor_col > 0 {
            self.edit_cursor_col -= 1;
        } else if self.edit_cursor_line > 0 {
            self.edit_cursor_line -= 1;
            self.edit_cursor_col = self.current_edit_line().chars().count();
        }
    }

    /// Move cursor right
    fn edit_cursor_right(&mut self) {
        let line_len = self.current_edit_line().chars().count();
        if self.edit_cursor_col < line_len {
            self.edit_cursor_col += 1;
        } else if self.edit_cursor_line < self.edit_line_count().saturating_sub(1) {
            self.edit_cursor_line += 1;
            self.edit_cursor_col = 0;
        }
    }

    /// Move cursor up
    fn edit_cursor_up(&mut self) {
        if self.edit_cursor_line > 0 {
            self.edit_cursor_line -= 1;
            // Clamp column to line length
            let line_len = self.current_edit_line().chars().count();
            self.edit_cursor_col = self.edit_cursor_col.min(line_len);
        }
    }

    /// Move cursor down
    fn edit_cursor_down(&mut self) {
        let line_count = self.edit_line_count();
        if self.edit_cursor_line < line_count.saturating_sub(1) {
            self.edit_cursor_line += 1;
            // Clamp column to line length
            let line_len = self.current_edit_line().chars().count();
            self.edit_cursor_col = self.edit_cursor_col.min(line_len);
        }
    }

    /// Move cursor to start of line
    fn edit_cursor_home(&mut self) {
        self.edit_cursor_col = 0;
    }

    /// Move cursor to end of line
    fn edit_cursor_end(&mut self) {
        self.edit_cursor_col = self.current_edit_line().chars().count();
    }

    /// Calculate byte position from line/col
    fn cursor_byte_position(&self) -> usize {
        let mut byte_pos = 0;
        for (line_idx, line) in self.edit_buffer.lines().enumerate() {
            if line_idx == self.edit_cursor_line {
                // Add bytes up to cursor column
                for (col, c) in line.chars().enumerate() {
                    if col >= self.edit_cursor_col {
                        break;
                    }
                    byte_pos += c.len_utf8();
                }
                return byte_pos;
            }
            byte_pos += line.len() + 1; // +1 for newline
        }
        self.edit_buffer.len()
    }

    /// Update scroll offset to keep cursor visible
    fn edit_ensure_cursor_visible(&mut self, visible_lines: usize) {
        if self.edit_cursor_line < self.edit_scroll_offset {
            self.edit_scroll_offset = self.edit_cursor_line;
        } else if self.edit_cursor_line >= self.edit_scroll_offset + visible_lines {
            self.edit_scroll_offset = self.edit_cursor_line.saturating_sub(visible_lines) + 1;
        }
    }

    fn load_children_for_node(&mut self, db: &Database, node_idx: usize) -> Result<(), String> {
        if self.nodes[node_idx].children_loaded {
            return Ok(());
        }

        let parent_id = self.nodes[node_idx].id.clone();
        let children = db.get_children(&parent_id).map_err(|e| e.to_string())?;

        // Insert children right after the parent node
        let insert_pos = node_idx + 1;

        for (i, child) in children.into_iter().enumerate() {
            self.nodes.insert(insert_pos + i, TreeNode {
                id: child.id.clone(),
                parent_id: Some(parent_id.clone()),
                title: child.ai_title.clone().unwrap_or(child.title.clone()),
                emoji: child.emoji.clone(),
                depth: child.depth,
                child_count: child.child_count,
                is_item: child.is_item,
                is_expanded: false,
                is_universe: false,
                children_loaded: false,
                created_at: child.created_at,
                latest_child_date: child.latest_child_date,
            });
        }

        self.nodes[node_idx].children_loaded = true;
        Ok(())
    }

    fn update_visible_nodes(&mut self) {
        self.visible_nodes.clear();

        // With CD-style navigation, all direct children of current_root are at depth 0
        // They are always visible. Only their expanded children need ancestor checking.

        // Build set of expanded node IDs for quick lookup
        let expanded_ids: std::collections::HashSet<String> = self.nodes.iter()
            .filter(|n| n.is_expanded)
            .map(|n| n.id.clone())
            .collect();

        for (idx, node) in self.nodes.iter().enumerate() {
            // Depth 0 nodes are direct children of current root - always visible
            if node.depth == 0 {
                self.visible_nodes.push(idx);
                continue;
            }

            // For deeper nodes, check if all ancestors are expanded
            if self.is_ancestor_chain_expanded(idx, &expanded_ids) {
                self.visible_nodes.push(idx);
            }
        }
    }

    fn is_ancestor_chain_expanded(&self, idx: usize, expanded_ids: &std::collections::HashSet<String>) -> bool {
        let node = &self.nodes[idx];

        // Check if parent is expanded
        if let Some(ref parent_id) = node.parent_id {
            // If parent is the current root, it's implicitly expanded
            if *parent_id == self.current_root_id {
                return true;
            }

            if !expanded_ids.contains(parent_id) {
                return false;
            }
            // Recursively check parent's ancestors
            for (i, n) in self.nodes.iter().enumerate() {
                if n.id == *parent_id {
                    return self.is_ancestor_chain_expanded(i, expanded_ids);
                }
            }
        }
        true
    }

    fn toggle_expand(&mut self, db: &Database) {
        if let Some(selected) = self.list_state.selected() {
            if selected < self.visible_nodes.len() {
                let node_idx = self.visible_nodes[selected];
                let node = &self.nodes[node_idx];

                if !node.is_item && node.child_count > 0 {
                    // Toggle expansion
                    let was_expanded = self.nodes[node_idx].is_expanded;
                    self.nodes[node_idx].is_expanded = !was_expanded;

                    if !was_expanded {
                        // Load children if not already loaded
                        let _ = self.load_children_for_node(db, node_idx);
                    }

                    self.update_visible_nodes();

                    // Adjust selection if needed (visible_nodes may have changed)
                    if selected >= self.visible_nodes.len() {
                        self.list_state.select(Some(self.visible_nodes.len().saturating_sub(1)));
                    }
                }
            }
        }
    }

    fn select_next(&mut self) {
        if let Some(selected) = self.list_state.selected() {
            if selected < self.visible_nodes.len().saturating_sub(1) {
                self.list_state.select(Some(selected + 1));
            }
        }
    }

    fn select_prev(&mut self) {
        if let Some(selected) = self.list_state.selected() {
            if selected > 0 {
                self.list_state.select(Some(selected - 1));
            }
        }
    }

    fn get_selected_node(&self, db: &Database) -> Option<Node> {
        if let Some(selected) = self.list_state.selected() {
            if selected < self.visible_nodes.len() {
                let node_idx = self.visible_nodes[selected];
                let tree_node = &self.nodes[node_idx];
                return db.get_node(&tree_node.id).ok().flatten();
            }
        }
        None
    }
}

pub(crate) async fn run_tui(db: &Database) -> Result<(), String> {
    // Setup terminal
    enable_raw_mode().map_err(|e| e.to_string())?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(|e| e.to_string())?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;

    // Create app and load data
    let mut app = TuiApp::new();
    app.load_tree(db)?;
    app.status_message = format!("Loaded {} nodes. Press ? for help, q to quit.", app.nodes.len());

    // Main loop
    let result = run_tui_loop(&mut terminal, &mut app, db);

    // Restore terminal
    disable_raw_mode().map_err(|e| e.to_string())?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    ).map_err(|e| e.to_string())?;
    terminal.show_cursor().map_err(|e| e.to_string())?;

    result
}

fn run_tui_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut TuiApp,
    db: &Database,
) -> Result<(), String> {
    loop {
        // Update selected node details (only in Navigation mode)
        if app.mode == TuiMode::Navigation && !app.search_mode {
            app.selected_node = app.get_selected_node(db);
        }

        // Draw UI
        terminal.draw(|f| draw_ui(f, app)).map_err(|e| e.to_string())?;

        // Handle input
        if event::poll(std::time::Duration::from_millis(100)).map_err(|e| e.to_string())? {
            if let Event::Key(key) = event::read().map_err(|e| e.to_string())? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Handle search mode (overlay on Navigation)
                if app.search_mode {
                    match key.code {
                        KeyCode::Esc => {
                            app.search_mode = false;
                            app.search_query.clear();
                            app.status_message = "Search cancelled".to_string();
                        }
                        KeyCode::Enter => {
                            app.search_mode = false;
                            // Perform search
                            if !app.search_query.is_empty() {
                                if let Ok(results) = db.search_nodes(&app.search_query) {
                                    app.status_message = format!("Found {} results for '{}'", results.len(), app.search_query);
                                    app.search_results = results.clone();
                                    // Jump to first result if found in current view
                                    if let Some(first) = results.first() {
                                        for (i, &idx) in app.visible_nodes.iter().enumerate() {
                                            if app.nodes[idx].id == first.id {
                                                app.list_state.select(Some(i));
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            app.search_query.clear();
                        }
                        KeyCode::Backspace => {
                            app.search_query.pop();
                        }
                        KeyCode::Char(c) => {
                            app.search_query.push(c);
                        }
                        _ => {}
                    }
                    continue;
                }

                // Handle input based on current mode
                match app.mode {
                    TuiMode::Navigation => {
                        match key.code {
                            KeyCode::Char('q') => return Ok(()),

                            // Tab: Cycle focus between Tree → Pins → Recents
                            KeyCode::Tab => {
                                app.nav_focus = match app.nav_focus {
                                    NavFocus::Tree => NavFocus::Pins,
                                    NavFocus::Pins => NavFocus::Recents,
                                    NavFocus::Recents => NavFocus::Tree,
                                };
                                app.status_message = match app.nav_focus {
                                    NavFocus::Tree => "Focus: Tree".to_string(),
                                    NavFocus::Pins => "Focus: Pins".to_string(),
                                    NavFocus::Recents => "Focus: Recents".to_string(),
                                };
                            }
                            // Shift+Tab: Cycle focus in reverse
                            KeyCode::BackTab => {
                                app.nav_focus = match app.nav_focus {
                                    NavFocus::Tree => NavFocus::Recents,
                                    NavFocus::Pins => NavFocus::Tree,
                                    NavFocus::Recents => NavFocus::Pins,
                                };
                                app.status_message = match app.nav_focus {
                                    NavFocus::Tree => "Focus: Tree".to_string(),
                                    NavFocus::Pins => "Focus: Pins".to_string(),
                                    NavFocus::Recents => "Focus: Recents".to_string(),
                                };
                            }

                            // j/k: Navigate in focused pane
                            KeyCode::Char('j') | KeyCode::Down => {
                                match app.nav_focus {
                                    NavFocus::Tree => app.select_next(),
                                    NavFocus::Pins => {
                                        if !app.pinned_nodes.is_empty() {
                                            app.pins_selected = (app.pins_selected + 1).min(app.pinned_nodes.len() - 1);
                                        }
                                    }
                                    NavFocus::Recents => {
                                        if !app.recent_nodes.is_empty() {
                                            app.recents_selected = (app.recents_selected + 1).min(app.recent_nodes.len() - 1);
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                match app.nav_focus {
                                    NavFocus::Tree => app.select_prev(),
                                    NavFocus::Pins => {
                                        app.pins_selected = app.pins_selected.saturating_sub(1);
                                    }
                                    NavFocus::Recents => {
                                        app.recents_selected = app.recents_selected.saturating_sub(1);
                                    }
                                }
                            }

                            // Enter: Action depends on focused pane
                            KeyCode::Enter => {
                                match app.nav_focus {
                                    NavFocus::Tree => {
                                        if let Some(selected) = app.list_state.selected() {
                                            if selected < app.visible_nodes.len() {
                                                let node_idx = app.visible_nodes[selected];
                                                let node = &app.nodes[node_idx];

                                                if node.is_item {
                                                    let node_id = node.id.clone();
                                                    if let Err(e) = app.enter_leaf_view(db, &node_id) {
                                                        app.status_message = format!("Error: {}", e);
                                                    }
                                                } else if node.child_count > 0 {
                                                    let node_id = node.id.clone();
                                                    if let Err(e) = app.cd_into(db, &node_id) {
                                                        app.status_message = format!("Error: {}", e);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    NavFocus::Pins => {
                                        if app.pins_selected < app.pinned_nodes.len() {
                                            let node = &app.pinned_nodes[app.pins_selected];
                                            let node_id = node.id.clone();
                                            let is_item = node.is_item;
                                            if is_item {
                                                if let Err(e) = app.enter_leaf_view(db, &node_id) {
                                                    app.status_message = format!("Error: {}", e);
                                                }
                                            } else {
                                                if let Err(e) = app.cd_into(db, &node_id) {
                                                    app.status_message = format!("Error: {}", e);
                                                }
                                            }
                                        }
                                    }
                                    NavFocus::Recents => {
                                        if app.recents_selected < app.recent_nodes.len() {
                                            let node = &app.recent_nodes[app.recents_selected];
                                            let node_id = node.id.clone();
                                            let is_item = node.is_item;
                                            if is_item {
                                                if let Err(e) = app.enter_leaf_view(db, &node_id) {
                                                    app.status_message = format!("Error: {}", e);
                                                }
                                            } else {
                                                if let Err(e) = app.cd_into(db, &node_id) {
                                                    app.status_message = format!("Error: {}", e);
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // l/Right: Expand children inline (toggle) - only in Tree focus
                            KeyCode::Char('l') | KeyCode::Right => {
                                if app.nav_focus == NavFocus::Tree {
                                    app.toggle_expand(db);
                                }
                            }

                            // h/Left: Collapse children - only in Tree focus
                            KeyCode::Char('h') | KeyCode::Left => {
                                if app.nav_focus == NavFocus::Tree {
                                    if let Some(selected) = app.list_state.selected() {
                                        if selected < app.visible_nodes.len() {
                                            let node_idx = app.visible_nodes[selected];
                                            if app.nodes[node_idx].is_expanded {
                                                app.nodes[node_idx].is_expanded = false;
                                                app.update_visible_nodes();
                                            }
                                        }
                                    }
                                }
                            }

                            // Backspace/-/Esc: Go up one level (cd ..)
                            KeyCode::Backspace | KeyCode::Char('-') | KeyCode::Esc => {
                                if let Err(e) = app.cd_up(db) {
                                    app.status_message = format!("Error: {}", e);
                                }
                            }

                            KeyCode::Char('/') => {
                                app.search_mode = true;
                                app.search_query.clear();
                                app.status_message = "Search: ".to_string();
                            }
                            KeyCode::Char('?') => {
                                app.status_message = "Tab:focus  Enter:cd/view  l:expand  h:collapse  -:up  /:search  q:quit".to_string();
                            }
                            KeyCode::Char('g') => {
                                match app.nav_focus {
                                    NavFocus::Tree => app.list_state.select(Some(0)),
                                    NavFocus::Pins => app.pins_selected = 0,
                                    NavFocus::Recents => app.recents_selected = 0,
                                }
                            }
                            KeyCode::Char('G') => {
                                match app.nav_focus {
                                    NavFocus::Tree => {
                                        if !app.visible_nodes.is_empty() {
                                            app.list_state.select(Some(app.visible_nodes.len() - 1));
                                        }
                                    }
                                    NavFocus::Pins => {
                                        if !app.pinned_nodes.is_empty() {
                                            app.pins_selected = app.pinned_nodes.len() - 1;
                                        }
                                    }
                                    NavFocus::Recents => {
                                        if !app.recent_nodes.is_empty() {
                                            app.recents_selected = app.recent_nodes.len() - 1;
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('r') => {
                                let _ = app.load_tree(db);
                                app.status_message = format!("Reloaded {} nodes", app.nodes.len());
                            }
                            KeyCode::Char('p') => {
                                // Toggle pin for selected node (works in any focus)
                                if let Some(ref node) = app.selected_node {
                                    let new_pinned = !node.is_pinned;
                                    if db.set_node_pinned(&node.id, new_pinned).is_ok() {
                                        app.pinned_nodes = db.get_pinned_nodes().unwrap_or_default();
                                        app.status_message = if new_pinned {
                                            format!("Pinned: {}", node.ai_title.as_ref().unwrap_or(&node.title))
                                        } else {
                                            format!("Unpinned: {}", node.ai_title.as_ref().unwrap_or(&node.title))
                                        };
                                    }
                                }
                            }
                            _ => {}
                        }
                    }

                    TuiMode::LeafView => {
                        match key.code {
                            // q/Esc: Back to navigation
                            KeyCode::Char('q') | KeyCode::Esc => {
                                app.exit_leaf_view();
                            }

                            // j/k: Scroll content OR navigate in sidebar based on focus
                            KeyCode::Char('j') | KeyCode::Down => {
                                match app.leaf_focus {
                                    LeafFocus::Content => {
                                        // Calculate visual line count for bounds checking
                                        if let Some(content) = &app.leaf_content {
                                            let size = terminal.size().unwrap_or(ratatui::layout::Rect::new(0, 0, 80, 24));
                                            let visible_lines = size.height.saturating_sub(5) as usize;
                                            // Content width: 60% of terminal - borders(2)
                                            let content_width = ((size.width as usize * 60) / 100).saturating_sub(2).max(1);

                                            // Calculate total visual lines after wrapping
                                            let total_visual_lines: usize = content.lines()
                                                .map(|line| {
                                                    let len = line.chars().count();
                                                    if len == 0 { 1 } else { (len + content_width - 1) / content_width }
                                                })
                                                .sum();

                                            // Only scroll if there's more content below
                                            let max_scroll = total_visual_lines.saturating_sub(visible_lines);
                                            if (app.leaf_scroll_offset as usize) < max_scroll {
                                                app.leaf_scroll_offset = app.leaf_scroll_offset.saturating_add(1);
                                            }
                                        }
                                    }
                                    LeafFocus::Similar => {
                                        if !app.similar_nodes.is_empty() {
                                            app.similar_selected = (app.similar_selected + 1).min(app.similar_nodes.len() - 1);
                                        }
                                    }
                                    LeafFocus::Calls => {
                                        if !app.calls_for_node.is_empty() {
                                            app.calls_selected = (app.calls_selected + 1).min(app.calls_for_node.len() - 1);
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                match app.leaf_focus {
                                    LeafFocus::Content => {
                                        app.leaf_scroll_offset = app.leaf_scroll_offset.saturating_sub(1);
                                    }
                                    LeafFocus::Similar => {
                                        app.similar_selected = app.similar_selected.saturating_sub(1);
                                    }
                                    LeafFocus::Calls => {
                                        app.calls_selected = app.calls_selected.saturating_sub(1);
                                    }
                                }
                            }

                            // Page down/up (only in Content focus)
                            KeyCode::Char('d') => {
                                if app.leaf_focus == LeafFocus::Content {
                                    if let Some(content) = &app.leaf_content {
                                        let size = terminal.size().unwrap_or(ratatui::layout::Rect::new(0, 0, 80, 24));
                                        let visible_lines = size.height.saturating_sub(5) as usize;
                                        let content_width = ((size.width as usize * 60) / 100).saturating_sub(2).max(1);

                                        let total_visual_lines: usize = content.lines()
                                            .map(|line| {
                                                let len = line.chars().count();
                                                if len == 0 { 1 } else { (len + content_width - 1) / content_width }
                                            })
                                            .sum();

                                        let max_scroll = total_visual_lines.saturating_sub(visible_lines);
                                        let new_offset = (app.leaf_scroll_offset as usize).saturating_add(visible_lines / 2).min(max_scroll);
                                        app.leaf_scroll_offset = new_offset as u16;
                                    }
                                }
                            }
                            KeyCode::Char('u') => {
                                if app.leaf_focus == LeafFocus::Content {
                                    let size = terminal.size().unwrap_or(ratatui::layout::Rect::new(0, 0, 80, 24));
                                    let visible_lines = size.height.saturating_sub(5) as usize;
                                    app.leaf_scroll_offset = app.leaf_scroll_offset.saturating_sub(visible_lines as u16 / 2);
                                }
                            }

                            // Tab: Cycle focus Content → Similar → Calls (if code node)
                            KeyCode::Tab => {
                                app.leaf_focus = match app.leaf_focus {
                                    LeafFocus::Content => LeafFocus::Similar,
                                    LeafFocus::Similar => {
                                        // Only cycle to Calls if this is a code node with call edges
                                        if app.is_code_node && !app.calls_for_node.is_empty() {
                                            LeafFocus::Calls
                                        } else {
                                            LeafFocus::Content
                                        }
                                    }
                                    LeafFocus::Calls => LeafFocus::Content,
                                };
                                app.status_message = match app.leaf_focus {
                                    LeafFocus::Content => "Focus: Content".to_string(),
                                    LeafFocus::Similar => "Focus: Similar".to_string(),
                                    LeafFocus::Calls => "Focus: Calls".to_string(),
                                };
                            }
                            // Shift+Tab: Cycle focus in reverse
                            KeyCode::BackTab => {
                                app.leaf_focus = match app.leaf_focus {
                                    LeafFocus::Content => {
                                        // Only cycle to Calls if this is a code node with call edges
                                        if app.is_code_node && !app.calls_for_node.is_empty() {
                                            LeafFocus::Calls
                                        } else {
                                            LeafFocus::Similar
                                        }
                                    }
                                    LeafFocus::Similar => LeafFocus::Content,
                                    LeafFocus::Calls => LeafFocus::Similar,
                                };
                                app.status_message = match app.leaf_focus {
                                    LeafFocus::Content => "Focus: Content".to_string(),
                                    LeafFocus::Similar => "Focus: Similar".to_string(),
                                    LeafFocus::Calls => "Focus: Calls".to_string(),
                                };
                            }

                            // n/N: Navigate similar nodes (quick access from any focus)
                            KeyCode::Char('n') => {
                                if !app.similar_nodes.is_empty() {
                                    app.similar_selected = (app.similar_selected + 1) % app.similar_nodes.len();
                                }
                            }
                            KeyCode::Char('N') => {
                                if !app.similar_nodes.is_empty() {
                                    app.similar_selected = if app.similar_selected == 0 {
                                        app.similar_nodes.len() - 1
                                    } else {
                                        app.similar_selected - 1
                                    };
                                }
                            }

                            // Enter: Navigate to selected similar/call node
                            KeyCode::Enter => {
                                let target_id = match app.leaf_focus {
                                    LeafFocus::Similar if !app.similar_nodes.is_empty() => {
                                        Some(app.similar_nodes[app.similar_selected].id.clone())
                                    }
                                    LeafFocus::Calls if !app.calls_for_node.is_empty() => {
                                        Some(app.calls_for_node[app.calls_selected].0.clone())
                                    }
                                    _ => None,
                                };
                                if let Some(id) = target_id {
                                    app.exit_leaf_view();
                                    if let Err(e) = app.enter_leaf_view(db, &id) {
                                        app.status_message = format!("Error: {}", e);
                                    }
                                }
                            }

                            // e: Enter edit mode
                            KeyCode::Char('e') => {
                                app.enter_edit_mode();
                            }

                            // v: View PDF in external viewer
                            KeyCode::Char('v') => {
                                if let Some(ref node) = app.selected_node {
                                    if node.pdf_available == Some(true) {
                                        match db.get_paper_document(&node.id) {
                                            Ok(Some((doc_data, format))) => {
                                                let title = node.ai_title.as_ref().unwrap_or(&node.title);
                                                let safe_name: String = title.chars()
                                                    .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
                                                    .take(50)
                                                    .collect();
                                                let safe_name = safe_name.trim().replace(' ', "_");

                                                let temp_dir = std::env::temp_dir();
                                                let file_path = temp_dir.join(format!("{}.{}", safe_name, format));

                                                match std::fs::File::create(&file_path) {
                                                    Ok(mut file) => {
                                                        if let Err(e) = file.write_all(&doc_data) {
                                                            app.status_message = format!("Failed to write temp file: {}", e);
                                                        } else {
                                                            #[cfg(target_os = "linux")]
                                                            let result = std::process::Command::new("xdg-open")
                                                                .arg(&file_path)
                                                                .spawn();
                                                            #[cfg(target_os = "macos")]
                                                            let result = std::process::Command::new("open")
                                                                .arg(&file_path)
                                                                .spawn();
                                                            #[cfg(target_os = "windows")]
                                                            let result = std::process::Command::new("cmd")
                                                                .args(["/C", "start", "", &file_path.to_string_lossy()])
                                                                .spawn();

                                                            match result {
                                                                Ok(_) => app.status_message = format!("Opening {}...", format.to_uppercase()),
                                                                Err(e) => app.status_message = format!("Failed to open viewer: {}", e),
                                                            }
                                                        }
                                                    }
                                                    Err(e) => app.status_message = format!("Failed to create temp file: {}", e),
                                                }
                                            }
                                            Ok(None) => app.status_message = "PDF not available (not downloaded)".to_string(),
                                            Err(e) => app.status_message = format!("Database error: {}", e),
                                        }
                                    } else {
                                        app.status_message = "No PDF available for this paper".to_string();
                                    }
                                }
                            }

                            // o: Open URL in browser
                            KeyCode::Char('o') => {
                                if let Some(ref node) = app.selected_node {
                                    if let Some(ref url) = node.url {
                                        #[cfg(target_os = "linux")]
                                        let result = std::process::Command::new("xdg-open")
                                            .arg(url)
                                            .spawn();
                                        #[cfg(target_os = "macos")]
                                        let result = std::process::Command::new("open")
                                            .arg(url)
                                            .spawn();
                                        #[cfg(target_os = "windows")]
                                        let result = std::process::Command::new("cmd")
                                            .args(["/C", "start", "", url])
                                            .spawn();

                                        match result {
                                            Ok(_) => app.status_message = format!("Opening {}...", url),
                                            Err(e) => app.status_message = format!("Failed to open browser: {}", e),
                                        }
                                    } else {
                                        app.status_message = "No URL available for this node".to_string();
                                    }
                                }
                            }

                            KeyCode::Char('?') => {
                                app.status_message = "Tab:focus  j/k:nav  v:pdf  o:url  e:edit  n/N:similar  Enter:goto  q:back".to_string();
                            }
                            _ => {}
                        }
                    }

                    TuiMode::Edit => {
                        use crossterm::event::KeyModifiers;

                        match key.code {
                            // Ctrl+S: Save
                            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                if let Err(e) = app.save_edit(db) {
                                    app.status_message = format!("Save error: {}", e);
                                }
                            }

                            // Esc: Cancel edit
                            KeyCode::Esc => {
                                app.cancel_edit();
                            }

                            // Arrow keys: Move cursor
                            KeyCode::Left => app.edit_cursor_left(),
                            KeyCode::Right => app.edit_cursor_right(),
                            KeyCode::Up => app.edit_cursor_up(),
                            KeyCode::Down => app.edit_cursor_down(),

                            // Home/End: Jump to line start/end
                            KeyCode::Home => app.edit_cursor_home(),
                            KeyCode::End => app.edit_cursor_end(),

                            // Backspace: Delete character before cursor
                            KeyCode::Backspace => app.edit_backspace(),

                            // Delete: Delete character at cursor
                            KeyCode::Delete => app.edit_delete(),

                            // Enter: Insert newline
                            KeyCode::Enter => app.edit_insert_char('\n'),

                            // Tab: Insert 4 spaces (or actual tab)
                            KeyCode::Tab => {
                                for _ in 0..4 {
                                    app.edit_insert_char(' ');
                                }
                            }

                            // Regular character input
                            KeyCode::Char(c) => {
                                app.edit_insert_char(c);
                            }

                            _ => {}
                        }

                        // Keep cursor visible (estimate ~20 visible lines)
                        let visible_lines = 20;
                        app.edit_ensure_cursor_visible(visible_lines);

                        // Update status with cursor position
                        let dirty_marker = if app.edit_dirty { " [modified]" } else { "" };
                        app.status_message = format!(
                            "Edit mode: Ln {}, Col {} {} | Ctrl+S save, Esc cancel",
                            app.edit_cursor_line + 1,
                            app.edit_cursor_col + 1,
                            dirty_marker
                        );
                    }

                    // Other modes (Maintenance, Settings, Jobs) - placeholder
                    _ => {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                app.mode = TuiMode::Navigation;
                                app.status_message = "Back to navigation".to_string();
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

fn draw_ui(f: &mut Frame, app: &TuiApp) {
    match app.mode {
        TuiMode::Navigation => draw_navigation_mode(f, app),
        TuiMode::LeafView => draw_leaf_view_mode(f, app),
        TuiMode::Edit => draw_edit_mode(f, app),
        _ => draw_navigation_mode(f, app), // Fallback for unimplemented modes
    }
}

fn draw_navigation_mode(f: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // Breadcrumb bar
            Constraint::Min(0),     // Main content
            Constraint::Length(1),  // Status bar
        ])
        .split(f.size());

    // Breadcrumb bar
    draw_breadcrumb(f, app, chunks[0]);

    // 3-column layout: Tree (50%) | Pins+Recents (25%) | Preview (25%)
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(chunks[1]);

    // Tree view
    draw_tree(f, app, main_chunks[0]);

    // Pins + Recents pane
    draw_pins_recents(f, app, main_chunks[1]);

    // Preview pane
    draw_preview(f, app, main_chunks[2]);

    // Status bar
    let status = if app.search_mode {
        format!("Search: {}_", app.search_query)
    } else {
        app.status_message.clone()
    };
    let status_bar = Paragraph::new(status)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White));
    f.render_widget(status_bar, chunks[2]);
}

fn draw_pins_recents(f: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),  // Pinned
            Constraint::Percentage(50),  // Recent
        ])
        .split(area);

    // Border styles based on focus
    let pins_border = if app.nav_focus == NavFocus::Pins {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let recents_border = if app.nav_focus == NavFocus::Recents {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Pinned nodes with selection highlight
    let pinned_items: Vec<ListItem> = app.pinned_nodes.iter().enumerate().take(10).map(|(i, node)| {
        let emoji = node.emoji.as_deref().unwrap_or("📌");
        let title = node.ai_title.as_ref().unwrap_or(&node.title);
        let truncated = if title.len() > 20 {
            format!("{}...", &title[..17])
        } else {
            title.clone()
        };
        let content = format!("{} {}", emoji, truncated);

        // Highlight selected item when Pins pane is focused
        if app.nav_focus == NavFocus::Pins && i == app.pins_selected {
            ListItem::new(content).style(Style::default().bg(Color::Blue).fg(Color::White))
        } else {
            ListItem::new(content)
        }
    }).collect();

    let pinned_list = List::new(pinned_items)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(pins_border)
            .title(format!(" 📌 Pinned ({}) ", app.pinned_nodes.len())));
    f.render_widget(pinned_list, chunks[0]);

    // Recent nodes with selection highlight
    let recent_items: Vec<ListItem> = app.recent_nodes.iter().enumerate().take(10).map(|(i, node)| {
        let emoji = node.emoji.as_deref().unwrap_or("📄");
        let title = node.ai_title.as_ref().unwrap_or(&node.title);
        let truncated = if title.len() > 20 {
            format!("{}...", &title[..17])
        } else {
            title.clone()
        };
        let content = format!("{} {}", emoji, truncated);

        // Highlight selected item when Recents pane is focused
        if app.nav_focus == NavFocus::Recents && i == app.recents_selected {
            ListItem::new(content).style(Style::default().bg(Color::Blue).fg(Color::White))
        } else {
            ListItem::new(content)
        }
    }).collect();

    let recent_list = List::new(recent_items)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(recents_border)
            .title(format!(" 🕐 Recent ({}) ", app.recent_nodes.len())));
    f.render_widget(recent_list, chunks[1]);
}

fn draw_preview(f: &mut Frame, app: &TuiApp, area: Rect) {
    let content = if let Some(ref node) = app.selected_node {
        let emoji = node.emoji.as_deref().unwrap_or("");
        let title = node.ai_title.as_ref().unwrap_or(&node.title);
        let node_type = if node.is_item { "Item" } else { "Category" };

        let mut lines = vec![
            Line::from(vec![
                Span::styled(emoji, Style::default()),
                Span::raw(" "),
                Span::styled(title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Type: ", Style::default().fg(Color::Yellow)),
                Span::raw(node_type),
            ]),
            Line::from(vec![
                Span::styled("Children: ", Style::default().fg(Color::Yellow)),
                Span::raw(node.child_count.to_string()),
            ]),
        ];

        // Add date (use derived date for clusters, own date for items)
        let effective_date = node.latest_child_date.unwrap_or(node.created_at);
        let date_str = format_date_time(effective_date);
        let date_color = date_color(effective_date, app.date_min, app.date_max);
        lines.push(Line::from(vec![
            Span::styled("Date: ", Style::default().fg(Color::Yellow)),
            Span::styled(date_str, Style::default().fg(date_color)),
        ]));

        // Add tags if present
        if let Some(ref tags) = node.tags {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("Tags: ", Style::default().fg(Color::Cyan)),
            ]));
            // Wrap tags
            for chunk in tags.chars().collect::<Vec<_>>().chunks(25) {
                lines.push(Line::from(chunk.iter().collect::<String>()));
            }
        }

        // Add summary if present
        if let Some(ref summary) = node.summary {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Summary:", Style::default().fg(Color::Magenta))));
            // Wrap summary
            let preview = if summary.len() > 500 {
                format!("{}...", &summary[..497])
            } else {
                summary.clone()
            };
            for chunk in preview.chars().collect::<Vec<_>>().chunks(45) {
                lines.push(Line::from(chunk.iter().collect::<String>()));
            }
        }

        Text::from(lines)
    } else {
        Text::from("No node selected")
    };

    let preview = Paragraph::new(content)
        .block(Block::default().borders(Borders::ALL).title(" Preview "))
        .wrap(Wrap { trim: false });

    f.render_widget(preview, area);
}

fn draw_breadcrumb(f: &mut Frame, app: &TuiApp, area: Rect) {
    let mut spans = vec![Span::styled(" ", Style::default().bg(Color::Rgb(40, 40, 60)))];

    for (i, (_, title)) in app.breadcrumb_path.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" > ", Style::default().fg(Color::DarkGray).bg(Color::Rgb(40, 40, 60))));
        }

        let style = if i == app.breadcrumb_path.len() - 1 {
            // Current location (highlighted)
            Style::default().fg(Color::Cyan).bg(Color::Rgb(40, 40, 60)).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray).bg(Color::Rgb(40, 40, 60))
        };

        // Truncate long titles
        let display_title = if title.len() > 20 {
            format!("{}...", &title[..17])
        } else {
            title.clone()
        };
        spans.push(Span::styled(display_title, style));
    }

    // Add hint for going back
    if app.breadcrumb_path.len() > 1 {
        spans.push(Span::styled(
            "   [Esc/Backspace: up]",
            Style::default().fg(Color::DarkGray).bg(Color::Rgb(40, 40, 60))
        ));
    }

    let breadcrumb = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(Color::Rgb(40, 40, 60)));
    f.render_widget(breadcrumb, area);
}

fn draw_leaf_view_mode(f: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),  // Header
            Constraint::Min(0),     // Main content
            Constraint::Length(1),  // Status bar
        ])
        .split(f.size());

    // Header with title and back hint
    let title = if let Some(ref node) = app.selected_node {
        let emoji = node.emoji.as_deref().unwrap_or("");
        let title = node.ai_title.as_ref().unwrap_or(&node.title);
        format!("{} {} ", emoji, title)
    } else {
        "Content".to_string()
    };

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(" ← [q/Esc] Back", Style::default().fg(Color::Yellow)),
            Span::raw("   "),
            Span::styled(&title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]),
    ])
    .style(Style::default().bg(Color::Rgb(30, 30, 50)));
    f.render_widget(header, chunks[0]);

    // Main content area: Content (60%) | Sidebar (40%)
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(60),
            Constraint::Percentage(40),
        ])
        .split(chunks[1]);

    // Content pane
    draw_leaf_content(f, app, main_chunks[0]);

    // Sidebar
    draw_leaf_sidebar(f, app, main_chunks[1]);

    // Status bar
    let status_bar = Paragraph::new(&*app.status_message)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White));
    f.render_widget(status_bar, chunks[2]);
}

fn draw_leaf_content(f: &mut Frame, app: &TuiApp, area: Rect) {
    let content = app.leaf_content.as_deref().unwrap_or("No content");

    let border_style = if app.leaf_focus == LeafFocus::Content {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let scroll_info = format!(" Content (line {}) ", app.leaf_scroll_offset + 1);

    // Use Paragraph's native scroll - this handles wrapped text properly
    // by scrolling visual lines, not raw newline-separated lines
    let paragraph = Paragraph::new(content)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(scroll_info))
        .wrap(Wrap { trim: false })
        .scroll((app.leaf_scroll_offset, 0));

    f.render_widget(paragraph, area);
}

fn draw_leaf_sidebar(f: &mut Frame, app: &TuiApp, area: Rect) {
    // Only show Calls section for code nodes with call edges
    let show_calls = app.is_code_node && !app.calls_for_node.is_empty();

    let chunks = if show_calls {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(50),  // Similar nodes
                Constraint::Percentage(50),  // Calls
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(100)])  // Only Similar
            .split(area)
    };

    // Border style for Similar section
    let similar_border = if app.leaf_focus == LeafFocus::Similar {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Similar nodes section with selection highlight
    // Calculate min/max for normalization (like GraphCanvas.tsx line 346)
    let (min_sim, max_sim) = if app.similar_nodes.is_empty() {
        (0.5, 1.0)
    } else {
        let min = app.similar_nodes.iter().map(|s| s.similarity).fold(f32::MAX, f32::min);
        let max = app.similar_nodes.iter().map(|s| s.similarity).fold(f32::MIN, f32::max);
        (min as f64, max as f64)
    };

    // Calculate available width for titles (area width - borders - emoji - percentage - spaces)
    let available_width = area.width.saturating_sub(2 + 2 + 5) as usize; // borders + emoji + " XX%"
    let title_max_len = available_width.saturating_sub(2).max(10); // at least 10 chars

    let similar_items: Vec<ListItem> = app.similar_nodes.iter().enumerate().map(|(i, sim)| {
        let emoji = sim.emoji.as_deref().unwrap_or("📄");
        let similarity_pct = (sim.similarity * 100.0) as i32;
        // Normalized gradient: spreads colors across visible range (red→yellow | blue→cyan)
        let color = similarity_color_normalized(sim.similarity as f64, min_sim, max_sim);

        let truncated_title = utils::safe_truncate(&sim.title, title_max_len);
        let content = format!("{} {} {}%", emoji, truncated_title, similarity_pct);

        // Highlight selected item when Similar section is focused
        if i == app.similar_selected && app.leaf_focus == LeafFocus::Similar {
            ListItem::new(Span::styled(content, Style::default().bg(Color::Blue).fg(Color::White)))
        } else {
            ListItem::new(Span::styled(content, Style::default().fg(color)))
        }
    }).collect();

    let similar_list = List::new(similar_items)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(similar_border)
            .title(format!(" Similar ({}) ", app.similar_nodes.len())));
    f.render_widget(similar_list, chunks[0]);

    // Calls section - only shown for code nodes with call edges
    if show_calls {
        let calls_border = if app.leaf_focus == LeafFocus::Calls {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        // Calculate available width for call titles (area width - borders - arrow - space)
        let calls_title_max = area.width.saturating_sub(2 + 2 + 1) as usize; // borders + "→ "

        let call_items: Vec<ListItem> = app.calls_for_node.iter().enumerate().map(|(i, (_, title, is_outgoing))| {
            // Direction indicator: → for outgoing (calls), ← for incoming (called by)
            let arrow = if *is_outgoing { "→" } else { "←" };
            let truncated_title = utils::safe_truncate(title, calls_title_max.max(10));
            let content = format!("{} {}", arrow, truncated_title);

            // Highlight selected item when Calls section is focused
            if i == app.calls_selected && app.leaf_focus == LeafFocus::Calls {
                ListItem::new(Span::styled(content, Style::default().bg(Color::Blue).fg(Color::White)))
            } else {
                // Color by direction: outgoing = green, incoming = yellow
                let color = if *is_outgoing { Color::Green } else { Color::Yellow };
                ListItem::new(Span::styled(content, Style::default().fg(color)))
            }
        }).collect();

        let calls_list = List::new(call_items)
            .block(Block::default()
                .borders(Borders::ALL)
                .border_style(calls_border)
                .title(format!(" Calls ({}) ", app.calls_for_node.len())));
        f.render_widget(calls_list, chunks[1]);
    }
}

fn draw_edit_mode(f: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),  // Header
            Constraint::Min(0),     // Editor
            Constraint::Length(1),  // Status bar
        ])
        .split(f.size());

    // Header with title and mode indicator
    let title = if let Some(ref node) = app.selected_node {
        let emoji = node.emoji.as_deref().unwrap_or("");
        let title = node.ai_title.as_ref().unwrap_or(&node.title);
        format!("{} {} ", emoji, title)
    } else {
        "Editing".to_string()
    };

    let dirty_indicator = if app.edit_dirty { " [modified]" } else { "" };
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(" ✏️  EDIT MODE", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(dirty_indicator, Style::default().fg(Color::Red)),
            Span::raw("   "),
            Span::styled(&title, Style::default().fg(Color::White)),
        ]),
    ])
    .style(Style::default().bg(Color::Rgb(50, 30, 30)));
    f.render_widget(header, chunks[0]);

    // Calculate visible area for the editor
    let editor_area = chunks[1];
    let inner_height = editor_area.height.saturating_sub(2) as usize; // Account for borders

    // Build editor lines with line numbers and cursor
    let lines: Vec<&str> = app.edit_buffer.lines().collect();
    let total_lines = lines.len().max(1);

    // Calculate line number width (for alignment)
    let line_num_width = total_lines.to_string().len();

    // Build styled lines
    let mut styled_lines: Vec<Line> = Vec::new();

    // Handle empty buffer case
    if app.edit_buffer.is_empty() {
        let line_num = format!("{:>width$} │ ", 1, width = line_num_width);
        styled_lines.push(Line::from(vec![
            Span::styled(line_num, Style::default().fg(Color::DarkGray)),
            Span::styled("█", Style::default().bg(Color::White).fg(Color::Black)), // Cursor
        ]));
    } else {
        for (line_idx, line_content) in lines.iter().enumerate() {
            // Skip lines before scroll offset
            if line_idx < app.edit_scroll_offset {
                continue;
            }
            // Stop if we've filled the visible area
            if styled_lines.len() >= inner_height {
                break;
            }

            let line_num = format!("{:>width$} │ ", line_idx + 1, width = line_num_width);
            let is_cursor_line = line_idx == app.edit_cursor_line;

            if is_cursor_line {
                // Build line with cursor
                let chars: Vec<char> = line_content.chars().collect();
                let mut spans = vec![
                    Span::styled(line_num, Style::default().fg(Color::Yellow)),
                ];

                // Characters before cursor
                if app.edit_cursor_col > 0 {
                    let before: String = chars[..app.edit_cursor_col.min(chars.len())].iter().collect();
                    spans.push(Span::raw(before));
                }

                // Cursor character (or space if at end of line)
                if app.edit_cursor_col < chars.len() {
                    let cursor_char = chars[app.edit_cursor_col].to_string();
                    spans.push(Span::styled(cursor_char, Style::default().bg(Color::White).fg(Color::Black)));
                } else {
                    // Cursor at end of line
                    spans.push(Span::styled(" ", Style::default().bg(Color::White).fg(Color::Black)));
                }

                // Characters after cursor
                if app.edit_cursor_col + 1 < chars.len() {
                    let after: String = chars[app.edit_cursor_col + 1..].iter().collect();
                    spans.push(Span::raw(after));
                }

                styled_lines.push(Line::from(spans));
            } else {
                // Regular line without cursor
                styled_lines.push(Line::from(vec![
                    Span::styled(line_num, Style::default().fg(Color::DarkGray)),
                    Span::raw(*line_content),
                ]));
            }
        }
    }

    // Editor pane
    let scroll_info = format!(
        " Editor - Ln {}/{}, Col {} ",
        app.edit_cursor_line + 1,
        total_lines,
        app.edit_cursor_col + 1
    );

    let editor = Paragraph::new(styled_lines)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(scroll_info))
        .wrap(Wrap { trim: false });
    f.render_widget(editor, editor_area);

    // Status bar with keybindings
    let status_bar = Paragraph::new(&*app.status_message)
        .style(Style::default().bg(Color::Rgb(60, 40, 40)).fg(Color::White));
    f.render_widget(status_bar, chunks[2]);
}

fn draw_tree(f: &mut Frame, app: &TuiApp, area: Rect) {
    // Fixed date width: "01 Jan 2020" = 11 chars
    const DATE_WIDTH: usize = 11;
    // Account for: borders (2), highlight symbol "→ " (3), separator space (1)
    let usable_width = area.width.saturating_sub(6) as usize;
    // Reserve space for date at the end
    let title_max_width = usable_width.saturating_sub(DATE_WIDTH + 1);

    let items: Vec<ListItem> = app.visible_nodes.iter().map(|&idx| {
        let node = &app.nodes[idx];
        let indent = "  ".repeat(node.depth as usize);

        // Use emoji if available, otherwise default icons
        let prefix = if node.is_item {
            node.emoji.as_deref().unwrap_or("📄").to_string()
        } else if node.is_expanded {
            "▼".to_string()
        } else if node.child_count > 0 {
            node.emoji.as_deref().unwrap_or("▶").to_string()
        } else {
            node.emoji.as_deref().unwrap_or("○").to_string()
        };

        let count = if !node.is_item && node.child_count > 0 {
            format!(" ({})", node.child_count)
        } else {
            String::new()
        };

        // Use effective date (derived from children for clusters, own date for items)
        let effective_date = node.latest_child_date.unwrap_or(node.created_at);

        // Calculate date color using graph-matching gradient (red=old → cyan=new)
        let node_date_color = date_color(effective_date, app.date_min, app.date_max);

        // Format date (date only, no time, for hierarchy view)
        let date_str = format_date_only(effective_date);

        // Build title with indent, prefix, title, and count
        let full_title = format!("{}{} {}{}", indent, prefix, node.title, count);

        // Truncate title if needed, accounting for unicode graphemes
        let title_chars: Vec<char> = full_title.chars().collect();
        let truncated_title = if title_chars.len() > title_max_width {
            let truncate_at = title_max_width.saturating_sub(1);
            let truncated: String = title_chars.iter().take(truncate_at).collect();
            format!("{}…", truncated)
        } else {
            full_title
        };

        // Pad title to align dates (left-aligned dates at fixed position)
        let display_width = truncated_title.chars().count();
        let padding = title_max_width.saturating_sub(display_width);
        let padded_title = format!("{}{}", truncated_title, " ".repeat(padding));

        // Create styled content with colored date
        let content = Line::from(vec![
            Span::raw(padded_title),
            Span::raw(" "),
            Span::styled(date_str, Style::default().fg(node_date_color)),
        ]);

        ListItem::new(content)
    }).collect();

    // Border style based on focus
    let border_style = if app.nav_focus == NavFocus::Tree {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let tree = List::new(items)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(" Hierarchy "))
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White).add_modifier(Modifier::BOLD))
        .highlight_symbol("→ ");

    f.render_stateful_widget(tree, area, &mut app.list_state.clone());
}

/// Convert HSL to RGB Color
fn hsl_to_color(hue: f64, saturation: f64, lightness: f64) -> Color {
    let h = (hue % 360.0) / 360.0;
    let s = saturation.clamp(0.0, 1.0);
    let l = lightness.clamp(0.0, 1.0);

    let (r, g, b) = if s == 0.0 {
        let v = (l * 255.0) as u8;
        (v, v, v)
    } else {
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;

        let hue_to_rgb = |p: f64, q: f64, mut t: f64| -> f64 {
            if t < 0.0 { t += 1.0; }
            if t > 1.0 { t -= 1.0; }
            if t < 1.0 / 6.0 { return p + (q - p) * 6.0 * t; }
            if t < 1.0 / 2.0 { return q; }
            if t < 2.0 / 3.0 { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
            p
        };

        let r = (hue_to_rgb(p, q, h + 1.0 / 3.0) * 255.0) as u8;
        let g = (hue_to_rgb(p, q, h) * 255.0) as u8;
        let b = (hue_to_rgb(p, q, h - 1.0 / 3.0) * 255.0) as u8;
        (r, g, b)
    };

    Color::Rgb(r, g, b)
}

/// Get similarity color with range normalization (like GraphCanvas.tsx line 346)
/// Normalizes similarity to visible range, then applies two-segment gradient.
/// RED (0°) → YELLOW (60°) | BLUE (210°) → CYAN (180°)
fn similarity_color_normalized(similarity: f64, min_sim: f64, max_sim: f64) -> Color {
    // Normalize to 0-1 based on visible range (exactly like graph edges)
    let range = (max_sim - min_sim).max(0.01); // avoid div by zero
    let t = ((similarity - min_sim) / range).clamp(0.0, 1.0);

    // Two-segment gradient from getEdgeColor
    let hue = if t < 0.5 {
        t * 2.0 * 60.0              // RED (0°) → YELLOW (60°)
    } else {
        210.0 - (t - 0.5) * 2.0 * 30.0  // BLUE (210°) → CYAN (180°)
    };

    hsl_to_color(hue, 0.80, 0.50)
}

/// Get similarity color with default 0.5-1.0 range normalization
fn similarity_color(similarity: f64) -> Color {
    similarity_color_normalized(similarity, 0.5, 1.0)
}

/// Get date color using EXACT formula from GraphCanvas.tsx getDateColor
/// RED (0°) → YELLOW (60°) at 50% | BLUE (210°) → CYAN (180°) at 100%
/// NO GREEN anywhere. NO saturation tricks.
fn date_color(timestamp: i64, min_date: i64, max_date: i64) -> Color {
    if max_date <= min_date {
        return Color::Gray;
    }
    let t = (timestamp - min_date) as f64 / (max_date - min_date) as f64;

    // EXACT formula from GraphCanvas.tsx getEdgeColor (lines 160-168):
    let hue = if t <= 0.5 {
        t * 2.0 * 60.0              // 0→0°, 50%→60° (red to yellow)
    } else {
        210.0 - (t - 0.5) * 2.0 * 30.0  // 50%→210°, 100%→180° (blue to cyan)
    };

    // Match GraphStatusBar.tsx legend: hsl(h, 75%, 65%)
    hsl_to_color(hue, 0.75, 0.65)
}

fn format_date_time(timestamp: i64) -> String {
    // timestamp is in milliseconds; 0 = unknown date
    if timestamp == 0 {
        return "Unknown".to_string();
    }
    chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|dt| {
            // Only show time if not midnight (papers only have dates, shown as 00:00)
            if dt.hour() == 0 && dt.minute() == 0 {
                dt.format("%d %b %Y").to_string()
            } else {
                dt.format("%d %b %Y %H:%M").to_string()
            }
        })
        .unwrap_or_else(|| "Unknown".to_string())
}

/// Format date only (no time) - fixed width of 11 chars for alignment
fn format_date_only(timestamp: i64) -> String {
    // timestamp is in milliseconds; 0 = unknown date
    if timestamp == 0 {
        return "    Unknown".to_string(); // Pad to 11 chars
    }
    chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|dt| dt.format("%d %b %Y").to_string()) // Always 11 chars: "01 Jan 2020"
        .unwrap_or_else(|| "    Unknown".to_string())
}
