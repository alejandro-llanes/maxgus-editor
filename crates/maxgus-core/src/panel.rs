//! The side panel: the file tree, the symbols in the current buffer, and the
//! list of open buffers, stacked in one window.
//!
//! The panel is backed by an ordinary read-only buffer with one line per row,
//! exactly as the file tree alone used to be. Point moves through it with the
//! ordinary motion commands and every panel command asks what row point is on
//! rather than tracking a cursor of its own — which is why `n` and `p` in the
//! panel are the same `next-line` and `previous-line` as everywhere else, and
//! why a section can be folded away without anything having to be told.
//!
//! Sections are switched on in configuration and folded by hand, and the
//! symbol section additionally disappears when there is no language server to
//! ask, because an empty outline is worse than no outline.

use maxgus_text::BufferId;

/// The three things the panel can show, in the order they are stacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PanelSection {
    Tree,
    Symbols,
    Buffers,
}

pub const SECTIONS: [PanelSection; 3] = [
    PanelSection::Tree,
    PanelSection::Symbols,
    PanelSection::Buffers,
];

impl PanelSection {
    /// The heading, which is also what `describe-key` calls the section.
    pub fn title(self) -> &'static str {
        match self {
            PanelSection::Tree => "FILES",
            PanelSection::Symbols => "SYMBOLS",
            PanelSection::Buffers => "BUFFERS",
        }
    }

    /// The name the `panel` configuration block uses.
    pub fn key(self) -> &'static str {
        match self {
            PanelSection::Tree => "tree",
            PanelSection::Symbols => "symbols",
            PanelSection::Buffers => "buffers",
        }
    }

    pub fn from_key(key: &str) -> Option<PanelSection> {
        SECTIONS.into_iter().find(|section| section.key() == key)
    }

    fn index(self) -> usize {
        match self {
            PanelSection::Tree => 0,
            PanelSection::Symbols => 1,
            PanelSection::Buffers => 2,
        }
    }
}

/// One symbol from the language server, flattened out of the tree the server
/// sends so that a row is an index and folding is a flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// The LSP `SymbolKind`, 1–26.
    pub kind: u8,
    pub detail: Option<String>,
    /// Zero-based position of the symbol's name, which is where `RET` goes.
    pub line: usize,
    pub column: usize,
    /// Nesting level; a top-level symbol is zero.
    pub depth: usize,
    /// How many symbols follow that are inside this one. Kept so that folding
    /// is a slice rather than a walk.
    pub descendants: usize,
    pub expanded: bool,
}

impl Symbol {
    pub fn has_children(&self) -> bool {
        self.descendants > 0
    }

    /// The face the name is drawn in, by kind.
    pub fn face(&self) -> &'static str {
        match self.kind {
            5 | 23 => "font-lock-type",              // class, struct
            6 | 9 | 12 => "font-lock-function-name", // method, constructor, function
            8 | 7 => "font-lock-property",           // field, property
            13 | 14 | 22 => "font-lock-constant",    // variable, constant, enum member
            10 => "font-lock-type",                  // enum
            11 => "font-lock-type",                  // interface
            2..=4 => "font-lock-preprocessor",       // module, namespace, package
            _ => "font-lock-variable-name",
        }
    }

    /// A short word for the kind, shown after the name.
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            1 => "file",
            2 => "module",
            3 => "namespace",
            4 => "package",
            5 => "class",
            6 => "method",
            7 => "property",
            8 => "field",
            9 => "new",
            10 => "enum",
            11 => "interface",
            12 => "fn",
            13 => "var",
            14 => "const",
            15 => "string",
            16 => "number",
            17 => "bool",
            18 => "array",
            19 => "object",
            20 => "key",
            21 => "null",
            22 => "variant",
            23 => "struct",
            24 => "event",
            25 => "operator",
            26 => "type",
            _ => "",
        }
    }

    /// The arrow drawn ahead of a symbol that contains others.
    pub fn arrow(&self) -> &'static str {
        match (self.has_children(), self.expanded) {
            (false, _) => "  ",
            (true, true) => "v ",
            (true, false) => "> ",
        }
    }
}

