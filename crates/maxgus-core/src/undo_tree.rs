//! Drawing the undo history.
//!
//! The history is a tree; this is how it is read. One line per change,
//! indented by how deep in the history it is, with the branch that is
//! currently in effect marked and the one the file on disk holds marked too.
//!
//! Kept apart from the commands so it can be checked against a shape without
//! a buffer, a window or a keypress.

use maxgus_text::undo::TreeNode;

/// One line of the visualiser, and the node it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub node: usize,
    pub text: String,
}

/// Lays the history out, depth first, so a branch reads as a block.
///
/// Depth first rather than by node number: the numbers are the order the
/// changes were *made*, which interleaves branches and reads as noise.
pub fn lay_out(shape: &[TreeNode], name: &str) -> Vec<Line> {
    let mut lines = vec![Line {
        node: 0,
        text: format!(
            "Undo history for `{name}` — {}",
            crate::count(shape.len().saturating_sub(1), "change")
        ),
    }];
    lines.push(Line {
        node: 0,
        text: String::new(),
    });
    if shape.is_empty() {
        return lines;
    }
    walk(shape, 0, &mut lines);
    lines.push(Line {
        node: 0,
        text: String::new(),
    });
    lines.push(Line {
        node: 0,
        text: "p undo · n redo · b other branch · q close".to_string(),
    });
    lines
}

fn walk(shape: &[TreeNode], id: usize, lines: &mut Vec<Line>) {
    let Some(node) = shape.get(id) else {
        return;
    };
    lines.push(Line {
        node: id,
        text: describe(node),
    });
    // The last child first: it is the one a plain redo takes, and reading it
    // directly under its parent is what makes the current branch legible.
    for child in node.children.iter().rev() {
        walk(shape, *child, lines);
    }
}

fn describe(node: &TreeNode) -> String {
    let indent = "  ".repeat(node.depth);
    let mark = match (node.current, node.on_current_path) {
        (true, _) => '*',
        (false, true) => '|',
        (false, false) => 'o',
    };
    let what = match node.parent {
        None => "the file as it was opened".to_string(),
        Some(_) => match node.edits {
            1 => "1 change".to_string(),
            n => format!("{n} changes"),
        },
    };
    let mut text = format!("{indent}{mark} {:<3} {what}", node.id);
    if node.saved {
        text.push_str("   (on disk)");
    }
    if node.current {
        text.push_str("   ← here");
    }
    if node.children.len() > 1 {
        text.push_str(&format!("   [{} branches]", node.children.len()));
    }
    text
}

/// The line a node is drawn on.
pub fn line_of(lines: &[Line], node: usize) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .skip(2)
        .find(|(_, line)| line.node == node && !line.text.is_empty())
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: usize, parent: Option<usize>, children: Vec<usize>, depth: usize) -> TreeNode {
        TreeNode {
            id,
            parent,
            children,
            depth,
            current: false,
            saved: false,
            on_current_path: false,
            edits: 1,
        }
    }

    /// A root with two branches, the second of which is current.
    fn shape() -> Vec<TreeNode> {
        let mut nodes = vec![
            node(0, None, vec![1, 2], 0),
            node(1, Some(0), vec![], 1),
            node(2, Some(0), vec![], 1),
        ];
        nodes[0].on_current_path = true;
        nodes[0].saved = true;
        nodes[2].current = true;
        nodes[2].on_current_path = true;
        nodes
    }

    #[test]
    fn every_node_gets_a_line() {
        let lines = lay_out(&shape(), "main.rs");
        let drawn: Vec<usize> = lines
            .iter()
            .skip(2)
            .filter(|line| !line.text.is_empty() && !line.text.starts_with('p'))
            .map(|line| line.node)
            .collect();
        assert_eq!(drawn.len(), 3, "a node was not drawn: {lines:#?}");
    }

    #[test]
    fn the_branch_a_redo_would_take_is_drawn_under_its_parent() {
        // Node 2 is the last child, so it is the one a redo takes and the one
        // that reads as the continuation.
        let lines = lay_out(&shape(), "main.rs");
        let order: Vec<usize> = lines
            .iter()
            .skip(2)
            .filter(|line| !line.text.is_empty() && !line.text.starts_with('p'))
            .map(|line| line.node)
            .collect();
        assert_eq!(order, vec![0, 2, 1]);
    }

    #[test]
    fn the_current_node_is_marked_and_so_is_the_one_on_disk() {
        let lines = lay_out(&shape(), "main.rs");
        let text: String = lines
            .iter()
            .map(|line| format!("{}\n", line.text))
            .collect();
        assert!(text.contains("← here"), "the current node is not marked");
        assert!(text.contains("(on disk)"), "the saved node is not marked");
        assert!(text.contains("[2 branches]"), "the fork is not marked");
    }

    #[test]
    fn depth_becomes_indentation() {
        let lines = lay_out(&shape(), "main.rs");
        let root = &lines[2].text;
        let child = &lines[3].text;
        assert!(!root.starts_with(' '), "the root is indented: `{root}`");
        assert!(child.starts_with("  "), "a child is not: `{child}`");
    }

    #[test]
    fn the_first_line_says_how_many_changes_there_are() {
        let lines = lay_out(&shape(), "main.rs");
        assert!(lines[0].text.contains("main.rs"), "no buffer name");
        assert!(
            lines[0].text.contains("2 changes"),
            "got `{}`",
            lines[0].text
        );
    }

    #[test]
    fn a_node_can_be_found_by_the_line_it_is_on() {
        let lines = lay_out(&shape(), "main.rs");
        assert_eq!(line_of(&lines, 2), Some(3));
        assert_eq!(line_of(&lines, 1), Some(4));
        assert_eq!(line_of(&lines, 99), None);
    }

    #[test]
    fn a_history_with_nothing_in_it_still_draws() {
        let lines = lay_out(&[node(0, None, vec![], 0)], "scratch");
        assert!(lines[0].text.contains("0 changes"));
        assert!(lines[2].text.contains("as it was opened"));
    }
}
