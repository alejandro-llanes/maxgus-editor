//! Rectangles and sizes, in terminal cells.

/// A terminal size in columns and rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}

impl Size {
    pub fn new(width: u16, height: u16) -> Size {
        Size { width, height }
    }

    pub fn area(self) -> usize {
        self.width as usize * self.height as usize
    }

    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// A rectangular region, addressed from the top-left of the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Rect {
        Rect { x, y, width, height }
    }

    /// The whole terminal.
    pub fn from_size(size: Size) -> Rect {
        Rect::new(0, 0, size.width, size.height)
    }

    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    pub fn area(&self) -> usize {
        self.width as usize * self.height as usize
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// One past the rightmost column.
    pub fn right(&self) -> u16 {
        self.x.saturating_add(self.width)
    }

    /// One past the bottom row.
    pub fn bottom(&self) -> u16 {
        self.y.saturating_add(self.height)
    }

    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// The overlap of two rectangles, if any.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (x < right && y < bottom).then(|| Rect::new(x, y, right - x, bottom - y))
    }

    /// Splits off `rows` from the top, returning (top, rest). A request for
    /// more rows than exist takes everything.
    pub fn split_top(&self, rows: u16) -> (Rect, Rect) {
        let rows = rows.min(self.height);
        (
            Rect::new(self.x, self.y, self.width, rows),
            Rect::new(self.x, self.y + rows, self.width, self.height - rows),
        )
    }

    /// Splits off `rows` from the bottom, returning (rest, bottom). This is
    /// how the mode line and echo area are carved out.
    pub fn split_bottom(&self, rows: u16) -> (Rect, Rect) {
        let rows = rows.min(self.height);
        let kept = self.height - rows;
        (
            Rect::new(self.x, self.y, self.width, kept),
            Rect::new(self.x, self.y + kept, self.width, rows),
        )
    }

    /// Splits off `columns` from the left, returning (left, rest). The file
    /// tree takes its side window this way.
    pub fn split_left(&self, columns: u16) -> (Rect, Rect) {
        let columns = columns.min(self.width);
        (
            Rect::new(self.x, self.y, columns, self.height),
            Rect::new(self.x + columns, self.y, self.width - columns, self.height),
        )
    }

    /// Splits off `columns` from the right, returning (rest, right).
    pub fn split_right(&self, columns: u16) -> (Rect, Rect) {
        let columns = columns.min(self.width);
        let kept = self.width - columns;
        (
            Rect::new(self.x, self.y, kept, self.height),
            Rect::new(self.x + kept, self.y, columns, self.height),
        )
    }

    /// Splits into two halves side by side, as `split-window-right` does. The
    /// left half keeps the extra column when the width is odd.
    pub fn split_horizontally(&self) -> (Rect, Rect) {
        self.split_left(self.width.div_ceil(2))
    }

    /// Splits into two halves stacked vertically, as `split-window-below`
    /// does. The top half keeps the extra row when the height is odd.
    pub fn split_vertically(&self) -> (Rect, Rect) {
        self.split_top(self.height.div_ceil(2))
    }

    /// Shrinks the rectangle by `amount` on every side.
    pub fn inset(&self, amount: u16) -> Rect {
        let doubled = amount.saturating_mul(2);
        Rect::new(
            self.x.saturating_add(amount),
            self.y.saturating_add(amount),
            self.width.saturating_sub(doubled),
            self.height.saturating_sub(doubled),
        )
    }

    /// Every (x, y) inside, row by row.
    pub fn cells(&self) -> impl Iterator<Item = (u16, u16)> + '_ {
        (self.y..self.bottom()).flat_map(move |y| (self.x..self.right()).map(move |x| (x, y)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rect_reports_its_edges() {
        let r = Rect::new(2, 3, 10, 5);
        assert_eq!(r.right(), 12);
        assert_eq!(r.bottom(), 8);
        assert_eq!(r.area(), 50);
        assert_eq!(r.size(), Size::new(10, 5));
        assert!(!r.is_empty());
        assert!(Rect::new(0, 0, 0, 5).is_empty());
    }

    #[test]
    fn containment_is_half_open_on_both_axes() {
        let r = Rect::new(2, 3, 4, 4);
        assert!(r.contains(2, 3));
        assert!(r.contains(5, 6));
        assert!(!r.contains(6, 6), "right edge is exclusive");
        assert!(!r.contains(5, 7), "bottom edge is exclusive");
        assert!(!r.contains(1, 3));
    }

    #[test]
    fn intersection_finds_the_overlap() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(&b), Some(Rect::new(5, 5, 5, 5)));
        assert_eq!(a.intersect(&Rect::new(20, 20, 5, 5)), None);
        assert_eq!(a.intersect(&Rect::new(10, 0, 5, 5)), None, "touching is not overlapping");
    }

    #[test]
    fn splitting_off_the_bottom_carves_out_the_mode_line() {
        let screen = Rect::new(0, 0, 80, 24);
        let (body, echo) = screen.split_bottom(1);
        assert_eq!(body, Rect::new(0, 0, 80, 23));
        assert_eq!(echo, Rect::new(0, 23, 80, 1));
        let (text, mode_line) = body.split_bottom(1);
        assert_eq!(text.height, 22);
        assert_eq!(mode_line, Rect::new(0, 22, 80, 1));
    }

    #[test]
    fn splitting_off_the_top_works_the_same_way() {
        let (top, rest) = Rect::new(0, 0, 80, 24).split_top(3);
        assert_eq!(top, Rect::new(0, 0, 80, 3));
        assert_eq!(rest, Rect::new(0, 3, 80, 21));
    }

    #[test]
    fn splitting_off_the_left_carves_out_the_tree_window() {
        let (tree, rest) = Rect::new(0, 0, 80, 24).split_left(32);
        assert_eq!(tree, Rect::new(0, 0, 32, 24));
        assert_eq!(rest, Rect::new(32, 0, 48, 24));
    }

    #[test]
    fn splitting_off_the_right_works_the_same_way() {
        let (rest, right) = Rect::new(0, 0, 80, 24).split_right(20);
        assert_eq!(rest, Rect::new(0, 0, 60, 24));
        assert_eq!(right, Rect::new(60, 0, 20, 24));
    }

    #[test]
    fn asking_for_more_than_exists_takes_everything() {
        let r = Rect::new(0, 0, 80, 24);
        let (top, rest) = r.split_top(100);
        assert_eq!(top, r);
        assert!(rest.is_empty());
        let (left, rest) = r.split_left(100);
        assert_eq!(left, r);
        assert!(rest.is_empty());
    }

    #[test]
    fn even_splits_give_the_extra_cell_to_the_first_half() {
        let (left, right) = Rect::new(0, 0, 81, 24).split_horizontally();
        assert_eq!(left.width, 41);
        assert_eq!(right.width, 40);
        let (top, bottom) = Rect::new(0, 0, 80, 25).split_vertically();
        assert_eq!(top.height, 13);
        assert_eq!(bottom.height, 12);
    }

    #[test]
    fn splits_partition_the_original_exactly() {
        let r = Rect::new(4, 6, 37, 19);
        for (a, b) in [r.split_horizontally(), r.split_vertically()] {
            assert_eq!(a.area() + b.area(), r.area());
            assert_eq!(a.intersect(&b), None, "halves must not overlap");
        }
    }

    #[test]
    fn inset_shrinks_from_every_side_and_saturates() {
        assert_eq!(Rect::new(0, 0, 10, 10).inset(2), Rect::new(2, 2, 6, 6));
        assert!(Rect::new(0, 0, 3, 3).inset(5).is_empty());
    }

    #[test]
    fn cells_visits_every_position_row_by_row() {
        let r = Rect::new(1, 1, 2, 2);
        assert_eq!(r.cells().collect::<Vec<_>>(), vec![(1, 1), (2, 1), (1, 2), (2, 2)]);
        assert_eq!(r.cells().count(), r.area());
        assert_eq!(Rect::new(0, 0, 0, 5).cells().count(), 0);
    }

    #[test]
    fn a_rect_can_be_built_from_a_size() {
        assert_eq!(Rect::from_size(Size::new(80, 24)), Rect::new(0, 0, 80, 24));
        assert_eq!(Size::new(80, 24).area(), 1920);
        assert!(Size::new(0, 24).is_empty());
    }
}
