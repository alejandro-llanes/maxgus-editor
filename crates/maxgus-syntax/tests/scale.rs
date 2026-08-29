//! How highlighting behaves on a file large enough to matter.

use maxgus_syntax::{Highlighter, InputEdit};
use std::time::Instant;

fn source(lines: usize) -> String {
    (0..lines)
        .map(|n| format!("fn function_{n}(argument: &str) -> usize {{ argument.len() + {n} }}\n"))
        .collect()
}

/// A byte index at or after `at` that sits on a character boundary.
fn boundary(text: &str, at: usize) -> usize {
    (at..text.len()).find(|i| text.is_char_boundary(*i)).unwrap_or(text.len())
}

#[test]
fn an_incremental_parse_is_far_cheaper_than_a_full_one() {
    let text = source(20_000);
    println!("file: {} lines, {} bytes", text.lines().count(), text.len());

    let mut highlighter = Highlighter::new("rust").unwrap();
    let start = Instant::now();
    highlighter.parse(&text).unwrap();
    let full = start.elapsed();
    println!("full parse:        {full:>8.2?}");

    // One character typed in the middle, the way editing actually happens.
    let at = boundary(&text, text.len() / 2);
    let mut edited = text.clone();
    edited.insert(at, 'x');

    let start = Instant::now();
    highlighter.edit(InputEdit::insertion(at, 1), &text, &edited);
    highlighter.parse(&edited).unwrap();
    let incremental = start.elapsed();
    println!("after one edit:    {incremental:>8.2?}");

    let start = Instant::now();
    let spans = highlighter.highlights_in(&edited, 0..4_000);
    println!("highlight a screen:{:>8.2?}", start.elapsed());
    assert!(!spans.is_empty());

    let ratio = full.as_secs_f64() / incremental.as_secs_f64().max(1e-9);
    println!("cheaper by:        {ratio:>8.1}x");
    assert!(
        ratio > 5.0,
        "an incremental parse was only {ratio:.1}x cheaper than a full one; \
         is the tree being reused?"
    );
}

#[test]
fn resetting_throws_away_work_a_reparse_could_have_reused() {
    let text = source(5_000);
    let mut highlighter = Highlighter::new("rust").unwrap();
    highlighter.parse(&text).unwrap();

    let at = boundary(&text, text.len() / 2);
    let mut edited = text.clone();
    edited.insert(at, 'x');

    // With the tree kept and told about the edit.
    let start = Instant::now();
    highlighter.edit(InputEdit::insertion(at, 1), &text, &edited);
    highlighter.parse(&edited).unwrap();
    let reused = start.elapsed();

    // With the tree thrown away first, which is what `reset` does.
    highlighter.reset();
    let start = Instant::now();
    highlighter.parse(&edited).unwrap();
    let from_scratch = start.elapsed();

    println!("reusing the tree:  {reused:>8.2?}");
    println!("from scratch:      {from_scratch:>8.2?}");
    assert!(
        reused < from_scratch,
        "reusing the tree ({reused:?}) was not cheaper than starting over ({from_scratch:?})"
    );
}
