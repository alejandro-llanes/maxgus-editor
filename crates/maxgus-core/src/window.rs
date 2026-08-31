//! Windows and the layout tree.
//!
//! Emacs splits a frame into a binary tree of windows. Each leaf shows one
//! buffer and remembers its own point and scroll position, so the same buffer
//! displayed twice can be looked at in two places at once.

use maxgus_text::BufferId;
use maxgus_tui::Rect;
use std::collections::HashMap;

/// Which way a split divides its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// `split-window-below`: children are stacked.
    Vertical,
    /// `split-window-right`: children sit side by side.
    Horizontal,
}

/// A window handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub u64);

impl std::fmt::Display for WindowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<window {}>", self.0)
    }
}

/// One window: a view onto a buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub id: WindowId,
    pub buffer: BufferId,
    /// Point as this window sees it. Saved back to the buffer when the window
    /// is selected, so switching windows does not move the other's cursor.
    pub point: usize,
    /// First buffer line displayed.
    pub top_line: usize,
    /// Which screen row *of* `top_line` is the first one shown.
    ///
    /// Always nought when lines are truncated, where a line is one row. A
    /// window that wraps needs it: a line long enough to fill the window on
    /// its own would otherwise have everything past its first screenful
    /// unreachable, since scrolling could only ever move to the next line.
    pub top_row: usize,
    /// Leftmost display column, for horizontal scrolling of truncated lines.
    pub left_column: usize,
    /// Sticky column for `next-line`, kept per window.
    pub goal_column: Option<usize>,
    /// The area assigned by the most recent layout pass, including the mode
    /// line row.
    pub rect: Rect,
    /// True for a side window such as the file tree, which `other-window`
    /// visits but `balance-windows` leaves at its fixed width.
    pub side: bool,
}

impl Window {
    pub fn new(id: WindowId, buffer: BufferId) -> Window {
        Window {
            id,
            buffer,
            point: 0,
            top_line: 0,
            top_row: 0,
            left_column: 0,
            goal_column: None,
            rect: Rect::default(),
            side: false,
        }
    }

    /// Rows available for buffer text: the window minus its mode line.
    pub fn text_height(&self) -> usize {
        self.rect.height.saturating_sub(1) as usize
    }

    /// Columns available for buffer text.
    pub fn text_width(&self) -> usize {
        self.rect.width as usize
    }

    /// The area the buffer text is drawn into.
    pub fn text_area(&self) -> Rect {
        Rect::new(
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height.saturating_sub(1),
        )
    }

    /// The single row the mode line occupies.
    pub fn mode_line_area(&self) -> Rect {
        let y = self.rect.y + self.rect.height.saturating_sub(1);
        Rect::new(self.rect.x, y, self.rect.width, self.rect.height.min(1))
    }

    /// The last buffer line displayed, given the current scroll position.
    pub fn bottom_line(&self) -> usize {
        self.top_line + self.text_height().saturating_sub(1)
    }

    /// True when `line` is currently on screen.
    pub fn shows_line(&self, line: usize) -> bool {
        self.text_height() > 0 && line >= self.top_line && line <= self.bottom_line()
    }

    /// Scrolls the minimum amount that brings `line` into view, keeping
    /// `margin` rows of context above and below where possible.
    ///
    /// Returns whether the scroll position moved.
    pub fn scroll_to_show(&mut self, line: usize, total_lines: usize, margin: usize) -> bool {
        let before = self.top_line;
        // Before anything else, and whatever the height: a window with no
        // room still must not claim to start past the end of its buffer. A
        // split can squeeze one to nothing and the buffer can shrink under
        // it, and the two together leave it pointing at a line that is gone.
        self.top_line = self.top_line.min(total_lines.saturating_sub(1));
        let height = self.text_height();
        if height == 0 {
            return self.top_line != before;
        }
        // A margin larger than half the window would fight itself.
        let margin = margin.min(height.saturating_sub(1) / 2);

        if line < self.top_line + margin {
            self.top_line = line.saturating_sub(margin);
        } else if line + margin >= self.top_line + height {
            self.top_line = line + margin + 1 - height;
        }
        // Again after the arithmetic: with the margin capped at half the
        // window it cannot exceed this, and the guarantee should hold if that
        // cap ever changes.
        self.top_line = self.top_line.min(total_lines.saturating_sub(1));

        self.top_line != before
    }

    /// Scrolls horizontally so `column` is visible in a window that truncates
    /// long lines.
    pub fn scroll_to_column(&mut self, column: usize) -> bool {
        let width = self.text_width();
        if width == 0 {
            return false;
        }
        let before = self.left_column;
        if column < self.left_column {
            self.left_column = column;
        } else if column >= self.left_column + width {
            self.left_column = column + 1 - width;
        }
        self.left_column != before
    }
}

/// A node in the layout tree.
#[derive(Debug, Clone, PartialEq)]
enum Node {
    Leaf(WindowId),
    Split {
        direction: Direction,
        first: Box<Node>,
        second: Box<Node>,
    },
}

