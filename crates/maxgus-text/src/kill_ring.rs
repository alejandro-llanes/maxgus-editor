//! The kill ring.
//!
//! A bounded ring of killed text with a yank pointer, following the Emacs
//! model: `kill-new` pushes, `kill-append` extends the head when kills are
//! consecutive, `yank` inserts the entry under the pointer, and `yank-pop`
//! rotates the pointer backwards through the history.

/// Emacs' default `kill-ring-max`.
pub const DEFAULT_KILL_RING_MAX: usize = 120;

#[derive(Debug, Clone)]
pub struct KillRing {
    /// Newest kill first.
    entries: Vec<String>,
    /// Index into `entries` that `yank` will insert.
    yank_pointer: usize,
    max: usize,
}

impl Default for KillRing {
    fn default() -> Self {
        Self::new(DEFAULT_KILL_RING_MAX)
    }
}

impl KillRing {
    pub fn new(max: usize) -> Self {
        Self {
            entries: Vec::new(),
            yank_pointer: 0,
            max: max.max(1),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `kill-new`: pushes `text` as the newest entry and resets the yank
    /// pointer to it. Empty strings are ignored, as in Emacs.
    pub fn kill_new(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        self.entries.insert(0, text);
        self.entries.truncate(self.max);
        self.yank_pointer = 0;
    }

    /// `kill-append`: extends the newest entry instead of pushing. `before` is
    /// true for backward kills such as `backward-kill-word`, which prepend.
    pub fn kill_append(&mut self, text: impl AsRef<str>, before: bool) {
        let text = text.as_ref();
        if text.is_empty() {
            return;
        }
        match self.entries.first_mut() {
            Some(head) => {
                if before {
                    head.insert_str(0, text);
                } else {
                    head.push_str(text);
                }
                self.yank_pointer = 0;
            }
            None => self.kill_new(text),
        }
    }

    /// The entry `yank` would insert.
    pub fn front(&self) -> Option<&str> {
        self.entries.get(self.yank_pointer).map(String::as_str)
    }

    /// `current-kill`: rotates the yank pointer by `n` and returns the entry it
    /// lands on. Positive `n` moves towards older kills, wrapping around.
    pub fn rotate(&mut self, n: isize) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        let len = self.entries.len() as isize;
        let idx = (self.yank_pointer as isize + n).rem_euclid(len);
        self.yank_pointer = idx as usize;
        self.front()
    }

    /// Resets the yank pointer to the newest entry without modifying the ring.
    pub fn reset_pointer(&mut self) {
        self.yank_pointer = 0;
    }

    /// Iterates from the yank pointer towards older entries, wrapping.
    pub fn iter_from_pointer(&self) -> impl Iterator<Item = &str> {
        let len = self.entries.len();
        (0..len).map(move |i| self.entries[(self.yank_pointer + i) % len].as_str())
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.yank_pointer = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_new_pushes_to_the_front() {
        let mut r = KillRing::default();
        r.kill_new("one");
        r.kill_new("two");
        assert_eq!(r.front(), Some("two"));
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn empty_kills_are_ignored() {
        let mut r = KillRing::default();
        r.kill_new("");
        r.kill_append("", false);
        assert!(r.is_empty());
    }

    #[test]
    fn kill_append_extends_in_both_directions() {
        let mut r = KillRing::default();
        r.kill_new("middle");
        r.kill_append(" end", false);
        r.kill_append("start ", true);
        assert_eq!(r.front(), Some("start middle end"));
        assert_eq!(r.len(), 1, "appending never grows the ring");
    }

    #[test]
    fn kill_append_on_empty_ring_behaves_like_kill_new() {
        let mut r = KillRing::default();
        r.kill_append("text", false);
        assert_eq!(r.front(), Some("text"));
    }

    #[test]
    fn yank_pop_rotates_backwards_and_wraps() {
        let mut r = KillRing::default();
        r.kill_new("a");
        r.kill_new("b");
        r.kill_new("c");
        assert_eq!(r.front(), Some("c"));
        assert_eq!(r.rotate(1), Some("b"));
        assert_eq!(r.rotate(1), Some("a"));
        assert_eq!(r.rotate(1), Some("c"), "wraps around to the newest entry");
        assert_eq!(
            r.rotate(-1),
            Some("a"),
            "negative rotation walks the other way"
        );
    }

    #[test]
    fn rotate_on_empty_ring_yields_nothing() {
        let mut r = KillRing::default();
        assert_eq!(r.rotate(1), None);
    }

    #[test]
    fn ring_is_bounded_by_max() {
        let mut r = KillRing::new(2);
        r.kill_new("a");
        r.kill_new("b");
        r.kill_new("c");
        assert_eq!(r.len(), 2);
        assert_eq!(r.iter_from_pointer().collect::<Vec<_>>(), vec!["c", "b"]);
    }

    #[test]
    fn kill_new_resets_the_yank_pointer() {
        let mut r = KillRing::default();
        r.kill_new("a");
        r.kill_new("b");
        r.rotate(1);
        assert_eq!(r.front(), Some("a"));
        r.kill_new("c");
        assert_eq!(r.front(), Some("c"));
    }
}
