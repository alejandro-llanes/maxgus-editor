//! Language-server commands.
//!
//! Like the file commands, none of these talk to a server directly: each works
//! out what to ask and queues an [`LspQuery`]. The event loop sends it, and the
//! reply comes back through `Editor::apply_lsp_response`, which is where the
//! protocol's JSON is turned into editor state.

use crate::{
    MinibufferKind, Result, command,
    command::{Args, Registry},
    editor::Editor,
    task::{LspQuery, Task},
};
use maxgus_lsp::{LspPosition, LspRange, PositionEncoding};

/// The buffer definitions, references and symbols are listed into.
pub const XREF_NAME: &str = "*xref*";

/// Registers the language-server commands.
pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "lsp-find-definition",
            "Go to the definition of the symbol at point.",
            find_definition
        ),
        command!(
            "lsp-find-references",
            "List the references to the symbol at point.",
            find_references
        ),
        command!(
            "lsp-describe-thing-at-point",
            "Describe the symbol at point.",
            describe_thing
        ),
        command!(
            "lsp-rename",
            "Rename the symbol at point everywhere.",
            rename
        ),
        command!("lsp-format-buffer", "Reformat this buffer.", format_buffer),
        command!(
            "lsp-code-action",
            "Offer the server's fixes for this line.",
            code_action
        ),
        command!(
            "lsp-signature-help",
            "Show the signature of the call around point.",
            signature_help
        ),
        command!(
            "lsp-workspace-symbol",
            "Find a symbol anywhere in the project.",
            workspace_symbol
        ),
        command!(
            "lsp-document-symbols",
            "List the symbols in this buffer.",
            document_symbols
        ),
        command!(
            "lsp-restart-server",
            "Stop and restart the language server.",
            restart_server
        ),
        command!(
            "completion-at-point",
            "Complete the symbol at point.",
            completion_at_point
        ),
        command!("next-error", "Go to the next diagnostic.", next_error),
        command!(
            "previous-error",
            "Go to the previous diagnostic.",
            previous_error
        ),
    ]);
}

/// The language and document URI of the current buffer, or an error explaining
/// why there is no server to ask.
fn document(editor: &mut Editor) -> Result<(String, String)> {
    if !editor.settings.lsp_enabled {
        return Err(crate::CoreError::Message(
            "Language server support is off".into(),
        ));
    }
    editor.sync_to_buffer();
    let buffer = editor.current_buffer();
    let Some(language) = buffer.language().map(str::to_string) else {
        return Err(crate::CoreError::Message("Buffer has no language".into()));
    };
    let Some(path) = buffer.path() else {
        return Err(crate::CoreError::Message(
            "Buffer is not visiting a file".into(),
        ));
    };
    Ok((language, maxgus_lsp::client::path_to_uri(path)))
}

/// Point as the server counts it.
pub fn point_position(editor: &Editor, encoding: PositionEncoding) -> LspPosition {
    let buffer = editor.current_buffer();
    crate::position::position_of_offset(buffer, buffer.point(), encoding)
}

/// Queues `query` against the current document.
fn ask(editor: &mut Editor, query: LspQuery) -> Result<()> {
    let (language, uri) = document(editor)?;
    editor.message(format!("Language server: {}...", query.description()));
    editor.spawn(Task::LspRequest {
        language,
        uri,
        query,
        announced: true,
    });
    Ok(())
}

/// The encoding the server for the current buffer negotiated.
fn encoding(editor: &Editor) -> PositionEncoding {
    editor
        .current_buffer()
        .language()
        .and_then(|language| editor.lsp_encodings.iter().find(|(l, _)| l == language))
        .map(|(_, e)| *e)
        .unwrap_or_default()
}

fn find_definition(editor: &mut Editor, _: &Args) -> Result<()> {
    let position = point_position(editor, encoding(editor));
    ask(editor, LspQuery::Definition(position))
}

fn find_references(editor: &mut Editor, _: &Args) -> Result<()> {
    let position = point_position(editor, encoding(editor));
    ask(editor, LspQuery::References(position))
}

fn signature_help(editor: &mut Editor, _: &Args) -> Result<()> {
    let position = point_position(editor, encoding(editor));
    ask(editor, LspQuery::SignatureHelp(position))
}

/// Asks what the symbol under point is, without saying so in the echo area.
///
/// The idle path into the same question `C-c c k` asks out loud: nobody
/// pressed anything, so nothing should be announced, and an answer that
/// never comes should leave no trace.
pub fn ask_for_doc(editor: &mut Editor) {
    let position = point_position(editor, encoding(editor));
    let Ok((language, uri)) = document(editor) else {
        return;
    };
    editor.spawn(Task::LspRequest {
        language,
        uri,
        // Nobody asked out loud, so nothing is said if nobody answers.
        query: LspQuery::Hover(position),
        announced: false,
    });
}

fn describe_thing(editor: &mut Editor, _: &Args) -> Result<()> {
    // Asked for out loud, so an answer of "nothing" is worth saying.
    editor.doc_asked_at = None;
    let position = point_position(editor, encoding(editor));
    ask(editor, LspQuery::Hover(position))
}

fn completion_at_point(editor: &mut Editor, _: &Args) -> Result<()> {
    let position = point_position(editor, encoding(editor));
    ask(editor, LspQuery::Completion(position))
}

fn rename(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(new_name) = args.input.clone() else {
        // The symbol under point is offered as the starting point.
        let current = symbol_at_point(editor);
        editor.prompt_for(
            "lsp-rename",
            MinibufferKind::Text,
            "Rename to: ",
            &current,
            Vec::new(),
        );
        return Ok(());
    };
    if new_name.trim().is_empty() {
        return Err(crate::CoreError::Message("No new name given".into()));
    }
    let position = point_position(editor, encoding(editor));
    ask(editor, LspQuery::Rename { position, new_name })
}

/// The identifier point is on, for pre-filling the rename prompt.
fn symbol_at_point(editor: &Editor) -> String {
    let buffer = editor.current_buffer();
    match maxgus_text::Motion::word_bounds(buffer.rope(), buffer.point()) {
        Some((start, end)) => buffer.slice(maxgus_text::Range::new(start, end)),
        None => String::new(),
    }
}

fn format_buffer(editor: &mut Editor, _: &Args) -> Result<()> {
    let (tab_size, insert_spaces) = (editor.settings.tab_width, !editor.settings.indent_with_tabs);
    ask(
        editor,
        LspQuery::Format {
            tab_size,
            insert_spaces,
        },
    )
}

fn code_action(editor: &mut Editor, _: &Args) -> Result<()> {
    let encoding = encoding(editor);
    // The range is the region when there is one, otherwise the line.
    let range = match editor.region() {
        Ok(region) => {
            let buffer = editor.current_buffer();
            LspRange::new(
                crate::position::position_of_offset(buffer, region.start, encoding),
                crate::position::position_of_offset(buffer, region.end, encoding),
            )
        }
        Err(_) => LspRange::empty(point_position(editor, encoding)),
    };
    // The diagnostics inside the range go with the request; without them a
    // server has nothing to offer a quick fix for.
    let diagnostics = match editor.current_buffer().path() {
        Some(path) => {
            let uri = maxgus_lsp::client::path_to_uri(path);
            editor
                .diagnostics
                .for_uri(&uri)
                .iter()
                .filter(|d| d.range.start <= range.end && range.start <= d.range.end)
                .cloned()
                .collect()
        }
        None => Vec::new(),
    };
    ask(editor, LspQuery::CodeAction { range, diagnostics })
}

