//! Fuzzy matching for the completion prompts.
//!
//! A query matches when its characters appear in the candidate in order, not
//! necessarily together: `mkbuf` finds `make-buffer-list`. Matching is
//! case-insensitive until the query has an uppercase letter in it, which is
//! the same smart-case rule search uses.
//!
//! Candidates are ranked rather than merely filtered, because with a
//! subsequence match almost everything matches a short query and the order is
//! then the only thing that makes the list useful.

/// How well `query` matches `candidate`, or `None` when it does not.
///
/// Higher is better. The scale has no meaning beyond comparing two candidates
/// against the same query.
pub fn score(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        // Everything matches nothing, and the order is then the caller's.
        return Some(0);
    }
    let fold = !query.chars().any(char::is_uppercase);
    let text: Vec<char> = candidate.chars().collect();
    let wanted: Vec<char> = query.chars().collect();

    let mut score = 0;
    let mut at = 0usize;
    let mut previous: Option<usize> = None;

    for &c in &wanted {
        let found = text[at..].iter().position(|&t| same(t, c, fold))? + at;

        // A match at the start of a word is what a person is usually aiming
        // at: the `b` of `-buffer` rather than the `b` inside `about`.
        let boundary = found == 0
            || text
                .get(found - 1)
                .is_some_and(|&p| matches!(p, '-' | '_' | '/' | '.' | ' ' | ':'));
        if boundary {
            score += 16;
        }
        // The very first character is the strongest anchor of all. Without
        // this, `stb` prefers `list-buffers` — whose `st` happens to sit
        // together inside `list` — over `switch-to-buffer`, whose letters are
        // the initials someone typing `stb` had in mind.
        if found == 0 {
            score += 14;
        }
        // Letters that follow one another are the strongest signal there is —
        // worth more than a word start, or `buf` would rank `back-up-file`
        // above `buffer-menu` on three boundary bonuses.
        match previous {
            Some(p) if found == p + 1 => score += 20,
            // A long jump is weak evidence, but never worth more than the
            // gap costs, or a distant match could outscore a near one.
            Some(p) => score -= ((found - p - 1) as i32).min(12),
            None => score -= (found as i32).min(12),
        }
        previous = Some(found);
        at = found + 1;
    }

    // Between two candidates that match equally well, the shorter one is more
    // likely to be the one meant.
    score -= (text.len() as i32) / 8;
    Some(score)
}

fn same(a: char, b: char, fold: bool) -> bool {
    match fold {
        true => a.eq_ignore_ascii_case(&b) || a.to_lowercase().eq(b.to_lowercase()),
        false => a == b,
    }
}

/// The candidates `query` matches, best first.
///
/// Ties keep the order they came in, so an unfiltered list stays in whatever
/// order the caller built it.
pub fn matches<'a>(query: &str, candidates: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut scored: Vec<(i32, usize, &'a String)> = candidates
        .enumerate()
        .filter_map(|(i, c)| score(query, c).map(|s| (s, i, c)))
        .collect();
    // Descending by score, then by original position.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, c)| c.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn best(query: &str, candidates: &[&str]) -> Vec<String> {
        let owned: Vec<String> = candidates.iter().map(|c| (*c).to_string()).collect();
        matches(query, owned.iter())
    }

    #[test]
    fn a_query_matches_letters_in_order_but_not_together() {
        assert!(score("sbuf", "switch-to-buffer").is_some());
        assert!(score("stb", "switch-to-buffer").is_some());
    }

    #[test]
    fn letters_out_of_order_do_not_match() {
        assert!(score("fub", "switch-to-buffer").is_none());
    }

    #[test]
    fn a_letter_that_is_not_there_at_all_does_not_match() {
        assert!(score("zzz", "switch-to-buffer").is_none());
    }

    #[test]
    fn an_empty_query_matches_everything() {
        assert_eq!(score("", "anything"), Some(0));
        assert_eq!(best("", &["a", "b"]), vec!["a", "b"], "and keeps the order given");
    }

    #[test]
    fn word_starts_beat_letters_buried_in_the_middle() {
        // `stb` as the initials of the words, against the same letters found
        // anywhere.
        let boundaries = score("stb", "switch-to-buffer").expect("matches");
        let buried = score("stb", "constants-table").unwrap_or(i32::MIN);
        assert!(boundaries > buried, "{boundaries} should beat {buried}");
    }

    #[test]
    fn a_run_of_letters_beats_the_same_letters_scattered() {
        let together = score("buf", "buffer-menu").expect("matches");
        let scattered = score("buf", "back-up-file").expect("matches");
        assert!(together > scattered, "{together} should beat {scattered}");
    }

    #[test]
    fn the_shorter_of_two_equal_matches_comes_first() {
        let ranked = best("sb", &["save-buffer-and-do-something-else-entirely", "save-buffer"]);
        assert_eq!(ranked[0], "save-buffer");
    }

    #[test]
    fn matching_folds_case_until_the_query_has_some() {
        assert!(score("abc", "ABC").is_some(), "a lowercase query folds");
        assert!(score("ABC", "abc").is_none(), "an uppercase one does not");
        assert!(score("ABC", "ABC").is_some());
    }

    #[test]
    fn the_obvious_command_comes_first_for_its_obvious_query() {
        let commands = [
            "save-some-buffers",
            "save-buffer",
            "switch-to-buffer",
            "save-buffers-kill-terminal",
            "list-buffers",
        ];
        assert_eq!(best("savebuf", &commands)[0], "save-buffer");
        assert_eq!(best("stb", &commands)[0], "switch-to-buffer");
    }

    #[test]
    fn candidates_that_do_not_match_are_left_out_entirely() {
        let ranked = best("xyz", &["save-buffer", "switch-to-buffer"]);
        assert!(ranked.is_empty(), "got {ranked:?}");
    }
}
