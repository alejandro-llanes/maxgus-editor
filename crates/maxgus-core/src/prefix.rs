//! The universal argument.
//!
//! `C-u` alone means four, `C-u C-u` sixteen, and so on; `C-u 30` or `M-3 0`
//! means thirty; `C-u -` or `M--` means minus one. Commands read it through
//! [`Prefix::count`], and the few that care whether the user typed anything at
//! all — `C-u C-SPC`, for instance — read [`Prefix::is_raw`].

/// The prefix argument in effect for the command about to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Prefix {
    /// No argument was given; commands act once.
    #[default]
    None,
    /// `C-u` repeated `n` times, with no digits typed.
    Universal(u32),
    /// An explicit number, from digits or `C-u <digits>`.
    Numeric(i32),
    /// A bare minus sign, which Emacs treats as negative one but keeps
    /// distinct so digits typed after it extend it.
    Negative,
}

impl Prefix {
    /// The repeat count. `C-u` is four, `C-u C-u` sixteen, and so on.
    pub fn count(self) -> i32 {
        match self {
            Prefix::None => 1,
            Prefix::Universal(n) => 4i32.saturating_pow(n),
            Prefix::Numeric(n) => n,
            Prefix::Negative => -1,
        }
    }

    /// The count clamped to at least zero, for commands that cannot run a
    /// negative number of times.
    pub fn positive_count(self) -> usize {
        self.count().max(0) as usize
    }

    /// True when the user supplied any argument at all.
    pub fn is_present(self) -> bool {
        !matches!(self, Prefix::None)
    }

    /// True when the argument was `C-u` with no digits, which some commands
    /// treat as a mode switch rather than a count.
    pub fn is_raw(self) -> bool {
        matches!(self, Prefix::Universal(_))
    }

    /// True when the argument is negative, as `M--` gives.
    pub fn is_negative(self) -> bool {
        self.count() < 0
    }

    /// Applies another `C-u`.
    pub fn universal(self) -> Prefix {
        match self {
            Prefix::Universal(n) => Prefix::Universal(n + 1),
            // `C-u` after digits restarts the multiplier.
            _ => Prefix::Universal(1),
        }
    }

    /// Applies a minus sign. A second one cancels back to no argument, which
    /// is what Emacs does.
    pub fn minus(self) -> Prefix {
        match self {
            Prefix::Negative => Prefix::None,
            Prefix::Numeric(n) => Prefix::Numeric(-n),
            _ => Prefix::Negative,
        }
    }

    /// Appends a digit, extending a number already being typed.
    pub fn digit(self, digit: u32) -> Prefix {
        let digit = (digit % 10) as i32;
        match self {
            // Digits after `C-u` replace the multiplier rather than extend it.
            Prefix::None | Prefix::Universal(_) => Prefix::Numeric(digit),
            Prefix::Negative => Prefix::Numeric(-digit),
            Prefix::Numeric(n) if n < 0 => {
                Prefix::Numeric(n.saturating_mul(10).saturating_sub(digit))
            }
            Prefix::Numeric(n) => Prefix::Numeric(n.saturating_mul(10).saturating_add(digit)),
        }
    }

    /// How the argument is echoed in the minibuffer while it is being typed.
    pub fn echo(self) -> String {
        match self {
            Prefix::None => String::new(),
            Prefix::Universal(n) => format!("C-u{} ", " C-u".repeat(n as usize - 1)),
            Prefix::Numeric(n) => format!("C-u {n} "),
            Prefix::Negative => "C-u - ".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_argument_means_once() {
        let p = Prefix::None;
        assert_eq!(p.count(), 1);
        assert!(!p.is_present());
        assert!(!p.is_raw());
        assert_eq!(p.echo(), "");
    }

    #[test]
    fn repeated_universals_multiply_by_four() {
        let one = Prefix::None.universal();
        assert_eq!(one, Prefix::Universal(1));
        assert_eq!(one.count(), 4);
        assert_eq!(one.universal().count(), 16);
        assert_eq!(one.universal().universal().count(), 64);
        assert!(one.is_raw());
        assert!(one.is_present());
    }

    #[test]
    fn digits_accumulate_into_a_number() {
        let p = Prefix::None.digit(3).digit(0);
        assert_eq!(p, Prefix::Numeric(30));
        assert_eq!(p.count(), 30);
        assert!(!p.is_raw(), "an explicit count is not a raw C-u");
    }

    #[test]
    fn digits_after_a_universal_replace_the_multiplier() {
        // `C-u 5` is five, not twenty.
        let p = Prefix::None.universal().digit(5);
        assert_eq!(p.count(), 5);
    }

    #[test]
    fn a_universal_after_digits_starts_over() {
        let p = Prefix::None.digit(7).universal();
        assert_eq!(p, Prefix::Universal(1));
        assert_eq!(p.count(), 4);
    }

    #[test]
    fn a_minus_sign_gives_negative_one_and_extends_with_digits() {
        let minus = Prefix::None.minus();
        assert_eq!(minus.count(), -1);
        assert!(minus.is_negative());
        assert_eq!(minus.digit(5).count(), -5);
        assert_eq!(minus.digit(1).digit(2).count(), -12);
    }

    #[test]
    fn a_second_minus_cancels_the_argument() {
        assert_eq!(Prefix::None.minus().minus(), Prefix::None);
    }

    #[test]
    fn a_minus_after_digits_negates_them() {
        assert_eq!(Prefix::None.digit(4).minus().count(), -4);
    }

    #[test]
    fn positive_count_floors_at_zero() {
        assert_eq!(Prefix::None.minus().positive_count(), 0);
        assert_eq!(Prefix::Numeric(-9).positive_count(), 0);
        assert_eq!(Prefix::Numeric(9).positive_count(), 9);
        assert_eq!(Prefix::None.positive_count(), 1);
    }

    #[test]
    fn counts_saturate_rather_than_overflowing() {
        let mut p = Prefix::Numeric(1);
        for _ in 0..20 {
            p = p.digit(9);
        }
        assert_eq!(p.count(), i32::MAX);
        assert_eq!(Prefix::Universal(64).count(), i32::MAX);
    }

    #[test]
    fn the_echo_shows_what_has_been_typed() {
        assert_eq!(Prefix::None.universal().echo(), "C-u ");
        assert_eq!(Prefix::None.universal().universal().echo(), "C-u C-u ");
        assert_eq!(Prefix::None.digit(1).digit(2).echo(), "C-u 12 ");
        assert_eq!(Prefix::None.minus().echo(), "C-u - ");
    }

    #[test]
    fn a_zero_digit_is_accepted() {
        assert_eq!(Prefix::None.digit(0).count(), 0);
        assert_eq!(Prefix::None.digit(1).digit(0).count(), 10);
    }
}
