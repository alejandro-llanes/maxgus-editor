//! The results of a project search, and what an edited results buffer means.
//!
//! Laid out the way the git views are: a list of rows, each of which knows
//! what it stands for, so point moving through the buffer is point moving
//! through the results. The buffer is what the user reads *and*, once it is
//! made editable, what they write into — a rename across a project is this
//! buffer with its lines edited and applied.

use maxgus_grep::{Hit, Replacement};
use std::path::PathBuf;

/// One line of the results buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// The pattern, and how much was found.
    Title,
    Blank,
    /// A file, and how many of its lines matched.
    File(usize),
    /// A matching line: which file, and which hit of that file's.
    Hit(usize, usize),
    /// Said when the search was cut short.
    Truncated,
}

/// A file's hits, gathered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHits {
    pub path: PathBuf,
    pub hits: Vec<Hit>,
}

/// Everything a search found, ready to be drawn and edited.
#[derive(Debug, Clone, Default)]
pub struct GrepView {
    pub pattern: String,
    pub files: Vec<FileHits>,
    pub files_searched: usize,
    pub truncated: bool,
    /// True once the buffer has been made editable, which is what turns a
    /// listing into a rename.
    pub editable: bool,
    rows: Vec<Row>,
}

impl GrepView {
    /// Gathers hits by file, keeping the order the search found them in.
    pub fn new(pattern: &str, found: maxgus_grep::Found) -> GrepView {
        let mut files: Vec<FileHits> = Vec::new();
        for hit in found.hits {
            match files.last_mut() {
                Some(file) if file.path == hit.path => file.hits.push(hit),
                _ => files.push(FileHits {
                    path: hit.path.clone(),
                    hits: vec![hit],
                }),
            }
        }
        let mut view = GrepView {
            pattern: pattern.to_string(),
            files,
            files_searched: found.files_searched,
            truncated: found.truncated,
            editable: false,
            rows: Vec::new(),
        };
        view.lay_out();
        view
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn row(&self, line: usize) -> Option<&Row> {
        self.rows.get(line)
    }

    pub fn hits(&self) -> usize {
        self.files.iter().map(|f| f.hits.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The hit a row stands for.
    pub fn hit(&self, row: &Row) -> Option<&Hit> {
        match row {
            Row::Hit(file, hit) => self.files.get(*file)?.hits.get(*hit),
            _ => None,
        }
    }

    /// The line of the first hit, which is where point starts: a results
    /// buffer opens on a result rather than on its own title.
    pub fn first_hit_line(&self) -> usize {
        self.rows
            .iter()
            .position(|row| matches!(row, Row::Hit(_, _)))
            .unwrap_or(0)
    }

    /// The next or previous hit's line, wrapping at neither end: a search
    /// that has run out has run out, and saying so is more use than a cycle.
    pub fn step(&self, from: usize, forward: bool) -> Option<usize> {
        let candidates: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, Row::Hit(_, _)))
            .map(|(line, _)| line)
            .collect();
        match forward {
            true => candidates.into_iter().find(|line| *line > from),
            false => candidates.into_iter().rfind(|line| *line < from),
        }
    }

    fn lay_out(&mut self) {
        let mut rows = vec![Row::Title, Row::Blank];
        for (file, hits) in self.files.iter().enumerate() {
            rows.push(Row::File(file));
            for hit in 0..hits.hits.len() {
                rows.push(Row::Hit(file, hit));
            }
            rows.push(Row::Blank);
        }
        if self.truncated {
            rows.push(Row::Truncated);
        }
        self.rows = rows;
    }

    /// The text of one row, which is what point moves through.
    ///
    /// A hit's line is written with its number in front, and the number is
    /// what an edited buffer has to be read back around: everything after the
    /// colon is the line's own text.
    pub fn row_text(&self, row: &Row) -> String {
        match row {
            Row::Title => {
                let files = self.files.len();
                format!(
                    "{} matches for `{}` in {files} file(s), of {} searched",
                    self.hits(),
                    self.pattern,
                    self.files_searched
                )
            }
            Row::Blank => String::new(),
            Row::File(index) => self
                .files
                .get(*index)
                .map(|f| f.path.display().to_string())
                .unwrap_or_default(),
            Row::Hit(file, hit) => match self.files.get(*file).and_then(|f| f.hits.get(*hit)) {
                Some(hit) => format!("{:>6}:{}", hit.line + 1, hit.text),
                None => String::new(),
            },
            Row::Truncated => "… stopped early: there were more matches than the limit".to_string(),
        }
    }

    /// The whole buffer.
    pub fn text(&self) -> String {
        self.rows
            .iter()
            .map(|row| format!("{}\n", self.row_text(row)))
            .collect()
    }

    /// Reads an edited buffer back into the replacements it describes.
    ///
    /// Only hit rows are read, and only the text after the line number: a
    /// buffer whose headings have been mangled still produces exactly the
    /// edits its result lines describe.
    pub fn replacements(&self, edited: &str) -> Vec<Replacement> {
        let mut out = Vec::new();
        for (line, text) in edited.lines().enumerate() {
            let Some(Row::Hit(file, hit)) = self.rows.get(line) else {
                continue;
            };
            let Some(hit) = self.files.get(*file).and_then(|f| f.hits.get(*hit)) else {
                continue;
            };
            let Some(now) = text.split_once(':').map(|(_, rest)| rest) else {
                continue;
            };
            if now == hit.text {
                continue;
            }
            out.push(Replacement {
                path: hit.path.clone(),
                line: hit.line,
                was: hit.text.clone(),
                now: now.to_string(),
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(path: &str, line: usize, text: &str) -> Hit {
        Hit {
            path: PathBuf::from(path),
            line,
            column: 0,
            length: 1,
            text: text.to_string(),
        }
    }

    fn view() -> GrepView {
        GrepView::new(
            "alpha",
            maxgus_grep::Found {
                hits: vec![
                    hit("src/a.rs", 0, "fn alpha() {}"),
                    hit("src/a.rs", 4, "// alpha"),
                    hit("src/b.rs", 2, "let alpha = 1;"),
                ],
                files_searched: 9,
                truncated: false,
            },
        )
    }

    #[test]
    fn hits_are_gathered_under_the_file_they_are_in() {
        let view = view();
        assert_eq!(view.files.len(), 2);
        assert_eq!(view.files[0].hits.len(), 2);
        assert_eq!(view.hits(), 3);
    }

    #[test]
    fn the_buffer_reads_as_a_list_of_files_and_their_lines() {
        let text = view().text();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("3 matches for `alpha` in 2 file(s)"));
        assert_eq!(lines[2], "src/a.rs");
        assert_eq!(lines[3], "     1:fn alpha() {}");
        assert_eq!(lines[4], "     5:// alpha");
        assert_eq!(lines[6], "src/b.rs");
        assert_eq!(lines[7], "     3:let alpha = 1;");
    }

    #[test]
    fn point_starts_on_a_result_rather_than_on_the_title() {
        let view = view();
        assert_eq!(view.first_hit_line(), 3);
        assert!(matches!(view.row(3), Some(Row::Hit(0, 0))));
    }

    #[test]
    fn stepping_walks_the_results_and_stops_at_the_ends() {
        let view = view();
        assert_eq!(view.step(3, true), Some(4));
        assert_eq!(view.step(4, true), Some(7));
        assert_eq!(view.step(7, true), None, "it wrapped instead of stopping");
        assert_eq!(view.step(7, false), Some(4));
        assert_eq!(view.step(3, false), None);
    }

    #[test]
    fn an_untouched_buffer_describes_no_edits() {
        let view = view();
        assert!(view.replacements(&view.text()).is_empty());
    }

    #[test]
    fn an_edited_line_becomes_a_replacement_for_the_file_it_came_from() {
        let view = view();
        let edited = view.text().replace("fn alpha() {}", "fn renamed() {}");
        let replacements = view.replacements(&edited);
        assert_eq!(replacements.len(), 1);
        assert_eq!(replacements[0].path, PathBuf::from("src/a.rs"));
        assert_eq!(replacements[0].line, 0, "the file's line, not the buffer's");
        assert_eq!(replacements[0].was, "fn alpha() {}");
        assert_eq!(replacements[0].now, "fn renamed() {}");
    }

    #[test]
    fn editing_a_heading_is_not_an_edit_to_a_file() {
        let view = view();
        let edited = view.text().replace("src/a.rs", "nonsense");
        assert!(
            view.replacements(&edited).is_empty(),
            "a mangled heading produced an edit"
        );
    }

    #[test]
    fn the_line_number_is_not_part_of_what_gets_written() {
        // Everything after the first colon is the line; the number in front
        // of it is the buffer's own furniture.
        let view = view();
        let edited = view
            .text()
            .replace("     1:fn alpha() {}", "     1:fn alpha() {} // note");
        let replacements = view.replacements(&edited);
        assert_eq!(replacements[0].now, "fn alpha() {} // note");
    }

    #[test]
    fn a_truncated_search_says_so_at_the_end() {
        let view = GrepView::new(
            "e",
            maxgus_grep::Found {
                hits: vec![hit("a", 0, "e")],
                files_searched: 1,
                truncated: true,
            },
        );
        assert!(view.text().contains("stopped early"));
    }

    #[test]
    fn a_search_that_found_nothing_is_a_view_with_no_files() {
        let view = GrepView::new("zzz", maxgus_grep::Found::default());
        assert!(view.is_empty());
        assert_eq!(view.first_hit_line(), 0);
        assert_eq!(view.step(0, true), None);
    }
}