fn document_symbols(editor: &mut Editor, _: &Args) -> Result<()> {
    ask(editor, LspQuery::DocumentSymbols { for_panel: false })
}

fn workspace_symbol(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(query) = args.input.clone() else {
        editor.prompt_for(
            "lsp-workspace-symbol",
            MinibufferKind::Text,
            "Find symbol: ",
            &symbol_at_point(editor),
            Vec::new(),
        );
        return Ok(());
    };
    ask(editor, LspQuery::WorkspaceSymbols(query))
}

fn restart_server(editor: &mut Editor, _: &Args) -> Result<()> {
    let (language, _) = document(editor)?;
    editor.spawn(Task::StopLanguageServer {
        language: language.clone(),
    });
    editor.spawn(Task::StartLanguageServer {
        language: language.clone(),
    });
    let id = editor.current_buffer_id();
    editor.request_language_server(id);
    editor.message(format!("Restarting the {language} language server"));
    Ok(())
}

// ---- diagnostics --------------------------------------------------------

/// Moves to the next or previous diagnostic in this buffer.
fn step_error(editor: &mut Editor, forward: bool) -> Result<()> {
    let Some(path) = editor.current_buffer().path() else {
        return Err(crate::CoreError::Message(
            "Buffer is not visiting a file".into(),
        ));
    };
    let uri = maxgus_lsp::client::path_to_uri(path);
    let encoding = encoding(editor);
    let here = point_position(editor, encoding);

    let found = if forward {
        editor.diagnostics.next_after(&uri, here).cloned()
    } else {
        editor.diagnostics.previous_before(&uri, here).cloned()
    };
    let Some(diagnostic) = found else {
        return Err(crate::CoreError::Message(
            if forward {
                "No further diagnostics"
            } else {
                "No previous diagnostics"
            }
            .into(),
        ));
    };
    let offset = crate::position::offset_of_position(
        editor.current_buffer(),
        diagnostic.range.start,
        encoding,
    );
    editor.with_current_buffer(|b| b.set_point(offset));
    editor.follow_point();
    editor.message(diagnostic.summary());
    Ok(())
}

fn next_error(editor: &mut Editor, _: &Args) -> Result<()> {
    step_error(editor, true)
}

fn previous_error(editor: &mut Editor, _: &Args) -> Result<()> {
    step_error(editor, false)
}

// ---- applying responses -------------------------------------------------

/// Folds a language server's answer into editor state.
///
/// The protocol allows several shapes for most replies — a definition may be
/// one location, a list of them, or a list of links — so each is accepted
/// rather than insisting on the one a particular server happens to send.
pub fn apply_response(editor: &mut Editor, query: &LspQuery, result: &serde_json::Value) {
    match query {
        LspQuery::Definition(_) => apply_definition(editor, result),
        LspQuery::References(_) => list_locations(editor, "References", result),
        LspQuery::Hover(_) => apply_hover(editor, result),
        LspQuery::Completion(_) => apply_completion(editor, result),
        LspQuery::SignatureHelp(_) => apply_signature_help(editor, result),
        LspQuery::Rename { .. } => {
            apply_workspace_edit(editor, result);
        }
        LspQuery::Format { .. } => apply_text_edits(editor, result.as_array().map(Vec::as_slice)),
        LspQuery::CodeAction { .. } => list_code_actions(editor, result),
        LspQuery::DocumentSymbols { for_panel } => {
            // Both askers get their answer: the outline is filed either way,
            // and the listing buffer opens only for the request a person
            // made. Reading `for_panel` rather than the editor's pending flag
            // is what keeps a second answer from popping a listing over the
            // file, once the first has already cleared the flag.
            if let Some(buffer) = editor.panel.symbols_buffer {
                editor
                    .panel
                    .set_symbols(buffer, crate::panel::symbols_from_lsp(result));
                editor.render_panel_buffer();
            }
            if !*for_panel {
                list_symbols(editor, "Document symbols", result);
            }
        }
        LspQuery::WorkspaceSymbols(_) => list_symbols(editor, "Workspace symbols", result),
    }
}

/// One location, however the server chose to express it.
fn parse_location(value: &serde_json::Value) -> Option<(String, LspPosition)> {
    let object = value.as_object()?;
    // A `LocationLink` names the target differently from a `Location`.
    let uri = object
        .get("uri")
        .or_else(|| object.get("targetUri"))
        .and_then(|v| v.as_str())?
        .to_string();
    let range = object
        .get("range")
        .or_else(|| object.get("targetSelectionRange"))
        .or_else(|| object.get("targetRange"))?;
    let start = range.get("start")?;
    Some((
        uri,
        LspPosition::new(
            start.get("line")?.as_u64()? as u32,
            start.get("character")?.as_u64()? as u32,
        ),
    ))
}

/// Every location in a reply that may be one, several, or none.
fn parse_locations(value: &serde_json::Value) -> Vec<(String, LspPosition)> {
    match value {
        serde_json::Value::Array(items) => items.iter().filter_map(parse_location).collect(),
        serde_json::Value::Null => Vec::new(),
        single => parse_location(single).into_iter().collect(),
    }
}

fn apply_definition(editor: &mut Editor, result: &serde_json::Value) {
    let locations = parse_locations(result);
    let Some((uri, position)) = locations.first().cloned() else {
        editor.error("No definition found");
        return;
    };
    // More than one definition is worth listing rather than picking from.
    if locations.len() > 1 {
        list_locations(editor, "Definitions", result);
        return;
    }
    jump_to(editor, &uri, position);
}

/// Moves point to `position` in `uri`, opening the file when it is not already
/// in a buffer.
fn jump_to(editor: &mut Editor, uri: &str, position: LspPosition) {
    let Some(path) = maxgus_lsp::client::uri_to_path(uri) else {
        editor.error(format!("Cannot open `{uri}`"));
        return;
    };
    // The place being left goes on the mark ring, so `M-,` comes back.
    editor.with_current_buffer(|b| {
        let from = b.point();
        b.push_mark(from);
    });
    match editor.buffers.find_by_path(&path) {
        Some(id) => {
            if editor.switch_to_buffer(id).is_err() {
                return;
            }
            let encoding = encoding(editor);
            let offset =
                crate::position::offset_of_position(editor.current_buffer(), position, encoding);
            editor.with_current_buffer(|b| b.set_point(offset));
            editor.follow_point();
            // The "finding definition..." message has to be replaced, or it
            // sits there afterwards suggesting the request never finished.
            let name = editor.current_buffer().name().to_string();
            editor.message(format!("{name}:{}  (M-, to come back)", position.line + 1));
        }
        None => {
            // The file has to be read first; point is set once it arrives.
            editor.pending_jump = Some((path.clone(), position));
            editor.spawn(Task::ReadFile {
                path,
                reverting: None,
                other_window: false,
            });
        }
    }
}

fn apply_hover(editor: &mut Editor, result: &serde_json::Value) {
    let Some(text) = hover_text(result) else {
        // An idle pause over a symbol nothing is known about should say
        // nothing; only someone who asked deserves an answer either way.
        editor.doc = None;
        if editor.doc_asked_at.is_none() {
            editor.error("Nothing to describe here");
        }
        return;
    };
    // A box beside the symbol rather than a window over the code: what
    // `lsp-ui-doc` does, and what makes reading a type worth the keystroke.
    let window = editor.windows.current_id();
    let line = {
        let point = editor.windows.current().point;
        editor.current_buffer().line_of(point)
    };
    editor.doc = Some(crate::Doc { text, line, window });
    // The "Language server: describing..." message has done its job.
    editor.message(String::new());
}