/// The panel's own state: which of its three windows exist, and the outline
/// of whatever buffer was last asked about.
///
/// The three sections are three *windows*, each with its own buffer of one
/// line per item. Moving between them is therefore ordinary window movement,
/// each keeps its own point, and each scrolls on its own — which is what a
/// single buffer with headings in it could never do.
#[derive(Debug, Clone)]
pub struct Panel {
    /// Switched on in configuration, per section.
    enabled: [bool; 3],
    pub symbols: Vec<Symbol>,
    /// The buffer the outline belongs to. Symbols for one buffer must never
    /// be shown against another, so this is checked rather than assumed.
    pub symbols_buffer: Option<BufferId>,
    /// True once a server has been asked and has not yet answered, which is
    /// the difference between "no symbols" and "not asked".
    pub symbols_pending: bool,
}

impl Default for Panel {
    fn default() -> Panel {
        Panel {
            enabled: [true; 3],
            symbols: Vec::new(),
            symbols_buffer: None,
            symbols_pending: false,
        }
    }
}

impl Panel {
    pub fn new() -> Panel {
        Panel::default()
    }

    /// A panel with the sections the configuration asks for.
    pub fn from_settings(settings: &maxgus_config::Settings) -> Panel {
        let mut panel = Panel::default();
        panel.set_enabled(PanelSection::Tree, settings.panel_tree);
        panel.set_enabled(PanelSection::Symbols, settings.panel_symbols);
        panel.set_enabled(PanelSection::Buffers, settings.panel_buffers);
        panel
    }

    pub fn is_enabled(&self, section: PanelSection) -> bool {
        self.enabled[section.index()]
    }

    pub fn set_enabled(&mut self, section: PanelSection, on: bool) {
        self.enabled[section.index()] = on;
    }

    /// How many sections are switched on.
    pub fn enabled_count(&self) -> usize {
        self.enabled.iter().filter(|on| **on).count()
    }

    /// Replaces the outline with one the server sent for `buffer`.
    pub fn set_symbols(&mut self, buffer: BufferId, symbols: Vec<Symbol>) {
        // Folding is worth keeping across a reparse: the outline is rebuilt
        // on every save, and losing the shape each time would make the
        // section unusable on a large file.
        let folded: Vec<(String, usize)> = self
            .symbols
            .iter()
            .filter(|symbol| !symbol.expanded && symbol.has_children())
            .map(|symbol| (symbol.name.clone(), symbol.depth))
            .collect();
        self.symbols = symbols;
        if self.symbols_buffer == Some(buffer) {
            for symbol in &mut self.symbols {
                if folded
                    .iter()
                    .any(|(name, depth)| *name == symbol.name && *depth == symbol.depth)
                {
                    symbol.expanded = false;
                }
            }
        }
        self.symbols_buffer = Some(buffer);
        self.symbols_pending = false;
    }

    /// Drops the outline, because it belongs to a buffer no longer shown.
    pub fn forget_symbols(&mut self) {
        self.symbols.clear();
        self.symbols_buffer = None;
        self.symbols_pending = false;
    }

    /// Expands or folds the symbol at `index`, and says whether anything
    /// changed so the caller knows whether to redraw.
    pub fn toggle_symbol(&mut self, index: usize) -> bool {
        match self.symbols.get_mut(index) {
            Some(symbol) if symbol.descendants > 0 => {
                symbol.expanded = !symbol.expanded;
                true
            }
            _ => false,
        }
    }

    pub fn set_symbol_expanded(&mut self, index: usize, expanded: bool) -> bool {
        match self.symbols.get_mut(index) {
            Some(symbol) if symbol.descendants > 0 && symbol.expanded != expanded => {
                symbol.expanded = expanded;
                true
            }
            _ => false,
        }
    }

    /// The symbols that are actually on screen: a symbol is shown when every
    /// symbol containing it is expanded.
    pub fn visible_symbols(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut index = 0;
        while index < self.symbols.len() {
            out.push(index);
            let symbol = &self.symbols[index];
            index += if symbol.expanded {
                1
            } else {
                1 + symbol.descendants
            };
        }
        out
    }

    /// The symbol on `line` of the outline window.
    pub fn symbol_on_line(&self, line: usize) -> Option<usize> {
        self.visible_symbols().get(line).copied()
    }

