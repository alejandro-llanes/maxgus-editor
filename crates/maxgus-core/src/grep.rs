//! The results of a project search, and what an edited results buffer means.
//!
//! Laid out the way the git views are: a list of rows, each of which knows
//! what it stands for, so point moving through the buffer is point moving
//! through the results. The buffer is what the user reads *and*, once it is
//! made editable, what they write into — a rename across a project is this
//! buffer with its lines edited and applied.

use crate::{CoreError, Result};
use maxgus_grep::{Hit, Replacement};
use std::path::{Path, PathBuf};

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

/// A file the edited results were written to, and the lines it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenFile {
    pub path: PathBuf,
    pub lines: Vec<Replacement>,
    /// When it was written, so a buffer showing it can be marked as holding
    /// what is on disk.
    pub disk_time: Option<std::time::SystemTime>,
}

/// Everything a search found, ready to be drawn and edited.
#[derive(Debug, Clone, Default)]
pub struct GrepView {
    pub pattern: String,
    /// Where the search was run, which the files are named relative to and
    /// which running it again searches.
    pub root: PathBuf,
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
    pub fn new(pattern: &str, root: &Path, found: maxgus_grep::Found) -> GrepView {
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
            root: root.to_path_buf(),
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
                format!(
                    "{} for `{}` in {}, of {} searched",
                    crate::count(self.hits(), "match"),
                    self.pattern,
                    crate::count(self.files.len(), "file"),
                    self.files_searched
                )
            }
            Row::Blank => String::new(),
            // Named from the root the search ran at: the whole path of
            // every file repeated the same long prefix above each of them,
            // and in a deep enough directory was cut off before the name.
            Row::File(index) => self
                .files
                .get(*index)
                .map(|f| {
                    f.path
                        .strip_prefix(&self.root)
                        .unwrap_or(&f.path)
                        .display()
                        .to_string()
                })
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
    ///
    /// The buffer is read line for line against the rows it was made from,
    /// so a line added or taken away moves every result under it onto the
    /// row of another — and the text of one line would be written over the
    /// next. That is refused, as is a result whose line number is no longer
    /// the one it had, rather than guessed at.
    pub fn replacements(&self, edited: &str) -> Result<Vec<Replacement>> {
        let lines: Vec<&str> = edited.lines().collect();
        if lines.len() != self.rows.len() {
            return Err(CoreError::Message(
                "Lines were added to the results or taken out of them: only the text of \
                 a result can be changed (C-c C-k puts them back)"
                    .into(),
            ));
        }
        let mut out = Vec::new();
        for (line, text) in lines.into_iter().enumerate() {
            let Some(Row::Hit(file, hit)) = self.rows.get(line) else {
                continue;
            };
            let Some(hit) = self.files.get(*file).and_then(|f| f.hits.get(*hit)) else {
                continue;
            };
            let number = (hit.line + 1).to_string();
            let now = match text.split_once(':') {
                Some((written, now)) if written.trim() == number => now,
                _ => {
                    return Err(CoreError::Message(format!(
                        "Line {} of the results no longer starts with {number}:, the line \
                         of the file it stands for (C-c C-k puts it back)",
                        line + 1
                    )));
                }
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
        Ok(out)
    }

    /// Takes in replacements that have been made, so the results say what
    /// the files now do: a second `C-c C-c` has nothing left to write, and a
    /// line written once is not refused as changed when it is edited again.
    pub fn applied(&mut self, replacements: &[Replacement]) {
        for replacement in replacements {
            let hit = self
                .files
                .iter_mut()
                .filter(|file| file.path == replacement.path)
                .flat_map(|file| file.hits.iter_mut())
                .find(|hit| hit.line == replacement.line);
            if let Some(hit) = hit {
                hit.text = replacement.now.clone();
                // Where the match was is not where it is in the new text.
                hit.length = 0;
            }
        }
    }

    /// Where on `line` of the buffer its match is, in characters from the
    /// start of the line. `None` for a row that is not a result, and for a
    /// result whose line has been rewritten.
    pub fn match_on(&self, line: usize) -> Option<std::ops::Range<usize>> {
        let hit = self.hit(self.rows.get(line)?)?;
        if hit.length == 0 {
            return None;
        }
        let start = format!("{:>6}:", hit.line + 1).chars().count() + hit.column;
        Some(start..start + hit.length)
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
            Path::new("/project"),
            maxgus_grep::Found {
                hits: vec![
                    hit("/project/src/a.rs", 0, "fn alpha() {}"),
                    hit("/project/src/a.rs", 4, "// alpha"),
                    hit("/project/src/b.rs", 2, "let alpha = 1;"),
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
        assert!(lines[0].starts_with("3 matches for `alpha` in 2 files"));
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
        assert!(view.replacements(&view.text()).unwrap().is_empty());
    }

    #[test]
    fn an_edited_line_becomes_a_replacement_for_the_file_it_came_from() {
        let view = view();
        let edited = view.text().replace("fn alpha() {}", "fn renamed() {}");
        let replacements = view.replacements(&edited).unwrap();
        assert_eq!(replacements.len(), 1);
        assert_eq!(replacements[0].path, PathBuf::from("/project/src/a.rs"));
        assert_eq!(replacements[0].line, 0, "the file's line, not the buffer's");
        assert_eq!(replacements[0].was, "fn alpha() {}");
        assert_eq!(replacements[0].now, "fn renamed() {}");
    }

    #[test]
    fn editing_a_heading_is_not_an_edit_to_a_file() {
        let view = view();
        let edited = view.text().replace("src/a.rs", "nonsense");
        assert!(
            view.replacements(&edited).unwrap().is_empty(),
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
        let replacements = view.replacements(&edited).unwrap();
        assert_eq!(replacements[0].now, "fn alpha() {} // note");
    }

    #[test]
    fn a_line_taken_out_of_the_results_is_refused_rather_than_misread() {
        // Without the check, the line under the deleted one was read as the
        // deleted one's new text, and written over it.
        let view = view();
        let edited: String = view
            .text()
            .lines()
            .filter(|line| *line != "     1:fn alpha() {}")
            .map(|line| format!("{line}\n"))
            .collect();
        assert!(view.replacements(&edited).is_err());
        let split = view.text().replace("fn alpha() {}", "fn alpha()\n{}");
        assert!(view.replacements(&split).is_err(), "a line was added");
    }

    #[test]
    fn a_result_whose_line_number_was_changed_is_refused() {
        let view = view();
        let renumbered = view
            .text()
            .replace("     1:fn alpha() {}", "     2:fn alpha() {}");
        assert!(view.replacements(&renumbered).is_err());
        let colonless = view.text().replace("     1:fn alpha", "     1 fn alpha");
        assert!(view.replacements(&colonless).is_err());
    }

    #[test]
    fn files_are_named_from_where_the_search_ran() {
        let text = view().text();
        assert!(!text.contains("/project/"), "{text}");
    }

    #[test]
    fn a_written_line_is_what_the_results_hold_afterwards() {
        let mut view = view();
        let edited = view.text().replace("fn alpha() {}", "fn renamed() {}");
        let replacements = view.replacements(&edited).unwrap();
        view.applied(&replacements);
        assert!(view.text().contains("     1:fn renamed() {}"));
        assert!(
            view.replacements(&edited).unwrap().is_empty(),
            "the same edit was offered a second time"
        );
        assert_eq!(
            view.match_on(3),
            None,
            "the old match is not in the new text"
        );
        // Past the seven characters of "     5:".
        assert_eq!(view.match_on(4), Some(7..8));
    }

    #[test]
    fn a_truncated_search_says_so_at_the_end() {
        let view = GrepView::new(
            "e",
            Path::new("/"),
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
        let view = GrepView::new("zzz", Path::new("/"), maxgus_grep::Found::default());
        assert!(view.is_empty());
        assert_eq!(view.first_hit_line(), 0);
        assert_eq!(view.step(0, true), None);
    }
}
