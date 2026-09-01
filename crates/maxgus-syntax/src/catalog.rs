//! The list of parsers that exist, as tree-sitter itself publishes it.
//!
//! <https://github.com/tree-sitter/tree-sitter/wiki/List-of-parsers> is a
//! wiki page, and a GitHub wiki is a git repository — `tree-sitter.wiki.git`,
//! holding `List-of-parsers.md`. So the list is fetched by cloning it rather
//! than by scraping HTML off a page whose markup is nobody's contract, and it
//! arrives as the markdown table this module reads:
//!
//! ```text
//! | name | url | last commit date | abi | grammar.json | external scanner |
//! | --- | --- | --- | --- | --- | --- |
//! | zig | [github.com/…/tree-sitter-zig](https://github.com/…) | 2025-01-01 | 14 | yes | yes |
//! ```
//!
//! Parsing is pure and total: a row that does not look like a row is skipped
//! rather than failing the load, because one malformed line on a wiki anybody
//! can edit must not cost the user the other five hundred.

/// One row: a parser, and where it lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parser {
    /// The grammar's own name, lower-cased, as `grammar.js` gives it —
    /// `c_sharp` rather than `c-sharp`.
    pub name: String,
    pub url: String,
    /// `YYYY-MM-DD`, which sorts correctly as text.
    pub last_commit: String,
    /// The ABI of the `src/parser.c` in the repository, when it has one.
    pub abi: Option<usize>,
    /// Whether `src/grammar.json` is committed. Without it — and without
    /// `src/parser.c` — the repository needs `tree-sitter generate` and a
    /// toolchain this editor does not ask you to have.
    pub grammar_json: bool,
    pub external_scanner: bool,
}

impl Parser {
    /// Whether this build of tree-sitter could read the parser as committed.
    ///
    /// An unknown ABI is not a no: the column is `-` when the repository
    /// ships no pre-generated `parser.c`, which is a different problem and
    /// one the install reports for itself.
    pub fn abi_is_readable(&self) -> bool {
        match self.abi {
            Some(abi) => (crate::MIN_ABI..=crate::MAX_ABI).contains(&abi),
            None => true,
        }
    }

    /// The row as one line, for the picker.
    pub fn label(&self) -> String {
        let host = self
            .url
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let abi = match self.abi {
            Some(abi) => format!("abi {abi}"),
            None => "no parser.c".to_string(),
        };
        format!("{}  —  {host}  ({abi}, {})", self.name, self.last_commit)
    }
}

/// The names on that list when this editor was built, one per line, already
/// lower-cased with `-` flattened to `_`.
///
/// Written by `scripts/update-parser-names.sh` from the same page the
/// catalog is read from. It exists to answer one question — is a parser for
/// this language worth asking about? — before anything has been fetched.
/// Opening a `.txt` file must not produce a question about installing a
/// `txt` grammar, and only the list knows that `zig` is a language and `txt`
/// is not.
///
/// Names, not repositories: where a grammar is cloned from is read from the
/// wiki when the user asks for it, so a repository that moves is followed
/// without rebuilding the editor. A language that gains its first parser
/// after this build is offered once the list has been fetched for any other
/// reason.
const KNOWN_PARSERS: &str = include_str!("parser-names.txt");

/// Whether anybody has written a parser for `language`, as far as this build
/// knows.
pub fn is_known(language: &str) -> bool {
    let wanted = normalise(language);
    KNOWN_PARSERS.lines().any(|name| name == wanted)
}

/// Every parser the wiki lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    entries: Vec<Parser>,
}

impl Catalog {
    /// Reads the wiki's markdown table. Anything that is not a table row of
    /// the right shape is ignored, prose and all.
    pub fn parse(markdown: &str) -> Catalog {
        let entries = markdown.lines().filter_map(row).collect();
        Catalog { entries }
    }

    pub fn entries(&self) -> &[Parser] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Every parser that could colour `language`, best first.
    ///
    /// A language name and a grammar name disagree about punctuation often
    /// enough to matter — a `.cs` file is `c-sharp` here and the grammar is
    /// `c_sharp` — so the two are compared with that difference flattened.
    pub fn for_language(&self, language: &str) -> Vec<&Parser> {
        let wanted = normalise(language);
        let mut found: Vec<&Parser> = self
            .entries
            .iter()
            .filter(|p| normalise(&p.name) == wanted)
            .collect();
        found.sort_by(|a, b| rank(b).cmp(&rank(a)));
        found
    }

    /// The one to offer for `language`, when there is an obvious choice.
    pub fn best_for(&self, language: &str) -> Option<&Parser> {
        self.for_language(language).into_iter().next()
    }

    /// Rows whose name contains `query`, for the picker's own filtering to
    /// narrow further. Empty query means everything.
    pub fn matching(&self, query: &str) -> Vec<&Parser> {
        let query = normalise(query);
        self.entries
            .iter()
            .filter(|p| query.is_empty() || normalise(&p.name).contains(&query))
            .collect()
    }