    /// The line the symbol at `index` is drawn on, if it is shown at all.
    pub fn line_of_symbol(&self, index: usize) -> Option<usize> {
        self.visible_symbols()
            .iter()
            .position(|found| *found == index)
    }

    /// The symbol that encloses `line` most closely, which is what follow
    /// mode marks as point moves through the buffer.
    pub fn symbol_at_line(&self, line: usize) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (index, symbol) in self.symbols.iter().enumerate() {
            if symbol.line <= line {
                let ends_after = self
                    .symbols
                    .get(index + symbol.descendants + 1)
                    .is_none_or(|next| next.line > line);
                if ends_after {
                    best = Some(index);
                }
            }
        }
        best
    }
}

/// Reads a `textDocument/documentSymbol` answer into a flat outline.
///
/// Servers answer in either of two shapes: `DocumentSymbol[]`, which nests,
/// or the older flat `SymbolInformation[]`. Both are accepted, because which
/// one arrives is the server's choice and not something a user should have to
/// know about.
pub fn symbols_from_lsp(value: &serde_json::Value) -> Vec<Symbol> {
    let mut out = Vec::new();
    let Some(items) = value.as_array() else {
        return out;
    };
    for item in items {
        push_symbol(item, 0, &mut out);
    }
    out
}

fn push_symbol(value: &serde_json::Value, depth: usize, out: &mut Vec<Symbol>) {
    let Some(name) = value.get("name").and_then(|n| n.as_str()) else {
        return;
    };
    let kind = value.get("kind").and_then(|k| k.as_u64()).unwrap_or(0) as u8;
    // `DocumentSymbol` puts the name's own range in `selectionRange`; the
    // flat shape wraps a `Location`. Jumping to the name rather than to the
    // start of the whole definition is what makes `RET` land where a reader
    // expects.
    let position = value
        .get("selectionRange")
        .or_else(|| value.get("range"))
        .or_else(|| value.get("location").and_then(|l| l.get("range")))
        .and_then(|range| range.get("start"));
    let line = position
        .and_then(|p| p.get("line"))
        .and_then(|l| l.as_u64())
        .unwrap_or(0) as usize;
    let column = position
        .and_then(|p| p.get("character"))
        .and_then(|c| c.as_u64())
        .unwrap_or(0) as usize;

    let at = out.len();
    out.push(Symbol {
        name: name.to_string(),
        kind,
        detail: value
            .get("detail")
            .and_then(|d| d.as_str())
            .map(str::to_string),
        line,
        column,
        depth,
        descendants: 0,
        expanded: true,
    });
    if let Some(children) = value.get("children").and_then(|c| c.as_array()) {
        for child in children {
            push_symbol(child, depth + 1, out);
        }
    }
    // Filled in on the way back out, when every descendant has been pushed.
    out[at].descendants = out.len() - at - 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `rust-analyzer`'s shape: a struct with fields, then a function.
    fn nested() -> serde_json::Value {
        json!([
            {
                "name": "Panel", "kind": 23,
                "selectionRange": {"start": {"line": 10, "character": 11}},
                "children": [
                    {"name": "enabled", "kind": 8,
                     "selectionRange": {"start": {"line": 11, "character": 4}}},
                    {"name": "rows", "kind": 8,
                     "selectionRange": {"start": {"line": 12, "character": 4}}}
                ]
            },
            {"name": "lay_out", "kind": 12, "detail": "fn(&mut self)",
             "selectionRange": {"start": {"line": 30, "character": 7}}}
        ])
    }

    #[test]
    fn an_outline_is_flattened_with_each_symbols_reach_recorded() {
        let symbols = symbols_from_lsp(&nested());
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Panel", "enabled", "rows", "lay_out"]);
        assert_eq!(symbols[0].depth, 0);
        assert_eq!(symbols[1].depth, 1, "a field is inside its struct");
        // `descendants` is what folding slices on, so it has to count the
        // whole subtree and not just the immediate children.
        assert_eq!(symbols[0].descendants, 2);
        assert_eq!(symbols[3].descendants, 0);
        assert_eq!((symbols[0].line, symbols[0].column), (10, 11));
        assert_eq!(symbols[3].detail.as_deref(), Some("fn(&mut self)"));
    }

    #[test]
    fn the_older_flat_symbol_shape_is_read_too() {
        // `SymbolInformation` wraps the position in a `location`, and which
        // shape arrives is the server's choice rather than the user's.
        let flat = json!([
            {"name": "main", "kind": 12,
             "location": {"uri": "file:///p/a.rs",
                          "range": {"start": {"line": 3, "character": 3}}}}
        ]);
        let symbols = symbols_from_lsp(&flat);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].line, 3);
        assert_eq!(symbols[0].column, 3);
    }

    #[test]
    fn folding_a_symbol_hides_everything_inside_it() {
        let mut panel = Panel::new();
        panel.set_symbols(BufferId(1), symbols_from_lsp(&nested()));
        assert_eq!(panel.visible_symbols(), [0, 1, 2, 3]);

        assert!(panel.toggle_symbol(0), "the struct has children to fold");
        assert_eq!(
            panel.visible_symbols(),
            [0, 3],
            "the fields are still shown"
        );

        assert!(!panel.toggle_symbol(3), "a leaf has nothing to fold");
    }

    #[test]
    fn a_line_of_the_outline_window_is_the_symbol_drawn_on_it() {
        // The window has one line per *visible* symbol, so folding changes
        // which symbol a line stands for.
        let mut panel = Panel::new();
        panel.set_symbols(BufferId(1), symbols_from_lsp(&nested()));
        assert_eq!(panel.symbol_on_line(3), Some(3));
        assert_eq!(panel.line_of_symbol(3), Some(3));

        panel.toggle_symbol(0);
        assert_eq!(
            panel.symbol_on_line(1),
            Some(3),
            "the fold did not move the line"
        );
        assert_eq!(panel.line_of_symbol(3), Some(1));
        assert_eq!(
            panel.line_of_symbol(1),
            None,
            "a folded symbol is on no line"
        );
        assert_eq!(panel.symbol_on_line(99), None);
    }

    #[test]
    fn folding_survives_the_outline_being_rebuilt() {
        // The outline is asked for again on every save. Losing the shape each
        // time would make the section useless on a large file.
        let mut panel = Panel::new();
        panel.set_symbols(BufferId(1), symbols_from_lsp(&nested()));
        panel.toggle_symbol(0);

        panel.set_symbols(BufferId(1), symbols_from_lsp(&nested()));
        assert_eq!(panel.visible_symbols(), [0, 3], "the fold was forgotten");
    }

    #[test]
    fn an_outline_for_a_different_buffer_starts_unfolded() {
        // Names collide between files; carrying folds across would fold
        // whatever happened to share a name in the file just opened.
        let mut panel = Panel::new();
        panel.set_symbols(BufferId(1), symbols_from_lsp(&nested()));
        panel.toggle_symbol(0);

        panel.set_symbols(BufferId(2), symbols_from_lsp(&nested()));
        assert_eq!(panel.visible_symbols(), [0, 1, 2, 3]);
    }

    #[test]
    fn the_innermost_symbol_containing_a_line_is_the_one_found() {
        let mut panel = Panel::new();
        panel.set_symbols(BufferId(1), symbols_from_lsp(&nested()));
        let name = |index: Option<usize>| {
            index
                .and_then(|i| panel.symbols.get(i))
                .map(|s| s.name.as_str())
        };
        // Line 12 is `rows`, which is inside `Panel`; the field is the answer.
        assert_eq!(name(panel.symbol_at_line(12)), Some("rows"));
        assert_eq!(name(panel.symbol_at_line(31)), Some("lay_out"));
        assert_eq!(
            panel.symbol_at_line(0),
            None,
            "nothing starts before the first symbol"
        );
    }

    #[test]
    fn a_section_key_names_it_in_configuration_both_ways() {
        for section in SECTIONS {
            assert_eq!(PanelSection::from_key(section.key()), Some(section));
        }
        assert_eq!(PanelSection::from_key("nonsense"), None);
    }

    #[test]
    fn the_sections_come_from_the_configuration() {
        let settings = maxgus_config::Settings {
            panel_symbols: false,
            ..maxgus_config::Settings::default()
        };
        let panel = Panel::from_settings(&settings);
        assert!(panel.is_enabled(PanelSection::Tree));
        assert!(!panel.is_enabled(PanelSection::Symbols));
        assert_eq!(panel.enabled_count(), 2);
    }
}