impl Node {
    /// Window ids in layout order: left to right, top to bottom. This is the
    /// order `other-window` walks.
    fn leaves(&self, out: &mut Vec<WindowId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { first, second, .. } => {
                first.leaves(out);
                second.leaves(out);
            }
        }
    }

    fn contains(&self, id: WindowId) -> bool {
        match self {
            Node::Leaf(leaf) => *leaf == id,
            Node::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    /// Replaces the leaf for `id` with `replacement`.
    fn replace_leaf(&mut self, id: WindowId, replacement: Node) -> bool {
        match self {
            Node::Leaf(leaf) if *leaf == id => {
                *self = replacement;
                true
            }
            Node::Leaf(_) => false,
            Node::Split { first, second, .. } => {
                first.replace_leaf(id, replacement.clone()) || second.replace_leaf(id, replacement)
            }
        }
    }

    /// Removes the leaf for `id`, collapsing the split it belonged to.
    fn remove_leaf(&mut self, id: WindowId) -> bool {
        let Node::Split { first, second, .. } = self else {
            return false;
        };
        if **first == Node::Leaf(id) {
            *self = (**second).clone();
            return true;
        }
        if **second == Node::Leaf(id) {
            *self = (**first).clone();
            return true;
        }
        first.remove_leaf(id) || second.remove_leaf(id)
    }

    /// Assigns a rectangle to every leaf.
    fn layout(
        &self,
        rect: Rect,
        widths: &HashMap<WindowId, u16>,
        heights: &HashMap<WindowId, u16>,
        out: &mut HashMap<WindowId, Rect>,
    ) {
        match self {
            Node::Leaf(id) => {
                out.insert(*id, rect);
            }
            Node::Split {
                direction,
                first,
                second,
            } => match direction {
                Direction::Horizontal => {
                    // A side window keeps its configured width; the rest is
                    // divided evenly. A *column* of side windows pins the
                    // width on its topmost member, so the whole column is
                    // measured by it.
                    let width = first
                        .single_fixed(widths)
                        .or_else(|| first.first_leaf().and_then(|id| widths.get(&id).copied()))
                        .unwrap_or_else(|| rect.width.div_ceil(2))
                        .min(rect.width);
                    let (left, right) = rect.split_left(width);
                    first.layout(left, widths, heights, out);
                    second.layout(right, widths, heights, out);
                }
                Direction::Vertical => {
                    // A panel along the bottom keeps its configured height,
                    // whichever side of the split it is on; everything else
                    // splits evenly.
                    let (top, bottom) =
                        match (first.single_fixed(heights), second.single_fixed(heights)) {
                            (_, Some(height)) => rect.split_bottom(height.min(rect.height)),
                            (Some(height), _) => rect.split_top(height.min(rect.height)),
                            _ => rect.split_top(rect.height.div_ceil(2)),
                        };
                    first.layout(top, widths, heights, out);
                    second.layout(bottom, widths, heights, out);
                }
            },
        }
    }

    /// The first leaf in layout order, which for a side column is the window
    /// whose fixed width stands for the whole column.
    fn first_leaf(&self) -> Option<WindowId> {
        match self {
            Node::Leaf(id) => Some(*id),
            Node::Split { first, .. } => first.first_leaf(),
        }
    }

    /// The fixed size of this node, when it is a single fixed-size leaf.
    fn single_fixed(&self, fixed: &HashMap<WindowId, u16>) -> Option<u16> {
        match self {
            Node::Leaf(id) => fixed.get(id).copied(),
            Node::Split { .. } => None,
        }
    }
}

/// A direction to look in for a neighbouring window.
///
/// Distinct from [`Direction`], which says how a window is *split*; this says
/// where on the screen to go next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Towards {
    Left,
    Right,
    Up,
    Down,
}

/// Every window in the frame, plus the tree describing how they are arranged.
#[derive(Debug, Clone)]
pub struct WindowTree {
    root: Node,
    windows: HashMap<WindowId, Window>,
    current: WindowId,
    next_id: u64,
    /// Windows whose width the layout must not change, keyed by id.
    fixed_widths: HashMap<WindowId, u16>,
    /// Windows whose height the layout must not change: the terminal panel
    /// along the bottom, which should not halve the buffer above it.
    fixed_heights: HashMap<WindowId, u16>,
    /// The area the whole tree was last laid out into.
    frame: Rect,
}

impl WindowTree {
    /// A frame holding one window showing `buffer`.
    pub fn new(buffer: BufferId, frame: Rect) -> WindowTree {
        let id = WindowId(1);
        let mut windows = HashMap::new();
        windows.insert(id, Window::new(id, buffer));
        let mut tree = WindowTree {
            root: Node::Leaf(id),
            windows,
            current: id,
            next_id: 2,
            fixed_widths: HashMap::new(),
            fixed_heights: HashMap::new(),
            frame,
        };
        tree.layout(frame);
        tree
    }

    pub fn current_id(&self) -> WindowId {
        self.current
    }