/// The plain text of a hover reply, in any of the shapes the protocol allows.
fn hover_text(result: &serde_json::Value) -> Option<String> {
    let contents = result.get("contents")?;
    let text = match contents {
        serde_json::Value::String(s) => s.clone(),
        // `MarkupContent`.
        serde_json::Value::Object(o) => o.get("value")?.as_str()?.to_string(),
        // A list of strings or marked-up strings.
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Object(o) => {
                    o.get("value").and_then(|v| v.as_str()).map(str::to_string)
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let trimmed = text.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn apply_completion(editor: &mut Editor, result: &serde_json::Value) {
    // The reply is either a list or an object holding one.
    let items = result
        .get("items")
        .and_then(|v| v.as_array())
        .or_else(|| result.as_array())
        .cloned()
        .unwrap_or_default();
    let labels: Vec<String> = items
        .iter()
        .filter_map(|item| {
            // `insertText` is what to type; `label` is what to show.
            item.get("insertText")
                .or_else(|| item.get("label"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect();
    if labels.is_empty() {
        editor.error("No completions");
        return;
    }
    let prefix = symbol_at_point(editor);
    let matching: Vec<String> = labels
        .into_iter()
        .filter(|l| l.starts_with(&prefix))
        .collect();
    if matching.is_empty() {
        editor.error("No matching completions");
        return;
    }
    // One candidate is inserted outright; several are offered.
    if matching.len() == 1 {
        insert_completion(editor, &prefix, &matching[0]);
        editor.message(format!("Completed to `{}`", matching[0]));
        return;
    }
    let common = longest_common_prefix(&matching);
    if common.len() > prefix.len() {
        insert_completion(editor, &prefix, &common);
    }
    editor.message(format!(
        "{} completions: {}",
        matching.len(),
        preview(&matching)
    ));
    editor.completion_candidates = matching;
}

/// Replaces the partial symbol at point with `completion`.
fn insert_completion(editor: &mut Editor, prefix: &str, completion: &str) {
    let remainder = completion.strip_prefix(prefix).unwrap_or(completion);
    if remainder.is_empty() {
        return;
    }
    if editor
        .with_current_buffer(|b| b.insert_at_point(remainder))
        .is_err()
    {
        editor.error("Buffer is read-only");
        return;
    }
    editor.follow_point();
}

/// The first few of a list, for a one-line message.
fn preview(items: &[String]) -> String {
    let shown: Vec<&str> = items.iter().take(5).map(String::as_str).collect();
    let mut text = shown.join(", ");
    if items.len() > shown.len() {
        text.push_str(", ...");
    }
    text
}

fn longest_common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else {
        return String::new();
    };
    let mut prefix: Vec<char> = first.chars().collect();
    for item in &items[1..] {
        let shared = prefix
            .iter()
            .zip(item.chars())
            .take_while(|(a, b)| **a == *b)
            .count();
        prefix.truncate(shared);
    }
    prefix.into_iter().collect()
}

fn apply_signature_help(editor: &mut Editor, result: &serde_json::Value) {
    let Some(label) = result
        .get("signatures")
        .and_then(|v| v.as_array())
        .and_then(|list| list.first())
        .and_then(|s| s.get("label"))
        .and_then(|v| v.as_str())
    else {
        editor.error("No signature help");
        return;
    };
    editor.message(label.to_string());
}

/// Applies a list of `TextEdit`s to the current buffer.
fn apply_text_edits(editor: &mut Editor, edits: Option<&[serde_json::Value]>) {
    let Some(edits) = edits else {
        editor.message("Nothing to change");
        return;
    };
    if edits.is_empty() {
        editor.message("Nothing to change");
        return;
    }
    let encoding = encoding(editor);
    let buffer = editor.current_buffer();

    // Edits are expressed against the original text, so they are applied from
    // the end backwards and the earlier offsets stay valid.
    let mut resolved: Vec<(maxgus_text::Range, String)> = edits
        .iter()
        .filter_map(|edit| {
            let range = edit.get("range")?;
            let position = |key: &str| -> Option<LspPosition> {
                let p = range.get(key)?;
                Some(LspPosition::new(
                    p.get("line")?.as_u64()? as u32,
                    p.get("character")?.as_u64()? as u32,
                ))
            };
            let start = crate::position::offset_of_position(buffer, position("start")?, encoding);
            let end = crate::position::offset_of_position(buffer, position("end")?, encoding);
            let replacement = edit.get("newText")?.as_str()?.to_string();
            Some((
                maxgus_text::Range::new(start.min(end), end.max(start)),
                replacement,
            ))
        })
        .collect();
    resolved.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));

    let point = editor.current_buffer().point();
    let applied = editor.with_current_buffer(|buffer| {
        buffer.transact(false, |buffer| {
            for (range, replacement) in &resolved {
                buffer.replace(*range, replacement)?;
            }
            Ok::<(), maxgus_text::TextError>(())
        })
    });
    if let Err(error) = applied {
        editor.error(error.to_string());
        return;
    }
    editor.with_current_buffer(|b| b.set_point(point.min(b.point_max())));
    editor.follow_point();
    editor.message(format!("Applied {} change(s)", resolved.len()));
}

/// Applies a `WorkspaceEdit`, which a rename produces and which a server may
/// also ask for unprompted through `workspace/applyEdit`.
///
/// Returns how many edits went in, because a server that asked for this is
/// waiting to be told whether they did.
pub(crate) fn apply_workspace_edit(editor: &mut Editor, result: &serde_json::Value) -> usize {
    let Some(changes) = result.get("changes").and_then(|v| v.as_object()) else {
        // `documentChanges` is the other spelling.
        if let Some(document_changes) = result.get("documentChanges").and_then(|v| v.as_array()) {
            let mut total = 0usize;
            // `documentChanges` may also hold `create`, `rename` and `delete`
            // operations on the files themselves. Those are not carried out —
            // and must not pass unmentioned, because a rename whose text edits
            // land while its file operation does not leaves the project
            // referring to a file that was never moved.
            let mut skipped: Vec<String> = Vec::new();
            for change in document_changes {
                let Some(uri) = change
                    .get("textDocument")
                    .and_then(|d| d.get("uri"))
                    .and_then(|v| v.as_str())
                else {
                    if let Some(kind) = change.get("kind").and_then(|v| v.as_str()) {
                        skipped.push(kind.to_string());
                    }
                    continue;
                };
                let edits = change.get("edits").and_then(|v| v.as_array());
                total += edits.map_or(0, |e| e.len());
                apply_edits_to(editor, uri, edits);
            }
            match skipped.is_empty() {
                true => editor.message(format!("Applied {total} change(s)")),
                // Said as an error so it is not talked over: the edit only
                // half happened and the user has to know which half.
                false => editor.error(format!(
                    "Applied {total} change(s), but not {} file operation(s) the server asked for \
                     ({}); those files were left alone",
                    skipped.len(),
                    skipped.join(", ")
                )),
            }
            return total;
        }
        editor.error("The server proposed no changes");
        return 0;
    };
    let mut total = 0usize;
    for (uri, edits) in changes {
        let edits = edits.as_array();
        total += edits.map_or(0, |e| e.len());
        apply_edits_to(editor, uri, edits);
    }
    editor.message(format!("Applied {total} change(s)"));
    total
}

/// Applies edits to whichever buffer holds `uri`, if one is open.
fn apply_edits_to(editor: &mut Editor, uri: &str, edits: Option<&Vec<serde_json::Value>>) {
    let Some(path) = maxgus_lsp::client::uri_to_path(uri) else {
        return;
    };
    let Some(id) = editor.buffers.find_by_path(&path) else {
        // A rename can touch files that are not open; those are left for the
        // user to visit rather than edited behind their back.
        editor.message(format!(
            "`{}` was not changed: it is not open",
            path.display()
        ));
        return;
    };
    let previous = editor.current_buffer_id();
    if editor.switch_to_buffer(id).is_err() {
        return;
    }
    apply_text_edits(editor, edits.map(Vec::as_slice));
    editor.switch_to_buffer(previous).ok();
}

/// Lists locations into the cross-reference buffer.
fn list_locations(editor: &mut Editor, heading: &str, result: &serde_json::Value) {
    let locations = parse_locations(result);
    if locations.is_empty() {
        editor.error(format!("No {}", heading.to_lowercase()));
        return;
    }
    let root = editor
        .tree_root
        .clone()
        .unwrap_or_else(|| editor.default_directory());
    let mut text = format!("{heading} ({})\n\n", locations.len());
    for (uri, position) in &locations {
        let shown = maxgus_lsp::client::uri_to_path(uri)
            .map(|path| display_path(&path, &root))
            .unwrap_or_else(|| uri.clone());
        text.push_str(&format!(
            "{shown}:{}:{}\n",
            position.line + 1,
            position.character + 1
        ));
    }
    let count = locations.len();
    show_listing(editor, &text);
    editor.message(format!("{count} {}", heading.to_lowercase()));
}

/// A path as it should appear in a listing: relative to the project root when
/// it is inside it, so a full screen width is not spent on a shared prefix.
fn display_path(path: &std::path::Path, root: &std::path::Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn list_code_actions(editor: &mut Editor, result: &serde_json::Value) {
    let Some(actions) = result.as_array().filter(|a| !a.is_empty()) else {
        editor.error("No code actions available");
        return;
    };
    let titles: Vec<&str> = actions
        .iter()
        .filter_map(|a| a.get("title").and_then(|v| v.as_str()))
        .collect();
    if titles.is_empty() {
        editor.error("No code actions available");
        return;
    }
    // Actions carrying an edit are applied; the rest are only described,
    // because running a command needs a round trip this client does not make.
    let mut text = format!("Code actions ({})\n\n", titles.len());
    for (index, title) in titles.iter().enumerate() {
        text.push_str(&format!("{}. {title}\n", index + 1));
    }
    if let Some(edit) = actions.iter().find_map(|a| a.get("edit")) {
        apply_workspace_edit(editor, edit);
        return;
    }
    let count = titles.len();
    show_listing(editor, &text);
    editor.message(format!("{count} code action(s); no edit was offered"));
}

fn list_symbols(editor: &mut Editor, heading: &str, result: &serde_json::Value) {
    let Some(symbols) = result.as_array().filter(|a| !a.is_empty()) else {
        editor.error(format!("No {}", heading.to_lowercase()));
        return;
    };
    let mut text = format!("{heading} ({})\n\n", symbols.len());
    collect_symbols(symbols, 0, &mut text);
    let count = symbols.len();
    show_listing(editor, &text);
    editor.message(format!("{count} {}", heading.to_lowercase()));
}

/// Walks the symbol tree, which may be flat or nested.
fn collect_symbols(symbols: &[serde_json::Value], depth: usize, out: &mut String) {
    for symbol in symbols {
        let Some(name) = symbol.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let kind = symbol.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
        let line = symbol
            .get("range")
            .or_else(|| symbol.get("location").and_then(|l| l.get("range")))
            .and_then(|r| r.get("start"))
            .and_then(|s| s.get("line"))
            .and_then(|v| v.as_u64())
            .map(|l| l + 1);
        let indent = "  ".repeat(depth);
        match line {
            Some(line) => out.push_str(&format!(
                "{indent}{name}  [{}]  line {line}\n",
                symbol_kind(kind)
            )),
            None => out.push_str(&format!("{indent}{name}  [{}]\n", symbol_kind(kind))),
        }
        if let Some(children) = symbol.get("children").and_then(|v| v.as_array()) {
            collect_symbols(children, depth + 1, out);
        }
    }
}

/// The protocol's `SymbolKind` numbering, as a word.
fn symbol_kind(kind: u64) -> &'static str {
    match kind {
        1 => "file",
        2 => "module",
        3 => "namespace",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        15 => "string",
        16 => "number",
        17 => "boolean",
        18 => "array",
        23 => "struct",
        26 => "type",
        _ => "symbol",
    }
}

/// Shows a listing in the cross-reference buffer.
fn show_listing(editor: &mut Editor, text: &str) {
    let id = match editor.buffers.find_by_name(XREF_NAME) {
        Some(id) => {
            if editor.replace_buffer_contents(id, text).is_err() {
                return;
            }
            id
        }
        None => editor.buffers.create_with_text(XREF_NAME, text),
    };
    if let Some(buffer) = editor.buffers.get_mut(id) {
        buffer.set_read_only(true);
    }
    editor.show_in_editing_window(id).ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dispatch, Dispatcher};
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_lsp::{Diagnostic, Severity};
    use maxgus_tui::Rect;

    fn setup(text: &str) -> (Dispatcher, Editor) {
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        let id = editor.buffers.visit_file("/project/main.rs", text);
        editor.switch_to_buffer(id).unwrap();
        editor.with_current_buffer(|b| b.set_point(0));
        editor.tasks.drain();
        let registry = crate::commands::standard_registry();
        editor.command_names = registry.interactive_names();
        (Dispatcher::new(registry), editor)
    }

    /// The answer the editor queued for the server, if it queued one.
    fn queued_answer(e: &mut Editor) -> Option<(String, maxgus_lsp::RequestId, bool)> {
        e.tasks.drain().into_iter().find_map(|task| match task {
            crate::task::Task::LspRespond {
                language,
                id,
                applied,
            } => Some((language, id, applied)),
            _ => None,
        })
    }

    #[test]
    fn a_definition_given_as_a_location_link_is_understood() {
        // `linkSupport` is declared false, so a server should send a plain
        // `Location` — but several send a `LocationLink` regardless, and this
        // is the only editor path that reads `targetUri`.
        let (_d, mut e) = setup("fn main() {}\n");
        e.tasks.drain();
        e.apply_lsp_response(crate::task::TaskResult::LspResponse {
            language: "rust".into(),
            uri: "file:///project/main.rs".into(),
            query: crate::task::LspQuery::Definition(maxgus_lsp::LspPosition::new(0, 3)),
            result: serde_json::json!([{
                "targetUri": "file:///project/main.rs",
                "targetSelectionRange": {
                    "start": {"line": 0, "character": 3},
                    "end": {"line": 0, "character": 7}
                },
                "targetRange": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 12}
                }
            }]),
        });
        assert_eq!(
            e.current_buffer().line_of(e.current_buffer().point()),
            0,
            "the jump landed somewhere"
        );
        assert!(
            !e.minibuffer.message_is_error(),
            "got `{}`",
            e.minibuffer.display()
        );
    }

    #[test]
    fn a_rename_given_as_document_changes_is_applied() {
        // The other spelling of `WorkspaceEdit`, which several servers use in
        // preference to `changes`. Nothing exercised it.
        let (_d, mut e) = setup("let old = 1;\n");
        let applied = apply_workspace_edit(
            &mut e,
            &serde_json::json!({"documentChanges": [{
                "textDocument": {"uri": "file:///project/main.rs", "version": 1},
                "edits": [{
                    "range": {"start": {"line": 0, "character": 4},
                              "end": {"line": 0, "character": 7}},
                    "newText": "new"
                }]
            }]}),
        );
        assert_eq!(applied, 1);
        assert_eq!(e.current_buffer().text(), "let new = 1;\n");
    }

    #[test]
    fn a_file_operation_the_editor_cannot_do_is_said_out_loud() {
        // Renaming a Rust module gets a `rename` operation alongside the text
        // edits. Dropping it silently leaves `mod new;` in the source and no
        // `new.rs` on disk — the edit half happened and said "Applied 1".
        let (_d, mut e) = setup("mod old;\n");
        let applied = apply_workspace_edit(
            &mut e,
            &serde_json::json!({"documentChanges": [
                {
                    "textDocument": {"uri": "file:///project/main.rs", "version": 1},
                    "edits": [{
                        "range": {"start": {"line": 0, "character": 4},
                                  "end": {"line": 0, "character": 7}},
                        "newText": "new"
                    }]
                },
                {"kind": "rename", "oldUri": "file:///project/old.rs",
                 "newUri": "file:///project/new.rs"}
            ]}),
        );

        assert_eq!(applied, 1, "the text edit still goes in");
        assert_eq!(e.current_buffer().text(), "mod new;\n");
        assert!(
            e.minibuffer.message_is_error(),
            "a half-done edit is not good news"
        );
        let said = e.minibuffer.display();
        assert!(said.contains("rename"), "it does not say which: `{said}`");
        assert!(said.contains("file operation"), "got `{said}`");
    }

    #[test]
    fn an_edit_with_no_file_operations_reports_plainly() {
        // The other half: warning about skipped operations when none were
        // asked for would cry wolf on every ordinary rename.
        let (_d, mut e) = setup("let old = 1;\n");
        apply_workspace_edit(
            &mut e,
            &serde_json::json!({"documentChanges": [{
                "textDocument": {"uri": "file:///project/main.rs", "version": 1},
                "edits": [{
                    "range": {"start": {"line": 0, "character": 4},
                              "end": {"line": 0, "character": 7}},
                    "newText": "new"
                }]
            }]}),
        );
        assert!(
            !e.minibuffer.message_is_error(),
            "got `{}`",
            e.minibuffer.display()
        );
        assert_eq!(e.minibuffer.display(), "Applied 1 change(s)");
    }

    #[test]
    fn a_server_asking_to_apply_an_edit_gets_its_edit_and_an_answer() {
        // `workspace/applyEdit` is a request, not a notification: the server
        // waits to be told whether the edit went in.
        let (_d, mut e) = setup("let old = 1;\n");
        e.apply_lsp_response(crate::task::TaskResult::LspApplyEdit {
            language: "rust".into(),
            id: maxgus_lsp::RequestId::Number(5),
            edit: serde_json::json!({"changes": {"file:///project/main.rs": [
                {"range": {"start": {"line": 0, "character": 4},
                           "end": {"line": 0, "character": 7}},
                 "newText": "new"}
            ]}}),
        });

        assert_eq!(e.current_buffer().text(), "let new = 1;\n");
        let (language, id, applied) = queued_answer(&mut e).expect("an answer for the server");
        assert_eq!(language, "rust");
        assert_eq!(id, maxgus_lsp::RequestId::Number(5));
        assert!(applied, "the edit went in, so the server is told so");
    }

    #[test]
    fn an_edit_that_changes_nothing_is_answered_honestly() {
        // Saying `applied: true` when nothing happened would have the server
        // believe its change landed.
        let (_d, mut e) = setup("let old = 1;\n");
        e.apply_lsp_response(crate::task::TaskResult::LspApplyEdit {
            language: "rust".into(),
            id: maxgus_lsp::RequestId::Number(6),
            edit: serde_json::json!({ "changes": {} }),
        });

        assert_eq!(e.current_buffer().text(), "let old = 1;\n");
        let (_, _, applied) = queued_answer(&mut e).expect("an answer even so");
        assert!(!applied, "nothing was applied, and the server is told that");
    }

    fn run(d: &mut Dispatcher, e: &mut Editor, command: &str) {
        let out = d.execute(e, command, None);
        assert!(
            !matches!(out, Dispatch::Failed { .. }),
            "`{command}` failed: {out:?}"
        );
    }

    fn fails(d: &mut Dispatcher, e: &mut Editor, command: &str) -> String {
        match d.execute(e, command, None) {
            Dispatch::Failed { message, .. } => message,
            other => panic!("`{command}` should have failed, got {other:?}"),
        }
    }

    /// The query the command queued.
    fn queued(e: &mut Editor) -> LspQuery {
        let tasks = e.tasks.drain();
        let Some(Task::LspRequest { query, .. }) = tasks.into_iter().next() else {
            panic!("no request was queued");
        };
        query
    }

    #[test]
    fn every_lsp_binding_is_registered() {
        let registry = crate::commands::standard_registry();
        for name in [
            "lsp-find-definition",
            "lsp-find-references",
            "lsp-describe-thing-at-point",
            "lsp-rename",
            "lsp-format-buffer",
            "lsp-code-action",
            "lsp-workspace-symbol",
            "lsp-document-symbols",
            "lsp-restart-server",
            "completion-at-point",
            "next-error",
            "previous-error",
        ] {
            assert!(registry.contains(name), "`{name}` is missing");
        }
    }

    #[test]
    fn the_navigation_commands_queue_a_request_at_point() {
        let (mut d, mut e) = setup("fn main() {\n    helper();\n}\n");
        e.with_current_buffer(|b| b.set_point(b.line_start(1) + 4));

        run(&mut d, &mut e, "lsp-find-definition");
        assert_eq!(queued(&mut e), LspQuery::Definition(LspPosition::new(1, 4)));

        run(&mut d, &mut e, "lsp-find-references");
        assert_eq!(queued(&mut e), LspQuery::References(LspPosition::new(1, 4)));

        run(&mut d, &mut e, "lsp-describe-thing-at-point");
        assert_eq!(queued(&mut e), LspQuery::Hover(LspPosition::new(1, 4)));

        run(&mut d, &mut e, "completion-at-point");
        assert_eq!(queued(&mut e), LspQuery::Completion(LspPosition::new(1, 4)));
    }

    #[test]
    fn positions_are_counted_in_the_servers_encoding() {
        let (mut d, mut e) = setup("let s = \"héllo wörld\";\n");
        // Past both accented characters.
        e.with_current_buffer(|b| b.set_point(16));
        run(&mut d, &mut e, "lsp-find-definition");
        let LspQuery::Definition(position) = queued(&mut e) else {
            panic!()
        };
        assert_eq!(position.line, 0);
        assert_eq!(position.character, 16, "UTF-16 units, the protocol default");
    }

    #[test]
    fn formatting_passes_the_indentation_settings() {
        let (mut d, mut e) = setup("fn main() {}\n");
        e.settings.tab_width = 2;
        e.settings.indent_with_tabs = true;
        run(&mut d, &mut e, "lsp-format-buffer");
        assert_eq!(
            queued(&mut e),
            LspQuery::Format {
                tab_size: 2,
                insert_spaces: false
            }
        );
    }

    #[test]
    fn a_code_action_covers_the_region_when_there_is_one() {
        let (mut d, mut e) = setup("one\ntwo\nthree\n");
        e.with_current_buffer(|b| {
            b.set_point(0);
            b.set_mark(0);
            b.set_point(7);
        });
        run(&mut d, &mut e, "lsp-code-action");
        let LspQuery::CodeAction { range, .. } = queued(&mut e) else {
            panic!()
        };
        assert_eq!(range.start, LspPosition::new(0, 0));
        assert_eq!(range.end, LspPosition::new(1, 3));
    }

    #[test]
    fn a_code_action_with_no_region_asks_about_point() {
        let (mut d, mut e) = setup("one\ntwo\n");
        e.with_current_buffer(|b| b.set_point(5));
        run(&mut d, &mut e, "lsp-code-action");
        let LspQuery::CodeAction { range, .. } = queued(&mut e) else {
            panic!()
        };
        assert!(range.is_empty());
        assert_eq!(range.start, LspPosition::new(1, 1));
    }

    #[test]
    fn a_code_action_request_carries_the_diagnostics_it_should_fix() {
        let (mut d, mut e) = setup("let unused = 1;\nfine();\n");
        let uri = maxgus_lsp::client::path_to_uri(std::path::Path::new("/project/main.rs"));
        let at = |line: u32| {
            maxgus_lsp::Diagnostic::new(
                maxgus_lsp::LspRange::new(LspPosition::new(line, 4), LspPosition::new(line, 10)),
                maxgus_lsp::Severity::Warning,
                format!("problem on line {line}"),
            )
        };
        e.diagnostics.replace(uri, vec![at(0), at(1)]);

        // Point on the first line, no region: only that line's diagnostic.
        e.with_current_buffer(|b| b.set_point(5));
        run(&mut d, &mut e, "lsp-code-action");
        let LspQuery::CodeAction { diagnostics, .. } = queued(&mut e) else {
            panic!()
        };
        assert_eq!(diagnostics.len(), 1, "got {diagnostics:?}");
        assert!(diagnostics[0].message.contains("line 0"));
    }

    #[test]
    fn a_code_action_over_a_region_carries_every_diagnostic_in_it() {
        let (mut d, mut e) = setup("let unused = 1;\nfine();\n");
        let uri = maxgus_lsp::client::path_to_uri(std::path::Path::new("/project/main.rs"));
        let at = |line: u32| {
            maxgus_lsp::Diagnostic::new(
                maxgus_lsp::LspRange::new(LspPosition::new(line, 0), LspPosition::new(line, 5)),
                maxgus_lsp::Severity::Warning,
                "problem",
            )
        };
        e.diagnostics.replace(uri, vec![at(0), at(1)]);
        e.with_current_buffer(|b| {
            b.set_point(0);
            b.set_mark(0);
            let end = b.len_chars();
            b.set_point(end);
        });
        run(&mut d, &mut e, "lsp-code-action");
        let LspQuery::CodeAction { diagnostics, .. } = queued(&mut e) else {
            panic!()
        };
        assert_eq!(diagnostics.len(), 2);
    }

    #[test]
    fn renaming_offers_the_symbol_under_point() {
        let (mut d, mut e) = setup("let helper = 1;\n");
        e.with_current_buffer(|b| b.set_point(6));
        run(&mut d, &mut e, "lsp-rename");
        assert_eq!(e.minibuffer.input(), "helper", "pre-filled with the symbol");

        e.minibuffer.kill_whole();
        for c in "renamed".chars() {
            e.minibuffer.insert_char(c);
        }
        d.handle_keys(&mut e, "RET");
        let LspQuery::Rename { new_name, .. } = queued(&mut e) else {
            panic!()
        };
        assert_eq!(new_name, "renamed");
    }

    #[test]
    fn renaming_to_nothing_is_refused() {
        let (mut d, mut e) = setup("let x = 1;\n");
        d.execute(&mut e, "lsp-rename", None);
        e.minibuffer.kill_whole();
        assert!(matches!(
            d.handle_keys(&mut e, "RET"),
            Dispatch::Failed { .. }
        ));
    }

    #[test]
    fn a_workspace_symbol_search_prompts_first() {
        let (mut d, mut e) = setup("fn helper() {}\n");
        e.with_current_buffer(|b| b.set_point(4));
        run(&mut d, &mut e, "lsp-workspace-symbol");
        assert_eq!(e.minibuffer.input(), "helper");
        d.handle_keys(&mut e, "RET");
        assert_eq!(queued(&mut e), LspQuery::WorkspaceSymbols("helper".into()));
    }

    #[test]
    fn signature_help_asks_about_point() {
        let (mut d, mut e) = setup("fn main() {\n    helper(\n}\n");
        e.with_current_buffer(|b| b.set_point(b.line_start(1) + 11));
        run(&mut d, &mut e, "lsp-signature-help");
        assert_eq!(
            queued(&mut e),
            LspQuery::SignatureHelp(LspPosition::new(1, 11))
        );
    }

    #[test]
    fn document_symbols_needs_no_argument() {
        let (mut d, mut e) = setup("fn main() {}\n");
        run(&mut d, &mut e, "lsp-document-symbols");
        assert_eq!(
            queued(&mut e),
            LspQuery::DocumentSymbols { for_panel: false }
        );
    }

    #[test]
    fn restarting_stops_and_starts_the_server_and_reopens_the_document() {
        let (mut d, mut e) = setup("fn main() {}\n");
        run(&mut d, &mut e, "lsp-restart-server");
        let tasks = e.tasks.drain();
        assert!(
            tasks
                .iter()
                .any(|t| matches!(t, Task::StopLanguageServer { .. }))
        );
        assert!(
            tasks
                .iter()
                .any(|t| matches!(t, Task::StartLanguageServer { .. }))
        );
        assert!(tasks.iter().any(|t| matches!(t, Task::LspDidOpen { .. })));
    }

    #[test]
    fn a_buffer_with_no_language_has_no_server_to_ask() {
        // A file with no extension has no language to guess from. One with
        // an extension nothing knows takes the extension as its language —
        // that is what lets a grammar be looked for — and is turned away
        // later, by the executor, which is what holds the server list.
        let (mut d, mut e) = setup("text");
        let id = e.buffers.visit_file("/project/NOTES", "text");
        e.switch_to_buffer(id).unwrap();
        assert!(fails(&mut d, &mut e, "lsp-find-definition").contains("no language"));
    }

    #[test]
    fn a_buffer_with_no_file_has_no_document_to_ask_about() {
        let (mut d, mut e) = setup("text");
        let id = e.buffers.create("notes");
        e.switch_to_buffer(id).unwrap();
        assert!(fails(&mut d, &mut e, "lsp-find-definition").contains("no language"));
    }

    #[test]
    fn the_commands_refuse_when_language_server_support_is_off() {
        let (mut d, mut e) = setup("fn main() {}\n");
        e.settings.lsp_enabled = false;
        assert!(fails(&mut d, &mut e, "lsp-find-definition").contains("support is off"));
    }

    // ---- diagnostics ----

    /// Attaches three diagnostics to the buffer under test.
    fn with_diagnostics(e: &mut Editor) {
        let at = |line: u32, severity| {
            Diagnostic::new(
                LspRange::new(LspPosition::new(line, 0), LspPosition::new(line, 3)),
                severity,
                format!("problem on line {line}"),
            )
        };
        e.diagnostics.replace(
            "file:///project/main.rs",
            vec![
                at(1, Severity::Error),
                at(3, Severity::Warning),
                at(5, Severity::Error),
            ],
        );
    }

    #[test]
    fn next_error_walks_forward_through_the_diagnostics() {
        let (mut d, mut e) = setup("a\nb\nc\nd\ne\nf\n");
        with_diagnostics(&mut e);

        run(&mut d, &mut e, "next-error");
        assert_eq!(e.current_buffer().line_of(e.windows.current().point), 1);
        assert!(e.minibuffer.display().contains("problem on line 1"));

        run(&mut d, &mut e, "next-error");
        assert_eq!(e.current_buffer().line_of(e.windows.current().point), 3);
        run(&mut d, &mut e, "next-error");
        assert_eq!(e.current_buffer().line_of(e.windows.current().point), 5);
        assert!(fails(&mut d, &mut e, "next-error").contains("No further"));
    }

    #[test]
    fn previous_error_walks_back_again() {
        let (mut d, mut e) = setup("a\nb\nc\nd\ne\nf\n");
        with_diagnostics(&mut e);
        e.with_current_buffer(|b| b.set_point(b.line_start(5)));

        run(&mut d, &mut e, "previous-error");
        assert_eq!(e.current_buffer().line_of(e.windows.current().point), 3);
        run(&mut d, &mut e, "previous-error");
        assert_eq!(e.current_buffer().line_of(e.windows.current().point), 1);
        assert!(fails(&mut d, &mut e, "previous-error").contains("No previous"));
    }

    #[test]
    fn error_navigation_in_a_clean_buffer_says_there_is_nothing() {
        let (mut d, mut e) = setup("a\nb\n");
        assert!(fails(&mut d, &mut e, "next-error").contains("No further"));
    }

    /// The location payload a server sends for a definition.
    fn location(uri: &str, line: u32, character: u32) -> serde_json::Value {
        serde_json::json!({
            "uri": uri,
            "range": {
                "start": {"line": line, "character": character},
                "end": {"line": line, "character": character + 4}
            }
        })
    }

    #[test]
    fn a_definition_response_moves_point_and_says_where_it_landed() {
        let (_d, mut e) = setup("fn helper() {}\n\nfn main() {\n    helper();\n}\n");
        e.with_current_buffer(|b| b.set_point(b.line_start(3) + 4));
        e.message("Language server: finding definition...");

        apply_response(
            &mut e,
            &LspQuery::Definition(LspPosition::new(3, 4)),
            &location("file:///project/main.rs", 0, 3),
        );

        assert_eq!(
            e.current_buffer().line_of(e.windows.current().point),
            0,
            "jumped to line 1"
        );
        // The in-flight message must not survive the jump.
        assert!(
            !e.minibuffer.display().contains("finding definition"),
            "a stale message was left behind: `{}`",
            e.minibuffer.display()
        );
        assert!(
            e.minibuffer.display().contains("main.rs:1"),
            "got `{}`",
            e.minibuffer.display()
        );
        assert!(
            e.minibuffer.display().contains("M-,"),
            "it says how to come back"
        );
    }

    #[test]
    fn a_definition_jump_leaves_the_mark_where_it_started() {
        let (_d, mut e) = setup("fn helper() {}\n\nfn main() {\n    helper();\n}\n");
        let from = e.current_buffer().line_start(3) + 4;
        e.with_current_buffer(|b| b.set_point(from));

        apply_response(
            &mut e,
            &LspQuery::Definition(LspPosition::new(3, 4)),
            &location("file:///project/main.rs", 0, 3),
        );
        assert_eq!(
            e.current_buffer().mark(),
            Some(from),
            "M-, has somewhere to go back to"
        );
    }

    #[test]
    fn a_definition_in_a_file_that_is_not_open_is_read_first() {
        let (_d, mut e) = setup("fn main() {}\n");
        e.tasks.drain();
        apply_response(
            &mut e,
            &LspQuery::Definition(LspPosition::ZERO),
            &location("file:///project/other.rs", 2, 0),
        );
        let tasks = e.tasks.drain();
        assert!(
            tasks
                .iter()
                .any(|t| matches!(t, Task::ReadFile { path, .. } if path.ends_with("other.rs"))),
            "got {tasks:?}"
        );
        // The jump is remembered until the file arrives.
        assert!(e.pending_jump.is_some());
    }

    #[test]
    fn several_definitions_are_listed_rather_than_guessed_between() {
        let (_d, mut e) = setup("fn main() {}\n");
        apply_response(
            &mut e,
            &LspQuery::Definition(LspPosition::ZERO),
            &serde_json::json!([
                location("file:///project/a.rs", 1, 0),
                location("file:///project/b.rs", 5, 0),
            ]),
        );
        assert_eq!(e.current_buffer().name(), XREF_NAME);
        let text = e.current_buffer().text();
        assert!(text.contains("Definitions (2)"), "got `{text}`");
        // Paths are shown relative to the project, which here is `/project`.
        assert!(text.contains("a.rs:2:1"), "got `{text}`");
    }

    #[test]
    fn no_definition_is_reported_rather_than_leaving_the_message_hanging() {
        let (_d, mut e) = setup("fn main() {}\n");
        e.message("Language server: finding definition...");
        apply_response(
            &mut e,
            &LspQuery::Definition(LspPosition::ZERO),
            &serde_json::Value::Null,
        );
        assert!(e.minibuffer.message_is_error());
        assert!(
            e.minibuffer.display().contains("No definition"),
            "got `{}`",
            e.minibuffer.display()
        );
    }

    /// A plausible reply for each kind of question, for checking that none of
    /// them leaves the "Language server: ..." message on screen.
    fn plausible_replies() -> Vec<(LspQuery, serde_json::Value)> {
        let range = serde_json::json!({
            "start": {"line": 0, "character": 0},
            "end": {"line": 0, "character": 3}
        });
        vec![
            (
                LspQuery::Definition(LspPosition::ZERO),
                location("file:///project/main.rs", 0, 0),
            ),
            (
                LspQuery::Definition(LspPosition::ZERO),
                serde_json::Value::Null,
            ),
            (
                LspQuery::References(LspPosition::ZERO),
                serde_json::json!([location("file:///project/main.rs", 1, 0)]),
            ),
            (
                LspQuery::References(LspPosition::ZERO),
                serde_json::json!([]),
            ),
            (
                LspQuery::Hover(LspPosition::ZERO),
                serde_json::json!({"contents": "one line"}),
            ),
            (
                LspQuery::Hover(LspPosition::ZERO),
                serde_json::json!({"contents": {"value": "first\n\nsecond"}}),
            ),
            (LspQuery::Hover(LspPosition::ZERO), serde_json::Value::Null),
            (
                LspQuery::Completion(LspPosition::ZERO),
                serde_json::json!([{"label": "fnamed"}]),
            ),
            (
                LspQuery::Completion(LspPosition::ZERO),
                serde_json::json!([{"label": "fnalpha"}, {"label": "fnbeta"}]),
            ),
            (
                LspQuery::Completion(LspPosition::ZERO),
                serde_json::json!([]),
            ),
            (
                LspQuery::SignatureHelp(LspPosition::ZERO),
                serde_json::json!({"signatures": [{"label": "fn(x: i32)"}]}),
            ),
            (
                LspQuery::SignatureHelp(LspPosition::ZERO),
                serde_json::Value::Null,
            ),
            (
                LspQuery::Rename {
                    position: LspPosition::ZERO,
                    new_name: "x".into(),
                },
                serde_json::json!({"changes": {"file:///project/main.rs": [
                    {"range": range, "newText": "xyz"}
                ]}}),
            ),
            (
                LspQuery::Rename {
                    position: LspPosition::ZERO,
                    new_name: "x".into(),
                },
                serde_json::json!({}),
            ),
            (
                LspQuery::Format {
                    tab_size: 4,
                    insert_spaces: true,
                },
                serde_json::json!([{"range": range, "newText": "abc"}]),
            ),
            (
                LspQuery::Format {
                    tab_size: 4,
                    insert_spaces: true,
                },
                serde_json::json!([]),
            ),
            (
                LspQuery::CodeAction {
                    range: maxgus_lsp::LspRange::empty(LspPosition::ZERO),
                    diagnostics: Vec::new(),
                },
                serde_json::json!([{"title": "do the thing"}]),
            ),
            (
                LspQuery::CodeAction {
                    range: maxgus_lsp::LspRange::empty(LspPosition::ZERO),
                    diagnostics: Vec::new(),
                },
                serde_json::json!([]),
            ),
            (
                LspQuery::DocumentSymbols { for_panel: false },
                serde_json::json!([{"name": "main", "kind": 12,
                    "range": {"start": {"line": 0, "character": 0}}}]),
            ),
            (
                LspQuery::DocumentSymbols { for_panel: false },
                serde_json::json!([]),
            ),
            (
                LspQuery::WorkspaceSymbols("m".into()),
                serde_json::json!([{"name": "main", "kind": 12,
                    "location": {"range": {"start": {"line": 0, "character": 0}}}}]),
            ),
        ]
    }

    #[test]
    fn no_reply_leaves_the_in_flight_message_on_screen() {
        // Every request puts "Language server: ..." in the echo area. If a
        // reply does not replace it, the user is left looking at a request
        // that finished long ago.
        for (query, reply) in plausible_replies() {
            let (_d, mut e) = setup("fn main() {}\nfn other() {}\n");
            e.message(format!("Language server: {}...", query.description()));
            apply_response(&mut e, &query, &reply);
            let shown = e.minibuffer.display();
            assert!(
                !shown.starts_with("Language server:"),
                "`{}` with reply `{reply}` left `{shown}` on screen",
                query.description()
            );
            // An empty echo area is only allowed when the answer went
            // somewhere the user can see instead.
            if shown.is_empty() {
                assert!(
                    matches!(query, LspQuery::Hover(_)) && e.doc.is_some(),
                    "`{}` said nothing at all",
                    query.description()
                );
            }
        }
    }

    #[test]
    fn a_listing_replaces_the_in_flight_message_with_its_result() {
        let (_d, mut e) = setup("fn main() {}\n");
        e.message("Language server: finding references...");
        apply_response(
            &mut e,
            &LspQuery::References(LspPosition::ZERO),
            &serde_json::json!([
                location("file:///project/a.rs", 1, 0),
                location("file:///project/b.rs", 5, 0),
            ]),
        );
        assert!(
            !e.minibuffer.display().contains("finding references"),
            "a stale message was left behind: `{}`",
            e.minibuffer.display()
        );
        assert_eq!(e.minibuffer.display(), "2 references");
    }

    #[test]
    fn a_listing_shows_paths_relative_to_the_project_root() {
        let (_d, mut e) = setup("fn main() {}\n");
        e.tree_root = Some(std::path::PathBuf::from("/project"));
        apply_response(
            &mut e,
            &LspQuery::References(LspPosition::ZERO),
            &serde_json::json!([location("file:///project/src/deep/thing.rs", 41, 7)]),
        );
        let text = e.current_buffer().text();
        assert!(text.contains("src/deep/thing.rs:42:8"), "got `{text}`");
        assert!(
            !text.contains("/project/src"),
            "the shared prefix is not repeated: `{text}`"
        );
    }

    #[test]
    fn a_path_outside_the_project_is_shown_in_full() {
        let root = std::path::Path::new("/project");
        assert_eq!(
            display_path(std::path::Path::new("/project/src/a.rs"), root),
            "src/a.rs"
        );
        assert_eq!(
            display_path(std::path::Path::new("/usr/include/stdio.h"), root),
            "/usr/include/stdio.h",
            "a system header is not under the project"
        );
    }

    #[test]
    fn document_symbols_report_their_count_when_listed() {
        let (_d, mut e) = setup("fn main() {}\n");
        e.message("Language server: document symbols...");
        apply_response(
            &mut e,
            &LspQuery::DocumentSymbols { for_panel: false },
            &serde_json::json!([
                {"name": "helper", "kind": 12, "range": {"start": {"line": 2, "character": 0}}},
                {"name": "main", "kind": 12, "range": {"start": {"line": 6, "character": 0}}},
            ]),
        );
        let text = e.current_buffer().text();
        assert!(text.contains("helper  [function]  line 3"), "got `{text}`");
        assert_eq!(e.minibuffer.display(), "2 document symbols");
    }

    #[test]
    fn a_hover_reply_is_shown_whatever_shape_it_arrives_in() {
        let (_d, mut e) = setup("let x = 1;\n");

        // A plain string.
        apply_response(
            &mut e,
            &LspQuery::Hover(LspPosition::ZERO),
            &serde_json::json!({"contents": "an integer"}),
        );
        assert_eq!(
            e.doc.as_ref().map(|doc| doc.text.as_str()),
            Some("an integer")
        );

        // `MarkupContent`, which is what clangd and rust-analyzer send.
        apply_response(
            &mut e,
            &LspQuery::Hover(LspPosition::ZERO),
            &serde_json::json!({"contents": {"kind": "markdown", "value": "variable x\n\nType: int"}}),
        );
        let doc = e.doc.as_ref().expect("a multi-line answer is kept whole");
        assert!(doc.text.contains("Type: int"), "got `{}`", doc.text);
        assert_ne!(
            e.current_buffer().name(),
            "*Help*",
            "the box goes beside the code rather than over it in a window"
        );
    }

    #[test]
    fn a_hover_with_nothing_in_it_says_so_only_when_it_was_asked_for() {
        let (_d, mut e) = setup("let x = 1;\n");
        // Out loud: `describe_thing` clears the mark that says an idle pause
        // asked, so an empty answer is worth reporting.
        e.doc_asked_at = None;
        apply_response(
            &mut e,
            &LspQuery::Hover(LspPosition::ZERO),
            &serde_json::json!({}),
        );
        assert!(e.minibuffer.display().contains("Nothing to describe"));

        // On an idle pause, nobody asked, so nothing is said.
        e.message(String::new());
        e.doc_asked_at = Some((e.current_buffer_id(), 0));
        apply_response(
            &mut e,
            &LspQuery::Hover(LspPosition::ZERO),
            &serde_json::json!({}),
        );
        assert_eq!(
            e.minibuffer.display(),
            "",
            "an unasked question was answered out loud"
        );
    }

    #[test]
    fn the_symbol_under_point_is_found_or_empty() {
        let (_d, mut e) = setup("let helper = 1;\n");
        e.with_current_buffer(|b| b.set_point(6));
        assert_eq!(symbol_at_point(&e), "helper");
        // `helper` spans 4..10, so offset 10 is immediately after it.
        e.with_current_buffer(|b| b.set_point(10));
        assert_eq!(
            symbol_at_point(&e),
            "helper",
            "point just after the word still counts"
        );
        e.with_current_buffer(|b| b.set_point(11));
        assert_eq!(symbol_at_point(&e), "", "on a space there is no symbol");
    }
}