    /// The parser a picked line came from. The label is generated here, so
    /// matching on it is matching on this module's own output rather than on
    /// anything the user typed.
    pub fn by_label(&self, label: &str) -> Option<&Parser> {
        self.entries.iter().find(|p| p.label() == label)
    }
}

/// How good a candidate is, highest first. Readable ABI beats everything —
/// a parser this build cannot load is not a candidate at all — then a
/// repository under tree-sitter's own organisations, then recency.
fn rank(parser: &Parser) -> (bool, bool, &str) {
    let official = parser.url.contains("github.com/tree-sitter/")
        || parser.url.contains("github.com/tree-sitter-grammars/");
    (
        parser.abi_is_readable(),
        official,
        parser.last_commit.as_str(),
    )
}

/// `c-sharp`, `c_sharp` and `C_Sharp` are one language for our purposes.
fn normalise(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace('-', "_")
}

/// One `| a | b | … |` line, if it is a parser rather than the header or the
/// `| --- |` rule under it.
fn row(line: &str) -> Option<Parser> {
    let line = line.trim();
    if !line.starts_with('|') {
        return None;
    }
    let cells: Vec<&str> = line
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect::<Vec<_>>();
    if cells.len() < 6 {
        return None;
    }
    let name = cells[0].to_ascii_lowercase();
    // The header and the rule under it both land here, and neither is a
    // parser: one is called `name` and the other is all dashes.
    if name.is_empty() || name == "name" || name.chars().all(|c| c == '-' || c == ':') {
        return None;
    }
    let url = link(cells[1])?;
    Some(Parser {
        name,
        url,
        last_commit: cells[2].to_string(),
        abi: cells[3].parse().ok(),
        grammar_json: yes(cells[4]),
        external_scanner: yes(cells[5]),
    })
}

/// The target of `[text](url)`, or a bare URL, made absolute.
///
/// The wiki writes the text without a scheme and the target with one, but
/// both spellings have appeared, and a row is worth keeping either way.
fn link(cell: &str) -> Option<String> {
    let url = match (cell.find("]("), cell.strip_prefix('[')) {
        (Some(at), Some(_)) => cell[at + 2..].trim_end_matches(')'),
        _ => cell,
    }
    .trim();
    if url.is_empty() || url.contains(char::is_whitespace) {
        return None;
    }
    match url.starts_with("http://") || url.starts_with("https://") {
        true => Some(url.to_string()),
        // A row that names a host and nothing else is still usable; git
        // needs a scheme, and https is the only one worth guessing.
        false if url.contains('.') && url.contains('/') => Some(format!("https://{url}")),
        false => None,
    }
}