    pub fn current(&self) -> &Window {
        self.windows
            .get(&self.current)
            .expect("the current window always exists")
    }

    pub fn current_mut(&mut self) -> &mut Window {
        self.windows
            .get_mut(&self.current)
            .expect("the current window always exists")
    }

    pub fn get(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }

    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.get_mut(&id)
    }

    /// Selects `id`, if it exists.
    pub fn select(&mut self, id: WindowId) -> bool {
        if self.windows.contains_key(&id) {
            self.current = id;
            return true;
        }
        false
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Window ids in layout order.
    pub fn ids(&self) -> Vec<WindowId> {
        let mut out = Vec::new();
        self.root.leaves(&mut out);
        out
    }

    /// Every window, in layout order.
    pub fn iter(&self) -> impl Iterator<Item = &Window> {
        self.ids()
            .into_iter()
            .filter_map(move |id| self.windows.get(&id))
    }

    /// `other-window`: selects the window `n` places along, wrapping.
    pub fn other_window(&mut self, n: i32) -> WindowId {
        let ids = self.ids();
        if ids.is_empty() {
            return self.current;
        }
        let position = ids.iter().position(|id| *id == self.current).unwrap_or(0) as i32;
        let index = (position + n).rem_euclid(ids.len() as i32) as usize;
        self.current = ids[index];
        self.current
    }

    /// `split-window-below` / `split-window-right`. The new window shows the
    /// same buffer at the same position, as Emacs does, and the original stays
    /// selected.
    pub fn split(&mut self, direction: Direction) -> crate::Result<WindowId> {
        let current = self.current().clone();
        // Each half needs a text row and a mode line, or two columns.
        let too_small = match direction {
            Direction::Vertical => current.rect.height < 4,
            Direction::Horizontal => current.rect.width < 4,
        };
        if too_small {
            return Err(crate::CoreError::TooSmallToSplit);
        }

        let id = WindowId(self.next_id);
        self.next_id += 1;
        let mut new_window = Window::new(id, current.buffer);
        new_window.point = current.point;
        new_window.top_line = current.top_line;
        self.windows.insert(id, new_window);

        let replacement = Node::Split {
            direction,
            first: Box::new(Node::Leaf(current.id)),
            second: Box::new(Node::Leaf(id)),
        };
        self.root.replace_leaf(current.id, replacement);
        self.layout(self.frame);
        Ok(id)
    }

    /// Adds a fixed-width side window on the left, as the file tree uses.
    /// Returns the new window's id and selects nothing.
    pub fn add_side_window(&mut self, buffer: BufferId, width: u16) -> WindowId {
        let id = WindowId(self.next_id);
        self.next_id += 1;
        let mut window = Window::new(id, buffer);
        window.side = true;
        self.windows.insert(id, window);
        self.fixed_widths.insert(id, width);

        // The side window sits to the left of everything else — but above a
        // bottom panel rather than beside it, so the terminal keeps the full
        // width of the frame.
        let bottom_panel = matches!(
            &self.root,
            Node::Split { direction: Direction::Vertical, second, .. }
                if second.single_fixed(&self.fixed_heights).is_some()
        );
        if bottom_panel && let Node::Split { first, .. } = &mut self.root {
            let above = std::mem::replace(&mut **first, Node::Leaf(id));
            **first = Node::Split {
                direction: Direction::Horizontal,
                first: Box::new(Node::Leaf(id)),
                second: Box::new(above),
            };
        } else {
            let existing = std::mem::replace(&mut self.root, Node::Leaf(id));
            self.root = Node::Split {
                direction: Direction::Horizontal,
                first: Box::new(Node::Leaf(id)),
                second: Box::new(existing),
            };
        }
        self.layout(self.frame);
        id
    }

    /// `delete-window`.
    pub fn delete(&mut self, id: WindowId) -> crate::Result<()> {
        if self.windows.len() <= 1 {
            return Err(crate::CoreError::OnlyWindow);
        }
        if !self.windows.contains_key(&id) {
            return Err(crate::CoreError::NoSuchWindow);
        }
        self.root.remove_leaf(id);
        self.windows.remove(&id);
        self.fixed_widths.remove(&id);
        self.fixed_heights.remove(&id);
        if self.current == id {
            // Selection falls to the first remaining window in layout order.
            self.current = self.ids().first().copied().expect("one window remains");
        }
        self.layout(self.frame);
        Ok(())
    }

    /// `delete-other-windows`: keeps only the current one.
    pub fn delete_others(&mut self) {
        let keep = self.current;
        self.windows.retain(|id, _| *id == keep);
        self.fixed_widths.retain(|id, _| *id == keep);
        self.fixed_heights.retain(|id, _| *id == keep);
        self.root = Node::Leaf(keep);
        self.layout(self.frame);
    }

    /// Recomputes every window's rectangle for `frame`.
    pub fn layout(&mut self, frame: Rect) {
        self.frame = frame;
        let mut rects = HashMap::new();
        self.root
            .layout(frame, &self.fixed_widths, &self.fixed_heights, &mut rects);
        for (id, rect) in rects {
            if let Some(window) = self.windows.get_mut(&id) {
                window.rect = rect;
            }
        }
    }

    /// The area the tree occupies.
    pub fn frame(&self) -> Rect {
        self.frame
    }

    /// Sets a side window's width and re-lays out.
    pub fn set_fixed_width(&mut self, id: WindowId, width: u16) {
        if self.windows.contains_key(&id) {
            self.fixed_widths.insert(id, width);
            self.layout(self.frame);
        }
    }

    /// Lets the layout decide a window's height again.
    pub fn clear_fixed_height(&mut self, id: WindowId) {
        if self.fixed_heights.remove(&id).is_some() {
            self.layout(self.frame);
        }
    }

    /// Sets a bottom panel's height and re-lays out.
    pub fn set_fixed_height(&mut self, id: WindowId, height: u16) {
        if self.windows.contains_key(&id) {
            self.fixed_heights.insert(id, height);
            self.layout(self.frame);
        }
    }

    /// Adds a column of stacked windows down the left, as the side panel uses.
    ///
    /// One window per entry, in order, each with the height it asks for; the
    /// first entry takes whatever is left. Built as one call rather than by
    /// adding them one at a time, because the shape has to come out as
    ///
    /// ```text
    /// Vertical{ Vertical{ first, second(fixed) }, third(fixed) }
    /// ```
    ///
    /// so that every fixed height is the *second* child of its split, which
    /// is the only arrangement the layout can honour.
    pub fn add_side_column(
        &mut self,
        entries: &[(BufferId, Option<u16>)],
        width: u16,
    ) -> Vec<WindowId> {
        let mut ids = Vec::new();
        let mut column: Option<Node> = None;
        for (buffer, height) in entries {
            let id = WindowId(self.next_id);
            self.next_id += 1;
            let mut window = Window::new(id, *buffer);
            window.side = true;
            self.windows.insert(id, window);
            if let Some(height) = height {
                self.fixed_heights.insert(id, *height);
            }
            ids.push(id);
            column = Some(match column {
                None => Node::Leaf(id),
                Some(above) => Node::Split {
                    direction: Direction::Vertical,
                    first: Box::new(above),
                    second: Box::new(Node::Leaf(id)),
                },
            });
        }
        let Some(column) = column else { return ids };
        // The column is as wide as the panel; the width belongs to the whole
        // of it, so it is pinned on the topmost window, which is the one the
        // horizontal split measures.
        if let Some(first) = ids.first() {
            self.fixed_widths.insert(*first, width);
        }

        // Beside everything else, but above a bottom panel rather than
        // beside it, so the terminal keeps the full width of the frame.
        let bottom_panel = matches!(
            &self.root,
            Node::Split { direction: Direction::Vertical, second, .. }
                if second.single_fixed(&self.fixed_heights).is_some()
        );
        if bottom_panel && let Node::Split { first, .. } = &mut self.root {
            let beside = std::mem::replace(&mut **first, Node::Leaf(WindowId(0)));
            **first = Node::Split {
                direction: Direction::Horizontal,
                first: Box::new(column),
                second: Box::new(beside),
            };
        } else {
            let existing = std::mem::replace(&mut self.root, Node::Leaf(WindowId(0)));
            self.root = Node::Split {
                direction: Direction::Horizontal,
                first: Box::new(column),
                second: Box::new(existing),
            };
        }
        self.layout(self.frame);
        ids
    }

    /// Adds a fixed-height panel across the bottom, as the terminal uses.
    ///
    /// It wraps the whole frame, so it spans the full width even when the
    /// side panel is open — a terminal tucked into the corner beside the file
    /// tree is not what anybody means by a terminal along the bottom.
    pub fn add_bottom_window(&mut self, buffer: BufferId, height: u16) -> WindowId {
        let id = WindowId(self.next_id);
        self.next_id += 1;
        let mut window = Window::new(id, buffer);
        window.side = true;
        self.windows.insert(id, window);
        self.fixed_heights.insert(id, height);

        let existing = std::mem::replace(&mut self.root, Node::Leaf(id));
        self.root = Node::Split {
            direction: Direction::Vertical,
            first: Box::new(existing),
            second: Box::new(Node::Leaf(id)),
        };
        self.layout(self.frame);
        id
    }

    /// The window at terminal coordinates, if any.
    /// The window next to `from` in `direction`, if there is one.
    ///
    /// Probed from the middle of the edge being crossed, so which window you
    /// land in follows the shape of the layout rather than the order the
    /// windows happen to be stored in. This is what `C-<left>` and friends
    /// use: with a file tree on the left, `C-<right>` from it is the code and
    /// `C-<left>` from the code is the tree, every time.
    pub fn neighbour(&self, from: WindowId, direction: Towards) -> Option<WindowId> {
        let rect = self.get(from)?.rect;
        // One cell beyond the edge, halfway along it.
        let (x, y) = match direction {
            Towards::Left => (rect.x.checked_sub(1)?, rect.y + rect.height / 2),
            Towards::Right => (rect.right(), rect.y + rect.height / 2),
            Towards::Up => (rect.x + rect.width / 2, rect.y.checked_sub(1)?),
            Towards::Down => (rect.x + rect.width / 2, rect.bottom()),
        };
        self.window_at(x, y).filter(|found| *found != from)
    }

    pub fn window_at(&self, x: u16, y: u16) -> Option<WindowId> {
        self.iter().find(|w| w.rect.contains(x, y)).map(|w| w.id)
    }

    /// Windows showing `buffer`, in layout order.
    pub fn showing(&self, buffer: BufferId) -> Vec<WindowId> {
        self.iter()
            .filter(|w| w.buffer == buffer)
            .map(|w| w.id)
            .collect()
    }

    /// Points every window showing `from` at `to`, as `kill-buffer` must so
    /// no window is left displaying a buffer that no longer exists.
    pub fn replace_buffer(&mut self, from: BufferId, to: BufferId) {
        for window in self.windows.values_mut() {
            if window.buffer == from {
                window.buffer = to;
                window.point = 0;
                window.top_line = 0;
            }
        }
    }

    /// True when the tree still describes exactly the windows that exist.
    pub fn is_consistent(&self) -> bool {
        let ids = self.ids();
        ids.len() == self.windows.len()
            && ids.iter().all(|id| self.windows.contains_key(id))
            && self.root.contains(self.current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Rect {
        Rect::new(0, 0, 80, 24)
    }

    fn tree() -> WindowTree {
        WindowTree::new(BufferId(1), frame())
    }

    #[test]
    fn a_new_frame_holds_one_window_filling_it() {
        let t = tree();
        assert_eq!(t.len(), 1);
        assert_eq!(t.current().rect, frame());
        assert_eq!(t.current().buffer, BufferId(1));
        assert!(t.is_consistent());
    }

    #[test]
    fn a_window_reserves_its_last_row_for_the_mode_line() {
        let t = tree();
        let w = t.current();
        assert_eq!(w.text_height(), 23);
        assert_eq!(w.text_area(), Rect::new(0, 0, 80, 23));
        assert_eq!(w.mode_line_area(), Rect::new(0, 23, 80, 1));
    }

    #[test]
    fn splitting_below_stacks_the_windows() {
        let mut t = tree();
        let new = t.split(Direction::Vertical).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t.current_id(), WindowId(1), "the original stays selected");
        let top = t.get(WindowId(1)).unwrap().rect;
        let bottom = t.get(new).unwrap().rect;
        assert_eq!(top, Rect::new(0, 0, 80, 12));
        assert_eq!(bottom, Rect::new(0, 12, 80, 12));
        assert!(t.is_consistent());
    }

    #[test]
    fn splitting_right_places_the_windows_side_by_side() {
        let mut t = tree();
        let new = t.split(Direction::Horizontal).unwrap();
        assert_eq!(t.get(WindowId(1)).unwrap().rect, Rect::new(0, 0, 40, 24));
        assert_eq!(t.get(new).unwrap().rect, Rect::new(40, 0, 40, 24));
    }

    #[test]
    fn a_split_window_shows_the_same_buffer_at_the_same_place() {
        let mut t = tree();
        t.current_mut().point = 42;
        t.current_mut().top_line = 7;
        let new = t.split(Direction::Vertical).unwrap();
        let w = t.get(new).unwrap();
        assert_eq!(w.buffer, BufferId(1));
        assert_eq!(w.point, 42);
        assert_eq!(w.top_line, 7);
    }

    #[test]
    fn a_window_too_small_to_split_refuses() {
        let mut t = WindowTree::new(BufferId(1), Rect::new(0, 0, 80, 3));
        assert!(matches!(
            t.split(Direction::Vertical),
            Err(crate::CoreError::TooSmallToSplit)
        ));
        let mut t = WindowTree::new(BufferId(1), Rect::new(0, 0, 3, 24));
        assert!(matches!(
            t.split(Direction::Horizontal),
            Err(crate::CoreError::TooSmallToSplit)
        ));
    }

    #[test]
    fn nested_splits_subdivide_correctly() {
        let mut t = tree();
        t.split(Direction::Horizontal).unwrap();
        // Split the left half vertically.
        t.split(Direction::Vertical).unwrap();
        assert_eq!(t.len(), 3);
        assert!(t.is_consistent());
        let total: usize = t.iter().map(|w| w.rect.area()).sum();
        assert_eq!(total, frame().area(), "the splits partition the frame");
    }

    #[test]
    fn other_window_cycles_in_layout_order_and_wraps() {
        let mut t = tree();
        let second = t.split(Direction::Horizontal).unwrap();
        assert_eq!(t.other_window(1), second);
        assert_eq!(t.other_window(1), WindowId(1), "wraps around");
        assert_eq!(t.other_window(-1), second, "and backwards");
    }

    #[test]
    fn other_window_on_a_single_window_frame_stays_put() {
        let mut t = tree();
        assert_eq!(t.other_window(1), WindowId(1));
        assert_eq!(t.other_window(5), WindowId(1));
    }

    #[test]
    fn deleting_a_window_collapses_its_split() {
        let mut t = tree();
        let second = t.split(Direction::Vertical).unwrap();
        t.delete(second).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t.current().rect, frame(), "the survivor reclaims the space");
        assert!(t.is_consistent());
    }

    #[test]
    fn deleting_the_selected_window_moves_the_selection() {
        let mut t = tree();
        let second = t.split(Direction::Vertical).unwrap();
        t.select(second);
        t.delete(second).unwrap();
        assert_eq!(t.current_id(), WindowId(1));
        assert!(t.is_consistent());
    }

    #[test]
    fn the_last_window_cannot_be_deleted() {
        let mut t = tree();
        assert!(matches!(
            t.delete(WindowId(1)),
            Err(crate::CoreError::OnlyWindow)
        ));
    }

    #[test]
    fn deleting_an_unknown_window_is_an_error() {
        let mut t = tree();
        t.split(Direction::Vertical).unwrap();
        assert!(matches!(
            t.delete(WindowId(99)),
            Err(crate::CoreError::NoSuchWindow)
        ));
    }

    #[test]
    fn delete_other_windows_keeps_only_the_current_one() {
        let mut t = tree();
        t.split(Direction::Vertical).unwrap();
        t.split(Direction::Horizontal).unwrap();
        assert_eq!(t.len(), 3);
        t.delete_others();
        assert_eq!(t.len(), 1);
        assert_eq!(t.current().rect, frame());
        assert!(t.is_consistent());
    }

    #[test]
    fn a_side_window_keeps_its_width_and_sits_on_the_left() {
        let mut t = tree();
        let side = t.add_side_window(BufferId(2), 32);
        assert_eq!(t.get(side).unwrap().rect, Rect::new(0, 0, 32, 24));
        assert_eq!(t.get(WindowId(1)).unwrap().rect, Rect::new(32, 0, 48, 24));
        assert!(t.get(side).unwrap().side);
        assert!(t.is_consistent());
    }

    #[test]
    fn a_side_window_survives_splits_of_the_main_area() {
        let mut t = tree();
        let side = t.add_side_window(BufferId(2), 32);
        t.split(Direction::Vertical).unwrap();
        assert_eq!(t.get(side).unwrap().rect.width, 32, "still fixed");
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn a_side_windows_width_can_be_changed() {
        let mut t = tree();
        let side = t.add_side_window(BufferId(2), 32);
        t.set_fixed_width(side, 20);
        assert_eq!(t.get(side).unwrap().rect.width, 20);
        assert_eq!(t.get(WindowId(1)).unwrap().rect.width, 60);
    }

    #[test]
    fn a_side_window_can_be_deleted_like_any_other() {
        let mut t = tree();
        let side = t.add_side_window(BufferId(2), 32);
        t.delete(side).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t.current().rect, frame());
    }

    #[test]
    fn resizing_the_frame_re_lays_out_every_window() {
        let mut t = tree();
        t.split(Direction::Horizontal).unwrap();
        t.layout(Rect::new(0, 0, 100, 30));
        let widths: Vec<u16> = t.iter().map(|w| w.rect.width).collect();
        assert_eq!(widths, vec![50, 50]);
        assert!(t.iter().all(|w| w.rect.height == 30));
    }

    #[test]
    fn windows_showing_a_buffer_can_be_found_and_redirected() {
        let mut t = tree();
        let second = t.split(Direction::Vertical).unwrap();
        assert_eq!(t.showing(BufferId(1)), vec![WindowId(1), second]);
        t.get_mut(second).unwrap().buffer = BufferId(2);
        assert_eq!(t.showing(BufferId(1)), vec![WindowId(1)]);

        t.replace_buffer(BufferId(1), BufferId(3));
        assert_eq!(t.get(WindowId(1)).unwrap().buffer, BufferId(3));
        assert_eq!(
            t.get(WindowId(1)).unwrap().point,
            0,
            "point resets in the new buffer"
        );
    }

    #[test]
    fn a_window_can_be_found_by_coordinates() {
        let mut t = tree();
        let second = t.split(Direction::Horizontal).unwrap();
        assert_eq!(t.window_at(5, 5), Some(WindowId(1)));
        assert_eq!(t.window_at(50, 5), Some(second));
        assert_eq!(t.window_at(200, 5), None);
    }

    #[test]
    fn selecting_an_unknown_window_fails_without_changing_anything() {
        let mut t = tree();
        assert!(!t.select(WindowId(99)));
        assert_eq!(t.current_id(), WindowId(1));
    }

    // ---- scrolling ----

    fn window(height: u16) -> Window {
        let mut w = Window::new(WindowId(1), BufferId(1));
        w.rect = Rect::new(0, 0, 80, height);
        w
    }

    #[test]
    fn a_line_already_visible_causes_no_scroll() {
        let mut w = window(11); // ten text rows
        assert!(!w.scroll_to_show(5, 100, 0));
        assert_eq!(w.top_line, 0);
        assert!(w.shows_line(5));
    }

    #[test]
    fn scrolling_down_moves_the_minimum_needed() {
        let mut w = window(11);
        assert!(w.scroll_to_show(10, 100, 0));
        assert_eq!(w.top_line, 1, "just enough to bring line 10 into view");
        assert_eq!(w.bottom_line(), 10);
    }

    #[test]
    fn scrolling_up_moves_the_minimum_needed() {
        let mut w = window(11);
        w.top_line = 20;
        assert!(w.scroll_to_show(18, 100, 0));
        assert_eq!(w.top_line, 18);
    }

    #[test]
    fn a_scroll_margin_keeps_context_around_point() {
        let mut w = window(11);
        w.scroll_to_show(10, 100, 3);
        assert_eq!(w.top_line, 4, "three rows below line 10 stay visible");
        w.top_line = 20;
        w.scroll_to_show(21, 100, 3);
        assert_eq!(w.top_line, 18, "three rows above line 21 stay visible");
    }

    #[test]
    fn an_oversized_margin_is_capped_at_half_the_window() {
        let mut w = window(11);
        w.scroll_to_show(50, 100, 99);
        // The margin caps at four, so line 50 sits four rows from the bottom.
        assert!(w.shows_line(50), "point stays visible whatever the margin");
    }

    #[test]
    fn scrolling_never_runs_past_the_end_of_the_buffer() {
        let mut w = window(11);
        w.scroll_to_show(9, 5, 0);
        assert!(w.top_line < 5);
    }

    #[test]
    fn a_zero_height_window_cannot_scroll() {
        let mut w = window(1); // mode line only
        assert_eq!(w.text_height(), 0);
        assert!(!w.scroll_to_show(50, 100, 0));
        assert!(!w.shows_line(0));
    }

    #[test]
    fn horizontal_scrolling_follows_the_column() {
        let mut w = window(11);
        assert!(!w.scroll_to_column(10), "already visible");
        assert!(w.scroll_to_column(100));
        assert_eq!(w.left_column, 21, "column 100 is the rightmost of eighty");
        assert!(w.scroll_to_column(5));
        assert_eq!(w.left_column, 5);
    }
    #[test]
    fn a_bottom_panel_takes_its_height_and_leaves_the_rest_above() {
        let mut t = WindowTree::new(BufferId(1), Rect::new(0, 0, 80, 24));
        let panel = t.add_bottom_window(BufferId(2), 8);

        let panel_rect = t.get(panel).unwrap().rect;
        assert_eq!(
            panel_rect.height, 8,
            "the panel is not the height it asked for"
        );
        assert_eq!(panel_rect.y, 16, "it is not at the bottom");
        assert_eq!(panel_rect.width, 80, "it does not span the frame");

        let above = t.ids().into_iter().find(|id| *id != panel).unwrap();
        assert_eq!(
            t.get(above).unwrap().rect.height,
            16,
            "the buffer was halved"
        );
    }

    #[test]
    fn a_bottom_panel_spans_the_frame_even_with_a_side_window_open() {
        // A terminal tucked into the corner beside the file tree is not what
        // anybody means by a terminal along the bottom. Both orders of
        // opening have to end up the same way round.
        for tree_first in [true, false] {
            let mut t = WindowTree::new(BufferId(1), Rect::new(0, 0, 80, 24));
            let (side, panel) = if tree_first {
                let side = t.add_side_window(BufferId(2), 30);
                (side, t.add_bottom_window(BufferId(3), 8))
            } else {
                let panel = t.add_bottom_window(BufferId(3), 8);
                (t.add_side_window(BufferId(2), 30), panel)
            };
            let panel_rect = t.get(panel).unwrap().rect;
            assert_eq!(
                panel_rect.width, 80,
                "tree_first={tree_first}: the panel was narrowed"
            );
            assert_eq!(panel_rect.y, 16, "tree_first={tree_first}");
            let side_rect = t.get(side).unwrap().rect;
            assert_eq!(
                side_rect.width, 30,
                "tree_first={tree_first}: the tree lost its width"
            );
            assert_eq!(
                side_rect.height, 16,
                "tree_first={tree_first}: the tree overlaps the panel"
            );
        }
    }

    #[test]
    fn closing_the_bottom_panel_gives_the_height_back() {
        let mut t = WindowTree::new(BufferId(1), Rect::new(0, 0, 80, 24));
        let panel = t.add_bottom_window(BufferId(2), 8);
        let above = t.ids().into_iter().find(|id| *id != panel).unwrap();
        t.delete(panel).unwrap();
        assert_eq!(t.get(above).unwrap().rect.height, 24);
        assert!(t.is_consistent());
    }

    #[test]
    fn a_bottom_panel_can_be_resized() {
        let mut t = WindowTree::new(BufferId(1), Rect::new(0, 0, 80, 24));
        let panel = t.add_bottom_window(BufferId(2), 8);
        t.set_fixed_height(panel, 12);
        assert_eq!(t.get(panel).unwrap().rect.height, 12);
        assert_eq!(t.get(panel).unwrap().rect.y, 12);
    }

    #[test]
    fn a_side_column_stacks_its_windows_down_the_left() {
        let mut t = WindowTree::new(BufferId(1), Rect::new(0, 0, 80, 30));
        let ids = t.add_side_column(
            &[
                (BufferId(2), None),
                (BufferId(3), Some(8)),
                (BufferId(4), Some(6)),
            ],
            30,
        );
        assert_eq!(ids.len(), 3);
        let rect = |id: WindowId| t.get(id).unwrap().rect;

        // All three are the column's width, stacked in order, filling it.
        for id in &ids {
            assert_eq!(
                rect(*id).width,
                30,
                "a member of the column is not its width"
            );
            assert_eq!(rect(*id).x, 0);
        }
        assert_eq!(rect(ids[1]).height, 8, "the middle window lost its height");
        assert_eq!(rect(ids[2]).height, 6, "the bottom window lost its height");
        assert_eq!(
            rect(ids[0]).height,
            16,
            "the top window did not take the rest"
        );
        assert_eq!(rect(ids[0]).y, 0);
        assert_eq!(rect(ids[1]).y, 16);
        assert_eq!(rect(ids[2]).y, 24);

        // And the buffer beside them keeps the rest of the frame.
        let beside = t.ids().into_iter().find(|id| !ids.contains(id)).unwrap();
        assert_eq!(rect(beside).x, 30);
        assert_eq!(rect(beside).width, 50);
        assert_eq!(rect(beside).height, 30);
        assert!(t.is_consistent());
    }

    #[test]
    fn the_arrow_keys_reach_every_window_of_a_side_column() {
        // The whole reason for making the panel three windows: moving between
        // its parts is ordinary window movement.
        let mut t = WindowTree::new(BufferId(1), Rect::new(0, 0, 80, 30));
        let ids = t.add_side_column(
            &[
                (BufferId(2), None),
                (BufferId(3), Some(8)),
                (BufferId(4), Some(6)),
            ],
            30,
        );
        assert_eq!(t.neighbour(ids[0], Towards::Down), Some(ids[1]));
        assert_eq!(t.neighbour(ids[1], Towards::Down), Some(ids[2]));
        assert_eq!(t.neighbour(ids[2], Towards::Up), Some(ids[1]));
        assert_eq!(t.neighbour(ids[1], Towards::Up), Some(ids[0]));
        // And out of the column to the buffer beside it.
        let beside = t.ids().into_iter().find(|id| !ids.contains(id)).unwrap();
        assert_eq!(t.neighbour(ids[1], Towards::Right), Some(beside));
        assert_eq!(t.neighbour(beside, Towards::Left), Some(ids[0]));
    }

    #[test]
    fn a_side_column_of_one_behaves_like_a_side_window() {
        let mut t = WindowTree::new(BufferId(1), Rect::new(0, 0, 80, 24));
        let ids = t.add_side_column(&[(BufferId(2), None)], 32);
        assert_eq!(t.get(ids[0]).unwrap().rect, Rect::new(0, 0, 32, 24));
    }

    #[test]
    fn a_side_column_sits_above_the_bottom_panel() {
        let mut t = WindowTree::new(BufferId(1), Rect::new(0, 0, 80, 30));
        let panel = t.add_bottom_window(BufferId(5), 8);
        let ids = t.add_side_column(&[(BufferId(2), None), (BufferId(3), Some(6))], 30);
        assert_eq!(
            t.get(panel).unwrap().rect.width,
            80,
            "the terminal was narrowed"
        );
        for id in &ids {
            assert!(
                t.get(*id).unwrap().rect.bottom() <= t.get(panel).unwrap().rect.y,
                "the column overlaps the terminal"
            );
        }
    }
}

#[cfg(test)]
mod scroll_tests {
    use super::*;

    fn window(height: u16) -> Window {
        let mut window = Window::new(WindowId(1), BufferId(1));
        window.rect = Rect::new(0, 0, 40, height);
        window.top_line = 30;
        window
    }

    #[test]
    fn a_window_never_starts_past_the_end_of_its_buffer() {
        let mut window = window(10);
        window.scroll_to_show(0, 1, 0);
        assert_eq!(window.top_line, 0);
    }

    #[test]
    fn a_window_with_no_room_is_still_brought_back_into_its_buffer() {
        // A split can squeeze a window to nothing, and the buffer it shows
        // can shrink under it. Neither is a reason to keep pointing at a line
        // that is no longer there.
        let mut window = window(1);
        assert_eq!(window.text_height(), 0, "the fixture should have no room");
        assert!(window.scroll_to_show(0, 1, 0), "it reported no change");
        assert_eq!(window.top_line, 0);
    }

    #[test]
    fn an_empty_buffer_leaves_the_window_at_its_first_line() {
        let mut window = window(10);
        window.scroll_to_show(0, 0, 0);
        assert_eq!(window.top_line, 0);
    }
}
