//! Timings on a large file, as a regression guard.
//!
//! Absolute times depend on the machine, so the assertions are *ratios*
//! between operations. That is what catches the failure mode that actually
//! happened: redisplay once rendered the whole buffer to a string for every
//! visible line when diagnostics were present, which made a frame take 1.67
//! seconds on this file instead of 361 microseconds. A ratio catches it on any
//! machine, in debug or release.
use maxgus_config::Settings;
use maxgus_core::{Dispatcher, Editor};
use maxgus_faces::defaults;
use maxgus_tui::{Rect, Size, Surface};
use std::time::Instant;

fn big_source(lines: usize) -> String {
    (0..lines)
        .map(|n| format!("fn function_{n}(argument: &str) -> usize {{ argument.len() + {n} }}\n"))
        .collect()
}

#[test]
fn operations_on_a_large_file_stay_responsive() {
    let text = big_source(20_000);
    println!("file: {} lines, {} bytes", text.lines().count(), text.len());

    let frame = Rect::new(0, 0, 100, 40);
    let mut editor =
        Editor::new(Settings::default(), defaults::builtin("maxgus-dark").unwrap(), frame);
    let registry = maxgus_core::standard_registry();
    editor.command_names = registry.interactive_names();
    let mut dispatcher = Dispatcher::new(registry);

    let start = Instant::now();
    let id = editor.buffers.visit_file("/project/big.rs", &text);
    editor.switch_to_buffer(id).unwrap();
    println!("open:              {:>8.2?}", start.elapsed());

    let mut surface = Surface::new(Size::new(100, 40));
    let start = Instant::now();
    for _ in 0..100 {
        maxgus_core::draw(&editor, &mut surface);
    }
    let plain_draw = start.elapsed() / 100;
    println!("draw (per frame):  {plain_draw:>8.2?}");

    editor.with_current_buffer(|b| b.set_point(0));
    let start = Instant::now();
    for _ in 0..1000 {
        dispatcher.execute(&mut editor, "next-line", None);
    }
    println!("next-line (each):  {:>8.2?}", start.elapsed() / 1000);

    let start = Instant::now();
    for _ in 0..200 {
        dispatcher.execute(&mut editor, "self-insert-command", Some(maxgus_keys::Key::char('x')));
    }
    println!("self-insert (each):{:>8.2?}", start.elapsed() / 200);

    // Incremental search: one re-search per character typed.
    editor.with_current_buffer(|b| b.set_point(0));
    dispatcher.execute(&mut editor, "isearch-forward", None);
    let start = Instant::now();
    for c in "function_19000".chars() {
        dispatcher.execute(&mut editor, "isearch-printing-char", Some(maxgus_keys::Key::char(c)));
    }
    println!("isearch late  (key):{:>8.2?}", start.elapsed() / 14);
    dispatcher.execute(&mut editor, "isearch-abort", None);

    // A match near the start: the regex stops scanning as soon as it finds one,
    // so this separates the scan cost from the per-keystroke overhead.
    editor.with_current_buffer(|b| b.set_point(0));
    dispatcher.execute(&mut editor, "isearch-forward", None);
    let start = Instant::now();
    for c in "function_2(".chars() {
        dispatcher.execute(&mut editor, "isearch-printing-char", Some(maxgus_keys::Key::char(c)));
    }
    println!("isearch early (key):{:>8.2?}", start.elapsed() / 11);
    dispatcher.execute(&mut editor, "isearch-abort", None);

    let start = Instant::now();
    editor.with_current_buffer(|b| {
        let end = b.len_chars();
        b.set_point(end);
    });
    println!("end-of-buffer:     {:>8.2?}", start.elapsed());

    // Drawing with diagnostics present is the case that matters: a language
    // server on a real project produces them constantly.
    let uri = maxgus_lsp::client::path_to_uri(std::path::Path::new("/project/big.rs"));
    let diagnostics: Vec<maxgus_lsp::Diagnostic> = (0..50)
        .map(|n| {
            maxgus_lsp::Diagnostic::new(
                maxgus_lsp::LspRange::new(
                    maxgus_lsp::LspPosition::new(n * 7, 3),
                    maxgus_lsp::LspPosition::new(n * 7, 11),
                ),
                maxgus_lsp::Severity::Warning,
                "something",
            )
        })
        .collect();
    editor.diagnostics.replace(uri, diagnostics);
    editor.with_current_buffer(|b| b.set_point(0));

    let start = Instant::now();
    for _ in 0..20 {
        maxgus_core::draw(&editor, &mut surface);
    }
    let with_diagnostics = start.elapsed() / 20;
    println!("draw + diagnostics:{with_diagnostics:>8.2?}");

    // Diagnostics are resolved once per window. If that ever goes back to
    // being per line, this ratio explodes: it was five thousand before.
    let ratio = with_diagnostics.as_secs_f64() / plain_draw.as_secs_f64();
    println!("ratio:             {ratio:>8.1}x");
    assert!(
        ratio < 20.0,
        "drawing with diagnostics is {ratio:.0}x slower than without; \
         they are probably being resolved per line again"
    );

    // A frame must stay far below a human noticing it, even unoptimised.
    let budget = if cfg!(debug_assertions) { 200 } else { 20 };
    assert!(
        with_diagnostics.as_millis() < budget,
        "a frame took {with_diagnostics:?}, over the {budget}ms budget"
    );
}