fn yes(cell: &str) -> bool {
    cell.eq_ignore_ascii_case("yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
Below is a table of collected tree-sitter parser information.

| name | url | last commit date | abi | grammar.json | external scanner |
| --- | --- | --- | --- | --- | --- |
| ada | [github.com/TamaMcGlinn/tree-sitter-ada](https://github.com/TamaMcGlinn/tree-sitter-ada) | 2022-06-25 | 13 | yes | no |
| ada | [github.com/briot/tree-sitter-ada](https://github.com/briot/tree-sitter-ada) | 2024-05-23 | 14 | yes | no |
| c_sharp | [github.com/tree-sitter/tree-sitter-c-sharp](https://github.com/tree-sitter/tree-sitter-c-sharp) | 2025-02-14 | 14 | yes | yes |
| kdl | [github.com/spaarmann/tree-sitter-kdl](https://github.com/spaarmann/tree-sitter-kdl) | 2024-02-03 | 14 | yes | yes |
| kdl | [github.com/tree-sitter-grammars/tree-sitter-kdl](https://github.com/tree-sitter-grammars/tree-sitter-kdl) | 2024-06-08 | 14 | yes | yes |
| bison | [gitlab.com/btuin2/tree-sitter-bison](https://gitlab.com/btuin2/tree-sitter-bison) | 2024-01-13 | 14 | yes | yes |
| ancient | [github.com/someone/tree-sitter-ancient](https://github.com/someone/tree-sitter-ancient) | 2019-01-01 | 9 | yes | no |
| nogen | [github.com/someone/tree-sitter-nogen](https://github.com/someone/tree-sitter-nogen) | 2025-01-01 | - | yes | no |
";

    #[test]
    fn the_wikis_table_becomes_rows_and_the_prose_around_it_does_not() {
        let catalog = Catalog::parse(TABLE);
        assert_eq!(catalog.len(), 8, "{:?}", catalog.entries());
        let ada = &catalog.entries()[0];
        assert_eq!(ada.name, "ada");
        assert_eq!(ada.url, "https://github.com/TamaMcGlinn/tree-sitter-ada");
        assert_eq!(ada.last_commit, "2022-06-25");
        assert_eq!(ada.abi, Some(13));
        assert!(ada.grammar_json);
        assert!(!ada.external_scanner);
    }

    #[test]
    fn a_repository_that_ships_no_parser_c_has_no_abi_rather_than_a_wrong_one() {
        let catalog = Catalog::parse(TABLE);
        let nogen = catalog.best_for("nogen").expect("listed");
        assert_eq!(nogen.abi, None);
        assert!(
            nogen.abi_is_readable(),
            "unknown is not the same as too old"
        );
    }

    #[test]
    fn punctuation_does_not_hide_a_grammar_from_the_language_it_colours() {
        let catalog = Catalog::parse(TABLE);
        // maxgus calls a `.cs` file `c-sharp`; the grammar calls itself
        // `c_sharp`. They are the same thing.
        let found = catalog.best_for("c-sharp").expect("c_sharp is listed");
        assert_eq!(found.name, "c_sharp");
        assert_eq!(
            catalog.best_for("C_SHARP").map(|p| p.name.as_str()),
            Some("c_sharp")
        );
    }

    #[test]
    fn the_offer_prefers_tree_sitters_own_repository_then_the_newer_one() {
        let catalog = Catalog::parse(TABLE);
        assert_eq!(
            catalog.best_for("kdl").map(|p| p.url.as_str()),
            Some("https://github.com/tree-sitter-grammars/tree-sitter-kdl"),
            "the organisation's copy wins over a personal one"
        );
        assert_eq!(
            catalog.best_for("ada").map(|p| p.last_commit.as_str()),
            Some("2024-05-23"),
            "neither is official, so the newer one wins"
        );
        assert_eq!(catalog.for_language("kdl").len(), 2, "both are offered");
    }

    #[test]
    fn a_parser_this_build_could_not_load_sinks_below_one_it_could() {
        let catalog = Catalog::parse(TABLE);
        let ancient = catalog.best_for("ancient").expect("listed");
        assert!(!ancient.abi_is_readable(), "ABI 9 is long gone");
    }

    #[test]
    fn a_host_that_is_not_github_is_kept_as_it_was_written() {
        let catalog = Catalog::parse(TABLE);
        assert_eq!(
            catalog.best_for("bison").map(|p| p.url.as_str()),
            Some("https://gitlab.com/btuin2/tree-sitter-bison")
        );
    }

    #[test]
    fn searching_narrows_by_name_and_an_empty_query_keeps_everything() {
        let catalog = Catalog::parse(TABLE);
        assert_eq!(catalog.matching("").len(), catalog.len());
        assert_eq!(catalog.matching("kd").len(), 2);
        assert!(catalog.matching("nothing-like-this").is_empty());
    }

    #[test]
    fn a_picked_line_finds_its_way_back_to_the_row_it_came_from() {
        let catalog = Catalog::parse(TABLE);
        let label = catalog.best_for("kdl").expect("listed").label();
        assert!(label.contains("kdl"), "{label}");
        assert!(label.contains("tree-sitter-grammars"), "{label}");
        assert_eq!(
            catalog.by_label(&label).map(|p| p.url.as_str()),
            Some("https://github.com/tree-sitter-grammars/tree-sitter-kdl")
        );
    }

    #[test]
    fn the_names_that_ship_say_what_is_a_language_and_what_is_a_file_extension() {
        // The whole point of shipping them: `.zig` is worth asking about
        // and `.txt` is not, and that has to be known before anything is
        // fetched.
        for language in ["zig", "kdl", "lua", "nix", "csv", "c_sharp"] {
            assert!(is_known(language), "`{language}` has a parser");
        }
        for not in ["txt", "log", "bak", "tmp", "orig"] {
            assert!(!is_known(not), "`{not}` is not a language");
        }
        // The editor spells some of them with a hyphen; the wiki does not.
        assert!(is_known("c-sharp"), "punctuation is not a difference");
        assert!(is_known("ZIG"), "nor is case");
    }

    #[test]
    fn the_shipped_names_are_a_clean_list() {
        let names: Vec<&str> = KNOWN_PARSERS.lines().collect();
        assert!(names.len() > 300, "only {} names", names.len());
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted, names,
            "parser-names.txt must be sorted and unique; run scripts/update-parser-names.sh"
        );
        assert!(
            names
                .iter()
                .all(|n| !n.contains('-') && *n == n.to_ascii_lowercase()),
            "names are stored normalised, so matching allocates nothing"
        );
    }

    #[test]
    fn nothing_that_is_not_a_row_becomes_one() {
        assert!(Catalog::parse("").is_empty());
        assert!(Catalog::parse("| name | url | a | b | c | d |").is_empty());
        assert!(Catalog::parse("| --- | --- | --- | --- | --- | --- |").is_empty());
        assert!(Catalog::parse("| too | few | cells |").is_empty());
        assert!(
            Catalog::parse("| x | not a link | 2025-01-01 | 14 | yes | no |").is_empty(),
            "a row with nothing clonable in it is not an install waiting to fail"
        );
    }
}
